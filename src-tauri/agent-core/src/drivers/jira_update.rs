use std::time::Duration;

use reqwest::blocking::{Client, multipart};
use serde_json::{Value, json};

pub use super::report::validate_report;
use super::report::*;
use crate::{AgentCredential, AgentJobSetStatus, AgentJobStatus};

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
    let jira = &process["details"]["jira"];
    let base = jira["url"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or("Jira URL is required.")?;
    let account = jira["account"]
        .as_str()
        .ok_or("Jira account is required.")?;
    let credential = credentials
        .iter()
        .find(|c| c.reference.eq_ignore_ascii_case(account))
        .ok_or_else(|| format!("Jira credential '{account}' is unavailable in Agent memory."))?;
    let issue = job.target.as_deref().ok_or("Jira issue is required.")?;
    if !issue.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') || issue.is_empty() {
        return Err("Invalid Jira issue key.".into());
    }
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let mut progress = job.report_progress.clone();
    let mut log = String::new();
    for report in prepared {
        // Attach logs first, then publish the comment. Retry keeps confirmed remote IDs.
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
            if let Some(value) = progress
                .get(&key)
                .filter(|value| value.as_str() != "retryable")
            {
                if value == "sending" {
                    return Err(format!(
                        "Jira update '{key}' has an unconfirmed result. Check the issue before restarting the report; it will not be sent twice automatically."
                    ));
                }
                log.push_str(&format!("Already confirmed Jira update: {value}\n"));
                on_output(&log);
                continue;
            }
            checkpoint(&key, "sending")?;
            let request = if let Some(attachment) = attachment {
                let part = multipart::Part::bytes(attachment.bytes)
                    .file_name(attachment.filename)
                    .mime_str(attachment.mime)
                    .map_err(|e| e.to_string())?;
                client
                    .post(format!(
                        "{}/rest/api/2/issue/{issue}/attachments",
                        base.trim_end_matches('/')
                    ))
                    .header("X-Atlassian-Token", "no-check")
                    .multipart(multipart::Form::new().part("file", part))
            } else {
                client
                    .post(format!(
                        "{}/rest/api/2/issue/{issue}/comment",
                        base.trim_end_matches('/')
                    ))
                    .json(&json!({"body":comment.unwrap()}))
            };
            let response = request
                .bearer_auth(&credential.password)
                .header("Accept", "application/json")
                .send()
                .map_err(|e| {
                    format!(
                        "Jira update result is unconfirmed: {e}. Check the issue before retrying."
                    )
                })?;
            let status = response.status();
            if !status.is_success() {
                // A rejected HTTP request can be retried after the cause is corrected.
                if status.is_client_error() && status.as_u16() != 408 {
                    checkpoint(&key, "retryable")?;
                }
                return Err(format!(
                    "Jira update failed for {issue} (HTTP {}). No further report operations were sent.",
                    status.as_u16()
                ));
            }
            let body: Value = response.json().map_err(|e| {
                format!("Jira response is unconfirmed: {e}. Check the issue before retrying.")
            })?;
            let id = body
                .get("id")
                .or_else(|| body.get(0).and_then(|v| v.get("id")))
                .and_then(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .or_else(|| v.as_i64().map(|v| v.to_string()))
                })
                .ok_or("Jira response did not contain an ID. Check the issue before retrying.")?;
            checkpoint(&key, &id)?;
            progress.insert(key, id.clone());
            log.push_str(&format!(
                "Jira {issue}: confirmed report operation ID {id}.\n"
            ));
            on_output(&log);
        }
    }
    Ok(log)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::io::Cursor;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Instant;

    fn mock(responses: Vec<(u16, Value)>) -> (String, thread::JoinHandle<Vec<(String, String)>>) {
        let (url, task) = mock_bytes(responses);
        (
            url,
            thread::spawn(move || {
                task.join()
                    .unwrap()
                    .into_iter()
                    .map(|(headers, body)| (headers, String::from_utf8(body).unwrap()))
                    .collect()
            }),
        )
    }

    pub(crate) fn mock_bytes(
        responses: Vec<(u16, Value)>,
    ) -> (String, thread::JoinHandle<Vec<(String, Vec<u8>)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                Instant::now() < deadline,
                                "Expected Jira request was not received"
                            );
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut headers = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse().unwrap();
                    }
                    headers.push_str(&line);
                }
                let mut data = vec![0; length];
                reader.read_exact(&mut data).unwrap();
                requests.push((headers, data));
                let body = body.to_string();
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            requests
        });
        (url, task)
    }

    pub(crate) fn context(
        url: &str,
        report: Value,
    ) -> (AgentJobSetStatus, AgentJobStatus, Vec<AgentCredential>) {
        let mut jobs = Vec::new();
        for (i, state) in ["completed", "disabled", "completed"].iter().enumerate() {
            jobs.push(json!({"id":format!("deploy-{i}"),"sequence":i+1,"name":format!("Kafka phase {i}"),"driver":"olam","action":"validate_deploy","target":format!("host-{i}"),"details":"","payload":json!({"playbook_name":"kafka"}).to_string(),"status":state,"log":format!("ok: [host-{i}] => {{\"msg\":[\"Ansible hostname: host-{i}\",\"Ansible ssh host: 192.0.2.{}\"]}}\nStarting OLAM deployment for host-{i}…\ndeployment-output-{i}",i+1),"validationJobId":100+i,"deploymentJobId":200+i,"finishedAt":"2026-10-19T10:00:00+00:00"}));
        }
        jobs.push(json!({"id":"update","sequence":4,"name":"Update Jira","driver":"jira","action":"update","target":"APP-1","details":"","payload":report.to_string(),"status":"running","log":""}));
        let set: AgentJobSetStatus = serde_json::from_value(json!({"id":"plan","planNumber":9,"name":"Kafka upgrade","processName":"Deploy","planDefinition":"{}","processDefinition":json!({"details":{"jira":{"url":url,"account":"jira"},"olam":{"url":"https://olam.test"}}}).to_string(),"status":"running","jobs":jobs})).unwrap();
        let job = set.jobs[3].clone();
        (
            set,
            job,
            vec![AgentCredential {
                reference: "jira".into(),
                username: "user".into(),
                password: "test-token".into(),
            }],
        )
    }

    fn report() -> Value {
        json!({"scope":"deployment","comment_template":"Following role has been deployed:\nRole name: {{role}}\nTarget: {{target}} / {{ip}}\nDatetime: {{datetime}}\nStatus: {{status}}\nJOB ID: {{job_id}}\n{{job_url}}", "attachments":[{"source":"deployment_log","filename_template":"{{issue}}-{{target}}-{{job_id}}.log"}]})
    }

    #[test]
    fn posts_configured_comments_and_multipart_logs_only_for_completed_deployments() {
        let (url, server) = mock(vec![
            (200, json!([{"id":"a1"}])),
            (201, json!({"id":"c1"})),
            (200, json!([{"id":"a2"}])),
            (201, json!({"id":"c2"})),
        ]);
        let (set, job, creds) = context(&url, report());
        let progress = RefCell::new(BTreeMap::new());
        update(
            &set,
            &job,
            &creds,
            &|k, v| {
                progress.borrow_mut().insert(k.to_string(), v.to_string());
                Ok(())
            },
            &|_| {},
        )
        .unwrap();
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 4);
        for (i, (headers, body)) in requests.iter().enumerate() {
            assert!(
                headers
                    .to_lowercase()
                    .contains("authorization: bearer test-token")
            );
            if i % 2 == 0 {
                assert!(headers.starts_with("POST /rest/api/2/issue/APP-1/attachments "));
                assert!(
                    headers
                        .to_lowercase()
                        .contains("x-atlassian-token: no-check")
                );
                assert!(
                    headers
                        .to_lowercase()
                        .contains("multipart/form-data; boundary=")
                );
                assert!(body.contains("name=\"file\""));
                assert!(body.contains(if i == 0 {
                    "APP-1-host-0-200.log"
                } else {
                    "APP-1-host-2-202.log"
                }));
                assert!(!body.contains("Ansible ssh host")); // Deployment log excludes validation.
            } else {
                let json: Value = serde_json::from_str(body).unwrap();
                let text = json["body"].as_str().unwrap();
                assert!(text.contains("Role name: kafka"));
                assert!(text.contains(if i == 1 {
                    "Target: host-0 / 192.0.2.1"
                } else {
                    "Target: host-2 / 192.0.2.3"
                }));
                assert!(text.contains("Datetime: 2026-10-19T10:00:00+00:00"));
            }
            assert!(!body.contains("host-1"));
            assert!(!body.contains("test-token"));
            assert!(!body.contains("{{"));
        }
        assert_eq!(progress.borrow().len(), 4);
        assert!(progress.borrow().values().all(|v| v != "sending"));
    }

    #[test]
    fn retry_reuses_uploaded_attachment_and_confirmed_comment_ids() {
        let (url, server) = mock(vec![
            (200, json!([{"id":"a1"}])),
            (403, json!({"error":"forbidden"})),
        ]);
        let (mut set, mut job, creds) = context(&url, report());
        set.jobs[2].status = "disabled".into();
        let progress = RefCell::new(BTreeMap::new());
        let checkpoint = |k: &str, v: &str| {
            progress.borrow_mut().insert(k.to_string(), v.to_string());
            Ok(())
        };
        assert!(
            update(&set, &job, &creds, &checkpoint, &|_| {})
                .unwrap_err()
                .contains("403")
        );
        assert_eq!(server.join().unwrap().len(), 2);
        job.report_progress =
            serde_json::from_str(&serde_json::to_string(&*progress.borrow()).unwrap()).unwrap();
        assert_eq!(job.report_progress["deploy-0:attachment:0"], "a1");
        let (url, server) = mock(vec![(201, json!({"id":"c1"}))]);
        let mut process: Value = serde_json::from_str(&set.process_definition).unwrap();
        process["details"]["jira"]["url"] = json!(url);
        set.process_definition = process.to_string();
        update(&set, &job, &creds, &checkpoint, &|_| {}).unwrap();
        assert!(
            server.join().unwrap()[0]
                .0
                .starts_with("POST /rest/api/2/issue/APP-1/comment ")
        );
        job.report_progress = progress.borrow().clone();
        // Both remote writes already succeeded. An unreachable endpoint proves no repeat POST.
        set.process_definition =
            r#"{"details":{"jira":{"url":"http://127.0.0.1:1","account":"jira"}}}"#.into();
        update(&set, &job, &creds, &checkpoint, &|_| {}).unwrap();
        job.report_progress
            .insert("deploy-0:comment".into(), "sending".into());
        assert!(
            update(&set, &job, &creds, &checkpoint, &|_| {})
                .unwrap_err()
                .contains("unconfirmed")
        );
    }

    #[test]
    fn plan_report_uses_explicit_summary_template_and_full_plan_log() {
        let (url, server) = mock(vec![(200, json!([{"id":"a1"}])), (201, json!({"id":"c1"}))]);
        let report = json!({"scope":"plan","comment_template":"Plan {{plan_number}}: {{plan_name}}\n{{deployments}}","deployment_template":"{{step}}: {{target}} — {{job_id}}","attachments":[{"source":"plan_log","filename_template":"plan-{{plan_number}}.log"}]});
        let (set, job, creds) = context(&url, report);
        update(&set, &job, &creds, &|_, _| Ok(()), &|_| {}).unwrap();
        let requests = server.join().unwrap();
        assert!(requests[0].1.contains("deployment-output-0"));
        assert!(requests[0].1.contains("deployment-output-2"));
        let body: Value = serde_json::from_str(&requests[1].1).unwrap();
        let text = body["body"].as_str().unwrap();
        assert!(text.contains("Plan 9: Kafka upgrade"));
        assert!(text.contains("Kafka phase 0: host-0 — 200"));
        assert!(text.contains("Kafka phase 2: host-2 — 202"));
        assert!(!text.contains("host-1"));
    }

    #[test]
    fn worker_executes_and_persists_report_checkpoints_across_retry() {
        use std::sync::{Arc, Mutex};
        let (url, server) = mock(vec![
            (200, json!([{"id":"a1"}])),
            (201, json!({"id":"c1"})),
            (200, json!([{"id":"a2"}])),
            (201, json!({"id":"c2"})),
        ]);
        let (set, job, creds) = context(&url, report());
        let path = std::env::temp_dir().join(format!(
            "gatetree-ops-jira-report-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mut manager = crate::JobSetManager::load(path.clone()).unwrap();
        manager.job_sets.push(set);
        manager.credentials.insert("plan".into(), creds.clone());
        let manager = Arc::new(Mutex::new(manager));
        crate::spawn_job_set_execution(manager.clone(), "plan".into(), creds.clone());
        let wait = || {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let set = manager.lock().unwrap().job_sets[0].clone();
                if set.status == "completed" {
                    break set;
                }
                assert!(
                    Instant::now() < deadline,
                    "{}: {}",
                    set.status,
                    set.jobs[3].log
                );
                thread::sleep(Duration::from_millis(5));
            }
        };
        let done = wait();
        assert_eq!(server.join().unwrap().len(), 4);
        assert_eq!(done.jobs[3].report_progress.len(), 4);
        assert!(done.jobs[3].finished_at.is_some());
        let persisted = crate::JobSetManager::load(path.clone()).unwrap();
        assert_eq!(
            persisted.job_sets[0].jobs[3].report_progress,
            done.jobs[3].report_progress
        );
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("test-token")
        );
        manager.lock().unwrap().job_sets[0].execution_revision = 3;
        let retried = manager
            .lock()
            .unwrap()
            .retry(&job.id, creds.clone())
            .unwrap();
        assert_eq!(retried.jobs[3].report_progress.len(), 4);
        crate::spawn_job_set_execution(manager.clone(), "plan".into(), creds);
        let done = wait(); // Closed mock listener proves retry did not send a second report.
        assert_eq!(done.jobs[3].status, "completed");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn zipped_plan_report_uploads_one_real_compressed_utf8_log_and_one_comment() {
        let (url, server) = mock_bytes(vec![
            (200, json!([{"id":"zip1"}])),
            (201, json!({"id":"c1"})),
        ]);
        let report = json!({"scope":"plan","comment_template":"Plan {{plan_number}}\n{{deployments}}","deployment_template":"{{target}}: {{job_id}}", "attachments":[{"source":"plan_log","compression":"zip","filename_template":"{{issue}}-plan-{{plan_number}}.zip"}]});
        let (mut set, job, creds) = context(&url, report);
        let repeated = "árvíztűrő tükörfúrógép: Kafka deployment output\n".repeat(5000);
        set.jobs[0].log.push_str(&repeated);
        update(&set, &job, &creds, &|_, _| Ok(()), &|_| {}).unwrap();
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        let (headers, body) = &requests[0];
        assert!(headers.starts_with("POST /rest/api/2/issue/APP-1/attachments "));
        let boundary = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-type: multipart/form-data; boundary=")
                    .map(str::to_owned)
            })
            .unwrap();
        let split = body
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap();
        let part_header = std::str::from_utf8(&body[..split]).unwrap();
        assert!(part_header.contains("filename=\"APP-1-plan-9.zip\""));
        assert!(part_header.contains("application/zip"));
        let suffix = format!("\r\n--{boundary}--\r\n");
        assert!(body.ends_with(suffix.as_bytes()));
        let zipped = &body[split + 4..body.len() - suffix.len()];
        assert!(zipped.len() < repeated.len() / 10);
        let mut archive = zip::ZipArchive::new(Cursor::new(zipped)).unwrap();
        assert_eq!(archive.len(), 1);
        let mut file = archive.by_name("APP-1-plan-9.log").unwrap();
        assert_eq!(file.compression(), zip::CompressionMethod::Deflated);
        let mut log = String::new();
        file.read_to_string(&mut log).unwrap();
        assert!(log.contains(&repeated));
        assert!(log.contains("deployment-output-2"));
        assert!(!log.contains("deployment-output-1"));
        let comment: Value = serde_json::from_slice(&requests[1].1).unwrap();
        assert!(comment["body"].as_str().unwrap().contains("host-0: 200"));
        assert!(comment["body"].as_str().unwrap().contains("host-2: 202"));
    }

    #[test]
    fn compression_requires_supported_format_and_zip_filename() {
        for (compression, name) in [("gzip", "plan.zip"), ("zip", "plan.log")] {
            assert!(validate_report(&json!({"scope":"plan","attachments":[{"source":"plan_log","compression":compression,"filename_template":name}]})).is_err());
        }
        let attachment =
            prepare_attachment("plan.log".into(), "UTF-8: árvíz".into(), &Compression::None)
                .unwrap();
        assert_eq!(attachment.bytes, "UTF-8: árvíz".as_bytes());
        assert_eq!(attachment.mime, "text/plain; charset=utf-8");
        // A single deployment uses the same one-entry ZIP format.
        let attachment =
            prepare_attachment("single.zip".into(), "single job".into(), &Compression::Zip)
                .unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(attachment.bytes)).unwrap();
        let mut text = String::new();
        archive
            .by_name("single.log")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "single job");
    }

    #[test]
    fn invalid_templates_and_missing_results_fail_before_http_or_checkpoint() {
        for value in [
            json!({"scope":"deployment"}),
            json!({"scope":"invalid","comment_template":"text"}),
            json!({"scope":"deployment","comment_template":"{{typo}}"}),
            json!({"scope":"deployment","comment_template":"{{job_id"}),
            json!({"scope":"plan","comment_template":"{{ deployments }}"}),
            json!({"scope":"deployment","attachments":[{"source":"plan_log","filename_template":"x.log"}]}),
        ] {
            assert!(validate_report(&value).is_err(), "{value}");
        }
        let (mut set, job, creds) = context("http://127.0.0.1:1", report());
        for j in &mut set.jobs[..3] {
            j.status = "disabled".into();
        }
        assert!(
            update(
                &set,
                &job,
                &creds,
                &|_, _| panic!("Must not write checkpoint"),
                &|_| {}
            )
            .unwrap_err()
            .contains("No completed")
        );
        let (set, job, creds) = context("http://127.0.0.1:1", report());
        assert!(
            update(&set, &job, &creds, &|_, _| Err("Stopped".into()), &|_| {})
                .unwrap_err()
                .contains("Stopped")
        );
    }
}
