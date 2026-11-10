use std::{
    thread,
    time::{Duration, Instant},
};

use reqwest::{
    blocking::{Client, Response},
    cookie::{CookieStore, Jar},
    header::{HeaderMap, HeaderValue, AUTHORIZATION},
};
use std::sync::Arc;
use serde_json::{Value, json};

use super::DriverOutcome;
use crate::{AgentCredential, AgentJobStatus};

fn client_builder(olam: &Value) -> reqwest::blocking::ClientBuilder {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .danger_accept_invalid_certs(
            !olam.get("verify_tls_certificate").and_then(Value::as_bool).unwrap_or(true),
        )
}

// Match the Ansible MCP: login with the KeePass account, then obtain an OAuth
// token from that session. Cookies are used only for this handshake; all API
// operations share a client with a sensitive Bearer header and no session cookie.
fn authenticated_client(base_url: &str, olam: &Value, account: &AgentCredential) -> Result<Client, String> {
    let base_url = base_url.trim_end_matches('/');
    let login_url = format!("{base_url}/api/login/");
    let token_url = format!("{base_url}/api/v2/tokens/");
    let cookies = Arc::new(Jar::default());
    let session = client_builder(olam).cookie_provider(cookies.clone()).build()
        .map_err(|_| "Could not initialize OLAM login client.".to_string())?;
    let page = session.get(&login_url).send()
        .map_err(|_| "Could not retrieve OLAM login page.".to_string())?;
    authentication_status(&page, "retrieve login page", false)?;
    let login_uri = reqwest::Url::parse(&login_url)
        .map_err(|_| "OLAM URL is invalid.".to_string())?;
    let csrf = csrf_cookie(&cookies, &login_uri).or_else(|| {
        page.text().ok().and_then(|html| csrf_input(&html))
    }).ok_or_else(|| "OLAM login page did not supply a CSRF token.".to_string())?;
    let response = session.post(&login_url)
        .header("Referer", &login_url)
        .header("X-CSRFToken", &csrf)
        .form(&[("username", account.username.as_str()), ("password", account.password.as_str()), ("next", "/api/v2/")])
        .send().map_err(|_| "Could not submit OLAM login.".to_string())?;
    authentication_status(&response, "log in", true)?;
    // Django rotates the CSRF cookie at login.
    let csrf = csrf_cookie(&cookies, &login_uri).unwrap_or(csrf);
    let response = session.post(&token_url)
        .header("Referer", &login_url)
        .header("X-CSRFToken", csrf)
        .json(&json!({"description":"GateTree-Ops Agent", "application":null, "scope":"write"}))
        .send().map_err(|_| "Could not request OLAM API token.".to_string())?;
    authentication_status(&response, "request API token", false)?;
    let value: Value = response.json()
        .map_err(|_| "OLAM token response is invalid JSON.".to_string())?;
    let token = value.get("token").and_then(Value::as_str).filter(|token| !token.trim().is_empty())
        .ok_or_else(|| "OLAM token response did not include an API token.".to_string())?;
    let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| "OLAM returned an invalid API token.".to_string())?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization);
    client_builder(olam).default_headers(headers).build()
        .map_err(|_| "Could not initialize OLAM API client.".to_string())
}

fn authentication_status(response: &Response, operation: &str, allow_login_redirect: bool) -> Result<(), String> {
    let status = response.status();
    if status.is_success() || (allow_login_redirect && matches!(status.as_u16(), 302 | 303)) {
        return Ok(());
    }
    // Authentication responses can echo passwords, cookies or token values.
    let node = api_node(response).map(|node| format!(" [OLAM API node: {node}]")).unwrap_or_default();
    Err(format!("Could not {operation} for OLAM authentication: HTTP {}{node}.", status.as_u16()))
}

fn csrf_cookie(cookies: &Jar, url: &reqwest::Url) -> Option<String> {
    let header = cookies.cookies(url)?;
    header.to_str().ok()?.split(';').find_map(|cookie| {
        cookie.trim().strip_prefix("csrftoken=").filter(|value| !value.is_empty()).map(str::to_owned)
    })
}

fn csrf_input(html: &str) -> Option<String> {
    // The normal Django response supplies a cookie; retain the MCP's hidden
    // input fallback for login pages which supply the token only in HTML.
    html.split('<').filter(|tag| tag.starts_with("input ")).find_map(|tag| {
        let tag = tag.split('>').next()?;
        if !tag.contains("name=\"csrfmiddlewaretoken\"") && !tag.contains("name='csrfmiddlewaretoken'") {
            return None;
        }
        let value = tag.split_once("value=")?.1;
        let quote = value.chars().next()?;
        if quote != '\'' && quote != '"' { return None; }
        let value = value[1..].split(quote).next()?;
        (!value.is_empty()).then(|| value.to_owned())
    })
}

pub fn validate_endpoint(
    process_definition: &str,
    credentials: &[AgentCredential],
) -> Result<String, String> {
    let process: Value = serde_json::from_str(process_definition)
        .map_err(|error| format!("OLAM Process Config is invalid: {error}"))?;
    let olam = process
        .pointer("/details/olam")
        .ok_or_else(|| "Process Config details.olam is required.".to_string())?;
    let base_url = required_string(olam, "url")?;
    let account = credential(credentials, required_string(olam, "account")?, "OLAM API")?;
    let client = authenticated_client(base_url, olam, account)?;
    checked(
        client
            .get(format!("{}/api/v2/me/", base_url.trim_end_matches('/')))
            .send()
            .map_err(|error| format!("Could not validate OLAM endpoint: {error}"))?,
        "validate OLAM endpoint and API authentication",
    )?
    .json::<Value>()
    .map_err(|error| format!("OLAM endpoint returned invalid JSON: {error}"))?;
    let mut log =
        format!("OLAM endpoint validation passed: {base_url}. API authentication successful.");
    for phase in ["validation", "deployment"] {
        let id_key = format!("{phase}_template_id");
        let name_key = format!("{phase}_template_job");
        if olam.get(&id_key).is_some() || olam.get(&name_key).is_some() {
            let id = required_id(olam, &id_key)?;
            let name = required_string(olam, &name_key)?;
            verify_template(&client, base_url, id, name)?;
            log.push_str(&format!("\nVerified {phase} template '{name}' (ID {id})."));
        }
    }
    log.push_str("\nNo Ansible job was launched.");
    Ok(log)
}

pub fn execute(
    process_definition: &str,
    plan_definition: &str,
    job: &AgentJobStatus,
    credentials: &[AgentCredential],
    on_started: &dyn Fn(&str, i64),
    on_output: &dyn Fn(&str),
) -> Result<DriverOutcome, String> {
    let process: Value = serde_json::from_str(process_definition)
        .map_err(|error| format!("OLAM Process Config is invalid: {error}"))?;
    let olam = process
        .pointer("/details/olam")
        .ok_or_else(|| "Process Config details.olam is required.".to_string())?;
    let base_url = required_string(olam, "url")?;
    let api_account = required_string(olam, "account")?;
    let environment = olam
        .get("payload_defaults")
        .ok_or_else(|| "Process Config details.olam.payload_defaults is required.".to_string())
        .and_then(|defaults| required_string(defaults, "ENVIRONMENT"))?;
    let phase = if job.action == "deploy"
        || (job.action == "validate_deploy" && job.phase.as_deref() == Some("deployment"))
    {
        "deployment"
    } else {
        "validation"
    };
    let template_key = if phase == "deployment" {
        "deployment_template_job"
    } else {
        "validation_template_job"
    };
    let template_id_key = if phase == "deployment" {
        "deployment_template_id"
    } else {
        "validation_template_id"
    };
    let template_name = required_string(olam, template_key)?;
    let template_id = required_id(olam, template_id_key)?;
    let api_credential = credential(credentials, api_account, "OLAM API")?;
    let client = authenticated_client(base_url, olam, api_credential)?;

    let existing_job_id = if phase == "deployment" {
        job.deployment_job_id
    } else {
        job.validation_job_id
    };
    let job_id = if let Some(id) = existing_job_id.filter(|id| *id > 0) {
        // GET/poll failures must never fall back to launching a duplicate.
        id
    } else {
        let deployment_account = deployment_account(plan_definition)?;
        let deployment_credential = credential(credentials, &deployment_account, "deployment")?;
        let template_node = verify_template(
            &client,
            base_url,
            template_id,
            template_name,
        )?;
        if let Some(node) = template_node {
            on_output(&format!("OLAM template '{template_name}' verified by API node {node}."));
        }
        let payload = resolved_payload(job, deployment_credential, environment)?;
        let launch_url = format!(
            "{}/api/v2/job_templates/{template_id}/launch/",
            base_url.trim_end_matches('/')
        );
        let launch: Value = checked(
            client
                .post(&launch_url)
                .json(&json!({ "extra_vars": payload }))
                .send()
                .map_err(|error| {
                    format!("Could not launch OLAM template '{template_name}': {error}")
                })?,
            &format!("launch OLAM template '{template_name}'"),
        )?
        .json()
        .map_err(|error| format!("OLAM launch response is invalid: {error}"))?;
        launch
            .get("id")
            .and_then(Value::as_i64)
            .filter(|id| *id > 0)
            .ok_or_else(|| "OLAM did not return an Ansible job ID.".to_string())?
    };
    on_started(phase, job_id);
    let output = wait_for_job(
        &client,
        base_url,
        job_id,
        phase == "deployment",
        on_output,
    )?;
    let log = format!(
        "OLAM {phase} template '{template_name}' (ID {template_id}) completed successfully as Ansible job {job_id}.\n{output}"
    );
    if output_indicates_no_hosts(&output) {
        return Ok(DriverOutcome::Warning(format!(
            "{log}\nWARNING: OLAM reported that no hosts matched. The Queue has stopped and no following job will start."
        )));
    }
    if job.action == "validate_deploy" && phase == "validation" {
        Ok(DriverOutcome::AwaitingApproval(format!(
            "{log}\nValidation passed. Review and approve this job to deploy to the same target."
        )))
    } else {
        Ok(DriverOutcome::Completed(log))
    }
}

fn output_indicates_no_hosts(output: &str) -> bool {
    let output = output.to_ascii_lowercase();
    output.contains("skipping: no hosts matched") || output.contains("no hosts matched")
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Process Config details.olam.{key} is required."))
}

fn required_id(value: &Value, key: &str) -> Result<i64, String> {
    let id = value.get(key).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
    });
    id.filter(|id| *id > 0)
        .ok_or_else(|| format!("Process Config details.olam.{key} must be a positive number."))
}

fn credential<'a>(
    credentials: &'a [AgentCredential],
    reference: &str,
    purpose: &str,
) -> Result<&'a AgentCredential, String> {
    credentials
        .iter()
        .find(|item| item.reference.eq_ignore_ascii_case(reference))
        .ok_or_else(|| {
            format!("{purpose} credential '{reference}' is unavailable in Agent memory.")
        })
}

fn deployment_account(plan_definition: &str) -> Result<String, String> {
    let plan: serde_yaml::Value = serde_yaml::from_str(plan_definition)
        .map_err(|error| format!("Deployment Plan YAML is invalid: {error}"))?;
    plan.get("inputs")
        .and_then(|inputs| inputs.get("olam"))
        .and_then(|olam| olam.get("account"))
        .and_then(serde_yaml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "Deployment Plan inputs.olam.account is required.".into())
}

fn resolved_payload(
    job: &AgentJobStatus,
    credential: &AgentCredential,
    environment: &str,
) -> Result<String, String> {
    let mut payload: Value = serde_json::from_str(job.payload.as_deref().unwrap_or("{}"))
        .map_err(|error| format!("OLAM job payload is invalid: {error}"))?;
    let object = payload
        .as_object_mut()
        .ok_or_else(|| "OLAM job payload must be a JSON object.".to_string())?;
    object.insert("ansible_user".into(), json!(credential.username));
    object.insert("ansible_password".into(), json!(credential.password));
    object.insert("ansible_become_pass".into(), json!(credential.password));
    object.insert("ENVIRONMENT".into(), json!(environment));
    serde_json::to_string(&payload).map_err(|error| error.to_string())
}

fn verify_template(
    client: &Client,
    base_url: &str,
    id: i64,
    name: &str,
) -> Result<Option<String>, String> {
    let url = format!(
        "{}/api/v2/job_templates/{id}/",
        base_url.trim_end_matches('/')
    );
    let response = checked(
        client
            .get(&url)
            .send()
            .map_err(|error| format!("Could not verify OLAM template ID {id}: {error}"))?,
        &format!("verify OLAM template ID {id}"),
    )?;
    let node = api_node(&response);
    let value: Value = response
        .json()
        .map_err(|error| format!("OLAM template response is invalid: {error}"))?;
    let actual_name = value.get("name").and_then(Value::as_str).unwrap_or("");
    if !actual_name.eq_ignore_ascii_case(name) {
        return Err(format!(
            "OLAM template ID {id} is '{actual_name}', but Process Config requires '{name}'. No job was started."
        ));
    }
    Ok(node)
}

fn wait_for_job(
    client: &Client,
    base_url: &str,
    job_id: i64,
    deployment: bool,
    on_output: &dyn Fn(&str),
) -> Result<String, String> {
    poll_job(
        client,
        base_url,
        job_id,
        deployment,
        on_output,
        Duration::from_secs(2),
        Duration::from_secs(10),
    )
}

fn poll_job(
    client: &Client,
    base_url: &str,
    job_id: i64,
    deployment: bool,
    on_output: &dyn Fn(&str),
    status_interval: Duration,
    output_interval: Duration,
) -> Result<String, String> {
    let attempts = if deployment { 21_600 } else { 300 };
    let mut last_output = String::new();
    let mut last_read = Instant::now();
    let mut status_auth_failures = 0;
    let url = format!("{}/api/v2/jobs/{job_id}/", base_url.trim_end_matches('/'));
    for _ in 0..attempts {
        let response = client
            .get(&url)
            .send();
        let unauthorized = response.as_ref().is_ok_and(|response| response.status() == reqwest::StatusCode::UNAUTHORIZED);
        let status_result = response
            .map_err(|error| format!("Could not poll OLAM job {job_id}: {error}"))
            .and_then(|response| checked(response, &format!("poll OLAM job {job_id}")))
            .and_then(|response| response.json::<Value>().map_err(|error| {
                format!("OLAM job {job_id} response is invalid: {error}")
            }));
        let value = match status_result {
            Ok(value) => { status_auth_failures = 0; value },
            Err(error) => {
                if unauthorized {
                    status_auth_failures += 1;
                    if status_auth_failures < 3 {
                        on_output(&format!("OLAM job {job_id} status request was rejected (HTTP 401). Retrying status lookup {status_auth_failures}/2; no new job is being launched.\n{error}\n{last_output}"));
                        thread::sleep(status_interval);
                        continue;
                    }
                }
                // Status and stdout are separate endpoints. Recover the actual
                // Ansible output even if the status request was rejected.
                let output = match job_output(client, base_url, job_id) {
                    Ok(output) => format!("Ansible output:\n{output}"),
                    Err(output_error) => format!("Output retrieval also failed: {output_error}\nLast retrieved Ansible output:\n{last_output}"),
                };
                return Err(format!(
                    "OLAM job {job_id} was started, but GT could not retrieve its status. The remote job result is unknown.\nMonitoring error: {error}\nOpen OLAM output: {}/#/jobs/playbook/{job_id}/output\n{output}",
                    base_url.trim_end_matches('/')
                ));
            }
        };
        match value.get("status").and_then(Value::as_str).unwrap_or("") {
            "successful" => {
                return job_output(client, base_url, job_id)
                    .map_err(|error| format!("{error}\n{last_output}"));
            }
            "failed" | "error" | "canceled" => {
                let output =
                    job_output(client, base_url, job_id).unwrap_or(last_output);
                return Err(format!("OLAM job {job_id} failed.\n{output}"));
            }
            _ => {
                if last_read.elapsed() >= output_interval {
                    // A temporary stdout failure must not stop the external deployment.
                    if let Ok(output) = job_output(client, base_url, job_id) {
                        if output != last_output {
                            on_output(&output);
                            last_output = output;
                        }
                    }
                    last_read = Instant::now();
                }
                thread::sleep(status_interval);
            }
        }
    }
    Err(format!(
        "OLAM job {job_id} did not finish before the timeout.\n{last_output}"
    ))
}

fn job_output(
    client: &Client,
    base_url: &str,
    job_id: i64,
) -> Result<String, String> {
    let url = format!(
        "{}/api/v2/jobs/{job_id}/stdout/?format=txt",
        base_url.trim_end_matches('/')
    );
    let response = checked(
        client
            .get(&url)
            .header("Accept", "text/plain, */*")
            .send()
            .map_err(|error| format!("Could not read OLAM job {job_id} output: {error}"))?,
        &format!("read OLAM job {job_id} output"),
    )?;
    let text = response.text().map_err(|error| error.to_string())?;
    Ok(text.chars().take(512_000).collect())
}

fn api_node(response: &Response) -> Option<String> {
    response.headers().get("X-API-Node")?.to_str().ok()
        .filter(|node| node.len() <= 255 && node.bytes().all(|byte| byte.is_ascii_alphanumeric() || b".-_:".contains(&byte)))
        .map(str::to_owned)
}

fn checked(response: Response, operation: &str) -> Result<Response, String> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let node = api_node(&response).map(|node| format!(" [OLAM API node: {node}]")).unwrap_or_default();
    let body = response.text().unwrap_or_default();
    Err(format!(
        "Could not {operation}: HTTP {}{node}: {}",
        status.as_u16(),
        body.chars().take(1000).collect::<String>()
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
    };

    fn test_api_client() -> Client {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer test-token"));
        Client::builder().default_headers(headers).build().unwrap()
    }

    pub(crate) fn serve_authentication(listener: &TcpListener) {
        for step in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            let path = if step == 2 { "/api/v2/tokens/" } else { "/api/login/" };
            let method = if step == 0 { "GET" } else { "POST" };
            assert!(request.starts_with(&format!("{method} {path} HTTP/")), "{request}");
            let mut headers = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" { break; }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
                headers.push_str(&line.to_ascii_lowercase());
            }
            assert!(!headers.contains("authorization:"));
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let body = String::from_utf8(body).unwrap();
            let (status, extra, response) = match step {
                0 => ("200 OK", "Set-Cookie: csrftoken=before-login; Path=/\r\n", "login"),
                1 => {
                    assert!(headers.contains("x-csrftoken: before-login"));
                    assert!(headers.contains("cookie: csrftoken=before-login"));
                    assert!(headers.contains("referer:"));
                    assert!(headers.contains("application/x-www-form-urlencoded"));
                    assert_eq!(body, "username=user&password=test-only&next=%2Fapi%2Fv2%2F");
                    ("302 Found", "Location: /api/v2/\r\nSet-Cookie: csrftoken=after-login; Path=/\r\nSet-Cookie: sessionid=session-only; Path=/\r\n", "")
                },
                _ => {
                    assert!(headers.contains("x-csrftoken: after-login"));
                    assert!(headers.contains("sessionid=session-only"));
                    let payload: Value = serde_json::from_str(&body).unwrap();
                    assert_eq!(payload["scope"], "write");
                    assert_eq!(payload["application"], Value::Null);
                    ("201 Created", "", r#"{"token":"test-token"}"#)
                }
            };
            write!(stream, "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        }
    }

    #[test]
    fn login_failure_and_invalid_token_stop_before_any_api_operation_without_leaking_secrets() {
        for (stop_at, status, body, expected) in [
            (0, 200, "no CSRF token", "CSRF token"),
            (1, 401, "test-only session-only", "HTTP 401"),
            (2, 403, "test-only session-only", "HTTP 403"),
            (2, 200, "test-only", "invalid JSON"),
            (2, 201, r#"{"token":""}"#, "did not include"),
            (2, 201, r#"{"token":"test-only\ninvalid"}"#, "invalid API token"),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                for step in 0..=stop_at {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut request = String::new();
                    reader.read_line(&mut request).unwrap();
                    assert!(request.contains(if step == 2 { "/api/v2/tokens/" } else { "/api/login/" }));
                    let mut length = 0;
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" { break; }
                        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                            length = value.trim().parse::<usize>().unwrap();
                        }
                    }
                    reader.read_exact(&mut vec![0;length]).unwrap();
                    let (code, payload, cookie) = if step == stop_at {
                        (status, body, "")
                    } else {
                        (200, "login", "Set-Cookie: csrftoken=test-csrf; Path=/\r\n")
                    };
                    write!(stream, "HTTP/1.1 {code} Test\r\n{cookie}Content-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len()).unwrap();
                }
                listener.set_nonblocking(true).unwrap();
                assert!(listener.accept().is_err(), "Authentication failure must not send API requests");
            });
            let error = authenticated_client(&format!("http://{address}"), &json!({}), &AgentCredential {
                reference: "api".into(), username: "user".into(), password: "test-only".into(),
            }).err().expect("authentication must fail");
            assert!(error.contains(expected), "{error}");
            assert!(!error.contains("test-only"));
            assert!(!error.contains("session-only"));
            server.join().unwrap();
        }
    }

    #[test]
    fn csrf_input_fallback_accepts_both_quote_styles_and_attribute_orders() {
        assert_eq!(csrf_input(r#"<input value="first" name="csrfmiddlewaretoken">"#).as_deref(), Some("first"));
        assert_eq!(csrf_input("<input name='csrfmiddlewaretoken' value='second'>").as_deref(), Some("second"));
        assert_eq!(csrf_input("<input name='unrelated' value='ignored'>"), None);
    }

    #[test]
    fn endpoint_validation_authenticates_then_only_gets_and_needs_no_deployment_inputs() {
        for authorized in [true, false] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                serve_authentication(&listener);
                let paths = if authorized {
                    vec![
                        "/api/v2/me/",
                        "/api/v2/job_templates/14/",
                        "/api/v2/job_templates/12/",
                    ]
                } else {
                    vec!["/api/v2/me/"]
                };
                for path in paths {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut request = String::new();
                    reader.read_line(&mut request).unwrap();
                    assert!(request.starts_with(&format!("GET {path} HTTP/")));
                    loop {
                        let mut header = String::new();
                        reader.read_line(&mut header).unwrap();
                        if header == "\r\n" {
                            break;
                        }
                    }
                    let body = match path {
                        "/api/v2/job_templates/14/" => r#"{"name":"Validate"}"#,
                        "/api/v2/job_templates/12/" => r#"{"name":"Deploy"}"#,
                        _ => r#"{"results":[{"username":"user"}]}"#,
                    };
                    let status = if authorized { 200 } else { 401 };
                    write!(stream,"HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                }
            });
            let process = json!({"details":{"olam": {
                "url":format!("http://{address}"), "account":"api",
                "validation_template_id":14,"validation_template_job":"Validate",
                "deployment_template_id":12,"deployment_template_job":"Deploy"
            }}})
            .to_string();
            let job = AgentJobStatus {
                id: "endpoint".into(),
                sequence: 1,
                name: "Validate OLAM".into(),
                driver: "olam".into(),
                action: "validate".into(),
                target: Some(format!("http://{address}")),
                details: String::new(),
                payload: None,
                status: "running".into(),
                log: String::new(),
                phase: None,
                validation_job_id: None,
                deployment_job_id: None,
                finished_at: None,
                report_progress: Default::default(),
            };
            let credentials = vec![AgentCredential {
                reference: "api".into(),
                username: "user".into(),
                password: "test-only".into(),
            }];
            let result = crate::drivers::execute(
                &process,
                "{}",
                &job,
                &credentials,
                &|_, _| panic!("endpoint validation must not launch a job"),
                &|_| panic!("endpoint validation must not poll job output"),
                &|| false,
            );
            match result {
                Ok(DriverOutcome::Completed(log)) if authorized => {
                    assert!(log.contains("API authentication successful"));
                    assert!(log.contains("Verified validation template"));
                    assert!(log.contains("Verified deployment template"));
                    assert!(log.contains("No Ansible job was launched"));
                }
                Err(error) if !authorized => assert!(error.contains("HTTP 401")),
                _ => panic!("unexpected endpoint validation result"),
            }
            server.join().unwrap();
        }
    }

    #[test]
    fn resumes_existing_olam_jobs_using_only_get_even_on_remote_failure() {
        for phase in ["validation", "deployment"] {
            for status in ["successful", "failed", "canceled", "not_found"] {
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                let address = listener.local_addr().unwrap();
                let server = std::thread::spawn(move || {
                serve_authentication(&listener);
                    let requests = if status == "not_found" { 2 } else { 3 };
                    for index in 0..requests {
                        let (mut stream, _) = listener.accept().unwrap();
                        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                        let mut reader = BufReader::new(stream.try_clone().unwrap());
                        let mut request = String::new();
                        reader.read_line(&mut request).unwrap();
                        let path = if index == 2 || (status == "not_found" && index == 1) { "/api/v2/jobs/68952/stdout/?format=txt" } else { "/api/v2/jobs/68952/" };
                        assert!(request.starts_with(&format!("GET {path} ")), "unexpected request: {request}");
                        loop {
                            let mut header = String::new();
                            reader.read_line(&mut header).unwrap();
                            if header == "\r\n" { break; }
                        }
                        let body = if index == 2 { "ok: [host-1]".into() }
                            else { json!({"status": if index == 0 { "running" } else { status }}).to_string() };
                        let code = if status == "not_found" { "404 Not Found" } else { "200 OK" };
                        write!(stream, "HTTP/1.1 {code}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                    }
                });
                let process = json!({"details":{"olam":{
                    "url":format!("http://{address}"), "account":"api", "payload_defaults":{"ENVIRONMENT":"test"},
                    "validation_template_job":"Validate", "validation_template_id":1,
                    "deployment_template_job":"Deploy", "deployment_template_id":2
                }}}).to_string();
                let job = AgentJobStatus {
                    id:"job".into(), sequence:1, name:"Upgrade".into(), driver:"olam".into(),
                    action: if phase == "deployment" { "deploy" } else { "validate_deployment" }.into(),
                    target:Some("host-1".into()), details:String::new(), payload:None, status:"running".into(),
                    log:String::new(), phase:Some(phase.into()),
                    validation_job_id:if phase == "validation" { Some(68952) } else { Some(100) },
                    deployment_job_id:if phase == "deployment" { Some(68952) } else { None },
                    finished_at:None, report_progress:Default::default(),
                };
                // Monitoring requires no deployment payload or deployment credential.
                let result = execute(&process, "{}", &job, &[AgentCredential {
                    reference:"api".into(), username:"user".into(), password:"test-only".into(),
                }], &|observed_phase,id| { assert_eq!(observed_phase,phase); assert_eq!(id,68952); }, &|_| {});
                if status == "successful" {
                    assert!(matches!(result, Ok(DriverOutcome::Completed(_))));
                } else {
                    assert!(result.is_err());
                }
                server.join().unwrap();
            }
        }
    }

    #[test]
    fn status_authentication_failure_recovers_stdout_and_distinguishes_unknown_result() {
        for output_available in [true, false] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                for path in ["/api/v2/jobs/381260/", "/api/v2/jobs/381260/", "/api/v2/jobs/381260/", "/api/v2/jobs/381260/stdout/?format=txt"] {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut request = String::new();
                    reader.read_line(&mut request).unwrap();
                    assert!(request.starts_with(&format!("GET {path} HTTP/")));
                    let mut authorization = false;
                    loop {
                        let mut header = String::new();
                        reader.read_line(&mut header).unwrap();
                        if header == "\r\n" { break; }
                        if header.to_ascii_lowercase().starts_with("authorization: bearer ") { authorization = true; }
                    }
                    assert!(authorization, "Both status and stdout requests must authenticate");
                    let (status, body) = if path.contains("stdout") && output_available {
                        ("200 OK", "TASK [Ping Linux Servers]\nfatal: [host-1]: FAILED! => {\"msg\": \"Incorrect sudo password\"}\nPLAY RECAP\nhost-1 : failed=1")
                    } else {
                        ("401 Unauthorized", r#"{"detail":"Authentication credentials were not provided."}"#)
                    };
                    write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                }
            });
            let error = poll_job(
                &test_api_client(), &format!("http://{address}"), 381260,
                false, &|_| {}, Duration::ZERO, Duration::ZERO,
            ).unwrap_err();
            assert!(error.contains("Monitoring error:"));
            assert!(error.contains("HTTP 401"));
            assert!(error.contains("remote job result is unknown"));
            assert!(error.contains("/#/jobs/playbook/381260/output"));
            assert!(!error.contains("test-only"));
            if output_available {
                assert!(error.contains("Incorrect sudo password"));
                assert!(error.contains("PLAY RECAP"));
            } else {
                assert!(error.contains("Output retrieval also failed"));
                assert!(!error.contains("OLAM job 381260 failed."));
            }
            server.join().unwrap();
        }
    }

    #[test]
    fn intermittent_status_401_retries_only_get_and_eventually_reports_remote_result() {
        for terminal_status in ["failed", "successful"] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                for status in ["401", "running", "401", "401", terminal_status, "stdout"] {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut request = String::new();reader.read_line(&mut request).unwrap();
                    let path = if status == "stdout" {"/api/v2/jobs/42/stdout/?format=txt"} else {"/api/v2/jobs/42/"};
                    assert!(request.starts_with(&format!("GET {path} HTTP/")), "Polling must never launch another job");
                    let mut authenticated = false;
                    loop {
                        let mut header = String::new();reader.read_line(&mut header).unwrap();
                        if header == "\r\n" {break;}
                        authenticated |= header.to_ascii_lowercase().starts_with("authorization: bearer ");
                    }
                    assert!(authenticated);
                    let (code,body) = match status {
                        "401" => ("401 Unauthorized",r#"{"detail":"Authentication credentials were not provided."}"#.to_owned()),
                        "stdout" => ("200 OK",if terminal_status == "failed" {"Incorrect sudo password"} else {"PLAY RECAP: failed=0"}.to_owned()),
                        _ => ("200 OK",json!({"status":status}).to_string()),
                    };
                    write!(stream,"HTTP/1.1 {code}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                }
            });
            let progress = Mutex::new(Vec::new());
            let result = poll_job(&test_api_client(),&format!("http://{address}"),42,
                false,&|line| progress.lock().unwrap().push(line.to_owned()),Duration::ZERO,Duration::from_secs(600));
            if terminal_status == "failed" {
                let error = result.unwrap_err();
                assert!(error.contains("OLAM job 42 failed."));
                assert!(error.contains("Incorrect sudo password"));
                assert!(!error.contains("result is unknown"));
            } else {
                assert!(result.unwrap().contains("failed=0"));
            }
            assert_eq!(progress.lock().unwrap().len(),3);
            server.join().unwrap();
        }
    }

    #[test]
    fn launch_authentication_failure_reports_node_after_successful_template_check() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
                serve_authentication(&listener);
            for (method, path, code, node, body) in [
                ("GET", "/api/v2/job_templates/14/", "200 OK", "node-a.example.test", r#"{"name":"Validate"}"#),
                ("POST", "/api/v2/job_templates/14/launch/", "401 Unauthorized", "node-b.example.test", r#"{"detail":"Authentication credentials were not provided."}"#),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();reader.read_line(&mut request).unwrap();
                assert!(request.starts_with(&format!("{method} {path} HTTP/")));
                let mut length = 0;let mut authorization = None;
                loop {
                    let mut header = String::new();reader.read_line(&mut header).unwrap();
                    if header == "\r\n" { break; }
                    let lower = header.to_ascii_lowercase();
                    if let Some(value) = lower.strip_prefix("content-length:") { length = value.trim().parse::<usize>().unwrap(); }
                    if lower.starts_with("authorization:") { authorization = Some(header.trim().to_owned()); }
                }
                assert_eq!(authorization.as_deref(), Some("authorization: Bearer test-token"));
                reader.read_exact(&mut vec![0;length]).unwrap();
                write!(stream,"HTTP/1.1 {code}\r\nX-API-Node: {node}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        let process = json!({"details":{"olam":{
            "url":format!("http://{address}"),"account":"api","payload_defaults":{"ENVIRONMENT":"test"},
            "validation_template_id":14,"validation_template_job":"Validate"
        }}}).to_string();
        let job: AgentJobStatus = serde_json::from_value(json!({
            "id":"job","sequence":1,"name":"Validate","driver":"olam","action":"validate_deployment",
            "status":"running","details":"","payload":"{}","log":""
        })).unwrap();
        let output = Mutex::new(String::new());
        let error = execute(&process,"inputs:\n  olam:\n    account: api",&job,
            &[AgentCredential {reference:"api".into(),username:"user".into(),password:"test-only".into()}],
            &|_,_| panic!("A rejected launch must not publish a remote job ID"),
            &|line| output.lock().unwrap().push_str(line),
        ).err().unwrap();
        assert!(output.lock().unwrap().contains("verified by API node node-a.example.test"));
        assert!(error.contains("Could not launch OLAM template"));
        assert!(error.contains("HTTP 401 [OLAM API node: node-b.example.test]"));
        assert!(!error.contains("test-only"));
        server.join().unwrap();
    }

    #[test]
    fn publishes_each_phase_job_id_before_polling_the_external_job() {
        for (phase, template, external_id) in [
            ("validation", "Validate", 133),
            ("deployment", "Deploy", 137),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let job = AgentJobStatus {
                id: "job".into(),
                sequence: 1,
                name: "Upgrade".into(),
                driver: "olam".into(),
                action: if phase == "deployment" {
                    "deploy"
                } else {
                    "validate_deployment"
                }
                .into(),
                target: Some("host-1".into()),
                details: String::new(),
                payload: Some("{}".into()),
                status: "running".into(),
                log: String::new(),
                phase: Some(phase.into()),
                validation_job_id: None,
                deployment_job_id: None,
                finished_at: None,
                report_progress: Default::default(),
            };
            // Existing persisted jobs and older Agents do not include the new fields.
            let mut legacy = serde_json::to_value(&job).unwrap();
            legacy.as_object_mut().unwrap().remove("validationJobId");
            legacy.as_object_mut().unwrap().remove("deploymentJobId");
            let restored: AgentJobStatus = serde_json::from_value(legacy).unwrap();
            assert_eq!(restored.validation_job_id, None);
            assert_eq!(restored.deployment_job_id, None);

            let progress = Arc::new(Mutex::new(job.clone()));
            let observed = progress.clone();
            let server = std::thread::spawn(move || {
                serve_authentication(&listener);
                for index in 0..4 {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut request = String::new();
                    reader.read_line(&mut request).unwrap();
                    let mut length = 0;
                    loop {
                        let mut header = String::new();
                        reader.read_line(&mut header).unwrap();
                        if header == "\r\n" {
                            break;
                        }
                        if let Some(value) =
                            header.to_ascii_lowercase().strip_prefix("content-length:")
                        {
                            length = value.trim().parse::<usize>().unwrap();
                        }
                    }
                    reader.read_exact(&mut vec![0; length]).unwrap();
                    let body = match index {
                        0 => json!({"name": template}).to_string(),
                        1 => json!({"id": external_id}).to_string(),
                        2 => {
                            assert!(
                                request.starts_with(&format!("GET /api/v2/jobs/{external_id}/ "))
                            );
                            let current = observed.lock().unwrap();
                            assert_eq!(
                                if phase == "validation" {
                                    current.validation_job_id
                                } else {
                                    current.deployment_job_id
                                },
                                Some(external_id)
                            );
                            assert!(current.log.contains(&format!(
                                "OLAM {phase} started as Ansible job {external_id}."
                            )));
                            json!({"status":"successful"}).to_string()
                        }
                        _ => "ok: [host-1]".into(),
                    };
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                }
            });
            let process = json!({"details":{"olam":{
                "url":format!("http://{address}"),"account":"api",
                "payload_defaults":{"ENVIRONMENT":"test"},
                "validation_template_job":"Validate","validation_template_id":1,
                "deployment_template_job":"Deploy","deployment_template_id":2
            }}})
            .to_string();
            let credentials = vec![
                AgentCredential {
                    reference: "api".into(),
                    username: "user".into(),
                    password: "test-only".into(),
                },
                AgentCredential {
                    reference: "deploy".into(),
                    username: "user".into(),
                    password: "test-only".into(),
                },
            ];
            let outcome = execute(
                &process,
                "inputs:\n  olam:\n    account: deploy",
                &job,
                &credentials,
                &|phase, id| {
                    crate::record_olam_launch(&mut progress.lock().unwrap(), phase, id);
                },
                &|_| {},
            )
            .unwrap();
            assert!(matches!(outcome, DriverOutcome::Completed(_)));
            server.join().unwrap();
        }
    }

    #[test]
    fn streams_running_output_and_survives_temporary_stdout_failure() {
        for (deployment, terminal_status) in [(false, "successful"), (true, "failed")] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let snapshots = Arc::new(Mutex::new(Vec::<String>::new()));
            let observed = snapshots.clone();
            let server = std::thread::spawn(move || {
                for index in 0..8 {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut request = String::new();
                    reader.read_line(&mut request).unwrap();
                    loop {
                        let mut header = String::new();
                        reader.read_line(&mut header).unwrap();
                        if header == "\r\n" {
                            break;
                        }
                    }
                    let (status, body) = if index % 2 == 0 {
                        assert!(request.starts_with("GET /api/v2/jobs/174/ HTTP/"));
                        if index == 6 {
                            assert_eq!(
                                observed.lock().unwrap().as_slice(),
                                ["line 1", "line 1\nline 2"]
                            );
                        }
                        (
                            200,
                            json!({"status": if index == 6 { terminal_status } else { "running" }})
                                .to_string(),
                        )
                    } else {
                        assert!(
                            request.starts_with("GET /api/v2/jobs/174/stdout/?format=txt HTTP/")
                        );
                        match index {
                            1 => (200, "line 1".into()),
                            3 => (503, "Temporarily unavailable".into()),
                            5 => (200, "line 1\nline 2".into()),
                            _ => (200, "line 1\nline 2\nfinished".into()),
                        }
                    };
                    write!(stream,"HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                }
            });
            let result = poll_job(
                &test_api_client(),
                &format!("http://{address}"),
                174,
                deployment,
                &|output| snapshots.lock().unwrap().push(output.into()),
                Duration::ZERO,
                Duration::ZERO,
            );
            if terminal_status == "successful" {
                assert_eq!(result.unwrap(), "line 1\nline 2\nfinished");
            } else {
                assert_eq!(
                    result.unwrap_err(),
                    "OLAM job 174 failed.\nline 1\nline 2\nfinished"
                );
            }
            server.join().unwrap();
        }
    }

    #[test]
    fn treats_no_matching_hosts_as_a_warning() {
        assert!(output_indicates_no_hosts(
            "[WARNING]: Could not match supplied host pattern\nPLAY [Ping Servers]\nskipping: no hosts matched"
        ));
        assert!(!output_indicates_no_hosts(
            "PLAY [Ping Servers]\nok: [host-1]"
        ));
    }
}
