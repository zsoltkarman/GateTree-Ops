use std::{
    thread,
    time::{Duration, Instant},
};

use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};

use super::DriverOutcome;
use crate::{AgentCredential, AgentJobStatus};

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
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .danger_accept_invalid_certs(
            !olam
                .get("verify_tls_certificate")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        )
        .build()
        .map_err(|error| format!("Could not initialize OLAM HTTP client: {error}"))?;
    checked(
        client
            .get(format!("{}/api/v2/me/", base_url.trim_end_matches('/')))
            .basic_auth(&account.username, Some(&account.password))
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
            verify_template(&client, base_url, id, name, account)?;
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
    let deployment_account = deployment_account(plan_definition)?;
    let deployment_credential = credential(credentials, &deployment_account, "deployment")?;
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .danger_accept_invalid_certs(
            !olam
                .get("verify_tls_certificate")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        )
        .build()
        .map_err(|error| format!("Could not initialize OLAM HTTP client: {error}"))?;

    verify_template(
        &client,
        base_url,
        template_id,
        template_name,
        api_credential,
    )?;
    let payload = resolved_payload(job, deployment_credential, environment)?;
    let launch_url = format!(
        "{}/api/v2/job_templates/{template_id}/launch/",
        base_url.trim_end_matches('/')
    );
    let launch: Value = checked(
        client
            .post(&launch_url)
            .basic_auth(&api_credential.username, Some(&api_credential.password))
            .json(&json!({ "extra_vars": payload }))
            .send()
            .map_err(|error| {
                format!("Could not launch OLAM template '{template_name}': {error}")
            })?,
        &format!("launch OLAM template '{template_name}'"),
    )?
    .json()
    .map_err(|error| format!("OLAM launch response is invalid: {error}"))?;
    let job_id = launch
        .get("id")
        .and_then(Value::as_i64)
        .filter(|id| *id > 0)
        .ok_or_else(|| "OLAM did not return an Ansible job ID.".to_string())?;
    on_started(phase, job_id);
    let output = wait_for_job(
        &client,
        base_url,
        job_id,
        api_credential,
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
    credential: &AgentCredential,
) -> Result<(), String> {
    let url = format!(
        "{}/api/v2/job_templates/{id}/",
        base_url.trim_end_matches('/')
    );
    let value: Value = checked(
        client
            .get(&url)
            .basic_auth(&credential.username, Some(&credential.password))
            .send()
            .map_err(|error| format!("Could not verify OLAM template ID {id}: {error}"))?,
        &format!("verify OLAM template ID {id}"),
    )?
    .json()
    .map_err(|error| format!("OLAM template response is invalid: {error}"))?;
    let actual_name = value.get("name").and_then(Value::as_str).unwrap_or("");
    if !actual_name.eq_ignore_ascii_case(name) {
        return Err(format!(
            "OLAM template ID {id} is '{actual_name}', but Process Config requires '{name}'. No job was started."
        ));
    }
    Ok(())
}

fn wait_for_job(
    client: &Client,
    base_url: &str,
    job_id: i64,
    credential: &AgentCredential,
    deployment: bool,
    on_output: &dyn Fn(&str),
) -> Result<String, String> {
    poll_job(
        client,
        base_url,
        job_id,
        credential,
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
    credential: &AgentCredential,
    deployment: bool,
    on_output: &dyn Fn(&str),
    status_interval: Duration,
    output_interval: Duration,
) -> Result<String, String> {
    let attempts = if deployment { 21_600 } else { 300 };
    let mut last_output = String::new();
    let mut last_read = Instant::now();
    let url = format!("{}/api/v2/jobs/{job_id}/", base_url.trim_end_matches('/'));
    for _ in 0..attempts {
        let value: Value = checked(
            client
                .get(&url)
                .basic_auth(&credential.username, Some(&credential.password))
                .send()
                .map_err(|error| {
                    format!("Could not poll OLAM job {job_id}: {error}\n{last_output}")
                })?,
            &format!("poll OLAM job {job_id}"),
        )
        .map_err(|error| format!("{error}\n{last_output}"))?
        .json()
        .map_err(|error| {
            format!("OLAM job {job_id} response is invalid: {error}\n{last_output}")
        })?;
        match value.get("status").and_then(Value::as_str).unwrap_or("") {
            "successful" => {
                return job_output(client, base_url, job_id, credential)
                    .map_err(|error| format!("{error}\n{last_output}"));
            }
            "failed" | "error" | "canceled" => {
                let output =
                    job_output(client, base_url, job_id, credential).unwrap_or(last_output);
                return Err(format!("OLAM job {job_id} failed.\n{output}"));
            }
            _ => {
                if last_read.elapsed() >= output_interval {
                    // A temporary stdout failure must not stop the external deployment.
                    if let Ok(output) = job_output(client, base_url, job_id, credential) {
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
    credential: &AgentCredential,
) -> Result<String, String> {
    let url = format!(
        "{}/api/v2/jobs/{job_id}/stdout/?format=txt",
        base_url.trim_end_matches('/')
    );
    let response = checked(
        client
            .get(&url)
            .basic_auth(&credential.username, Some(&credential.password))
            .header("Accept", "text/plain, */*")
            .send()
            .map_err(|error| format!("Could not read OLAM job {job_id} output: {error}"))?,
        &format!("read OLAM job {job_id} output"),
    )?;
    let text = response.text().map_err(|error| error.to_string())?;
    Ok(text.chars().take(512_000).collect())
}

fn checked(response: Response, operation: &str) -> Result<Response, String> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().unwrap_or_default();
    Err(format!(
        "Could not {operation}: HTTP {}: {}",
        status.as_u16(),
        body.chars().take(1000).collect::<String>()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
    };

    #[test]
    fn endpoint_validation_uses_only_get_and_needs_no_deployment_inputs() {
        for authorized in [true, false] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
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
            let credential = AgentCredential {
                reference: "api".into(),
                username: "user".into(),
                password: "test-only".into(),
            };
            let result = poll_job(
                &Client::new(),
                &format!("http://{address}"),
                174,
                &credential,
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
