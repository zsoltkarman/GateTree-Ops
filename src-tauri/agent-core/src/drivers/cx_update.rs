use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::blocking::Client;
use serde_json::{Value, json};

use super::report::prepare_reports;
use crate::{AgentCredential, AgentJobSetStatus, AgentJobStatus};

pub use super::report::validate_report;

fn resolve_requester(
    set: &AgentJobSetStatus,
    job: &AgentJobStatus,
    process: &Value,
) -> Result<String, String> {
    let plan: Value = serde_yaml::from_str(&set.plan_definition)
        .map_err(|e| format!("CX Plan is invalid: {e}"))?;
    let plan_requester = plan.pointer("/inputs/cx/requester");
    let steps: Vec<&Value> = process
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|step| step["driver"] == "cx" && step["action"] == "update")
        .collect();
    let step = if steps.is_empty() {
        None
    } else {
        let mut jobs: Vec<_> = set
            .jobs
            .iter()
            .filter(|item| item.driver == "cx" && item.action == "update")
            .collect();
        jobs.sort_by_key(|item| item.sequence);
        let index = jobs
            .iter()
            .position(|item| item.id == job.id)
            .ok_or("CX update job does not match its frozen Process step.")?;
        Some(
            *steps
                .get(index)
                .ok_or("CX update job does not match its frozen Process step.")?,
        )
    };
    plan_requester.into_iter().chain(step.and_then(|step| step.get("requester")))
        .chain(process.pointer("/details/cx/requester"))
        .filter_map(Value::as_str).map(str::trim).find(|value| !value.is_empty()).map(str::to_owned)
        .ok_or_else(|| "CX update requires inputs.cx.requester, the CX update step requester, or details.cx.requester.".into())
}

pub fn update(
    set: &AgentJobSetStatus,
    job: &AgentJobStatus,
    credentials: &[AgentCredential],
    checkpoint: &dyn Fn(&str, &str) -> Result<(), String>,
    on_output: &dyn Fn(&str),
) -> Result<String, String> {
    let prepared = prepare_reports(set, job)?;
    let process: Value =
        serde_json::from_str(&set.process_definition).map_err(|e| e.to_string())?;
    let cx = &process["details"]["cx"];
    let required = |name: &str| {
        cx[name]
            .as_str()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("CX update requires details.cx.{name}."))
    };
    let base = required("url")?;
    let account = required("account")?;
    let requester = resolve_requester(set, job, &process)?;
    let ticket = job
        .target
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or("CX update requires a request number.")?;
    if !ticket
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return Err("Invalid CX request number.".into());
    }
    let credential = credentials
        .iter()
        .find(|c| c.reference.eq_ignore_ascii_case(account))
        .ok_or_else(|| format!("CX credential '{account}' is unavailable in Agent memory."))?;
    // Keep the existing CX driver's certificate compatibility, with an explicit override.
    let verify_tls = cx["verify_tls_certificate"].as_bool().unwrap_or(false);
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .danger_accept_invalid_certs(!verify_tls)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let mut progress = job.report_progress.clone();
    let mut log = String::new();
    for report in prepared {
        let mut operations: Vec<_> = report
            .attachments
            .into_iter()
            .enumerate()
            .map(|(i, a)| (format!("{}:attachment:{i}", report.key), Some(a), None))
            .collect();
        if let Some(comment) = report.comment {
            operations.push((format!("{}:comment", report.key), None, Some(comment)));
        }
        for (key, attachment, comment) in operations {
            if let Some(value) = progress.get(&key).filter(|v| v.as_str() != "retryable") {
                if value == "sending" {
                    return Err(format!(
                        "CX update '{key}' has an unconfirmed result. Check the request before retrying; it will not be sent twice automatically."
                    ));
                }
                log.push_str(&format!("Already confirmed CX update: {value}\n"));
                on_output(&log);
                continue;
            }
            let request = if let Some(a) = attachment {
                client
                    .post(format!(
                        "{}/{ticket}/attachments",
                        base.trim_end_matches('/')
                    ))
                    .query(&[("requester", requester.as_str())])
                    .json(
                        &json!([{"fileName":a.filename,"data":STANDARD.encode(a.bytes),
                        "contentType":a.mime,"requester":requester}]),
                    )
            } else {
                client
                    .post(format!("{}/{ticket}/comments", base.trim_end_matches('/')))
                    .json(&json!({"comment":comment.unwrap(),"requester":requester}))
            };
            checkpoint(&key, "sending")?;
            let response = request
                .basic_auth(&credential.username, Some(&credential.password))
                .header("Accept", "application/json")
                .send()
                .map_err(|e| {
                    format!(
                        "CX update result is unconfirmed: {e}. Check the request before retrying."
                    )
                })?;
            let status = response.status();
            if !status.is_success() || status.as_u16() == 202 {
                if status.is_client_error() && status.as_u16() != 408 {
                    checkpoint(&key, "retryable")?;
                }
                return Err(format!(
                    "CX update for {ticket} returned HTTP {}. No further report operations were sent. Check the request if the result is unconfirmed.",
                    status.as_u16()
                ));
            }
            let bytes = response.bytes().map_err(|e| {
                format!("CX response is unconfirmed: {e}. Check the request before retrying.")
            })?;
            let body = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice::<Value>(&bytes)
                    .map_err(|_| "CX response is unconfirmed: unreadable JSON. Check the request before retrying.")?
            };
            if !bytes.is_empty() && !body.is_object() && !body.is_array() {
                return Err("CX response is unconfirmed: expected a JSON object or array. Check the request before retrying.".into());
            }
            // CX may acknowledge writes with an empty body or without an object ID.
            // Persist the successful acknowledgement so retry never republishes it.
            let id = body
                .get("id")
                .or_else(|| body.get(0).and_then(|v| v.get("id")))
                .and_then(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .or_else(|| v.as_i64().map(|v| v.to_string()))
                })
                .filter(|id| !id.is_empty() && !matches!(id.as_str(), "sending" | "retryable"))
                .unwrap_or_else(|| format!("confirmed-http-{}", status.as_u16()));
            checkpoint(&key, &id)?;
            progress.insert(key, id.clone());
            log.push_str(&format!("CX {ticket}: confirmed report operation {id}.\n"));
            on_output(&log);
        }
    }
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::super::jira_update::tests::{context as jira_context, mock_bytes};
    use super::*;
    use std::{
        cell::RefCell,
        collections::BTreeMap,
        io::{Cursor, Read},
        time::Instant,
    };

    fn context(url: &str) -> (AgentJobSetStatus, AgentJobStatus, Vec<AgentCredential>) {
        let report = json!({"scope":"plan","comment_template":"CX {{request}} / {{issue}}: {{plan_name}}\n{{deployments}}",
            "deployment_template":"{{role}} · {{target}} / {{ip}} · Job #{{job_id}} · {{datetime}}",
            "attachments":[{"source":"plan_log","compression":"zip","filename_template":"{{request}}-plan-{{plan_number}}.zip"}]});
        let (mut set, mut job, mut credentials) = jira_context(url, report);
        job.driver = "cx".into();
        job.name = "Update CX".into();
        job.target = Some("REQ-100".into());
        set.jobs[3] = job.clone();
        set.process_definition = json!({"details":{"cx":{"url":format!("{url}/requests"),"account":"cx","requester":"operator+ops@example.test"}}}).to_string();
        credentials[0].reference = "cx".into();
        (set, job, credentials)
    }

    #[test]
    fn requester_follows_plan_then_the_matching_step_then_legacy_default() {
        let (mut set, mut job, _) = context("https://cx.test");
        let process = json!({"details":{"cx":{"requester":"legacy@example.test"}},"steps":[
            {"driver":"cx","action":"validate","requester":"not-an-update@example.test"},
            {"driver":"cx","action":"update","requester":"first@example.test"},
            {"driver":"cx","action":"update","requester":"second@example.test"}
        ]});
        let first = job.clone();
        job.id = "second-update".into();
        job.sequence += 1;
        set.jobs.push(job.clone());
        assert_eq!(
            resolve_requester(&set, &first, &process).unwrap(),
            "first@example.test"
        );
        assert_eq!(
            resolve_requester(&set, &job, &process).unwrap(),
            "second@example.test"
        );
        set.jobs[3].status = "disabled".into();
        assert_eq!(
            resolve_requester(&set, &job, &process).unwrap(),
            "second@example.test"
        );
        set.plan_definition = "inputs:\n  cx:\n    requester: plan@example.test\n".into();
        assert_eq!(
            resolve_requester(&set, &first, &process).unwrap(),
            "plan@example.test"
        );
        assert_eq!(
            resolve_requester(&set, &job, &process).unwrap(),
            "plan@example.test"
        );
        set.plan_definition = "{}".into();
        let mut process = process;
        process["steps"][2]["requester"] = json!("  ");
        assert_eq!(
            resolve_requester(&set, &job, &process).unwrap(),
            "legacy@example.test"
        );
    }

    #[test]
    fn step_requester_is_used_for_the_attachment_and_comment_http_requests() {
        let (url, server) = mock_bytes(vec![(200, json!([{"id":12}])), (201, json!({}))]);
        let (mut set, job, credentials) = context(&url);
        let mut process: Value = serde_json::from_str(&set.process_definition).unwrap();
        process["steps"] =
            json!([{"driver":"cx","action":"update","requester":"step+ops@example.test"}]);
        set.process_definition = process.to_string();
        update(&set, &job, &credentials, &|_, _| Ok(()), &|_| {}).unwrap();
        let requests = server.join().unwrap();
        assert!(
            requests[0]
                .0
                .contains("requester=step%2Bops%40example.test")
        );
        let attachment: Value = serde_json::from_slice(&requests[0].1).unwrap();
        let comment: Value = serde_json::from_slice(&requests[1].1).unwrap();
        assert_eq!(attachment[0]["requester"], "step+ops@example.test");
        assert_eq!(comment["requester"], "step+ops@example.test");
    }

    #[test]
    fn plan_summary_posts_one_base64_zip_then_one_comment_using_cx_contract() {
        let (url, server) = mock_bytes(vec![(200, json!([{"id":12}])), (201, json!({}))]);
        let (set, job, credentials) = context(&url);
        let ledger = RefCell::new(BTreeMap::new());
        update(
            &set,
            &job,
            &credentials,
            &|k, v| {
                ledger.borrow_mut().insert(k.to_owned(), v.to_owned());
                Ok(())
            },
            &|_| {},
        )
        .unwrap();
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        let (headers, bytes) = &requests[0];
        assert!(headers.starts_with(
            "POST /requests/REQ-100/attachments?requester=operator%2Bops%40example.test "
        ));
        let auth = format!(
            "authorization: basic {}",
            STANDARD.encode("user:test-token").to_lowercase()
        );
        assert!(headers.to_lowercase().contains(&auth));
        let body: Value = serde_json::from_slice(bytes).unwrap();
        assert_eq!(body[0]["requester"], "operator+ops@example.test");
        assert_eq!(body[0]["fileName"], "REQ-100-plan-9.zip");
        assert_eq!(body[0]["contentType"], "application/zip");
        let zip = STANDARD.decode(body[0]["data"].as_str().unwrap()).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(zip)).unwrap();
        assert_eq!(archive.len(), 1);
        let mut text = String::new();
        archive
            .by_name("REQ-100-plan-9.log")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert!(text.contains("deployment-output-0") && text.contains("deployment-output-2"));
        assert!(!text.contains("deployment-output-1"));
        assert!(
            requests[1]
                .0
                .starts_with("POST /requests/REQ-100/comments ")
        );
        let body: Value = serde_json::from_slice(&requests[1].1).unwrap();
        let comment = body["comment"].as_str().unwrap();
        assert!(comment.contains("CX REQ-100 / REQ-100: Kafka upgrade"));
        assert!(comment.contains("host-0 / 192.0.2.1 · Job #200"));
        assert!(comment.contains("host-2 / 192.0.2.3 · Job #202"));
        assert!(!comment.contains("host-1") && !comment.contains("{{"));
        assert_eq!(ledger.borrow()["plan:attachment:0"], "12");
        assert_eq!(ledger.borrow()["plan:comment"], "confirmed-http-201");
    }

    #[test]
    fn rejected_comment_retry_reuses_attachment_and_successful_empty_response() {
        let (url, server) = mock_bytes(vec![(200, json!({})), (403, json!({}))]);
        let (mut set, mut job, credentials) = context(&url);
        let ledger = RefCell::new(BTreeMap::new());
        let checkpoint = |k: &str, v: &str| {
            ledger.borrow_mut().insert(k.to_owned(), v.to_owned());
            Ok(())
        };
        assert!(
            update(&set, &job, &credentials, &checkpoint, &|_| {})
                .unwrap_err()
                .contains("403")
        );
        assert_eq!(server.join().unwrap().len(), 2);
        assert_eq!(ledger.borrow()["plan:comment"], "retryable");
        job.report_progress = ledger.borrow().clone();
        let (url, server) = mock_bytes(vec![(204, Value::Null)]);
        let mut process: Value = serde_json::from_str(&set.process_definition).unwrap();
        process["details"]["cx"]["url"] = json!(format!("{url}/requests"));
        set.process_definition = process.to_string();
        update(&set, &job, &credentials, &checkpoint, &|_| {}).unwrap();
        assert!(
            server.join().unwrap()[0]
                .0
                .starts_with("POST /requests/REQ-100/comments ")
        );
        job.report_progress = ledger.borrow().clone();
        update(&set, &job, &credentials, &checkpoint, &|_| {}).unwrap(); // Closed listener: no repeat writes.
    }

    #[test]
    fn uncertain_write_blocks_retry_and_missing_requester_fails_before_writes() {
        let (url, server) = mock_bytes(vec![(500, json!({}))]);
        let (set, mut job, credentials) = context(&url);
        let ledger = RefCell::new(BTreeMap::new());
        let checkpoint = |k: &str, v: &str| {
            ledger.borrow_mut().insert(k.to_owned(), v.to_owned());
            Ok(())
        };
        assert!(update(&set, &job, &credentials, &checkpoint, &|_| {}).is_err());
        assert_eq!(server.join().unwrap().len(), 1);
        job.report_progress = ledger.borrow().clone();
        assert!(
            update(&set, &job, &credentials, &checkpoint, &|_| {})
                .unwrap_err()
                .contains("unconfirmed")
        );
        let (mut set, job, credentials) = context("http://127.0.0.1:1");
        let mut process: Value = serde_json::from_str(&set.process_definition).unwrap();
        process["details"]["cx"]
            .as_object_mut()
            .unwrap()
            .remove("requester");
        set.process_definition = process.to_string();
        assert!(
            update(
                &set,
                &job,
                &credentials,
                &|_, _| panic!("must not checkpoint"),
                &|_| {}
            )
            .unwrap_err()
            .contains("requester")
        );
    }

    #[test]
    fn worker_persists_cx_report_checkpoints_and_retries_without_duplicate_posts() {
        use std::sync::{Arc, Mutex};
        let (url, server) = mock_bytes(vec![(200, json!({})), (201, json!({"id":"comment-1"}))]);
        let (set, job, credentials) = context(&url);
        let path = std::env::temp_dir().join(format!(
            "gatetree-ops-cx-report-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mut manager = crate::JobSetManager::load(path.clone()).unwrap();
        manager.job_sets.push(set);
        manager
            .credentials
            .insert("plan".into(), credentials.clone());
        let manager = Arc::new(Mutex::new(manager));
        let wait = || {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let set = manager.lock().unwrap().job_sets[0].clone();
                if set.status == "completed" {
                    return set;
                }
                assert!(
                    Instant::now() < deadline,
                    "{}: {}",
                    set.status,
                    set.jobs[3].log
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        crate::spawn_job_set_execution(manager.clone(), "plan".into(), credentials.clone());
        let done = wait();
        assert_eq!(server.join().unwrap().len(), 2);
        assert_eq!(done.jobs[3].report_progress.len(), 2);
        let loaded = crate::JobSetManager::load(path.clone()).unwrap();
        assert_eq!(
            loaded.job_sets[0].jobs[3].report_progress,
            done.jobs[3].report_progress
        );
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("test-token")
        );
        manager.lock().unwrap().job_sets[0].execution_revision = 3;
        manager
            .lock()
            .unwrap()
            .retry(&job.id, credentials.clone())
            .unwrap();
        crate::spawn_job_set_execution(manager.clone(), "plan".into(), credentials);
        assert_eq!(wait().jobs[3].status, "completed");
        std::fs::remove_file(path).unwrap();
    }
}
