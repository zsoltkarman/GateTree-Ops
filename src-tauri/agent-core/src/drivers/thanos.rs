use crate::{AgentCredential, AgentJobStatus};
use serde::Deserialize;
use serde_json::Value;
use std::{
    io::Read,
    thread,
    time::{Duration, Instant},
};

#[derive(Deserialize)]
struct Config {
    url: String,
    timeout_seconds: u64,
    interval_seconds: u64,
    #[serde(default = "none_auth")]
    auth: String,
    #[serde(default)]
    account: String,
}
fn none_auth() -> String {
    "none".into()
}
fn config(value: &Value) -> Result<Config, String> {
    let c: Config = serde_json::from_value(value.clone())
        .map_err(|e| format!("Invalid details.thanos: {e}"))?;
    let url = reqwest::Url::parse(c.url.trim())
        .map_err(|_| "details.thanos.url must be an HTTP(S) base URL.")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("details.thanos.url must be an HTTP(S) base URL without credentials, query or fragment.".into());
    }
    if c.timeout_seconds == 0
        || c.timeout_seconds > 86400
        || c.interval_seconds == 0
        || c.interval_seconds > c.timeout_seconds
    {
        return Err(
            "Thanos timeout_seconds must be 1..86400; interval_seconds must be 1..timeout_seconds."
                .into(),
        );
    }
    if !matches!(c.auth.as_str(), "none" | "basic" | "bearer")
        || (c.auth != "none" && c.account.trim().is_empty())
    {
        return Err("Thanos auth must be none, basic or bearer; authenticated queries require a Vault account.".into());
    }
    Ok(c)
}
pub(crate) fn validate_config(value: &Value) -> Result<(), String> {
    config(value).map(|_| ())
}

fn query_result(body: &Value) -> Result<(bool, String), String> {
    if body["status"] != "success" {
        return Err("Thanos returned an unsuccessful query response.".into());
    }
    if body
        .get("warnings")
        .is_some_and(|v| v.as_array().is_none_or(|a| !a.is_empty()))
    {
        return Ok((
            false,
            "Query warnings/partial data; validation is unverified.".into(),
        ));
    }
    let data = &body["data"];
    let rows: Vec<&Value> = match data["resultType"].as_str() {
        Some("scalar") => vec![&data["result"]],
        Some("vector") => data["result"]
            .as_array()
            .ok_or("Invalid Thanos vector response.")?
            .iter()
            .map(|row| &row["value"])
            .collect(),
        _ => return Err(
            "Validation query must return an instant scalar or vector, with value 1 for success."
                .into(),
        ),
    };
    if rows.len() > 10000 {
        return Err("Validation query returned too many series (maximum 10000).".into());
    }
    let mut passed = 0;
    for row in &rows {
        let a = row
            .as_array()
            .filter(|a| a.len() == 2)
            .ok_or("Invalid Thanos sample.")?;
        let value = a[1]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or("Invalid Thanos sample value.")?;
        if value.is_finite() && value == 1.0 {
            passed += 1;
        }
    }
    let ok = !rows.is_empty() && passed == rows.len();
    Ok((
        ok,
        format!(
            "{passed}/{} query results equal 1{}",
            rows.len(),
            if rows.is_empty() { " (no data)" } else { "" }
        ),
    ))
}

pub(crate) fn payload_query(job: &AgentJobStatus) -> Result<String, String> {
    let payload: Value = serde_json::from_str(job.payload.as_deref().unwrap_or("{}"))
        .map_err(|_| "Invalid Thanos query payload.")?;
    if let Some(error) = payload["query_error"].as_str() { return Err(error.into()); }
    payload["query"].as_str().map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 32768 && !s.contains("${"))
        .map(str::to_owned).ok_or_else(|| "A complete PromQL query is required.".into())
}

pub fn execute(
    process: &str,
    job: &AgentJobStatus,
    credentials: &[AgentCredential],
    on_output: &dyn Fn(&str),
    cancelled: &dyn Fn() -> bool,
) -> Result<String, String> {
    execute_with_query(process, credentials, on_output, &|| payload_query(job), cancelled)
}

pub(crate) fn execute_with_query(
    process: &str,
    credentials: &[AgentCredential],
    on_output: &dyn Fn(&str),
    current_query: &dyn Fn() -> Result<String, String>,
    cancelled: &dyn Fn() -> bool,
) -> Result<String, String> {
    let process: Value = serde_json::from_str(process).map_err(|_| "Invalid Process Config.")?;
    let c = config(
        process
            .pointer("/details/thanos")
            .ok_or("Configure details.thanos in Deployments → Config.")?,
    )?;
    let credential = if c.auth == "none" {
        None
    } else {
        Some(
            credentials
                .iter()
                .find(|v| v.reference.eq_ignore_ascii_case(c.account.trim()))
                .ok_or("Thanos Vault credential is unavailable in Agent memory.")?,
        )
    };
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not initialize Thanos HTTP client.")?;
    let url = format!("{}/api/v1/query", c.url.trim().trim_end_matches('/'));
    let start = Instant::now();
    let timeout = Duration::from_secs(c.timeout_seconds);
    let mut log = format!(
        "Thanos validation\nTimeout: {}s; interval: {}s\nSuccess requires non-empty results, all equal to 1.\n",
        c.timeout_seconds, c.interval_seconds
    );
    let mut attempt = 0;
    let mut previous_query = String::new();
    loop {
        if cancelled() {
            return Err("Thanos validation stopped.".into());
        }
        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return Err(format!(
                "{log}\nThanos validation timed out after {} seconds.",
                c.timeout_seconds
            ));
        }
        let query = current_query().map_err(|error| format!("{log}\n{error}"))?;
        if query != previous_query {
            log.push_str(&format!("Query: {query}\n"));
            previous_query.clone_from(&query);
        }
        attempt += 1;
        let mut request = client
            .get(&url)
            .query(&[("query", query.as_str()), ("partial_response", "false")])
            .header("Accept", "application/json")
            .timeout(remaining.min(Duration::from_secs(15)));
        if let Some(credential) = credential {
            request = if c.auth == "basic" {
                request.basic_auth(&credential.username, Some(&credential.password))
            } else {
                request.bearer_auth(&credential.password)
            };
        }
        let mut result = (|| -> Result<(bool, String), String> { Ok(match request.send() {
            Ok(mut response) => {
                let status = response.status();
                if matches!(status.as_u16(), 400 | 401 | 403 | 404 | 422) { return Err(format!("Thanos query failed (HTTP {}). Check the query, URL and configured Vault account.", status.as_u16())); }
                if !status.is_success() { (false, format!("Thanos HTTP {}; retrying.", status.as_u16())) }
                else {
                    let mut bytes = Vec::new();
                    Read::take(&mut response, 2 * 1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|_| "Could not read Thanos response.")?;
                    if bytes.len() > 2 * 1024 * 1024 { return Err("Thanos response exceeds 2 MiB. Narrow the query.".into()); }
                    let body: Value = serde_json::from_slice(&bytes).map_err(|_| "Thanos returned invalid JSON.")?;
                    let (ok, summary) = query_result(&body)?;
                    // Keep the last response as evidence, bounded independently from the polling log.
                    let evidence = serde_json::to_string(&body["data"]).unwrap_or_default();
                    (ok, format!("{summary}\nResult: {}", evidence.chars().take(12000).collect::<String>()))
                }
            },
            Err(error) => (false, format!("Thanos connection/timeout error; retrying ({}).", if error.is_timeout() { "timeout" } else { "connection" })),
        }) })().map_err(|error| format!("{log}\n{error}"))?;
        // A Config edit during the request invalidates its response. The next
        // poll must verify the current expression before it can pass the job.
        if current_query().map_err(|error| format!("{log}\n{error}"))? != query {
            result = (false, "Saved query changed during the request; verifying the latest Config next.".into());
        }
        if log.len() > 48000 {
            log = format!("Thanos validation\nQuery: {query}\nEarlier attempts omitted.\n");
        }
        log.push_str(&format!(
            "\n{} - attempt {attempt}, elapsed {}s: {}\n",
            chrono::Utc::now().to_rfc3339(),
            start.elapsed().as_secs(),
            result.1
        ));
        on_output(&log);
        if cancelled() {
            return Err("Thanos validation stopped.".into());
        }
        if result.0 && start.elapsed() <= timeout {
            return Ok(format!("{log}\nThanos validation passed."));
        }
        let next = Instant::now()
            + Duration::from_secs(c.interval_seconds).min(timeout.saturating_sub(start.elapsed()));
        while Instant::now() < next {
            if cancelled() {
                return Err("Thanos validation stopped.".into());
            }
            thread::sleep(
                Duration::from_millis(100).min(next.saturating_duration_since(Instant::now())),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn job() -> AgentJobStatus {
        AgentJobStatus {
            id: "query-job".into(),
            sequence: 1,
            name: "Check".into(),
            driver: "thanos".into(),
            action: "query".into(),
            target: None,
            details: String::new(),
            payload: Some(json!({"query":"vector(1)"}).to_string()),
            status: "running".into(),
            log: String::new(),
            phase: None,
            validation_job_id: None,
            deployment_job_id: None,
            finished_at: None,
            report_progress: Default::default(),
        }
    }
    fn server(values: Vec<(&'static str, u16)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (value, status) in values {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(4)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut part = [0u8; 1024];
                while !bytes.windows(4).any(|v| v == b"\r\n\r\n") {
                    let n = stream.read(&mut part).unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&part[..n]);
                }
                requests.push(String::from_utf8(bytes).unwrap());
                let body =
                    json!({"status":"success","data":{"resultType":"scalar","result":[1,value]}})
                        .to_string();
                write!(stream,"HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
            requests
        });
        (url, handle)
    }
    #[test]
    fn http_poll_retries_then_passes_and_sends_basic_auth_from_memory() {
        let (url, handle) = server(vec![("0", 200), ("1", 200)]);
        let process=json!({"details":{"thanos":{"url":url,"timeout_seconds":4,"interval_seconds":1,"auth":"basic","account":"metrics"}}}).to_string();
        let credentials = [AgentCredential {
            reference: "metrics".into(),
            username: "reader".into(),
            password: "test-pass".into(),
        }];
        let log = execute(&process, &job(), &credentials, &|_| {}, &|| false).unwrap();
        assert!(log.contains("attempt 2"));
        assert!(log.contains("passed"));
        assert!(!log.contains("test-pass"));
        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].contains("GET /api/v1/query?"));
        assert!(requests[0].contains("partial_response=false"));
        assert!(
            requests[0]
                .to_ascii_lowercase()
                .contains("authorization: basic ")
        );
    }
    #[test]
    fn zero_times_out_and_auth_failure_does_not_retry() {
        let (url, handle) = server(vec![("0", 200)]);
        let process =
            json!({"details":{"thanos":{"url":url,"timeout_seconds":1,"interval_seconds":1}}})
                .to_string();
        assert!(
            execute(&process, &job(), &[], &|_| {}, &|| false)
                .unwrap_err()
                .contains("timed out")
        );
        handle.join().unwrap();
        let (url, handle) = server(vec![("1", 401)]);
        let process =
            json!({"details":{"thanos":{"url":url,"timeout_seconds":4,"interval_seconds":1}}})
                .to_string();
        assert!(
            execute(&process, &job(), &[], &|_| {}, &|| false)
                .unwrap_err()
                .contains("HTTP 401")
        );
        assert_eq!(handle.join().unwrap().len(), 1);
    }
    #[test]
    fn cancellation_interrupts_retry_wait() {
        let (url, handle) = server(vec![("0", 200)]);
        let process =
            json!({"details":{"thanos":{"url":url,"timeout_seconds":30,"interval_seconds":20}}})
                .to_string();
        let stopped = std::sync::atomic::AtomicBool::new(false);
        let started = Instant::now();
        let error = execute(
            &process,
            &job(),
            &[],
            &|_| {
                stopped.store(true, std::sync::atomic::Ordering::SeqCst);
            },
            &|| stopped.load(std::sync::atomic::Ordering::SeqCst),
        )
        .unwrap_err();
        assert!(error.contains("stopped"));
        assert!(started.elapsed() < Duration::from_secs(2));
        handle.join().unwrap();
    }
    #[test]
    fn only_complete_nonempty_all_one_results_pass() {
        let scalar = |value: &str| json!({"status":"success","data":{"resultType":"scalar","result":[1,value]}});
        for value in ["0", "2", "NaN", "+Inf", "-1"] {
            assert!(!query_result(&scalar(value)).unwrap().0);
        }
        assert!(query_result(&scalar("1")).unwrap().0);
        assert!(
            !query_result(&json!({"status":"success","data":{"resultType":"vector","result":[]}}))
                .unwrap()
                .0
        );
        assert!(!query_result(&json!({"status":"success","warnings":["partial"],"data":{"resultType":"scalar","result":[1,"1"]}})).unwrap().0);
        assert!(!query_result(&json!({"status":"success","data":{"resultType":"vector","result":[{"value":[1,"1"]},{"value":[1,"0"]}]}})).unwrap().0);
        assert!(query_result(&json!({"status":"error"})).is_err());
        assert!(
            query_result(&json!({"status":"success","data":{"resultType":"matrix","result":[]}}))
                .is_err()
        );
    }
    #[test]
    fn every_http_attempt_uses_latest_config_and_logs_the_actual_query() {
        let (url, handle) = server(vec![("0", 200), ("1", 200)]);
        let process = json!({"details":{"thanos":{"url":url,"timeout_seconds":5,"interval_seconds":1}}}).to_string();
        let query = std::cell::RefCell::new("vector(0)".to_string());
        let log = execute_with_query(&process, &[], &|_| {
            *query.borrow_mut() = "vector(1)".into();
        }, &|| Ok(query.borrow().clone()), &|| false).unwrap();
        let requests = handle.join().unwrap();
        assert!(requests[0].contains("query=vector%280%29"));
        assert!(requests[1].contains("query=vector%281%29"));
        assert!(log.contains("Query: vector(0)"));
        assert!(log.contains("Query: vector(1)"));
    }

    #[test]
    fn config_change_during_successful_request_requires_another_poll() {
        let (url, handle) = server(vec![("1", 200), ("1", 200)]);
        let process = json!({"details":{"thanos":{"url":url,"timeout_seconds":5,"interval_seconds":1}}}).to_string();
        let reads = std::cell::Cell::new(0);
        let log = execute_with_query(&process, &[], &|_| {}, &|| {
            let count = reads.get(); reads.set(count + 1);
            Ok(if count == 0 { "vector(1)" } else { "sum(vector(1))" }.into())
        }, &|| false).unwrap();
        assert_eq!(handle.join().unwrap().len(), 2);
        assert!(log.contains("changed during the request"));
        assert!(log.contains("Query: sum(vector(1))"));
    }

    #[test]
    fn config_requires_explicit_valid_connection_and_polling() {
        let valid = json!({"url":"https://thanos.example.test","timeout_seconds":300,"interval_seconds":30});
        validate_config(&valid).unwrap();
        for (field, value) in [
            ("url", json!("https://user:secret@example.test")),
            ("timeout_seconds", json!(0)),
            ("interval_seconds", json!(301)),
            ("auth", json!("bearer")),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            assert!(validate_config(&invalid).is_err());
        }
    }
}
