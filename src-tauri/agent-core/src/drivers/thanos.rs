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
    if let Some(error) = payload["query_error"].as_str() {
        return Err(error.into());
    }
    payload["query"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 32768 && !s.contains("${"))
        .map(str::to_owned)
        .ok_or_else(|| "A complete PromQL query is required.".into())
}

pub fn execute(
    process: &str,
    job: &AgentJobStatus,
    credentials: &[AgentCredential],
    on_output: &dyn Fn(&str),
    cancelled: &dyn Fn() -> bool,
) -> Result<String, String> {
    let payload: Value = serde_json::from_str(job.payload.as_deref().unwrap_or("{}"))
        .map_err(|_| "Invalid Thanos payload.")?;
    if payload.get("checks").is_some() {
        execute_with_queries(
            process,
            credentials,
            on_output,
            &|| Ok(payload.clone()),
            cancelled,
        )
    } else {
        execute_with_query(
            process,
            credentials,
            on_output,
            &|| payload_query(job),
            cancelled,
        )
    }
}

pub(crate) fn execute_with_query(
    process: &str,
    credentials: &[AgentCredential],
    on_output: &dyn Fn(&str),
    current_query: &dyn Fn() -> Result<String, String>,
    cancelled: &dyn Fn() -> bool,
) -> Result<String, String> {
    execute_with_queries(
        process,
        credentials,
        on_output,
        &|| Ok(serde_json::json!({"query": current_query()?})),
        cancelled,
    )
}

#[derive(Debug, Clone, PartialEq)]
struct Check {
    name: String,
    query: String,
    min_results: usize,
}

fn checks(payload: &Value) -> Result<Vec<Check>, String> {
    if let Some(error) = payload["query_error"].as_str() {
        return Err(error.into());
    }
    let entries: Vec<&Value> = if let Some(items) = payload.get("checks") {
        let items = items
            .as_array()
            .filter(|items| !items.is_empty() && items.len() <= 32)
            .ok_or("A Query Set requires 1..32 checks.")?;
        items.iter().collect()
    } else {
        vec![payload]
    };
    entries
        .into_iter()
        .map(|item| {
            if let Some(error) = item["query_error"].as_str() {
                return Err(error.into());
            }
            let query = item["query"]
                .as_str()
                .map(str::trim)
                .filter(|q| !q.is_empty() && q.len() <= 32768 && !q.contains("${"))
                .ok_or("A complete PromQL query is required for every check.")?;
            let min_results = item
                .get("min_results")
                .map_or(Some(1), Value::as_u64)
                .filter(|n| (1..=10000).contains(n))
                .ok_or("Invalid minimum result count.")? as usize;
            Ok(Check {
                name: item["query_name"].as_str().unwrap_or("Query").into(),
                query: query.into(),
                min_results,
            })
        })
        .collect()
}

fn result_for_check(
    body: &Value,
    check: &Check,
    evaluated_at: f64,
    max_age: Option<u64>,
) -> Result<(bool, String), String> {
    let (mut ok, mut summary) = query_result(body)?;
    let data = &body["data"];
    let rows: Vec<&Value> = if data["resultType"] == "scalar" {
        vec![&data["result"]]
    } else {
        data["result"]
            .as_array()
            .map(|a| a.iter().map(|r| &r["value"]).collect())
            .unwrap_or_default()
    };
    if rows.len() < check.min_results {
        ok = false;
        summary.push_str(&format!("; need at least {} results", check.min_results));
    }
    if let Some(age) = max_age {
        let fresh = rows.iter().all(|row| {
            row[0].as_f64().is_some_and(|t| {
                t.is_finite() && t <= evaluated_at + 1.0 && evaluated_at - t <= age as f64
            })
        });
        if !fresh {
            ok = false;
            summary.push_str("; stale or invalid result timestamp");
        }
    }
    Ok((ok, summary))
}

pub(crate) fn execute_with_queries(
    process: &str,
    credentials: &[AgentCredential],
    on_output: &dyn Fn(&str),
    current_payload: &dyn Fn() -> Result<Value, String>,
    cancelled: &dyn Fn() -> bool,
) -> Result<String, String> {
    let process: Value = serde_json::from_str(process).map_err(|_| "Invalid Process Config.")?;
    let mut c = config(
        process
            .pointer("/details/thanos")
            .ok_or("Configure details.thanos in Deployments → Config.")?,
    )?;
    let initial = current_payload()?;
    let is_set = initial.get("checks").is_some();
    let stable = if is_set {
        initial["stable_samples"]
            .as_u64()
            .filter(|v| (1..=100).contains(v))
            .ok_or("Invalid Query Set consecutive rounds.")?
    } else {
        1
    };
    let max_age = if is_set {
        Some(
            initial["max_age_seconds"]
                .as_u64()
                .filter(|v| (1..=3600).contains(v))
                .ok_or("Invalid Query Set maximum result age.")?,
        )
    } else {
        None
    };
    if is_set {
        c.timeout_seconds = initial["timeout_seconds"]
            .as_u64()
            .filter(|v| (1..=86400).contains(v))
            .ok_or("Invalid Query Set timeout.")?;
        c.interval_seconds = initial["interval_seconds"]
            .as_u64()
            .filter(|v| *v > 0 && *v <= c.timeout_seconds)
            .ok_or("Invalid Query Set interval.")?;
        if (stable - 1) * c.interval_seconds >= c.timeout_seconds {
            return Err("Query Set consecutive rounds cannot fit within its timeout.".into());
        }
    }
    checks(&initial)?;
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
    let mut previous_payload = Value::Null;
    let mut streak = 0;
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
        let payload = if attempt == 0 {
            initial.clone()
        } else {
            current_payload().map_err(|error| format!("{log}\n{error}"))?
        };
        let entries = checks(&payload).map_err(|error| format!("{log}\n{error}"))?;
        if payload != previous_payload {
            streak = 0;
            for check in &entries {
                log.push_str(&format!("{} — Query: {}\n", check.name, check.query));
            }
            previous_payload = payload.clone();
        }
        let evaluated_at = chrono::Utc::now().timestamp_millis() as f64 / 1000.0;
        let query_time = evaluated_at.to_string();
        let mut round_ok = true;
        let mut round_log = String::new();
        for check in &entries {
            if cancelled() {
                return Err("Thanos validation stopped.".into());
            }
            let remaining = timeout.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                round_ok = false;
                break;
            }
            let query = &check.query;
            // Every member is queried at the same evaluation time.

            let mut request = client
                .get(&url)
                .query(&[
                    ("query", query.as_str()),
                    ("partial_response", "false"),
                    ("time", query_time.as_str()),
                ])
                .header("Accept", "application/json")
                .timeout(remaining.min(Duration::from_secs(15)));
            if let Some(credential) = credential {
                request = if c.auth == "basic" {
                    request.basic_auth(&credential.username, Some(&credential.password))
                } else {
                    request.bearer_auth(&credential.password)
                };
            }
            let result = (|| -> Result<(bool, String), String> { Ok(match request.send() {
            Ok(mut response) => {
                let status = response.status();
                if matches!(status.as_u16(), 400 | 401 | 403 | 404 | 422) { return Err(format!("Thanos query failed (HTTP {}). Check the query, URL and configured Vault account.", status.as_u16())); }
                if !status.is_success() { (false, format!("Thanos HTTP {}; retrying.", status.as_u16())) }
                else {
                    let mut bytes = Vec::new();
                    Read::take(&mut response, 2 * 1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|_| "Could not read Thanos response.")?;
                    if bytes.len() > 2 * 1024 * 1024 { return Err("Thanos response exceeds 2 MiB. Narrow the query.".into()); }
                    let body: Value = serde_json::from_slice(&bytes).map_err(|_| "Thanos returned invalid JSON.")?;
                    let (ok, summary) = result_for_check(&body, check, evaluated_at, max_age)?;
                    // Keep the last response as evidence, bounded independently from the polling log.
                    let evidence = serde_json::to_string(&body["data"]).unwrap_or_default();
                    (ok, format!("{summary}\nResult: {}", evidence.chars().take(12000).collect::<String>()))
                }
            },
            Err(error) => (false, format!("Thanos connection/timeout error; retrying ({}).", if error.is_timeout() { "timeout" } else { "connection" })),
        }) })().map_err(|error| format!("{log}\n{}: {error}", check.name))?;
            round_ok &= result.0;
            round_log.push_str(&format!(
                "{}: {} — {}\n",
                check.name,
                if result.0 { "OK" } else { "WAITING" },
                result.1
            ));
        }
        // A library edit invalidates the entire round, including earlier successes.
        if current_payload().map_err(|error| format!("{log}\n{error}"))? != payload {
            round_ok = false;
            round_log.push_str(
                "Saved query changed during the request; verifying the latest Config next.\n",
            );
        }
        attempt += 1;
        streak = if round_ok { streak + 1 } else { 0 };
        if log.len() > 48000 {
            log = "Thanos validation\nEarlier attempts omitted.\n".into();
        }
        log.push_str(&format!(
            "\n{} - attempt {attempt}, elapsed {}s: {}\n",
            chrono::Utc::now().to_rfc3339(),
            start.elapsed().as_secs(),
            format!("{round_log}Consecutive healthy rounds: {streak}/{stable}")
        ));
        on_output(&log);
        if cancelled() {
            return Err("Thanos validation stopped.".into());
        }
        if round_ok
            && streak >= stable
            && start.elapsed() <= timeout
            && current_payload().map_err(|error| format!("{log}\n{error}"))? == payload
        {
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
                    json!({"status":"success","data":{"resultType":"scalar","result":[chrono::Utc::now().timestamp_millis() as f64 / 1000.0,value]}})
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
        let scalar = |value: &str| json!({"status":"success","data":{"resultType":"scalar","result":[chrono::Utc::now().timestamp_millis() as f64 / 1000.0,value]}});
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
        let process =
            json!({"details":{"thanos":{"url":url,"timeout_seconds":5,"interval_seconds":1}}})
                .to_string();
        let query = std::cell::RefCell::new("vector(0)".to_string());
        let log = execute_with_query(
            &process,
            &[],
            &|_| {
                *query.borrow_mut() = "vector(1)".into();
            },
            &|| Ok(query.borrow().clone()),
            &|| false,
        )
        .unwrap();
        let requests = handle.join().unwrap();
        assert!(requests[0].contains("query=vector%280%29"));
        assert!(requests[1].contains("query=vector%281%29"));
        assert!(log.contains("Query: vector(0)"));
        assert!(log.contains("Query: vector(1)"));
    }

    #[test]
    fn config_change_during_successful_request_requires_another_poll() {
        let (url, handle) = server(vec![("1", 200), ("1", 200)]);
        let process =
            json!({"details":{"thanos":{"url":url,"timeout_seconds":5,"interval_seconds":1}}})
                .to_string();
        let reads = std::cell::Cell::new(0);
        let log = execute_with_query(
            &process,
            &[],
            &|_| {},
            &|| {
                let count = reads.get();
                reads.set(count + 1);
                Ok(if count == 0 {
                    "vector(1)"
                } else {
                    "sum(vector(1))"
                }
                .into())
            },
            &|| false,
        )
        .unwrap();
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
    fn set_payload() -> Value {
        json!({"checks":[{"query_name":"Java","query":"vector(1)","min_results":1},
            {"query_name":"ISR","query":"sum(vector(1))","min_results":1}],
            "stable_samples":2,"interval_seconds":1,"timeout_seconds":10,"max_age_seconds":120})
    }
    #[test]
    fn query_set_requires_all_members_in_consecutive_rounds_and_resets_on_failure() {
        let (url, handle) = server(vec![
            ("1", 200),
            ("1", 200),
            ("1", 200),
            ("0", 200),
            ("1", 200),
            ("1", 200),
            ("1", 200),
            ("1", 200),
        ]);
        let process =
            json!({"details":{"thanos":{"url":url,"timeout_seconds":30,"interval_seconds":1}}})
                .to_string();
        let log =
            execute_with_queries(&process, &[], &|_| {}, &|| Ok(set_payload()), &|| false).unwrap();
        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 8);
        assert!(log.contains("attempt 4"));
        assert!(log.contains("Consecutive healthy rounds: 0/2"));
        assert!(log.contains("Consecutive healthy rounds: 2/2"));
        assert!(log.contains("ISR: WAITING"));
        for pair in requests.chunks(2) {
            let time = |request: &str| {
                request
                    .split("time=")
                    .nth(1)
                    .unwrap()
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .to_owned()
            };
            assert_eq!(time(&pair[0]), time(&pair[1]));
        }
    }
    #[test]
    fn query_set_library_changes_reset_the_entire_streak() {
        let (url, handle) = server(vec![("1", 200); 6]);
        let process =
            json!({"details":{"thanos":{"url":url,"timeout_seconds":10,"interval_seconds":1}}})
                .to_string();
        let payload = std::cell::RefCell::new(set_payload());
        let count = std::cell::Cell::new(0);
        let log = execute_with_queries(
            &process,
            &[],
            &|_| {
                if count.get() == 0 {
                    payload.borrow_mut()["checks"][0]["query"] = json!("max(vector(1))");
                }
                count.set(count.get() + 1);
            },
            &|| Ok(payload.borrow().clone()),
            &|| false,
        )
        .unwrap();
        assert_eq!(handle.join().unwrap().len(), 6);
        assert!(log.contains("attempt 3"));
    }
    #[test]
    fn sets_reject_empty_deleted_invalid_and_incomplete_checks() {
        assert!(checks(&json!({"checks":[]})).is_err());
        assert!(
            checks(&json!({"checks":[{"query":"vector(1)","query_error":"deleted"}]})).is_err()
        );
        assert!(
            checks(&json!({"checks":[{"query":"metric{host=\"${DEPLOY_HOST_REGEX}\"}"}]})).is_err()
        );
        assert!(checks(&json!({"checks":[{"query":"vector(1)","min_results":0}]})).is_err());
        let check = Check {
            name: "Brokers".into(),
            query: "x".into(),
            min_results: 3,
        };
        let body = json!({"status":"success","data":{"resultType":"vector","result":[{"value":[1000,"1"]},{"value":[1000,"1"]}]}});
        assert!(
            !result_for_check(&body, &check, 1000.0, Some(120))
                .unwrap()
                .0
        );
        let check = Check {
            min_results: 1,
            ..check
        };
        assert!(
            result_for_check(&body, &check, 1000.0, Some(120))
                .unwrap()
                .0
        );
        assert!(
            !result_for_check(&body, &check, 1201.0, Some(120))
                .unwrap()
                .0
        );
        assert!(!result_for_check(&body, &check, 998.0, Some(120)).unwrap().0);
    }
}
