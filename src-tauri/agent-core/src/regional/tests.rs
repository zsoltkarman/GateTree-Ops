use super::*;
use crate::*;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const HOSTS: [&str; 7] = ["a1", "a2", "b1", "b2", "c1", "c2", "d1"];

fn fixture(url: &str, automatic: bool) -> AgentJobSetSubmission {
    let mut jobs = vec![
        json!({"id":"before","sequence":0,"name":"Before","driver":"olam","action":"validate","target":url,"details":""}),
    ];
    for host in HOSTS {
        jobs.push(json!({"id":format!("deploy-{host}"),"sequence":jobs.len(),"name":"Deploy","driver":"olam","action":"deploy","target":host,"details":"","payload":json!({"ansible_limit":[host],"playbook_name":"exporter"}).to_string()}));
        jobs.push(json!({"id":format!("query-{host}"),"sequence":jobs.len(),"name":"Thanos Query","driver":"thanos","action":"query","target":host,"details":"","payload":json!({"query":host}).to_string()}));
    }
    jobs.push(json!({"id":"after","sequence":jobs.len(),"name":"After","driver":"olam","action":"validate","target":url,"details":""}));
    serde_json::from_value(json!({
        "approveAll":automatic,"id":"parallel","name":"Regional deployment","processName":"Deploy","planNumber":1,
        "planDefinition":json!({"inputs":{"olam":{"account":"deploy","parallel_regions":3,"targets":{"roles":["exporter"],"job_mode":"per_host","hosts":HOSTS,"sites":["a","b","c","d"],"host_sites":{"a1":"a","a2":"a","b1":"b","b2":"b","c1":"c","c2":"c","d1":"d"}}}}}).to_string(),
        "processDefinition":json!({"details":{"olam":{"url":url,"account":"api","payload_defaults":{"ENVIRONMENT":"test"},"deployment_template_id":2,"deployment_template_job":"Deploy"},"thanos":{"url":url,"timeout_seconds":1,"interval_seconds":1,"auth":"none"}}}).to_string(),
        "jobs":jobs,"credentials":[{"reference":"api","username":"test","password":"test"},{"reference":"deploy","username":"test","password":"test"}]
    })).unwrap()
}

fn manager(submission: AgentJobSetSubmission) -> Arc<Mutex<JobSetManager>> {
    let path = env::temp_dir().join(format!("gt-regional-{}.json", uuid::Uuid::new_v4()));
    let mut manager = JobSetManager::load(path).unwrap();
    let id = submission.id.clone();
    let credentials = submission.credentials.clone();
    manager.submit(submission, false).unwrap();
    manager.credentials.insert(id, credentials);
    Arc::new(Mutex::new(manager))
}

fn wait_for(
    manager: &Arc<Mutex<JobSetManager>>,
    condition: impl Fn(&AgentJobSetStatus) -> bool,
) -> AgentJobSetStatus {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let set = manager.lock().unwrap().job_sets[0].clone();
        if condition(&set) {
            return set;
        }
        assert!(
            Instant::now() < deadline,
            "timed out: {} {:?}",
            set.status,
            set.jobs
                .iter()
                .map(|j| (&j.id, &j.status, &j.log))
                .collect::<Vec<_>>()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

struct Server {
    url: String,
    events: Arc<Mutex<Vec<String>>>,
    peak: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new(fail_query: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let peak = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let ids = Arc::new(Mutex::new(HashMap::<usize, String>::new()));
        let (observed, maximum, running, stopped, hosts) =
            (events.clone(), peak.clone(), active, stop.clone(), ids);
        let worker = thread::spawn(move || {
            let mut handlers = Vec::new();
            while !stopped.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                let (observed, maximum, running, hosts) = (
                    observed.clone(),
                    maximum.clone(),
                    running.clone(),
                    hosts.clone(),
                );
                handlers.push(thread::spawn(move || {
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut request = String::new(); reader.read_line(&mut request).unwrap();
                    let mut length = 0;
                    loop {
                        let mut header = String::new(); reader.read_line(&mut header).unwrap();
                        if header == "\r\n" { break; }
                        if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") { length = value.trim().parse().unwrap(); }
                    }
                    let mut bytes = vec![0;length]; reader.read_exact(&mut bytes).unwrap();
                    let path = request.split_whitespace().nth(1).unwrap();
                    let mut status = "200 OK";
                    let body = if path == "/api/v2/me/" {
                        observed.lock().unwrap().push("barrier".into()); "{}".into()
                    } else if path.ends_with("/launch/") {
                        let body: Value = serde_json::from_slice(&bytes).unwrap();
                        let vars: Value = serde_json::from_str(body["extra_vars"].as_str().unwrap()).unwrap();
                        let host = vars["ansible_limit"][0].as_str().unwrap().to_string();
                        observed.lock().unwrap().push(format!("deploy-{host}"));
                        let mut hosts = hosts.lock().unwrap(); let id = hosts.len()+1; hosts.insert(id,host);
                        json!({"id":id}).to_string()
                    } else if path.starts_with("/api/v2/job_templates/") { json!({"name":"Deploy"}).to_string()
                    } else if path.contains("/stdout/") { "ok: [test-host]".into()
                    } else if path.starts_with("/api/v2/jobs/") {
                        let count = running.fetch_add(1,Ordering::SeqCst)+1; maximum.fetch_max(count,Ordering::SeqCst);
                        // Keep all three admitted lanes in flight long enough to
                        // prove overlap, and failure drains the slower two lanes.
                        let id:usize = path.split('/').nth(4).unwrap().parse().unwrap();
                        let host = hosts.lock().unwrap()[&id].clone();
                        thread::sleep(Duration::from_millis(if host == "a1" { 120 } else { 350 }));
                        running.fetch_sub(1,Ordering::SeqCst);
                        json!({"status":"successful"}).to_string()
                    } else if path.starts_with("/api/v1/query?") {
                        let parsed = reqwest::Url::parse(&format!("http://test{path}")).unwrap();
                        let host = parsed.query_pairs().find(|(key,_)| key == "query").unwrap().1.to_string();
                        observed.lock().unwrap().push(format!("query-{host}"));
                        if fail_query && host == "a1" { status = "401 Unauthorized"; "{}".into() }
                        else { json!({"status":"success","data":{"resultType":"vector","result":[{"metric":{},"value":[1,"1"]}]}}).to_string() }
                    } else { panic!("unexpected request: {request}"); };
                    let _ = write!(stream,"HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                }));
            }
            for handler in handlers {
                handler.join().unwrap();
            }
        });
        Self {
            url,
            events,
            peak,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.worker.take().unwrap().join();
    }
}

#[test]
fn parallel_regions_overlap_with_a_cap_and_preserve_deploy_query_order_and_barriers() {
    let server = Server::new(false);
    let submission = fixture(&server.url, true);
    let credentials = submission.credentials.clone();
    let manager = manager(submission);
    spawn_job_set_execution(manager.clone(), "parallel".into(), credentials);
    let set = wait_for(&manager, |set| set.status == "completed");
    assert!(set.jobs.iter().all(done));
    assert_eq!(server.peak.load(Ordering::SeqCst), 3);
    let events = server.events.lock().unwrap().clone();
    assert_eq!(events.first().unwrap(), "barrier");
    assert_eq!(events.last().unwrap(), "barrier");
    let position = |event: &str| events.iter().position(|item| item == event).unwrap();
    for host in HOSTS {
        assert!(position(&format!("deploy-{host}")) < position(&format!("query-{host}")));
    }
    for region in ["a", "b", "c"] {
        assert!(position(&format!("query-{region}1")) < position(&format!("deploy-{region}2")));
    }
    assert!(position("deploy-d1") > position("query-a2"));
    fs::remove_file(&manager.lock().unwrap().path).unwrap();
}

#[test]
fn failed_region_stops_its_hosts_while_healthy_and_new_regions_finish() {
    let server = Server::new(true);
    let submission = fixture(&server.url, true);
    let credentials = submission.credentials.clone();
    let manager = manager(submission);
    spawn_job_set_execution(manager.clone(), "parallel".into(), credentials);
    let set = wait_for(&manager, |set| set.status == "failed");
    let events = server.events.lock().unwrap().clone();
    for host in ["a1", "b1", "b2", "c1", "c2", "d1"] {
        assert!(events.contains(&format!("deploy-{host}")));
        assert!(events.contains(&format!("query-{host}")));
    }
    assert!(!events.contains(&"deploy-a2".into()));
    assert_eq!(set.jobs[2].status, "failed");
    assert_eq!(set.jobs[3].status, "stopped");
    assert_eq!(set.jobs[4].status, "stopped");
    for job in &set.jobs[5..15] {
        assert_eq!(job.status, "completed", "{}", job.id);
    }
    assert!(server.peak.load(Ordering::SeqCst) <= 3);
    assert_eq!(events.iter().filter(|item| *item == "barrier").count(), 1);
    assert_eq!(set.jobs.last().unwrap().status, "queued");
    fs::remove_file(&manager.lock().unwrap().path).unwrap();
}

#[test]
fn approving_another_region_does_not_cancel_workers_and_stop_stops_all_active_jobs() {
    let server = Server::new(false);
    let submission = fixture(&server.url, false);
    let credentials = submission.credentials.clone();
    let manager = manager(submission);
    spawn_job_set_execution(manager.clone(), "parallel".into(), credentials.clone());
    wait_for(&manager, |set| {
        set.jobs
            .iter()
            .filter(|job| job.status == "awaiting_approval")
            .count()
            == 3
    });
    let generation = manager.lock().unwrap().execution_generations["parallel"];
    for host in ["a1", "b1"] {
        manager
            .lock()
            .unwrap()
            .approve(&format!("deploy-{host}"))
            .unwrap();
        spawn_job_set_execution(manager.clone(), "parallel".into(), credentials.clone());
    }
    assert_eq!(
        manager.lock().unwrap().execution_generations["parallel"],
        generation
    );
    let set = manager.lock().unwrap().stop("parallel").unwrap();
    assert_eq!(set.status, "stopped");
    assert_eq!(
        set.jobs
            .iter()
            .filter(|job| job.status == "stopped")
            .count(),
        3
    );
    assert!(
        !set.jobs
            .iter()
            .any(|job| matches!(job.status.as_str(), "running" | "awaiting_approval"))
    );
    thread::sleep(Duration::from_millis(500));
    assert_eq!(manager.lock().unwrap().job_sets[0].status, "stopped");
    fs::remove_file(&manager.lock().unwrap().path).unwrap();
}

#[test]
fn retry_is_lane_scoped_preserves_completed_work_and_requeues_stopped_monitors() {
    let manager = manager(fixture("http://unused.test", true));
    {
        let mut manager = manager.lock().unwrap();
        let set = &mut manager.job_sets[0];
        set.status = "failed".into();
        set.jobs[0].status = "completed".into();
        for job in &mut set.jobs[1..] {
            job.status = if job.id == "query-a1" {
                "failed"
            } else if job.id == "deploy-b1" {
                "stopped"
            } else {
                "completed"
            }
            .into();
        }
        set.jobs[5].deployment_job_id = Some(42);
        let set = manager.retry("query-a1", vec![]).unwrap();
        assert_eq!(set.jobs[1].status, "completed");
        assert_eq!(set.jobs[3].status, "completed");
        assert_eq!(set.jobs[5].status, "queued");
        assert_eq!(set.jobs[5].deployment_job_id, Some(42));
        assert_eq!(set.jobs[2].status, "queued");
    }
    fs::remove_file(&manager.lock().unwrap().path).unwrap();
}

#[test]
fn malformed_parallel_configuration_is_rejected_before_activation() {
    let mut submission = fixture("http://unused.test", true);
    let mut value: Value = serde_json::from_str(&submission.plan_definition).unwrap();
    for limit in [json!(0), json!(17), json!(1.5), json!("3")] {
        value["inputs"]["olam"]["parallel_regions"] = limit;
        assert!(Schedule::parse(&value.to_string()).is_err());
    }
    value["inputs"]["olam"]["parallel_regions"] = json!(3);
    value["inputs"]["olam"]["targets"]["host_sites"]
        .as_object_mut()
        .unwrap()
        .remove("a1");
    submission.plan_definition = value.to_string();
    let path = env::temp_dir().join(format!("gt-reject-{}.json", uuid::Uuid::new_v4()));
    let mut manager = JobSetManager::load(path).unwrap();
    assert!(manager.submit(submission, false).is_err());
    assert!(manager.job_sets.is_empty());
    assert_eq!(Schedule::parse("{}").unwrap().limit, 1);
}

#[test]
fn multiple_failed_regions_can_be_retried_individually_without_redeploying_completed_hosts() {
    let server = Server::new(false);
    let submission = fixture(&server.url, true);
    let credentials = submission.credentials.clone();
    let manager = manager(submission);
    {
        let mut manager = manager.lock().unwrap();
        let set = &mut manager.job_sets[0];
        set.status = "failed".into();
        for job in &mut set.jobs {
            job.status = if matches!(job.id.as_str(), "query-a1" | "query-b1") {
                "failed"
            } else {
                "completed"
            }
            .into();
        }
        set.jobs.last_mut().unwrap().status = "queued".into();
        manager.retry("query-a1", vec![]).unwrap();
    }
    spawn_job_set_execution(manager.clone(), "parallel".into(), credentials.clone());
    let set = wait_for(&manager, |set| set.status == "failed");
    assert_eq!(set.jobs[2].status, "completed");
    assert_eq!(set.jobs[6].status, "failed");
    manager.lock().unwrap().retry("query-b1", vec![]).unwrap();
    spawn_job_set_execution(manager.clone(), "parallel".into(), credentials);
    wait_for(&manager, |set| set.status == "completed");
    assert_eq!(
        *server.events.lock().unwrap(),
        vec!["query-a1", "query-b1", "barrier"]
    );
    fs::remove_file(&manager.lock().unwrap().path).unwrap();
}

#[test]
fn a_failure_blocks_approval_only_in_its_own_region() {
    let manager = manager(fixture("http://unused.test", true));
    {
        let mut manager = manager.lock().unwrap();
        let set = &mut manager.job_sets[0];
        set.jobs[1].status = "failed".into();
        set.jobs[3].status = "awaiting_approval".into();
        set.jobs[3].phase = Some("deployment".into());
        apply_automatic_approval(set, 3);
        assert_eq!(set.jobs[3].status, "awaiting_approval");
        set.jobs[5].status = "awaiting_approval".into();
        set.jobs[5].phase = Some("deployment".into());
        apply_automatic_approval(set, 5);
        assert_eq!(set.jobs[5].status, "running");
        assert!(manager.approve("deploy-a2").is_err());
        manager.job_sets[0].jobs[5].status = "awaiting_approval".into();
        assert!(manager.approve("deploy-b1").is_ok());
    }
    fs::remove_file(&manager.lock().unwrap().path).unwrap();
}

#[test]
fn healthy_regions_can_wait_for_approval_after_another_region_failed() {
    let server = Server::new(true);
    let submission = fixture(&server.url, false);
    let credentials = submission.credentials.clone();
    let manager = manager(submission);
    spawn_job_set_execution(manager.clone(), "parallel".into(), credentials.clone());
    wait_for(&manager, |set| {
        set.jobs
            .iter()
            .filter(|job| job.status == "awaiting_approval")
            .count()
            == 3
    });
    manager.lock().unwrap().approve("deploy-a1").unwrap();
    spawn_job_set_execution(manager.clone(), "parallel".into(), credentials);
    let set = wait_for(&manager, |set| {
        set.jobs[2].status == "failed" && set.jobs[13].status == "awaiting_approval"
    });
    assert_eq!(set.status, "awaiting_approval");
    assert_eq!(set.jobs[3].status, "stopped");
    assert_eq!(set.jobs[5].status, "awaiting_approval");
    assert_eq!(set.jobs[9].status, "awaiting_approval");
    manager
        .lock()
        .unwrap()
        .set_approve_all("parallel", true)
        .unwrap();
    let set = wait_for(&manager, |set| set.status == "failed");
    for job in &set.jobs[5..15] {
        assert_eq!(job.status, "completed", "{}", job.id);
    }
    assert_eq!(set.jobs.last().unwrap().status, "queued");
    fs::remove_file(&manager.lock().unwrap().path).unwrap();
}

#[test]
fn restarting_deploy_after_failed_thanos_retries_only_that_pair_with_fresh_external_ids() {
    for limit in [1, 3] {
        let server = Server::new(false);
        let mut submission = fixture(&server.url, true);
        let mut plan: Value = serde_json::from_str(&submission.plan_definition).unwrap();
        plan["inputs"]["olam"]["parallel_regions"] = json!(limit);
        if limit == 1 {
            plan["inputs"]["olam"]["targets"]["host_sites"] = json!({});
        }
        submission.plan_definition = plan.to_string();
        let credentials = submission.credentials.clone();
        let manager = manager(submission);
        let others;
        {
            let mut manager = manager.lock().unwrap();
            let set = &mut manager.job_sets[0];
            set.status = "failed".into();
            for job in &mut set.jobs {
                job.status = "completed".into();
            }
            set.jobs[1].deployment_job_id = Some(999);
            set.jobs[1].validation_job_id = Some(888);
            set.jobs[2].status = "failed".into();
            set.jobs[2].log = "Wrong version".into();
            set.jobs[3].status = "stopped".into();
            set.jobs[4].status = "stopped".into();
            set.jobs.last_mut().unwrap().status = "queued".into();
            others = serde_json::to_value(
                set.jobs
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| ![1, 2].contains(i))
                    .map(|(_, job)| job)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            let retried = manager.retry("deploy-a1", vec![]).unwrap();
            assert_eq!(
                retried.retry_scope,
                Some(vec!["deploy-a1".into(), "query-a1".into()])
            );
            assert_eq!(retried.jobs[1].deployment_job_id, None);
            assert_eq!(retried.jobs[1].validation_job_id, None);
            assert_eq!(retried.jobs[1].status, "queued");
            assert_eq!(retried.jobs[2].status, "queued");
        }
        spawn_job_set_execution(manager.clone(), "parallel".into(), credentials);
        let set = wait_for(&manager, |set| set.status == "stopped");
        assert_eq!(set.jobs[1].status, "completed");
        assert_eq!(set.jobs[2].status, "completed");
        assert_ne!(set.jobs[1].deployment_job_id, Some(999));
        assert!(set.retry_scope.is_none());
        assert_eq!(
            serde_json::to_value(
                set.jobs
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| ![1, 2].contains(i))
                    .map(|(_, job)| job)
                    .collect::<Vec<_>>()
            )
            .unwrap(),
            others
        );
        assert_eq!(
            *server.events.lock().unwrap(),
            vec!["deploy-a1", "query-a1"]
        );
        fs::remove_file(&manager.lock().unwrap().path).unwrap();
    }
}

#[test]
fn pair_retry_preserves_healthy_running_lanes_and_their_execution_generation() {
    let manager = manager(fixture("http://unused.test", false));
    {
        let mut manager = manager.lock().unwrap();
        let set = &mut manager.job_sets[0];
        set.status = "running".into();
        set.jobs[0].status = "completed".into();
        set.jobs[1].status = "completed".into();
        set.jobs[2].status = "failed".into();
        set.jobs[5].status = "running".into();
        set.jobs[5].deployment_job_id = Some(77);
        let healthy = serde_json::to_value(&set.jobs[5..]).unwrap();
        let retried = manager.retry("deploy-a1", vec![]).unwrap();
        assert!(retried.retry_scope.is_none());
        assert_eq!(serde_json::to_value(&retried.jobs[5..]).unwrap(), healthy);
        manager.execution_generations.insert("parallel".into(), 42);
        manager.parallel_coordinators.insert("parallel".into(), 42);
    }
    spawn_job_set_execution(manager.clone(), "parallel".into(), vec![]);
    assert_eq!(
        manager.lock().unwrap().execution_generations["parallel"],
        42
    );
    fs::remove_file(&manager.lock().unwrap().path).unwrap();
}

#[test]
fn pair_scope_survives_agent_restart_and_manual_deploy_approval_is_preserved() {
    let server = Server::new(false);
    let submission = fixture(&server.url, false);
    let credentials = submission.credentials.clone();
    let manager = manager(submission);
    {
        let mut manager = manager.lock().unwrap();
        let set = &mut manager.job_sets[0];
        set.status = "failed".into();
        for job in &mut set.jobs {
            job.status = "completed".into();
        }
        set.jobs[2].status = "failed".into();
        set.jobs.last_mut().unwrap().status = "queued".into();
        manager.retry("deploy-a1", vec![]).unwrap();
    }
    let path = manager.lock().unwrap().path.clone();
    let mut restored = JobSetManager::load(path.clone()).unwrap();
    assert_eq!(
        restored.job_sets[0].retry_scope,
        Some(vec!["deploy-a1".into(), "query-a1".into()])
    );
    restored.retry("deploy-a1", credentials.clone()).unwrap();
    let manager = Arc::new(Mutex::new(restored));
    spawn_job_set_execution(manager.clone(), "parallel".into(), credentials.clone());
    wait_for(&manager, |set| set.status == "awaiting_approval");
    assert!(
        server.events.lock().unwrap().is_empty(),
        "a new deployment must wait for approval"
    );
    manager.lock().unwrap().approve("deploy-a1").unwrap();
    spawn_job_set_execution(manager.clone(), "parallel".into(), credentials);
    wait_for(&manager, |set| set.status == "stopped");
    assert_eq!(
        *server.events.lock().unwrap(),
        vec!["deploy-a1", "query-a1"]
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn combined_deployment_pair_revalidates_and_waits_for_approval_before_deploying() {
    let server = Server::new(false);
    let mut submission = fixture(&server.url, false);
    let mut process: Value = serde_json::from_str(&submission.process_definition).unwrap();
    process["details"]["olam"]["validation_template_id"] = json!(2);
    process["details"]["olam"]["validation_template_job"] = json!("Deploy");
    submission.process_definition = process.to_string();
    let credentials = submission.credentials.clone();
    let manager = manager(submission);
    {
        let mut manager = manager.lock().unwrap();
        let set = &mut manager.job_sets[0];
        set.status = "failed".into();
        for job in &mut set.jobs {
            job.status = "completed".into();
        }
        set.jobs[1].action = "validate_deploy".into();
        set.jobs[1].phase = Some("deployment".into());
        set.jobs[1].validation_job_id = Some(500);
        set.jobs[1].deployment_job_id = Some(501);
        set.jobs[2].status = "failed".into();
        set.jobs.last_mut().unwrap().status = "queued".into();
        let retried = manager.retry("deploy-a1", vec![]).unwrap();
        assert_eq!(retried.jobs[1].phase.as_deref(), Some("validation"));
    }
    spawn_job_set_execution(manager.clone(), "parallel".into(), credentials.clone());
    let set = wait_for(&manager, |set| set.status == "awaiting_approval");
    assert!(set.jobs[1].validation_job_id.is_some());
    assert_ne!(set.jobs[1].validation_job_id, Some(500));
    assert!(set.jobs[1].deployment_job_id.is_none());
    assert_eq!(*server.events.lock().unwrap(), vec!["deploy-a1"]);
    manager.lock().unwrap().approve("deploy-a1").unwrap();
    spawn_job_set_execution(manager.clone(), "parallel".into(), credentials);
    let set = wait_for(&manager, |set| set.status == "stopped");
    assert_eq!(set.jobs[2].status, "completed");
    assert_eq!(
        *server.events.lock().unwrap(),
        vec!["deploy-a1", "deploy-a1", "query-a1"]
    );
    fs::remove_file(&manager.lock().unwrap().path).unwrap();
}
