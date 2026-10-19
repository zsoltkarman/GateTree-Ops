use std::{
    collections::HashMap,
    env,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use fs2::FileExt;
use getrandom::fill;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod drivers;
mod ssh;

pub use ssh::{
    SshAuthentication, SshConnectRequest, SshHostKey, SshSessionStatus, SshTerminalChunk,
};

pub const PROTOCOL_VERSION: u16 = 16;
const ENDPOINT_FILE: &str = "agent-endpoint.json";
const LOCK_FILE: &str = "agent.lock";
const JOB_SETS_FILE: &str = "agent-job-sets.json";
const MAX_MESSAGE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJobSubmission {
    #[serde(default)]
    pub disabled: bool,
    pub id: String,
    pub sequence: i64,
    pub name: String,
    pub driver: String,
    pub action: String,
    pub target: Option<String>,
    pub details: String,
    pub payload: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJobSetSubmission {
    pub id: String,
    #[serde(default)]
    pub plan_number: i64,
    pub name: String,
    pub process_name: String,
    pub plan_definition: String,
    pub process_definition: String,
    pub jobs: Vec<AgentJobSubmission>,
    #[serde(default)]
    pub credentials: Vec<AgentCredential>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCredential {
    pub reference: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJobStatus {
    pub id: String,
    pub sequence: i64,
    pub name: String,
    pub driver: String,
    pub action: String,
    pub target: Option<String>,
    pub details: String,
    pub payload: Option<String>,
    pub status: String,
    pub log: String,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub validation_job_id: Option<i64>,
    #[serde(default)]
    pub deployment_job_id: Option<i64>,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub report_progress: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJobSetStatus {
    #[serde(default)]
    pub execution_revision: u32,
    #[serde(default)]
    pub credentials_required: bool,
    pub id: String,
    #[serde(default)]
    pub plan_number: i64,
    pub name: String,
    pub process_name: String,
    pub plan_definition: String,
    pub process_definition: String,
    pub status: String,
    pub jobs: Vec<AgentJobStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentEndpoint {
    protocol_version: u16,
    address: SocketAddr,
    token: String,
    pid: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentRequest {
    protocol_version: u16,
    token: String,
    action: AgentAction,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum AgentAction {
    Ping,
    Shutdown,
    PrepareUpdate,
    RefreshUpdate,
    CancelUpdate,
    ScanSshHostKey {
        host: String,
        port: u16,
    },
    ConnectSsh {
        request: SshConnectRequest,
    },
    ListSshSessions,
    DisconnectSsh {
        session_id: String,
    },
    ReadSshTerminal {
        session_id: String,
        offset: Option<u64>,
        maximum_bytes: usize,
    },
    WriteSshTerminal {
        session_id: String,
        data: Vec<u8>,
    },
    ResizeSshTerminal {
        session_id: String,
        columns: u16,
        rows: u16,
    },
    SubmitJobSet {
        submission: AgentJobSetSubmission,
        restart: bool,
    },
    RetryJob {
        job_id: String,
        credentials: Vec<AgentCredential>,
    },
    SetJobEnabled {
        job_id: String,
        enabled: bool,
    },
    ListJobSets,
    RestoreJobSetCredentials {
        job_set_id: String,
        credentials: Vec<AgentCredential>,
    },
    StopJobSet {
        job_set_id: String,
    },
    ApproveJob {
        job_id: String,
    },
    DenyJob {
        job_id: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentResponse {
    ok: bool,
    running: bool,
    protocol_version: u16,
    pid: u32,
    version: String,
    error: Option<String>,
    ssh_host_key: Option<SshHostKey>,
    ssh_session: Option<SshSessionStatus>,
    ssh_sessions: Option<Vec<SshSessionStatus>>,
    ssh_terminal_chunk: Option<SshTerminalChunk>,
    job_set: Option<AgentJobSetStatus>,
    job_sets: Option<Vec<AgentJobSetStatus>>,
}

#[derive(Debug, Clone)]
pub struct AgentProcessStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub version: Option<String>,
    pub protocol_version: Option<u16>,
}

impl AgentProcessStatus {
    pub fn stopped() -> Self {
        Self {
            running: false,
            pid: None,
            version: None,
            protocol_version: None,
        }
    }
}

pub fn validate_jira_update_report(report: &serde_json::Value) -> Result<(), String> {
    drivers::jira_update::validate_report(report)
}

pub fn validate_cx_update_report(report: &serde_json::Value) -> Result<(), String> {
    drivers::cx_update::validate_report(report)
}

pub fn status() -> Result<AgentProcessStatus, AgentIpcError> {
    status_at(&AgentPaths::discover()?)
}

pub fn start() -> Result<AgentProcessStatus, AgentIpcError> {
    let paths = AgentPaths::discover()?;
    if let Ok(status) = status_at(&paths)
        && status.running
    {
        if status.protocol_version != Some(PROTOCOL_VERSION) {
            return Err(AgentIpcError::ProtocolMismatch);
        }
        if status.version.as_deref() != Some(env!("CARGO_PKG_VERSION")) {
            return Err(AgentIpcError::AgentVersionMismatch);
        }
        return Ok(status);
    }

    let executable = agent_executable()?;
    if !executable.is_file() {
        return Err(AgentIpcError::ExecutableNotFound(executable));
    }
    let mut command = Command::new(executable);
    command
        .arg("--serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach_command(&mut command);
    let mut child = command.spawn()?;

    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if let Some(exit) = child.try_wait()? {
            return Err(AgentIpcError::Rejected(format!(
                "Agent exited during startup ({exit}). Check the Agent page for an older running Agent; use Stop or Kill Agent before starting again."
            )));
        }
        if let Ok(status) = status_at(&paths)
            && status.running
        {
            if status.protocol_version != Some(PROTOCOL_VERSION) {
                return Err(AgentIpcError::ProtocolMismatch);
            }
            if status.version.as_deref() != Some(env!("CARGO_PKG_VERSION")) {
                return Err(AgentIpcError::AgentVersionMismatch);
            }
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err(AgentIpcError::StartupTimeout)
}

pub fn stop() -> Result<AgentProcessStatus, AgentIpcError> {
    let paths = AgentPaths::discover()?;
    let endpoint = match read_endpoint(&paths) {
        Ok(endpoint) => endpoint,
        Err(AgentIpcError::NotRunning) => return Ok(AgentProcessStatus::stopped()),
        Err(error) => return Err(error),
    };
    let response = request(&endpoint, AgentAction::Shutdown)?;
    if !response.ok {
        return Err(AgentIpcError::Rejected(
            response
                .error
                .unwrap_or_else(|| "shutdown was rejected".into()),
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if !paths.endpoint.exists() || !status_at(&paths).is_ok_and(|status| status.running) {
            return Ok(AgentProcessStatus::stopped());
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err(AgentIpcError::ShutdownTimeout)
}

/// Atomically reserve an idle Agent for application replacement. While reserved,
/// every mutating operation except shutdown/cancellation is rejected.
pub fn prepare_update() -> Result<bool, AgentIpcError> {
    let paths = AgentPaths::discover()?;
    let endpoint = match read_endpoint(&paths) {
        Ok(endpoint) => endpoint,
        Err(AgentIpcError::NotRunning) => {
            start()?;
            read_endpoint(&paths)?
        }
        Err(error) => return Err(error),
    };
    reserve_update(&endpoint)?;
    Ok(true)
}

fn reserve_update(endpoint: &AgentEndpoint) -> Result<(), AgentIpcError> {
    let response = request(endpoint, AgentAction::PrepareUpdate)?;
    if !response.ok {
        return Err(AgentIpcError::Rejected(
            response
                .error
                .unwrap_or_else(|| "Agent update reservation failed.".into()),
        ));
    }
    Ok(())
}

pub fn refresh_update_reservation() -> Result<(), AgentIpcError> {
    let endpoint = read_endpoint(&AgentPaths::discover()?)?;
    let response = request(&endpoint, AgentAction::RefreshUpdate)?;
    if !response.ok {
        return Err(AgentIpcError::Rejected(response.error.unwrap_or_else(
            || "Update reservation expired; check for updates again.".into(),
        )));
    }
    Ok(())
}

pub fn cancel_update() -> Result<(), AgentIpcError> {
    let endpoint = read_endpoint(&AgentPaths::discover()?)?;
    let response = request(&endpoint, AgentAction::CancelUpdate)?;
    if !response.ok {
        return Err(AgentIpcError::Rejected(
            response
                .error
                .unwrap_or_else(|| "Agent update cancellation failed.".into()),
        ));
    }
    Ok(())
}

pub fn backup_update_state(destination: &Path) -> Result<(), AgentIpcError> {
    let paths = AgentPaths::discover()?;
    if paths.job_sets.exists() {
        fs::copy(&paths.job_sets, destination.join(JOB_SETS_FILE))?;
    }
    Ok(())
}

pub fn kill() -> Result<AgentProcessStatus, AgentIpcError> {
    kill_at(&AgentPaths::discover()?)
}

fn kill_at(paths: &AgentPaths) -> Result<AgentProcessStatus, AgentIpcError> {
    let endpoint = match read_endpoint(paths) {
        Ok(endpoint) => endpoint,
        Err(AgentIpcError::NotRunning) => return Ok(AgentProcessStatus::stopped()),
        Err(error) => return Err(error),
    };
    if endpoint.pid == std::process::id() {
        return Err(AgentIpcError::InvalidEndpoint);
    }
    let pid = endpoint.pid.to_string();
    #[cfg(unix)]
    {
        // Check the process name before signalling: an old endpoint can contain a reused PID.
        let process = Command::new("ps")
            .args(["-p", &pid, "-o", "comm="])
            .output()?;
        if !process.status.success() {
            if process.status.code() == Some(1) && process.stderr.is_empty() {
                remove_owned_endpoint(paths, &endpoint);
                return Ok(AgentProcessStatus::stopped());
            }
            return Err(AgentIpcError::Rejected(
                "Could not verify the Agent process before killing it".into(),
            ));
        }
        let name = String::from_utf8_lossy(&process.stdout);
        if Path::new(name.trim())
            .file_name()
            .and_then(|name| name.to_str())
            != Some("gatetree-ops-agent")
        {
            return Err(AgentIpcError::InvalidEndpoint);
        }
        let process_id = i32::try_from(endpoint.pid).map_err(|_| AgentIpcError::InvalidEndpoint)?;
        if unsafe { libc::kill(process_id, libc::SIGKILL) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(target_os = "windows")]
    {
        let result = Command::new("taskkill")
            .args([
                "/PID",
                &pid,
                "/F",
                "/FI",
                "IMAGENAME eq gatetree-ops-agent.exe",
            ])
            .output()?;
        if !result.status.success() {
            return Err(AgentIpcError::Rejected(
                "Could not kill the GateTree-Ops Agent process".into(),
            ));
        }
    }
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&paths.lock)?;
        if lock.try_lock_exclusive().is_ok() {
            remove_owned_endpoint(paths, &endpoint);
            return Ok(AgentProcessStatus::stopped());
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err(AgentIpcError::ShutdownTimeout)
}

pub fn scan_ssh_host_key(host: String, port: u16) -> Result<SshHostKey, AgentIpcError> {
    let endpoint = read_endpoint(&AgentPaths::discover()?)?;
    let response = request(&endpoint, AgentAction::ScanSshHostKey { host, port })?;
    response_value(response, |response| response.ssh_host_key, "SSH host key")
}

pub fn connect_ssh(request_value: SshConnectRequest) -> Result<SshSessionStatus, AgentIpcError> {
    let endpoint = read_endpoint(&AgentPaths::discover()?)?;
    let response = request(
        &endpoint,
        AgentAction::ConnectSsh {
            request: request_value,
        },
    )?;
    response_value(response, |response| response.ssh_session, "SSH session")
}

pub fn list_ssh_sessions() -> Result<Vec<SshSessionStatus>, AgentIpcError> {
    let endpoint = read_endpoint(&AgentPaths::discover()?)?;
    let response = request(&endpoint, AgentAction::ListSshSessions)?;
    response_value(
        response,
        |response| response.ssh_sessions,
        "SSH session list",
    )
}

pub fn disconnect_ssh(session_id: String) -> Result<(), AgentIpcError> {
    let endpoint = read_endpoint(&AgentPaths::discover()?)?;
    let response = request(&endpoint, AgentAction::DisconnectSsh { session_id })?;
    if response.ok {
        Ok(())
    } else {
        Err(rejected_response(response))
    }
}

pub fn read_ssh_terminal(
    session_id: String,
    offset: Option<u64>,
    maximum_bytes: usize,
) -> Result<SshTerminalChunk, AgentIpcError> {
    let endpoint = read_endpoint(&AgentPaths::discover()?)?;
    let response = request(
        &endpoint,
        AgentAction::ReadSshTerminal {
            session_id,
            offset,
            maximum_bytes,
        },
    )?;
    response_value(
        response,
        |response| response.ssh_terminal_chunk,
        "SSH terminal output",
    )
}

pub fn write_ssh_terminal(session_id: String, data: Vec<u8>) -> Result<(), AgentIpcError> {
    simple_request(AgentAction::WriteSshTerminal { session_id, data })
}

pub fn resize_ssh_terminal(
    session_id: String,
    columns: u16,
    rows: u16,
) -> Result<(), AgentIpcError> {
    simple_request(AgentAction::ResizeSshTerminal {
        session_id,
        columns,
        rows,
    })
}

pub fn submit_job_set(
    submission: AgentJobSetSubmission,
    restart: bool,
) -> Result<AgentJobSetStatus, AgentIpcError> {
    let endpoint = read_endpoint(&AgentPaths::discover()?)?;
    let response = request(
        &endpoint,
        AgentAction::SubmitJobSet {
            submission,
            restart,
        },
    )?;
    response_value(response, |response| response.job_set, "Agent job set")
}

pub fn retry_job(job_id: String, credentials: Vec<AgentCredential>) -> Result<(), AgentIpcError> {
    simple_request(AgentAction::RetryJob {
        job_id,
        credentials,
    })
}

pub fn restore_job_set_credentials(
    job_set_id: String,
    credentials: Vec<AgentCredential>,
) -> Result<(), AgentIpcError> {
    simple_request(AgentAction::RestoreJobSetCredentials {
        job_set_id,
        credentials,
    })
}

pub fn list_job_sets() -> Result<Vec<AgentJobSetStatus>, AgentIpcError> {
    let endpoint = read_endpoint(&AgentPaths::discover()?)?;
    let response = request(&endpoint, AgentAction::ListJobSets)?;
    response_value(response, |response| response.job_sets, "Agent job set list")
}

pub fn stop_job_set(job_set_id: String) -> Result<AgentJobSetStatus, AgentIpcError> {
    let endpoint = read_endpoint(&AgentPaths::discover()?)?;
    let response = request(&endpoint, AgentAction::StopJobSet { job_set_id })?;
    response_value(response, |response| response.job_set, "Agent job set")
}

pub fn set_job_enabled(job_id: String, enabled: bool) -> Result<AgentJobSetStatus, AgentIpcError> {
    let endpoint = read_endpoint(&AgentPaths::discover()?)?;
    let response = request(&endpoint, AgentAction::SetJobEnabled { job_id, enabled })?;
    response_value(response, |response| response.job_set, "Agent job set")
}

pub fn deny_job(job_id: String) -> Result<(), AgentIpcError> {
    simple_request(AgentAction::DenyJob { job_id })
}

pub fn approve_job(job_id: String) -> Result<AgentJobSetStatus, AgentIpcError> {
    let endpoint = read_endpoint(&AgentPaths::discover()?)?;
    let response = request(&endpoint, AgentAction::ApproveJob { job_id })?;
    response_value(response, |response| response.job_set, "Agent job set")
}

fn simple_request(action: AgentAction) -> Result<(), AgentIpcError> {
    let endpoint = read_endpoint(&AgentPaths::discover()?)?;
    let response = request(&endpoint, action)?;
    if response.ok {
        Ok(())
    } else {
        Err(rejected_response(response))
    }
}

struct JobSetManager {
    path: PathBuf,
    job_sets: Vec<AgentJobSetStatus>,
    credentials: HashMap<String, Vec<AgentCredential>>,
    execution_generations: HashMap<String, u64>,
}

impl JobSetManager {
    fn load(path: PathBuf) -> Result<Self, AgentIpcError> {
        let mut job_sets: Vec<AgentJobSetStatus> = match fs::read(&path) {
            Ok(contents) => serde_json::from_slice(&contents)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        let mut changed = false;
        let mut unique_names = Vec::<String>::new();
        job_sets.reverse();
        job_sets.retain(|set| {
            if unique_names
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&set.name))
            {
                changed = true;
                false
            } else {
                unique_names.push(set.name.clone());
                true
            }
        });
        job_sets.reverse();
        for set in &mut job_sets {
            if !matches!(set.status.as_str(), "running" | "awaiting_approval") {
                continue;
            }
            changed = true;
            set.status = "stopped".into();
            if let Some(job) = set
                .jobs
                .iter_mut()
                .find(|job| matches!(job.status.as_str(), "running" | "awaiting_approval"))
            {
                job.status = "stopped".into();
                append_job_log(
                    &mut job.log,
                    "Agent restarted. Unlock KeePass to restore credentials, then restart the Plan to run its configured steps.",
                );
            }
        }
        let manager = Self {
            path,
            job_sets,
            credentials: HashMap::new(),
            execution_generations: HashMap::new(),
        };
        if changed {
            manager.persist()?;
        }
        Ok(manager)
    }

    fn submit(
        &mut self,
        submission: AgentJobSetSubmission,
        restart: bool,
    ) -> Result<AgentJobSetStatus, AgentIpcError> {
        if submission.jobs.is_empty() {
            return Err(AgentIpcError::InvalidJobSet(
                "a job set must contain at least one job".into(),
            ));
        }
        if self
            .job_sets
            .iter()
            .any(|set| set.id == submission.id && set.status == "running")
            && restart
        {
            return Err(AgentIpcError::InvalidJobSet(
                "Stop the Plan before restarting it".into(),
            ));
        }
        let mut job_set = AgentJobSetStatus {
            execution_revision: 3,
            credentials_required: false,
            id: submission.id,
            plan_number: submission.plan_number,
            name: submission.name,
            process_name: submission.process_name,
            plan_definition: submission.plan_definition,
            process_definition: submission.process_definition,
            status: "running".into(),
            jobs: submission
                .jobs
                .into_iter()
                .map(|job| {
                    let phase = (job.driver == "olam" && job.action != "validate").then(|| {
                        if job.action == "deploy" {
                            "deployment".into()
                        } else {
                            "validation".into()
                        }
                    });
                    AgentJobStatus {
                        id: job.id,
                        sequence: job.sequence,
                        name: job.name,
                        driver: job.driver,
                        action: job.action,
                        target: job.target,
                        details: job.details,
                        payload: job.payload,
                        status: if job.disabled { "disabled" } else { "queued" }.into(),
                        log: String::new(),
                        phase,
                        validation_job_id: None,
                        deployment_job_id: None,
                        finished_at: None,
                        report_progress: Default::default(),
                    }
                })
                .collect(),
        };
        job_set.jobs.sort_by_key(|job| job.sequence);

        self.job_sets
            .retain(|item| item.id == job_set.id || !item.name.eq_ignore_ascii_case(&job_set.name));
        self.credentials
            .retain(|id, _| self.job_sets.iter().any(|job_set| &job_set.id == id));

        if let Some(existing) = self.job_sets.iter().position(|item| item.id == job_set.id) {
            let unchanged = self.job_sets[existing].plan_definition == job_set.plan_definition
                && self.job_sets[existing].process_definition == job_set.process_definition
                && self.job_sets[existing]
                    .jobs
                    .iter()
                    .map(|job| &job.id)
                    .eq(job_set.jobs.iter().map(|job| &job.id));
            // Starting a stopped Plan always reruns every step, including completed checks.
            if !restart && unchanged && self.job_sets[existing].status != "stopped" {
                let current = &mut self.job_sets[existing];
                if matches!(current.status.as_str(), "running" | "awaiting_approval") {
                    return Ok(current.clone());
                }
                if let Some(index) = current
                    .jobs
                    .iter()
                    .position(|job| !matches!(job.status.as_str(), "completed" | "disabled"))
                {
                    activate_job(current, index);
                } else {
                    current.status = "completed".into();
                }
                let result = current.clone();
                self.persist()?;
                return Ok(result);
            }
            self.job_sets.remove(existing);
        }

        activate_job(&mut job_set, 0);
        self.job_sets.push(job_set.clone());
        self.persist()?;
        Ok(job_set)
    }

    fn retry(
        &mut self,
        job_id: &str,
        credentials: Vec<AgentCredential>,
    ) -> Result<AgentJobSetStatus, AgentIpcError> {
        let set = self
            .job_sets
            .iter_mut()
            .find(|set| set.jobs.iter().any(|job| job.id == job_id))
            .ok_or_else(|| AgentIpcError::InvalidJobSet("job was not found".into()))?;
        if set.execution_revision != 3 {
            return Err(AgentIpcError::InvalidJobSet(
                "Restart the Plan to regenerate jobs from Process steps.".into(),
            ));
        }
        let index = set.jobs.iter().position(|job| job.id == job_id).unwrap();
        if set.jobs[index].status == "disabled" {
            return Err(AgentIpcError::InvalidJobSet(
                "Enable this job before retrying it.".into(),
            ));
        }
        if set.status == "running"
            || set.jobs[..index]
                .iter()
                .any(|job| !matches!(job.status.as_str(), "completed" | "disabled"))
        {
            return Err(AgentIpcError::InvalidJobSet(
                "Stop the Plan and complete preceding jobs before retrying".into(),
            ));
        }
        if !matches!(
            set.jobs[index].status.as_str(),
            "failed" | "warning" | "stopped" | "queued" | "completed" | "denied"
        ) {
            return Err(AgentIpcError::InvalidJobSet(
                "job cannot be retried in its current status".into(),
            ));
        }
        if credentials.is_empty() && !self.credentials.contains_key(&set.id) {
            return Err(AgentIpcError::InvalidJobSet(
                "Unlock KeePass before retrying the job".into(),
            ));
        }
        if !credentials.is_empty() {
            self.credentials.insert(set.id.clone(), credentials);
        }
        let retried_id = set.jobs[index].id.clone();
        for job in &mut set.jobs[index..] {
            if job.status == "disabled" {
                continue;
            }
            job.status = "queued".into();
            job.finished_at = None;
            if job.id != retried_id {
                job.report_progress.clear();
            }
            job.log.clear();
            job.validation_job_id = None;
            job.deployment_job_id = None;
            job.phase = if job.driver == "olam" && job.action != "validate" {
                Some(
                    if job.action == "deploy" {
                        "deployment"
                    } else {
                        "validation"
                    }
                    .into(),
                )
            } else {
                None
            };
        }
        activate_job(set, index);
        append_job_log(&mut set.jobs[index].log, "Retry requested by the operator.");
        let result = set.clone();
        self.persist()?;
        Ok(result)
    }

    fn list(&self) -> Vec<AgentJobSetStatus> {
        self.job_sets
            .iter()
            .cloned()
            .map(|mut set| {
                set.credentials_required = !self.credentials.contains_key(&set.id)
                    && set.status != "completed"
                    && set
                        .jobs
                        .iter()
                        .any(|job| matches!(job.driver.as_str(), "jira" | "cx" | "olam"));
                set
            })
            .collect()
    }

    fn restore_credentials(
        &mut self,
        id: &str,
        credentials: Vec<AgentCredential>,
    ) -> Result<(), AgentIpcError> {
        if !self.job_sets.iter().any(|set| set.id == id) || credentials.is_empty() {
            return Err(AgentIpcError::InvalidJobSet(
                "Plan not found or KeePass credentials are empty".into(),
            ));
        }
        self.credentials.insert(id.to_owned(), credentials);
        Ok(())
    }

    fn deny(&mut self, job_id: &str) -> Result<(), AgentIpcError> {
        let set = self
            .job_sets
            .iter_mut()
            .find(|set| set.jobs.iter().any(|job| job.id == job_id))
            .ok_or_else(|| AgentIpcError::InvalidJobSet("job was not found".into()))?;
        let job = set
            .jobs
            .iter_mut()
            .find(|job| job.id == job_id)
            .expect("job was located above");
        if set.status != "awaiting_approval" || job.status != "awaiting_approval" {
            return Err(AgentIpcError::InvalidJobSet(
                "only a validated job awaiting approval can be denied".into(),
            ));
        }
        job.status = "denied".into();
        append_job_log(
            &mut job.log,
            "Deployment denied by the operator. The Plan has stopped.",
        );
        set.status = "denied".into();
        self.persist()
    }

    fn set_job_enabled(
        &mut self,
        job_id: &str,
        enabled: bool,
    ) -> Result<AgentJobSetStatus, AgentIpcError> {
        let set = self
            .job_sets
            .iter_mut()
            .find(|set| set.jobs.iter().any(|job| job.id == job_id))
            .ok_or_else(|| AgentIpcError::InvalidJobSet("job was not found".into()))?;
        let index = set.jobs.iter().position(|job| job.id == job_id).unwrap();
        let status = set.jobs[index].status.as_str();
        if (enabled && status != "disabled")
            || (!enabled && !matches!(status, "queued" | "stopped"))
        {
            return Err(AgentIpcError::InvalidJobSet(
                "Only queued or stopped jobs can be disabled; only disabled jobs can be enabled."
                    .into(),
            ));
        }
        if enabled
            && matches!(set.status.as_str(), "running" | "awaiting_approval")
            && set.jobs[index + 1..]
                .iter()
                .any(|job| !matches!(job.status.as_str(), "queued" | "disabled"))
        {
            return Err(AgentIpcError::InvalidJobSet(
                "Stop the Plan before enabling a job whose position has already passed.".into(),
            ));
        }
        set.jobs[index].status = if enabled { "queued" } else { "disabled" }.into();
        let result = set.clone();
        self.persist()?;
        Ok(result)
    }

    fn approve(&mut self, job_id: &str) -> Result<AgentJobSetStatus, AgentIpcError> {
        let job_set = self
            .job_sets
            .iter_mut()
            .find(|set| set.jobs.iter().any(|job| job.id == job_id))
            .ok_or_else(|| AgentIpcError::InvalidJobSet("job was not found".into()))?;
        if job_set.execution_revision != 3 {
            return Err(AgentIpcError::InvalidJobSet(
                "Restart the Plan to regenerate jobs from Process steps.".into(),
            ));
        }
        let job = job_set
            .jobs
            .iter_mut()
            .find(|job| job.id == job_id)
            .expect("job was located above");
        if job.status != "awaiting_approval"
            || job.driver != "olam"
            || !matches!(job.action.as_str(), "deploy" | "validate_deploy")
        {
            return Err(AgentIpcError::InvalidJobSet(
                "only an OLAM deployment awaiting approval can be approved".into(),
            ));
        }
        if !self.credentials.contains_key(&job_set.id) {
            return Err(AgentIpcError::InvalidJobSet(
                "Agent credentials are no longer in memory; restart the Plan to run its configured steps"
                    .into(),
            ));
        }
        job.status = "running".into();
        job.phase = Some("deployment".into());
        append_job_log(&mut job.log, "Deployment approved by the operator.");
        job_set.status = "running".into();
        let result = job_set.clone();
        self.persist()?;
        Ok(result)
    }

    fn stop(&mut self, id: &str) -> Result<AgentJobSetStatus, AgentIpcError> {
        let job_set = self
            .job_sets
            .iter_mut()
            .find(|job_set| job_set.id == id)
            .ok_or_else(|| AgentIpcError::InvalidJobSet("job set was not found".into()))?;
        job_set.status = "stopped".into();
        if let Some(job) = job_set
            .jobs
            .iter_mut()
            .find(|job| matches!(job.status.as_str(), "running" | "awaiting_approval"))
        {
            job.status = "stopped".into();
        }
        let result = job_set.clone();
        *self.execution_generations.entry(id.into()).or_default() += 1;
        self.persist()?;
        Ok(result)
    }

    fn persist(&self) -> Result<(), AgentIpcError> {
        let temporary = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(&self.job_sets)?;
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(temporary, &self.path)?;
        Ok(())
    }
}

pub fn run_server() -> Result<(), AgentIpcError> {
    run_server_at(AgentPaths::discover()?)
}

fn status_at(paths: &AgentPaths) -> Result<AgentProcessStatus, AgentIpcError> {
    let endpoint = read_endpoint(paths)?;
    let response = request(&endpoint, AgentAction::Ping)?;
    if !response.ok || !response.running {
        return Err(AgentIpcError::NotRunning);
    }
    Ok(AgentProcessStatus {
        running: true,
        pid: Some(response.pid),
        version: Some(response.version),
        protocol_version: Some(response.protocol_version),
    })
}

fn run_server_at(paths: AgentPaths) -> Result<(), AgentIpcError> {
    fs::create_dir_all(&paths.runtime_dir)?;
    restrict_directory(&paths.runtime_dir)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&paths.lock)?;
    lock.try_lock_exclusive()
        .map_err(|_| AgentIpcError::AlreadyRunning)?;

    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let endpoint = AgentEndpoint {
        protocol_version: PROTOCOL_VERSION,
        address: listener.local_addr()?,
        token: new_token()?,
        pid: std::process::id(),
    };
    write_endpoint(&paths, &endpoint)?;

    let mut ssh_sessions =
        ssh::SshSessionManager::new().map_err(|error| AgentIpcError::Ssh(error.to_string()))?;
    let job_sets = Arc::new(Mutex::new(JobSetManager::load(paths.job_sets.clone())?));
    let result = serve(&listener, &endpoint, &mut ssh_sessions, job_sets);
    remove_owned_endpoint(&paths, &endpoint);
    drop(lock);
    result
}

fn serve(
    listener: &TcpListener,
    endpoint: &AgentEndpoint,
    ssh_sessions: &mut ssh::SshSessionManager,
    job_sets: Arc<Mutex<JobSetManager>>,
) -> Result<(), AgentIpcError> {
    let mut update_reserved = false;
    let mut reservation_deadline = Instant::now();
    for incoming in listener.incoming() {
        let mut stream = incoming?;
        if !stream.peer_addr()?.ip().is_loopback() {
            continue;
        }
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let request = read_request(&mut stream);
        if update_reserved && Instant::now() >= reservation_deadline {
            update_reserved = false;
        }
        let (response, shutdown) = match request {
            Ok(request)
                if request.protocol_version == PROTOCOL_VERSION
                    && request.token == endpoint.token =>
            {
                if update_reserved
                    && !matches!(
                        &request.action,
                        AgentAction::Ping
                            | AgentAction::ListSshSessions
                            | AgentAction::ListJobSets
                            | AgentAction::ReadSshTerminal { .. }
                            | AgentAction::Shutdown
                            | AgentAction::CancelUpdate
                            | AgentAction::RefreshUpdate
                    )
                {
                    (
                        error_response(
                            "Application update is in progress. Try again after restart.",
                        ),
                        false,
                    )
                } else {
                    match request.action {
                        AgentAction::PrepareUpdate => {
                            let manager = job_sets.lock().expect("job set lock");
                            let busy = manager.job_sets.iter().any(|set| {
                                matches!(set.status.as_str(), "running" | "awaiting_approval")
                                    || set.jobs.iter().any(|job| {
                                        matches!(
                                            job.status.as_str(),
                                            "running" | "awaiting_approval"
                                        )
                                    })
                            });
                            if busy || !ssh_sessions.list().is_empty() {
                                (
                                    error_response(
                                        "Finish or stop active deployments and close SSH sessions before installing the update.",
                                    ),
                                    false,
                                )
                            } else {
                                update_reserved = true;
                                reservation_deadline = Instant::now() + Duration::from_secs(600);
                                (success_response(), false)
                            }
                        }
                        AgentAction::RefreshUpdate => {
                            if update_reserved {
                                reservation_deadline = Instant::now() + Duration::from_secs(600);
                                (success_response(), false)
                            } else {
                                (
                                    error_response(
                                        "Update reservation expired. Check for updates again.",
                                    ),
                                    false,
                                )
                            }
                        }
                        AgentAction::CancelUpdate => {
                            update_reserved = false;
                            (success_response(), false)
                        }
                        AgentAction::Ping => (success_response(), false),
                        AgentAction::Shutdown => {
                            ssh_sessions.disconnect_all();
                            (success_response(), true)
                        }
                        AgentAction::ScanSshHostKey { host, port } => {
                            match ssh_sessions.scan_host_key(host, port) {
                                Ok(host_key) => {
                                    let mut response = success_response();
                                    response.ssh_host_key = Some(host_key);
                                    (response, false)
                                }
                                Err(error) => (error_response(&error.to_string()), false),
                            }
                        }
                        AgentAction::ConnectSsh { request } => {
                            match ssh_sessions.connect(request) {
                                Ok(session) => {
                                    let mut response = success_response();
                                    response.ssh_session = Some(session);
                                    (response, false)
                                }
                                Err(error) => (error_response(&error.to_string()), false),
                            }
                        }
                        AgentAction::ListSshSessions => {
                            let mut response = success_response();
                            response.ssh_sessions = Some(ssh_sessions.list());
                            (response, false)
                        }
                        AgentAction::DisconnectSsh { session_id } => {
                            match ssh_sessions.disconnect(&session_id) {
                                Ok(()) => (success_response(), false),
                                Err(error) => (error_response(&error.to_string()), false),
                            }
                        }
                        AgentAction::ReadSshTerminal {
                            session_id,
                            offset,
                            maximum_bytes,
                        } => match ssh_sessions.read_terminal(&session_id, offset, maximum_bytes) {
                            Ok(chunk) => {
                                let mut response = success_response();
                                response.ssh_terminal_chunk = Some(chunk);
                                (response, false)
                            }
                            Err(error) => (error_response(&error.to_string()), false),
                        },
                        AgentAction::WriteSshTerminal { session_id, data } => {
                            match ssh_sessions.write_terminal(&session_id, data) {
                                Ok(()) => (success_response(), false),
                                Err(error) => (error_response(&error.to_string()), false),
                            }
                        }
                        AgentAction::ResizeSshTerminal {
                            session_id,
                            columns,
                            rows,
                        } => match ssh_sessions.resize_terminal(&session_id, columns, rows) {
                            Ok(()) => (success_response(), false),
                            Err(error) => (error_response(&error.to_string()), false),
                        },
                        AgentAction::SubmitJobSet {
                            submission,
                            restart,
                        } => {
                            let credentials = submission.credentials.clone();
                            let job_set_id = submission.id.clone();
                            let (submitted, already_running) = {
                                let mut manager = job_sets.lock().expect("job set lock");
                                let already_running = !restart
                                    && manager
                                        .job_sets
                                        .iter()
                                        .any(|set| set.id == job_set_id && set.status == "running");
                                let result = manager.submit(submission, restart);
                                if result.is_ok() {
                                    manager
                                        .credentials
                                        .insert(job_set_id.clone(), credentials.clone());
                                }
                                (result, already_running)
                            };
                            match submitted {
                                Ok(job_set) => {
                                    if !already_running {
                                        spawn_job_set_execution(
                                            job_sets.clone(),
                                            job_set_id,
                                            credentials,
                                        );
                                    }
                                    let mut response = success_response();
                                    response.job_set = Some(job_set);
                                    (response, false)
                                }
                                Err(error) => (error_response(&error.to_string()), false),
                            }
                        }
                        AgentAction::RetryJob {
                            job_id,
                            credentials,
                        } => {
                            let retried = {
                                let mut manager = job_sets.lock().expect("job set lock");
                                manager.retry(&job_id, credentials).map(|set| {
                                    let credentials = manager
                                        .credentials
                                        .get(&set.id)
                                        .cloned()
                                        .unwrap_or_default();
                                    (set, credentials)
                                })
                            };
                            match retried {
                                Ok((set, credentials)) => {
                                    spawn_job_set_execution(job_sets.clone(), set.id, credentials);
                                    (success_response(), false)
                                }
                                Err(error) => (error_response(&error.to_string()), false),
                            }
                        }
                        AgentAction::RestoreJobSetCredentials {
                            job_set_id,
                            credentials,
                        } => {
                            match job_sets
                                .lock()
                                .expect("job set lock")
                                .restore_credentials(&job_set_id, credentials)
                            {
                                Ok(()) => (success_response(), false),
                                Err(error) => (error_response(&error.to_string()), false),
                            }
                        }
                        AgentAction::SetJobEnabled { job_id, enabled } => {
                            match job_sets
                                .lock()
                                .expect("job set lock")
                                .set_job_enabled(&job_id, enabled)
                            {
                                Ok(set) => {
                                    let mut response = success_response();
                                    response.job_set = Some(set);
                                    (response, false)
                                }
                                Err(error) => (error_response(&error.to_string()), false),
                            }
                        }
                        AgentAction::ListJobSets => {
                            let mut response = success_response();
                            response.job_sets = Some(job_sets.lock().expect("job set lock").list());
                            (response, false)
                        }
                        AgentAction::StopJobSet { job_set_id } => {
                            match job_sets.lock().expect("job set lock").stop(&job_set_id) {
                                Ok(job_set) => {
                                    let mut response = success_response();
                                    response.job_set = Some(job_set);
                                    (response, false)
                                }
                                Err(error) => (error_response(&error.to_string()), false),
                            }
                        }
                        AgentAction::DenyJob { job_id } => {
                            match job_sets.lock().expect("job set lock").deny(&job_id) {
                                Ok(()) => (success_response(), false),
                                Err(error) => (error_response(&error.to_string()), false),
                            }
                        }
                        AgentAction::ApproveJob { job_id } => {
                            let approved = {
                                let mut manager = job_sets.lock().expect("job set lock");
                                manager.approve(&job_id).map(|set| {
                                    let credentials = manager
                                        .credentials
                                        .get(&set.id)
                                        .cloned()
                                        .unwrap_or_default();
                                    (set, credentials)
                                })
                            };
                            match approved {
                                Ok((job_set, credentials)) => {
                                    spawn_job_set_execution(
                                        job_sets.clone(),
                                        job_set.id.clone(),
                                        credentials,
                                    );
                                    let mut response = success_response();
                                    response.job_set = Some(job_set);
                                    (response, false)
                                }
                                Err(error) => (error_response(&error.to_string()), false),
                            }
                        }
                    }
                }
            }
            Ok(_) => (error_response("IPC authentication failed"), false),
            Err(error) => (error_response(&error.to_string()), false),
        };
        write_response(&mut stream, &response)?;
        if shutdown {
            return Ok(());
        }
    }
    Ok(())
}

fn spawn_job_set_execution(
    job_sets: Arc<Mutex<JobSetManager>>,
    job_set_id: String,
    credentials: Vec<AgentCredential>,
) {
    let generation = {
        let mut manager = job_sets.lock().expect("job set lock");
        let generation = manager
            .execution_generations
            .entry(job_set_id.clone())
            .or_default();
        *generation += 1;
        *generation
    };
    thread::spawn(move || {
        loop {
            let current = {
                let manager = job_sets.lock().expect("job set lock");
                if manager.execution_generations.get(&job_set_id) != Some(&generation) {
                    return;
                }
                manager
                    .job_sets
                    .iter()
                    .find(|set| set.id == job_set_id && set.status == "running")
                    .and_then(|set| {
                        set.jobs
                            .iter()
                            .find(|job| job.status == "running")
                            .cloned()
                            .map(|job| {
                                (
                                    set.process_definition.clone(),
                                    set.plan_definition.clone(),
                                    job,
                                    set.clone(),
                                )
                            })
                    })
            };
            let Some((process_definition, plan_definition, job, report_context)) = current else {
                return;
            };
            {
                let mut manager = job_sets.lock().expect("job set lock");
                if manager.execution_generations.get(&job_set_id) != Some(&generation) {
                    return;
                }
                if let Some(set) = manager.job_sets.iter_mut().find(|set| set.id == job_set_id)
                    && let Some(active) = set.jobs.iter_mut().find(|item| item.id == job.id)
                {
                    if job.driver == "olam" && job.action == "deploy" {
                        if job.phase.as_deref() == Some("deployment") {
                            active.deployment_job_id = None;
                        } else {
                            active.validation_job_id = None;
                            active.deployment_job_id = None;
                        }
                    }
                    append_job_log(
                        &mut active.log,
                        &format!(
                            "Starting {} {} for {}…",
                            job.driver.to_uppercase(),
                            job.phase.as_deref().unwrap_or(&job.action),
                            job.target.as_deref().unwrap_or("plan")
                        ),
                    );
                    let _ = manager.persist();
                }
            }
            let on_started = |phase: &str, external_id: i64| {
                let mut manager = job_sets.lock().expect("job set lock");
                if manager.execution_generations.get(&job_set_id) != Some(&generation) {
                    return;
                }
                if let Some(set) = manager.job_sets.iter_mut().find(|set| set.id == job_set_id)
                    && set.status == "running"
                    && let Some(active) = set.jobs.iter_mut().find(|active| active.id == job.id)
                    && active.status == "running"
                    && active.phase.as_deref() == Some(phase)
                {
                    record_olam_launch(active, phase, external_id);
                    let _ = manager.persist();
                }
            };
            let log_prefix = std::cell::RefCell::new(None::<String>);
            let on_output = |output: &str| {
                let mut manager = job_sets.lock().expect("job set lock");
                if manager.execution_generations.get(&job_set_id) != Some(&generation) {
                    return;
                }
                if let Some(set) = manager.job_sets.iter_mut().find(|set| set.id == job_set_id)
                    && set.status == "running"
                    && let Some(active) = set.jobs.iter_mut().find(|active| active.id == job.id)
                    && active.status == "running"
                    && active.phase == job.phase
                {
                    let mut prefix = log_prefix.borrow_mut();
                    let prefix = prefix.get_or_insert_with(|| active.log.clone());
                    active.log.clone_from(prefix);
                    append_job_log(&mut active.log, output);
                    let _ = manager.persist();
                }
            };
            let checkpoint = |key: &str, value: &str| -> Result<(), String> {
                let mut manager = job_sets.lock().map_err(|_| "Agent job set lock failed.")?;
                if manager.execution_generations.get(&job_set_id) != Some(&generation) {
                    return Err("Update report stopped before the next operation.".into());
                }
                let set = manager
                    .job_sets
                    .iter_mut()
                    .find(|set| set.id == job_set_id && set.status == "running")
                    .ok_or("Update report is no longer running.")?;
                let active = set
                    .jobs
                    .iter_mut()
                    .find(|active| active.id == job.id && active.status == "running")
                    .ok_or("Update report job is no longer running.")?;
                active.report_progress.insert(key.into(), value.into());
                manager
                    .persist()
                    .map_err(|e| format!("Could not persist update report progress: {e}"))
            };
            let result = if matches!(job.driver.as_str(), "jira" | "cx") && job.action == "update" {
                let update = if job.driver == "cx" {
                    drivers::cx_update::update
                } else {
                    drivers::jira_update::update
                };
                update(&report_context, &job, &credentials, &checkpoint, &on_output)
                    .map(drivers::DriverOutcome::Completed)
            } else {
                drivers::execute(
                    &process_definition,
                    &plan_definition,
                    &job,
                    &credentials,
                    &on_started,
                    &on_output,
                )
            };
            let mut manager = job_sets.lock().expect("job set lock");
            if manager.execution_generations.get(&job_set_id) != Some(&generation) {
                return;
            }
            let Some(set) = manager.job_sets.iter_mut().find(|set| set.id == job_set_id) else {
                return;
            };
            if set.status != "running" {
                return;
            }
            let Some(index) = set.jobs.iter().position(|item| item.id == job.id) else {
                return;
            };
            // Replace the live snapshot with the final output rather than duplicating it.
            if let Some(prefix) = log_prefix.into_inner() {
                set.jobs[index].log = prefix;
            }
            match result {
                Ok(drivers::DriverOutcome::Completed(log)) => {
                    set.jobs[index].status = "completed".into();
                    set.jobs[index].finished_at = Some(chrono::Utc::now().to_rfc3339());
                    append_job_log(&mut set.jobs[index].log, &log);
                    if index + 1 < set.jobs.len() {
                        activate_job(set, index + 1);
                    } else {
                        set.status = "completed".into();
                    }
                }
                Ok(drivers::DriverOutcome::AwaitingApproval(log)) => {
                    set.jobs[index].status = "awaiting_approval".into();
                    set.jobs[index].phase = Some("deployment".into());
                    append_job_log(&mut set.jobs[index].log, &log);
                    set.status = "awaiting_approval".into();
                }
                Ok(drivers::DriverOutcome::Warning(log)) => {
                    set.jobs[index].status = "warning".into();
                    append_job_log(&mut set.jobs[index].log, &log);
                    set.status = "warning".into();
                }
                Err(log) => {
                    set.jobs[index].status = "failed".into();
                    append_job_log(&mut set.jobs[index].log, &log);
                    set.status = "failed".into();
                }
            }
            let should_stop = matches!(
                set.status.as_str(),
                "failed" | "warning" | "awaiting_approval"
            );
            let _ = manager.persist();
            if should_stop {
                return;
            }
        }
    });
}

// Activation is the only place that gates a deployment. Approve starts the
// already selected deploy action; it never launches an implicit validation.
fn activate_job(set: &mut AgentJobSetStatus, index: usize) {
    let Some(index) = (index..set.jobs.len()).find(|&i| set.jobs[i].status != "disabled") else {
        set.status = "completed".into();
        return;
    };
    if set.jobs[index].driver == "olam" && set.jobs[index].action == "deploy" {
        let deployment = &set.jobs[index];
        let validation = set.jobs[..index]
            .iter()
            .rev()
            .find(|candidate| {
                candidate.driver == "olam"
                    && candidate.action == "validate_deployment"
                    && candidate.status == "completed"
                    && candidate.target == deployment.target
                    && candidate
                        .payload
                        .as_deref()
                        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
                        .zip(deployment.payload.as_deref().and_then(|value| {
                            serde_json::from_str::<serde_json::Value>(value).ok()
                        }))
                        .is_some_and(|(a, b)| a == b)
            })
            .map(|validation| (validation.validation_job_id, validation.log.clone()));
        let job = &mut set.jobs[index];
        job.status = "awaiting_approval".into();
        job.phase = Some("deployment".into());
        job.validation_job_id = None;
        if let Some((id, log)) = validation {
            job.validation_job_id = id;
            job.log = log;
        } else {
            job.log = "No preceding matching validation was configured or completed. Review the deployment target and payload before approval.".into();
        }
        append_job_log(&mut job.log, "Deployment awaiting operator approval.");
        set.status = "awaiting_approval".into();
    } else {
        set.jobs[index].status = "running".into();
        set.status = "running".into();
    }
}

fn record_olam_launch(job: &mut AgentJobStatus, phase: &str, external_id: i64) {
    if external_id <= 0 {
        return;
    }
    match phase {
        "validation" => job.validation_job_id = Some(external_id),
        "deployment" => job.deployment_job_id = Some(external_id),
        _ => return,
    }
    append_job_log(
        &mut job.log,
        &format!("OLAM {phase} started as Ansible job {external_id}."),
    );
}

fn append_job_log(current: &mut String, message: &str) {
    if !current.is_empty() {
        current.push('\n');
    }
    current.push_str(message);
}

fn request(endpoint: &AgentEndpoint, action: AgentAction) -> Result<AgentResponse, AgentIpcError> {
    if !endpoint.address.ip().is_loopback() {
        return Err(AgentIpcError::InvalidEndpoint);
    }
    let lifecycle = matches!(&action, AgentAction::Ping | AgentAction::Shutdown);
    if !lifecycle && endpoint.protocol_version != PROTOCOL_VERSION {
        return Err(AgentIpcError::ProtocolMismatch);
    }
    let protocol_version = if lifecycle {
        endpoint.protocol_version
    } else {
        PROTOCOL_VERSION
    };
    let mut stream = TcpStream::connect_timeout(&endpoint.address, Duration::from_millis(600))?;
    let read_timeout = if matches!(
        &action,
        AgentAction::ScanSshHostKey { .. } | AgentAction::ConnectSsh { .. }
    ) {
        Duration::from_secs(20)
    } else {
        Duration::from_secs(2)
    };
    stream.set_read_timeout(Some(read_timeout))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let payload = AgentRequest {
        protocol_version,
        token: endpoint.token.clone(),
        action,
    };
    serde_json::to_writer(&mut stream, &payload)?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let mut line = String::new();
    BufReader::new(stream)
        .take(MAX_MESSAGE_BYTES)
        .read_to_string(&mut line)?;
    let response: AgentResponse = serde_json::from_str(line.trim())?;
    if response.protocol_version != protocol_version {
        return Err(AgentIpcError::ProtocolMismatch);
    }
    Ok(response)
}

fn read_request(stream: &mut TcpStream) -> Result<AgentRequest, AgentIpcError> {
    let mut line = String::new();
    BufReader::new(stream)
        .take(MAX_MESSAGE_BYTES)
        .read_line(&mut line)?;
    if line.len() as u64 >= MAX_MESSAGE_BYTES {
        return Err(AgentIpcError::MessageTooLarge);
    }
    Ok(serde_json::from_str(line.trim())?)
}

fn write_response(stream: &mut TcpStream, response: &AgentResponse) -> Result<(), AgentIpcError> {
    serde_json::to_writer(&mut *stream, response)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

fn success_response() -> AgentResponse {
    AgentResponse {
        ok: true,
        running: true,
        protocol_version: PROTOCOL_VERSION,
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").into(),
        error: None,
        ssh_host_key: None,
        ssh_session: None,
        ssh_sessions: None,
        ssh_terminal_chunk: None,
        job_set: None,
        job_sets: None,
    }
}

fn error_response(message: &str) -> AgentResponse {
    AgentResponse {
        ok: false,
        running: true,
        protocol_version: PROTOCOL_VERSION,
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").into(),
        error: Some(message.into()),
        ssh_host_key: None,
        ssh_session: None,
        ssh_sessions: None,
        ssh_terminal_chunk: None,
        job_set: None,
        job_sets: None,
    }
}

fn response_value<T>(
    response: AgentResponse,
    value: impl FnOnce(AgentResponse) -> Option<T>,
    description: &str,
) -> Result<T, AgentIpcError> {
    if !response.ok {
        return Err(rejected_response(response));
    }
    value(response).ok_or_else(|| AgentIpcError::InvalidResponse(description.into()))
}

fn rejected_response(response: AgentResponse) -> AgentIpcError {
    AgentIpcError::Rejected(
        response
            .error
            .unwrap_or_else(|| "request was rejected".into()),
    )
}

fn read_endpoint(paths: &AgentPaths) -> Result<AgentEndpoint, AgentIpcError> {
    let contents = fs::read_to_string(&paths.endpoint).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AgentIpcError::NotRunning
        } else {
            AgentIpcError::Io(error)
        }
    })?;
    let endpoint: AgentEndpoint = serde_json::from_str(&contents)?;
    if !endpoint.address.ip().is_loopback() || endpoint.pid <= 1 {
        return Err(AgentIpcError::InvalidEndpoint);
    }
    Ok(endpoint)
}

fn write_endpoint(paths: &AgentPaths, endpoint: &AgentEndpoint) -> Result<(), AgentIpcError> {
    let temporary = paths.endpoint.with_extension("json.tmp");
    let bytes = serde_json::to_vec(endpoint)?;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(temporary, &paths.endpoint)?;
    Ok(())
}

fn remove_owned_endpoint(paths: &AgentPaths, endpoint: &AgentEndpoint) {
    if read_endpoint(paths)
        .is_ok_and(|current| current.pid == endpoint.pid && current.token == endpoint.token)
    {
        let _ = fs::remove_file(&paths.endpoint);
    }
}

fn new_token() -> Result<String, AgentIpcError> {
    let mut bytes = [0_u8; 32];
    fill(&mut bytes).map_err(|error| AgentIpcError::Random(error.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[derive(Debug, Clone)]
struct AgentPaths {
    runtime_dir: PathBuf,
    endpoint: PathBuf,
    lock: PathBuf,
    job_sets: PathBuf,
}

impl AgentPaths {
    fn discover() -> Result<Self, AgentIpcError> {
        if let Some(value) = env::var_os("GATETREE_OPS_AGENT_RUNTIME_DIR")
            .or_else(|| env::var_os("PHAROS_AGENT_RUNTIME_DIR"))
        {
            return Ok(Self::at(PathBuf::from(value)));
        }
        let runtime_dir = default_runtime_dir()?;
        if let Some(base) = runtime_dir.parent().and_then(Path::parent) {
            migrate_legacy_runtime(&base.join("PHAROS").join("runtime"), &runtime_dir)?;
        }
        Ok(Self::at(runtime_dir))
    }

    fn at(runtime_dir: PathBuf) -> Self {
        Self {
            endpoint: runtime_dir.join(ENDPOINT_FILE),
            lock: runtime_dir.join(LOCK_FILE),
            job_sets: runtime_dir.join(JOB_SETS_FILE),
            runtime_dir,
        }
    }
}

// Import persisted plans once, without copying a live endpoint, token or credentials.
fn migrate_legacy_runtime(legacy: &Path, current: &Path) -> Result<(), AgentIpcError> {
    let source = legacy.join(JOB_SETS_FILE);
    let destination = current.join(JOB_SETS_FILE);
    if destination.exists() || !source.is_file() {
        return Ok(());
    }
    let old_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(legacy.join(LOCK_FILE))?;
    old_lock.try_lock_exclusive().map_err(|_| {
        AgentIpcError::Rejected(
            "The legacy PHAROS Agent is running. Stop it before migrating to GateTree-Ops.".into(),
        )
    })?;
    fs::create_dir_all(current)?;
    restrict_directory(current)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(current.join(LOCK_FILE))?;
    lock.lock_exclusive()?;
    if destination.exists() {
        return Ok(());
    }
    let bytes = fs::read(source)?;
    let _: Vec<AgentJobSetStatus> = serde_json::from_slice(&bytes)?;
    let temporary = current.join(format!("migration-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(temporary, destination)?;
    Ok(())
}

fn default_runtime_dir() -> Result<PathBuf, AgentIpcError> {
    #[cfg(target_os = "windows")]
    let base = env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join("Library").join("Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .map(|path| path.join(".local/state"))
        });
    base.map(|path| path.join("GateTree-Ops").join("runtime"))
        .ok_or(AgentIpcError::RuntimeDirectoryUnavailable)
}

fn agent_executable() -> Result<PathBuf, AgentIpcError> {
    if let Some(path) =
        env::var_os("GATETREE_OPS_AGENT_BIN").or_else(|| env::var_os("PHAROS_AGENT_BIN"))
    {
        return Ok(PathBuf::from(path));
    }
    let current = env::current_exe()?;
    let directory = current
        .parent()
        .ok_or(AgentIpcError::ExecutableDirectoryUnavailable)?;
    Ok(directory.join(if cfg!(target_os = "windows") {
        "gatetree-ops-agent.exe"
    } else {
        "gatetree-ops-agent"
    }))
}

#[cfg(unix)]
fn detach_command(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(target_os = "windows")]
fn detach_command(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

#[cfg(not(any(unix, target_os = "windows")))]
fn detach_command(_command: &mut Command) {}

#[cfg(unix)]
fn restrict_directory(path: &Path) -> Result<(), AgentIpcError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_directory(_path: &Path) -> Result<(), AgentIpcError> {
    Ok(())
}

#[derive(Debug, Error)]
pub enum AgentIpcError {
    #[error("GateTree-Ops Agent is not running")]
    NotRunning,
    #[error("GateTree-Ops Agent is already running")]
    AlreadyRunning,
    #[error("GateTree-Ops Agent executable was not found at {0}")]
    ExecutableNotFound(PathBuf),
    #[error("could not determine the GateTree-Ops Agent executable directory")]
    ExecutableDirectoryUnavailable,
    #[error("could not determine the GateTree-Ops Agent runtime directory")]
    RuntimeDirectoryUnavailable,
    #[error("GateTree-Ops Agent did not start in time")]
    StartupTimeout,
    #[error("GateTree-Ops Agent did not stop in time")]
    ShutdownTimeout,
    #[error("GateTree-Ops Agent IPC endpoint is invalid")]
    InvalidEndpoint,
    #[error(
        "An incompatible GateTree-Ops Agent is running. Stop it (or use Kill Agent), then Start the rebuilt Agent."
    )]
    ProtocolMismatch,
    #[error(
        "A different GateTree-Ops Agent version is running. Stop it (or use Kill Agent), then Start the rebuilt Agent."
    )]
    AgentVersionMismatch,
    #[error("GateTree-Ops Agent rejected the request: {0}")]
    Rejected(String),
    #[error("GateTree-Ops Agent IPC message is too large")]
    MessageTooLarge,
    #[error("GateTree-Ops Agent response did not contain {0}")]
    InvalidResponse(String),
    #[error("GateTree-Ops Agent SSH manager failed: {0}")]
    Ssh(String),
    #[error("GateTree-Ops Agent job set is invalid: {0}")]
    InvalidJobSet(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("could not create a secure IPC token: {0}")]
    Random(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_legacy_plan_state_once_without_endpoints_or_active_legacy_agents() {
        let directory = std::env::temp_dir().join(format!(
            "gatetree-ops-runtime-migration-{}",
            uuid::Uuid::new_v4()
        ));
        let legacy = directory.join("PHAROS/runtime");
        let current = directory.join("GateTree-Ops/runtime");
        fs::create_dir_all(&legacy).unwrap();
        let mut manager = JobSetManager::load(legacy.join(JOB_SETS_FILE)).unwrap();
        manager.submit(olam_submission(&["deploy"]), false).unwrap();
        fs::write(legacy.join(ENDPOINT_FILE), "do not copy").unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(legacy.join(LOCK_FILE))
            .unwrap();
        lock.try_lock_exclusive().unwrap();
        assert!(migrate_legacy_runtime(&legacy, &current).is_err());
        assert!(!current.exists());
        drop(lock);
        migrate_legacy_runtime(&legacy, &current).unwrap();
        assert!(!current.join(ENDPOINT_FILE).exists());
        assert_eq!(
            fs::read(legacy.join(JOB_SETS_FILE)).unwrap(),
            fs::read(current.join(JOB_SETS_FILE)).unwrap()
        );
        let loaded = JobSetManager::load(current.join(JOB_SETS_FILE)).unwrap();
        assert_eq!(loaded.job_sets.len(), 1);
        assert_eq!(loaded.job_sets[0].status, "stopped");
        let migrated = fs::read(current.join(JOB_SETS_FILE)).unwrap();
        migrate_legacy_runtime(&legacy, &current).unwrap();
        assert_eq!(fs::read(current.join(JOB_SETS_FILE)).unwrap(), migrated);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn inspects_and_stops_a_legacy_agent_but_rejects_job_requests() {
        let runtime = env::temp_dir().join(format!("gatetree-ops-legacy-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&runtime).unwrap();
        let paths = AgentPaths::at(runtime.clone());
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let endpoint = AgentEndpoint {
            protocol_version: PROTOCOL_VERSION - 1,
            address: listener.local_addr().unwrap(),
            token: "legacy-token".into(),
            pid: 12345,
        };
        write_endpoint(&paths, &endpoint).unwrap();
        let server = thread::spawn(move || {
            for expected in ["ping", "shutdown"] {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_request(&mut stream).unwrap();
                assert_eq!(request.protocol_version, PROTOCOL_VERSION - 1);
                assert_eq!(request.token, "legacy-token");
                assert!(matches!(
                    (&request.action, expected),
                    (AgentAction::Ping, "ping") | (AgentAction::Shutdown, "shutdown")
                ));
                let mut response = success_response();
                response.pid = 12345;
                response.protocol_version = PROTOCOL_VERSION - 1;
                response.version = "0.0.1".into();
                write_response(&mut stream, &response).unwrap();
            }
        });
        let status = status_at(&paths).unwrap();
        assert!(status.running);
        assert_eq!(status.protocol_version, Some(PROTOCOL_VERSION - 1));
        assert_eq!(status.version.as_deref(), Some("0.0.1"));
        assert!(matches!(
            request(&endpoint, AgentAction::ListJobSets),
            Err(AgentIpcError::ProtocolMismatch)
        ));
        assert!(request(&endpoint, AgentAction::Shutdown).unwrap().ok);
        server.join().unwrap();
        fs::remove_dir_all(runtime).unwrap();
    }

    fn olam_submission(actions: &[&str]) -> AgentJobSetSubmission {
        AgentJobSetSubmission {
            id: "plan".into(),
            plan_number: 1,
            name: "Plan".into(),
            process_name: "Process".into(),
            plan_definition: "inputs:\n  olam:\n    account: deploy".into(),
            process_definition: "{}".into(),
            credentials: vec![],
            jobs: actions
                .iter()
                .enumerate()
                .map(|(index, action)| AgentJobSubmission {
                    disabled: false,
                    id: format!("job-{index}"),
                    sequence: index as i64,
                    name: format!("Step {index}"),
                    driver: "olam".into(),
                    action: (*action).into(),
                    target: Some("host-1".into()),
                    details: String::new(),
                    payload: Some(r#"{"playbook_name":"service","VERSION":"1"}"#.into()),
                })
                .collect(),
        }
    }

    #[test]
    fn cx_update_requires_plan_context_without_an_external_request() {
        let mut job = olam_submission(&["deploy"]).jobs.remove(0);
        for driver in ["cx"] {
            job.driver = driver.into();
            job.action = "update".into();
            let job = AgentJobStatus {
                id: job.id.clone(),
                sequence: job.sequence,
                name: job.name.clone(),
                driver: job.driver.clone(),
                action: job.action.clone(),
                target: job.target.clone(),
                details: job.details.clone(),
                payload: None,
                status: "running".into(),
                log: String::new(),
                phase: None,
                validation_job_id: None,
                deployment_job_id: None,
                finished_at: None,
                report_progress: Default::default(),
            };
            let error = drivers::execute(
                "{}",
                "{}",
                &job,
                &[],
                &|_, _| panic!("must not launch"),
                &|_| panic!("must not execute"),
            );
            match error {
                Err(error) => {
                    assert!(error.contains("Plan report context"))
                }
                _ => panic!("update without Plan context must fail"),
            }
        }
    }

    #[test]
    fn disabled_jobs_are_persisted_skipped_reenabled_and_preserved_during_retry() {
        let path = env::temp_dir().join(format!(
            "gatetree-ops-disabled-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mut manager = JobSetManager::load(path.clone()).unwrap();
        let mut submission =
            olam_submission(&["validate_deployment", "deploy", "deploy", "deploy"]);
        submission.jobs[0].disabled = true;
        submission.jobs[2].disabled = true;
        let set = manager.submit(submission.clone(), false).unwrap();
        assert_eq!(set.jobs[0].status, "disabled");
        assert_eq!(set.jobs[1].status, "awaiting_approval");
        assert!(manager.set_job_enabled("job-1", false).is_err());
        assert!(manager.set_job_enabled("job-0", true).is_err()); // Already passed.
        manager.set_job_enabled("job-3", false).unwrap(); // Future queued job.
        manager.stop("plan").unwrap();
        manager.set_job_enabled("job-0", true).unwrap();
        manager.set_job_enabled("job-0", false).unwrap();
        let mut loaded = JobSetManager::load(path.clone()).unwrap();
        assert_eq!(loaded.job_sets[0].jobs[0].status, "disabled");
        loaded
            .retry(
                "job-1",
                vec![AgentCredential {
                    reference: "test".into(),
                    username: "u".into(),
                    password: "p".into(),
                }],
            )
            .unwrap();
        assert_eq!(loaded.job_sets[0].jobs[2].status, "disabled");
        assert_eq!(loaded.job_sets[0].jobs[3].status, "disabled");
        assert!(loaded.retry("job-2", vec![]).is_err());
        // A completed enabled row followed only by disabled rows completes the Plan.
        loaded.job_sets[0].jobs[1].status = "completed".into();
        activate_job(&mut loaded.job_sets[0], 2);
        assert_eq!(loaded.job_sets[0].status, "completed");
        for job in &mut submission.jobs {
            job.disabled = true;
        }
        let all_disabled = manager.submit(submission, true).unwrap();
        assert_eq!(all_disabled.status, "completed");
        assert!(all_disabled.jobs.iter().all(|job| job.status == "disabled"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn deploy_only_requires_approval_and_never_inserts_validation() {
        let path = env::temp_dir().join(format!(
            "gatetree-ops-deploy-only-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mut manager = JobSetManager::load(path.clone()).unwrap();
        let set = manager.submit(olam_submission(&["deploy"]), false).unwrap();
        assert_eq!(set.jobs.len(), 1);
        assert_eq!(set.status, "awaiting_approval");
        assert_eq!(set.jobs[0].phase.as_deref(), Some("deployment"));
        assert_eq!(set.jobs[0].validation_job_id, None);
        assert!(set.jobs[0].log.contains("No preceding matching validation"));
        assert!(manager.approve("job-0").is_err());
        manager
            .restore_credentials(
                "plan",
                vec![AgentCredential {
                    reference: "deploy".into(),
                    username: "user".into(),
                    password: "test-only".into(),
                }],
            )
            .unwrap();
        let approved = manager.approve("job-0").unwrap();
        assert_eq!(approved.status, "running");
        assert_eq!(approved.jobs[0].action, "deploy");
        assert_eq!(approved.jobs[0].phase.as_deref(), Some("deployment"));
        assert_eq!(approved.jobs[0].validation_job_id, None);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn approval_reuses_only_matching_completed_validation_and_keeps_legacy_jobs() {
        let path = env::temp_dir().join(format!(
            "gatetree-ops-validation-match-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mut manager = JobSetManager::load(path.clone()).unwrap();
        let mut set = manager
            .submit(olam_submission(&["validate_deployment", "deploy"]), false)
            .unwrap();
        set.jobs[0].status = "completed".into();
        set.jobs[0].validation_job_id = Some(111);
        set.jobs[0].log = "validated result".into();
        let original = set.clone();
        for difference in ["none", "target", "payload", "status"] {
            let mut set = original.clone();
            match difference {
                "target" => set.jobs[0].target = Some("host-2".into()),
                "payload" => {
                    set.jobs[0].payload =
                        Some(r#"{"playbook_name":"service","VERSION":"2"}"#.into())
                }
                "status" => set.jobs[0].status = "failed".into(),
                _ => {}
            }
            activate_job(&mut set, 1);
            assert_eq!(set.status, "awaiting_approval");
            assert_eq!(
                set.jobs[1].validation_job_id,
                if difference == "none" {
                    Some(111)
                } else {
                    None
                }
            );
            assert_eq!(
                set.jobs[1].log.contains("validated result"),
                difference == "none"
            );
        }
        manager.job_sets[0] = original;
        manager.job_sets[0].jobs[0].action = "validate".into(); // Legacy per-target validation.
        manager.job_sets[0].status = "stopped".into();
        manager.persist().unwrap();
        let mut persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        persisted[0]
            .as_object_mut()
            .unwrap()
            .remove("executionRevision");
        fs::write(&path, persisted.to_string()).unwrap();
        let mut loaded = JobSetManager::load(path.clone()).unwrap();
        assert_eq!(loaded.job_sets[0].jobs.len(), 2);
        assert_eq!(loaded.job_sets[0].jobs[0].action, "validate");
        assert_eq!(loaded.job_sets[0].jobs[0].log, "validated result");
        assert!(
            loaded
                .retry("job-1", vec![])
                .unwrap_err()
                .to_string()
                .contains("Restart the Plan")
        );
        assert!(
            loaded
                .approve("job-1")
                .unwrap_err()
                .to_string()
                .contains("Restart the Plan")
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn separate_validation_then_approved_deploy_launches_exactly_the_two_configured_templates() {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let observed = requests.clone();
        let server = thread::spawn(move || {
            for index in 0..8 {
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
                    if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:")
                    {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                reader.read_exact(&mut vec![0; length]).unwrap();
                let template = if index < 4 { 1 } else { 2 };
                let external_id = if index < 4 { 111 } else { 222 };
                let body = match index % 4 {
                    0 => {
                        assert!(
                            request.starts_with(&format!(
                                "GET /api/v2/job_templates/{template}/ HTTP/"
                            ))
                        );
                        serde_json::json!({"name": if template == 1 { "Validate" } else { "Deploy" }}).to_string()
                    }
                    1 => {
                        assert!(request.starts_with(&format!(
                            "POST /api/v2/job_templates/{template}/launch/ HTTP/"
                        )));
                        serde_json::json!({"id": external_id}).to_string()
                    }
                    2 => {
                        assert!(
                            request.starts_with(&format!("GET /api/v2/jobs/{external_id}/ HTTP/"))
                        );
                        r#"{"status":"successful"}"#.into()
                    }
                    _ => {
                        assert!(request.starts_with(&format!(
                            "GET /api/v2/jobs/{external_id}/stdout/?format=txt HTTP/"
                        )));
                        "ok: [host-1]".into()
                    }
                };
                observed.fetch_add(1, Ordering::SeqCst);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let path = env::temp_dir().join(format!(
            "gatetree-ops-explicit-actions-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mut manager = JobSetManager::load(path.clone()).unwrap();
        let mut submission = olam_submission(&["validate_deployment", "deploy"]);
        submission.process_definition = serde_json::json!({"details":{"olam":{
            "url": format!("http://{address}"), "account":"api", "payload_defaults":{"ENVIRONMENT":"test"},
            "validation_template_id":1, "validation_template_job":"Validate",
            "deployment_template_id":2, "deployment_template_job":"Deploy"
        }}}).to_string();
        let credentials: Vec<_> = ["api", "deploy"]
            .iter()
            .map(|reference| AgentCredential {
                reference: (*reference).into(),
                username: "user".into(),
                password: "test-only".into(),
            })
            .collect();
        manager.submit(submission, false).unwrap();
        manager
            .credentials
            .insert("plan".into(), credentials.clone());
        let manager = Arc::new(Mutex::new(manager));
        spawn_job_set_execution(manager.clone(), "plan".into(), credentials.clone());
        let await_status = |expected: &str| {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let set = manager.lock().unwrap().job_sets[0].clone();
                if set.status == expected {
                    return set;
                }
                assert!(
                    Instant::now() < deadline,
                    "unexpected state: {}",
                    set.status
                );
                thread::sleep(Duration::from_millis(10));
            }
        };
        let awaiting = await_status("awaiting_approval");
        assert_eq!(requests.load(Ordering::SeqCst), 4);
        assert_eq!(awaiting.jobs[0].status, "completed");
        assert_eq!(awaiting.jobs[0].validation_job_id, Some(111));
        assert_eq!(awaiting.jobs[0].deployment_job_id, None);
        assert_eq!(awaiting.jobs[1].validation_job_id, Some(111));
        assert!(awaiting.jobs[1].log.contains("ok: [host-1]"));
        manager.lock().unwrap().approve("job-1").unwrap();
        spawn_job_set_execution(manager.clone(), "plan".into(), credentials);
        let completed = await_status("completed");
        assert_eq!(requests.load(Ordering::SeqCst), 8);
        assert_eq!(completed.jobs.len(), 2);
        assert_eq!(completed.jobs[1].deployment_job_id, Some(222));
        server.join().unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn combined_workflow_finishes_each_host_before_validating_the_next() {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let observed = requests.clone();
        let server = thread::spawn(move || {
            for index in 0..16 {
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
                    if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:")
                    {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                reader.read_exact(&mut vec![0; length]).unwrap();
                let template = if index % 8 < 4 { 1 } else { 2 };
                let external_id = 111 + index / 4;
                let body = match index % 4 {
                    0 => {
                        assert!(
                            request.starts_with(&format!(
                                "GET /api/v2/job_templates/{template}/ HTTP/"
                            ))
                        );
                        serde_json::json!({"name": if template == 1 { "Validate" } else { "Deploy" }}).to_string()
                    }
                    1 => {
                        assert!(request.starts_with(&format!(
                            "POST /api/v2/job_templates/{template}/launch/ HTTP/"
                        )));
                        serde_json::json!({"id": external_id}).to_string()
                    }
                    2 => {
                        assert!(
                            request.starts_with(&format!("GET /api/v2/jobs/{external_id}/ HTTP/"))
                        );
                        r#"{"status":"successful"}"#.into()
                    }
                    _ => {
                        assert!(request.starts_with(&format!(
                            "GET /api/v2/jobs/{external_id}/stdout/?format=txt HTTP/"
                        )));
                        "ok: [host-1]".into()
                    }
                };
                observed.fetch_add(1, Ordering::SeqCst);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let path = env::temp_dir().join(format!(
            "gatetree-ops-explicit-actions-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mut manager = JobSetManager::load(path.clone()).unwrap();
        let mut submission = olam_submission(&[
            "validate_deploy",
            "validate_deploy",
            "validate_deploy",
            "validate_deploy",
        ]);
        submission.jobs[1].disabled = true;
        submission.jobs[3].disabled = true;
        submission.jobs[2].target = Some("host-2".into());
        submission.jobs[2].payload =
            Some(r#"{"ansible_limit":["host-2"],"playbook_name":"service"}"#.into());
        submission.process_definition = serde_json::json!({"details":{"olam":{
            "url": format!("http://{address}"), "account":"api", "payload_defaults":{"ENVIRONMENT":"test"},
            "validation_template_id":1, "validation_template_job":"Validate",
            "deployment_template_id":2, "deployment_template_job":"Deploy"
        }}}).to_string();
        let credentials: Vec<_> = ["api", "deploy"]
            .iter()
            .map(|reference| AgentCredential {
                reference: (*reference).into(),
                username: "user".into(),
                password: "test-only".into(),
            })
            .collect();
        manager.submit(submission, false).unwrap();
        manager
            .credentials
            .insert("plan".into(), credentials.clone());
        let manager = Arc::new(Mutex::new(manager));
        spawn_job_set_execution(manager.clone(), "plan".into(), credentials.clone());
        let await_status = |expected: &str| {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let set = manager.lock().unwrap().job_sets[0].clone();
                if set.status == expected {
                    return set;
                }
                assert!(
                    Instant::now() < deadline,
                    "unexpected state: {}",
                    set.status
                );
                thread::sleep(Duration::from_millis(10));
            }
        };
        let awaiting = await_status("awaiting_approval");
        assert_eq!(requests.load(Ordering::SeqCst), 4);
        assert_eq!(awaiting.jobs.len(), 4);
        assert_eq!(awaiting.jobs[0].status, "awaiting_approval");
        assert_eq!(awaiting.jobs[0].validation_job_id, Some(111));
        assert_eq!(awaiting.jobs[0].deployment_job_id, None);
        assert_eq!(awaiting.jobs[2].status, "queued");
        assert_eq!(awaiting.jobs[2].validation_job_id, None);
        manager.lock().unwrap().approve("job-0").unwrap();
        spawn_job_set_execution(manager.clone(), "plan".into(), credentials.clone());
        // Wait for the second host's approval, not the first host's old state.
        let deadline = Instant::now() + Duration::from_secs(5);
        let second = loop {
            let set = manager.lock().unwrap().job_sets[0].clone();
            if set.jobs[2].status == "awaiting_approval" {
                break set;
            }
            assert!(
                Instant::now() < deadline,
                "second host did not reach approval"
            );
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(requests.load(Ordering::SeqCst), 12);
        assert_eq!(second.jobs[0].status, "completed");
        assert_eq!(second.jobs[0].deployment_job_id, Some(112));
        assert_eq!(second.jobs[2].validation_job_id, Some(113));
        assert_eq!(second.jobs[2].deployment_job_id, None);
        manager.lock().unwrap().approve("job-2").unwrap();
        spawn_job_set_execution(manager.clone(), "plan".into(), credentials);
        let completed = await_status("completed");
        assert_eq!(requests.load(Ordering::SeqCst), 16);
        assert_eq!(completed.jobs.len(), 4);
        assert_eq!(completed.jobs[2].deployment_job_id, Some(114));
        assert_eq!(completed.jobs[1].status, "disabled");
        assert_eq!(completed.jobs[3].status, "disabled");
        server.join().unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn starting_a_stopped_plan_reruns_completed_checks_and_clears_old_runs() {
        let path = env::temp_dir().join(format!(
            "gatetree-ops-restart-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mut manager = JobSetManager::load(path.clone()).unwrap();
        let submission = AgentJobSetSubmission {
            id: "plan".into(),
            plan_number: 3,
            name: "Plan".into(),
            process_name: "Process".into(),
            plan_definition: "{}".into(),
            process_definition: "{}".into(),
            credentials: vec![],
            jobs: (0..4)
                .map(|index| AgentJobSubmission {
                    disabled: false,
                    id: format!("job-{index}"),
                    sequence: index,
                    name: format!("Step {index}"),
                    driver: if index < 2 { "jira" } else { "olam" }.into(),
                    action: if index < 2 { "validate" } else { "deploy" }.into(),
                    target: None,
                    details: String::new(),
                    payload: None,
                })
                .collect(),
        };
        manager.submit(submission.clone(), false).unwrap();
        manager.job_sets[0].jobs[0].status = "completed".into();
        manager.job_sets[0].jobs[1].status = "completed".into();
        manager.job_sets[0].jobs[2].status = "running".into();
        manager.job_sets[0].jobs[2].phase = Some("deployment".into());
        manager.job_sets[0].jobs[2].validation_job_id = Some(162);
        manager.job_sets[0].jobs[2].deployment_job_id = Some(166);
        for job in &mut manager.job_sets[0].jobs {
            job.log = "Previous run".into();
        }
        manager.execution_generations.insert("plan".into(), 1);
        manager.stop("plan").unwrap();
        // Stopping invalidates the old worker even before a new run starts.
        assert_eq!(manager.execution_generations["plan"], 2);
        let restarted = manager.submit(submission, false).unwrap();
        assert_eq!(restarted.status, "running");
        assert_eq!(restarted.jobs[0].status, "running");
        assert!(restarted.jobs[1..].iter().all(|job| job.status == "queued"));
        assert!(restarted.jobs.iter().all(|job| job.log.is_empty()
            && job.validation_job_id.is_none()
            && job.deployment_job_id.is_none()));
        assert_eq!(restarted.jobs[2].phase.as_deref(), Some("deployment"));
        let saved: Vec<AgentJobSetStatus> =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved[0].jobs[0].status, "running");
        assert_eq!(saved[0].jobs[1].status, "queued");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn denying_validation_stops_the_plan_until_an_explicit_retry() {
        let path = env::temp_dir().join(format!("gatetree-ops-deny-{}.json", uuid::Uuid::new_v4()));
        let mut manager = JobSetManager::load(path.clone()).unwrap();
        manager
            .submit(
                AgentJobSetSubmission {
                    id: "plan".into(),
                    plan_number: 1,
                    name: "Plan".into(),
                    process_name: "Process".into(),
                    plan_definition: "{}".into(),
                    process_definition: "{}".into(),
                    credentials: vec![],
                    jobs: (0..2)
                        .map(|index| AgentJobSubmission {
                            disabled: false,
                            id: format!("job-{index}"),
                            sequence: index,
                            name: format!("Step {index}"),
                            driver: "olam".into(),
                            action: "deploy".into(),
                            target: None,
                            details: String::new(),
                            payload: None,
                        })
                        .collect(),
                },
                false,
            )
            .unwrap();
        assert!(manager.deny("job-1").is_err());
        manager.job_sets[0].status = "awaiting_approval".into();
        manager.job_sets[0].jobs[0].status = "awaiting_approval".into();
        manager.job_sets[0].jobs[0].log = "Validation passed".into();
        manager.deny("job-0").unwrap();
        let mut manager = JobSetManager::load(path.clone()).unwrap();
        assert_eq!(manager.job_sets[0].status, "denied");
        assert_eq!(manager.job_sets[0].jobs[0].status, "denied");
        assert_eq!(
            manager.job_sets[0].jobs[0].phase.as_deref(),
            Some("deployment")
        );
        assert_eq!(manager.job_sets[0].jobs[1].status, "queued");
        assert!(
            manager.job_sets[0].jobs[0]
                .log
                .contains("Validation passed")
        );
        assert!(
            manager.job_sets[0].jobs[0]
                .log
                .contains("denied by the operator")
        );
        assert!(manager.approve("job-0").is_err());
        assert!(manager.deny("job-0").is_err());
        let retried = manager
            .retry(
                "job-0",
                vec![AgentCredential {
                    reference: "deploy".into(),
                    username: "user".into(),
                    password: "memory-only".into(),
                }],
            )
            .unwrap();
        assert_eq!(retried.jobs[0].phase.as_deref(), Some("deployment"));
        assert_eq!(retried.jobs[0].status, "awaiting_approval");
        assert_eq!(retried.jobs[1].status, "queued");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn restores_credentials_after_restart_and_retries_only_from_failed_job() {
        let path =
            env::temp_dir().join(format!("gatetree-ops-retry-{}.json", uuid::Uuid::new_v4()));
        let mut manager = JobSetManager::load(path.clone()).unwrap();
        let jobs = (0..3)
            .map(|index| AgentJobSubmission {
                disabled: false,
                id: format!("job-{index}"),
                sequence: index,
                name: format!("Step {index}"),
                driver: "olam".into(),
                action: "deploy".into(),
                target: None,
                details: String::new(),
                payload: None,
            })
            .collect();
        manager
            .submit(
                AgentJobSetSubmission {
                    id: "plan".into(),
                    plan_number: 1,
                    name: "Plan".into(),
                    process_name: "Process".into(),
                    plan_definition: "{}".into(),
                    process_definition: "{}".into(),
                    jobs,
                    credentials: vec![],
                },
                false,
            )
            .unwrap();
        manager.job_sets[0].jobs[0].status = "completed".into();
        manager.job_sets[0].jobs[1].status = "failed".into();
        manager.job_sets[0].jobs[1].phase = Some("deployment".into());
        manager.job_sets[0].jobs[0].validation_job_id = Some(101);
        manager.job_sets[0].jobs[0].deployment_job_id = Some(102);
        manager.job_sets[0].jobs[1].validation_job_id = Some(103);
        manager.job_sets[0].jobs[1].deployment_job_id = Some(108);
        manager.job_sets[0].jobs[0].log = "Previous successful step".into();
        manager.job_sets[0].jobs[1].log = "OLAM job 108 failed: UNREACHABLE".into();
        manager.job_sets[0].jobs[2].log = "Output from an earlier run".into();
        manager.job_sets[0].status = "failed".into();
        manager.persist().unwrap();
        let mut manager = JobSetManager::load(path.clone()).unwrap();
        assert!(manager.list()[0].credentials_required);
        assert!(manager.retry("job-1", vec![]).is_err());
        let credentials = vec![AgentCredential {
            reference: "deploy".into(),
            username: "user".into(),
            password: "secret-only-in-memory".into(),
        }];
        manager.restore_credentials("plan", credentials).unwrap();
        assert!(!manager.list()[0].credentials_required);
        assert_eq!(manager.job_sets[0].status, "failed");
        assert!(manager.retry("job-2", vec![]).is_err());
        let retried = manager.retry("job-1", vec![]).unwrap();
        assert_eq!(retried.status, "awaiting_approval");
        assert_eq!(retried.jobs[0].status, "completed");
        assert_eq!(retried.jobs[1].status, "awaiting_approval");
        assert_eq!(retried.jobs[1].phase.as_deref(), Some("deployment"));
        assert_eq!(retried.jobs[0].deployment_job_id, Some(102));
        assert_eq!(retried.jobs[1].validation_job_id, None);
        assert_eq!(retried.jobs[1].deployment_job_id, None);
        assert_eq!(retried.jobs[2].status, "queued");
        assert_eq!(retried.jobs[0].log, "Previous successful step");
        assert!(
            retried.jobs[1]
                .log
                .contains("No preceding matching validation")
        );
        assert!(
            retried.jobs[1]
                .log
                .contains("Retry requested by the operator.")
        );
        assert!(retried.jobs[2].log.is_empty());
        let saved: Vec<AgentJobSetStatus> =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved[0].jobs[1].log, retried.jobs[1].log);
        assert!(saved[0].jobs[2].log.is_empty());
        assert!(manager.retry("job-1", vec![]).is_err());
        assert!(
            !fs::read_to_string(&path)
                .unwrap()
                .contains("secret-only-in-memory")
        );
        let manager = JobSetManager::load(path.clone()).unwrap();
        assert!(manager.list()[0].credentials_required);
        assert_eq!(manager.job_sets[0].status, "stopped");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn update_reservation_rejects_active_plans_and_blocks_new_work_until_cancelled() {
        let runtime = env::temp_dir().join(format!(
            "gatetree-ops-update-agent-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = AgentPaths::at(runtime.clone());
        let server_paths = paths.clone();
        let server = thread::spawn(move || run_server_at(server_paths));
        let deadline = Instant::now() + Duration::from_secs(3);
        let endpoint = loop {
            if status_at(&paths).is_ok() {
                break read_endpoint(&paths).unwrap();
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        };
        reserve_update(&endpoint).unwrap();
        let blocked = request(
            &endpoint,
            AgentAction::SubmitJobSet {
                submission: olam_submission(&["deploy"]),
                restart: false,
            },
        )
        .unwrap();
        assert!(!blocked.ok && blocked.error.unwrap().contains("update is in progress"));
        assert!(
            request(&endpoint, AgentAction::ListJobSets)
                .unwrap()
                .job_sets
                .unwrap()
                .is_empty()
        );
        assert!(request(&endpoint, AgentAction::CancelUpdate).unwrap().ok);
        let submitted = request(
            &endpoint,
            AgentAction::SubmitJobSet {
                submission: olam_submission(&["deploy"]),
                restart: false,
            },
        )
        .unwrap();
        assert_eq!(submitted.job_set.unwrap().status, "awaiting_approval");
        assert!(
            reserve_update(&endpoint)
                .unwrap_err()
                .to_string()
                .contains("active deployments")
        );
        let sets = request(&endpoint, AgentAction::ListJobSets)
            .unwrap()
            .job_sets
            .unwrap();
        assert_eq!(sets[0].status, "awaiting_approval");
        assert!(
            request(
                &endpoint,
                AgentAction::StopJobSet {
                    job_set_id: "plan".into()
                }
            )
            .unwrap()
            .ok
        );
        reserve_update(&endpoint).unwrap();
        assert!(request(&endpoint, AgentAction::Shutdown).unwrap().ok);
        server.join().unwrap().unwrap();
        fs::remove_dir_all(runtime).unwrap();
    }

    #[test]
    fn serves_authenticated_status_and_shutdown_requests() {
        let runtime =
            env::temp_dir().join(format!("gatetree-ops-agent-test-{}", uuid::Uuid::new_v4()));
        let paths = AgentPaths::at(runtime.clone());
        let server_paths = paths.clone();
        let server = thread::spawn(move || run_server_at(server_paths));

        let deadline = Instant::now() + Duration::from_secs(3);
        let running = loop {
            if let Ok(status) = status_at(&paths) {
                break status;
            }
            assert!(Instant::now() < deadline, "agent test server did not start");
            thread::sleep(Duration::from_millis(20));
        };
        assert!(running.running);
        assert_eq!(running.pid, Some(std::process::id()));
        let sessions = request(
            &read_endpoint(&paths).unwrap(),
            AgentAction::ListSshSessions,
        )
        .unwrap()
        .ssh_sessions
        .unwrap();
        assert!(sessions.is_empty());
        let submission = AgentJobSetSubmission {
            id: "plan-1".into(),
            plan_number: 1,
            name: "Test plan".into(),
            process_name: "Test process".into(),
            plan_definition: "plan_name: Test plan".into(),
            process_definition: "{}".into(),
            jobs: vec![AgentJobSubmission {
                disabled: false,
                id: "job-1".into(),
                sequence: 1,
                name: "Validate".into(),
                driver: "test".into(),
                action: "validate".into(),
                target: None,
                details: String::new(),
                payload: None,
            }],
            credentials: vec![AgentCredential {
                reference: "test-account".into(),
                username: "test-user".into(),
                password: "must-never-be-persisted".into(),
            }],
        };
        let submitted = request(
            &read_endpoint(&paths).unwrap(),
            AgentAction::SubmitJobSet {
                submission,
                restart: false,
            },
        )
        .unwrap()
        .job_set
        .unwrap();
        assert_eq!(submitted.status, "running");
        assert_eq!(submitted.jobs[0].status, "running");
        let job_sets = request(&read_endpoint(&paths).unwrap(), AgentAction::ListJobSets)
            .unwrap()
            .job_sets
            .unwrap();
        assert_eq!(job_sets.len(), 1);
        let endpoint = read_endpoint(&paths).unwrap();
        assert!(
            request(
                &endpoint,
                AgentAction::StopJobSet {
                    job_set_id: "plan-1".into()
                }
            )
            .unwrap()
            .ok
        );
        assert!(
            request(
                &endpoint,
                AgentAction::RestoreJobSetCredentials {
                    job_set_id: "plan-1".into(),
                    credentials: vec![AgentCredential {
                        reference: "test-account".into(),
                        username: "user".into(),
                        password: "restored-secret".into(),
                    }],
                }
            )
            .unwrap()
            .ok
        );
        assert!(
            request(
                &endpoint,
                AgentAction::RetryJob {
                    job_id: "job-1".into(),
                    credentials: vec![]
                }
            )
            .unwrap()
            .ok
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        let retried = loop {
            let sets = request(&endpoint, AgentAction::ListJobSets)
                .unwrap()
                .job_sets
                .unwrap();
            if sets[0].status != "running" {
                break sets;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(retried[0].status, "failed");
        assert_eq!(retried[0].jobs[0].status, "failed");
        assert!(
            retried[0].jobs[0]
                .log
                .contains("Action test/validate is not implemented")
        );
        assert!(paths.job_sets.is_file());
        let persisted = fs::read_to_string(&paths.job_sets).unwrap();
        assert!(!persisted.contains("must-never-be-persisted"));
        assert!(!persisted.contains("restored-secret"));
        assert!(matches!(
            run_server_at(paths.clone()),
            Err(AgentIpcError::AlreadyRunning)
        ));

        let endpoint = read_endpoint(&paths).unwrap();
        let response = request(&endpoint, AgentAction::Shutdown).unwrap();
        assert!(response.ok);
        server.join().unwrap().unwrap();
        assert!(!paths.endpoint.exists());
        assert!(matches!(status_at(&paths), Err(AgentIpcError::NotRunning)));
        let _ = fs::remove_dir_all(runtime);
    }
}
