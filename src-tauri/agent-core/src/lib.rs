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

pub const PROTOCOL_VERSION: u16 = 10;
const ENDPOINT_FILE: &str = "agent-endpoint.json";
const LOCK_FILE: &str = "agent.lock";
const JOB_SETS_FILE: &str = "agent-job-sets.json";
const MAX_MESSAGE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJobSubmission {
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJobSetStatus {
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
            != Some("pharos-agent")
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
            .args(["/PID", &pid, "/F", "/FI", "IMAGENAME eq pharos-agent.exe"])
            .output()?;
        if !result.status.success() {
            return Err(AgentIpcError::Rejected(
                "Could not kill the PHAROS Agent process".into(),
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
            let previous_len = set.jobs.len();
            set.jobs
                .retain(|job| !(job.driver == "olam" && job.action == "validate"));
            if set.jobs.len() != previous_len {
                changed = true;
            }
            for job in &mut set.jobs {
                if job.driver == "olam" && job.action == "deploy" && job.phase.is_none() {
                    job.phase = Some("validation".into());
                    changed = true;
                }
            }
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
                    "Agent restarted. Unlock KeePass to restore credentials, then start the Deployment Plan to validate again.",
                );
            }
        }
        let manager = Self {
            path,
            job_sets,
            credentials: HashMap::new(),
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
        let mut job_set = AgentJobSetStatus {
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
                    let phase = (job.driver == "olam" && job.action == "deploy")
                        .then(|| "validation".into());
                    AgentJobStatus {
                        id: job.id,
                        sequence: job.sequence,
                        name: job.name,
                        driver: job.driver,
                        action: job.action,
                        target: job.target,
                        details: job.details,
                        payload: job.payload,
                        status: "queued".into(),
                        log: String::new(),
                        phase,
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
            if !restart && unchanged {
                let current = &mut self.job_sets[existing];
                if current.status == "running" {
                    return Ok(current.clone());
                }
                current.status = "running".into();
                if let Some(job) = current
                    .jobs
                    .iter_mut()
                    .find(|job| job.status != "completed")
                {
                    job.status = "running".into();
                }
                let result = current.clone();
                self.persist()?;
                return Ok(result);
            }
            self.job_sets.remove(existing);
        }

        if let Some(first) = job_set.jobs.first_mut() {
            first.status = "running".into();
        }
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
        let index = set.jobs.iter().position(|job| job.id == job_id).unwrap();
        if set.status == "running"
            || set.jobs[..index]
                .iter()
                .any(|job| job.status != "completed")
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
        for job in &mut set.jobs[index..] {
            job.status = "queued".into();
            job.log.clear();
            if job.driver == "olam" && job.action == "deploy" {
                job.phase = Some("validation".into());
            }
        }
        set.jobs[index].status = "running".into();
        append_job_log(
            &mut set.jobs[index].log,
            "Retry requested by the operator; starting a new validation run.",
        );
        set.status = "running".into();
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
                    && set.jobs.iter().any(|job| {
                        (matches!(job.driver.as_str(), "jira" | "cx") && job.action == "validate")
                            || (job.driver == "olam" && job.action == "deploy")
                    });
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

    fn approve(&mut self, job_id: &str) -> Result<AgentJobSetStatus, AgentIpcError> {
        let job_set = self
            .job_sets
            .iter_mut()
            .find(|set| set.jobs.iter().any(|job| job.id == job_id))
            .ok_or_else(|| AgentIpcError::InvalidJobSet("job was not found".into()))?;
        let job = job_set
            .jobs
            .iter_mut()
            .find(|job| job.id == job_id)
            .expect("job was located above");
        if job.status != "awaiting_approval" {
            return Err(AgentIpcError::InvalidJobSet(
                "only a successfully validated job can be approved".into(),
            ));
        }
        if !self.credentials.contains_key(&job_set.id) {
            return Err(AgentIpcError::InvalidJobSet(
                "Agent credentials are no longer in memory; restart the Plan and validate again"
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
        if let Some(job) = job_set.jobs.iter_mut().find(|job| job.status == "running") {
            job.status = "stopped".into();
        }
        let result = job_set.clone();
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
    for incoming in listener.incoming() {
        let mut stream = incoming?;
        if !stream.peer_addr()?.ip().is_loopback() {
            continue;
        }
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let request = read_request(&mut stream);
        let (response, shutdown) = match request {
            Ok(request)
                if request.protocol_version == PROTOCOL_VERSION
                    && request.token == endpoint.token =>
            {
                match request.action {
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
                    AgentAction::ConnectSsh { request } => match ssh_sessions.connect(request) {
                        Ok(session) => {
                            let mut response = success_response();
                            response.ssh_session = Some(session);
                            (response, false)
                        }
                        Err(error) => (error_response(&error.to_string()), false),
                    },
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
                        let submitted = {
                            let mut manager = job_sets.lock().expect("job set lock");
                            let result = manager.submit(submission, restart);
                            if result.is_ok() {
                                manager
                                    .credentials
                                    .insert(job_set_id.clone(), credentials.clone());
                            }
                            result
                        };
                        match submitted {
                            Ok(job_set) => {
                                spawn_job_set_execution(job_sets.clone(), job_set_id, credentials);
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
    thread::spawn(move || {
        loop {
            let current = {
                let manager = job_sets.lock().expect("job set lock");
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
                                )
                            })
                    })
            };
            let Some((process_definition, plan_definition, job)) = current else {
                return;
            };
            {
                let mut manager = job_sets.lock().expect("job set lock");
                if let Some(set) = manager.job_sets.iter_mut().find(|set| set.id == job_set_id)
                    && let Some(active) = set.jobs.iter_mut().find(|item| item.id == job.id)
                {
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
            let Some(result) =
                drivers::execute(&process_definition, &plan_definition, &job, &credentials)
            else {
                return;
            };
            let mut manager = job_sets.lock().expect("job set lock");
            let Some(set) = manager.job_sets.iter_mut().find(|set| set.id == job_set_id) else {
                return;
            };
            if set.status != "running" {
                return;
            }
            let Some(index) = set.jobs.iter().position(|item| item.id == job.id) else {
                return;
            };
            match result {
                Ok(drivers::DriverOutcome::Completed(log)) => {
                    set.jobs[index].status = "completed".into();
                    append_job_log(&mut set.jobs[index].log, &log);
                    if let Some(next) = set.jobs.get_mut(index + 1) {
                        next.status = "running".into();
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
        let runtime_dir = if let Some(value) = env::var_os("PHAROS_AGENT_RUNTIME_DIR") {
            PathBuf::from(value)
        } else {
            default_runtime_dir()?
        };
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
    base.map(|path| path.join("PHAROS").join("runtime"))
        .ok_or(AgentIpcError::RuntimeDirectoryUnavailable)
}

fn agent_executable() -> Result<PathBuf, AgentIpcError> {
    if let Some(path) = env::var_os("PHAROS_AGENT_BIN") {
        return Ok(PathBuf::from(path));
    }
    let current = env::current_exe()?;
    let directory = current
        .parent()
        .ok_or(AgentIpcError::ExecutableDirectoryUnavailable)?;
    Ok(directory.join(if cfg!(target_os = "windows") {
        "pharos-agent.exe"
    } else {
        "pharos-agent"
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
    #[error("PHAROS Agent is not running")]
    NotRunning,
    #[error("PHAROS Agent is already running")]
    AlreadyRunning,
    #[error("PHAROS Agent executable was not found at {0}")]
    ExecutableNotFound(PathBuf),
    #[error("could not determine the PHAROS Agent executable directory")]
    ExecutableDirectoryUnavailable,
    #[error("could not determine the PHAROS Agent runtime directory")]
    RuntimeDirectoryUnavailable,
    #[error("PHAROS Agent did not start in time")]
    StartupTimeout,
    #[error("PHAROS Agent did not stop in time")]
    ShutdownTimeout,
    #[error("PHAROS Agent IPC endpoint is invalid")]
    InvalidEndpoint,
    #[error(
        "An incompatible PHAROS Agent is running. Stop it (or use Kill Agent), then Start the rebuilt Agent."
    )]
    ProtocolMismatch,
    #[error(
        "A different PHAROS Agent version is running. Stop it (or use Kill Agent), then Start the rebuilt Agent."
    )]
    AgentVersionMismatch,
    #[error("PHAROS Agent rejected the request: {0}")]
    Rejected(String),
    #[error("PHAROS Agent IPC message is too large")]
    MessageTooLarge,
    #[error("PHAROS Agent response did not contain {0}")]
    InvalidResponse(String),
    #[error("PHAROS Agent SSH manager failed: {0}")]
    Ssh(String),
    #[error("PHAROS Agent job set is invalid: {0}")]
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
    fn inspects_and_stops_a_legacy_agent_but_rejects_job_requests() {
        let runtime = env::temp_dir().join(format!("pharos-legacy-{}", uuid::Uuid::new_v4()));
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

    #[test]
    fn denying_validation_stops_the_plan_until_an_explicit_retry() {
        let path = env::temp_dir().join(format!("pharos-deny-{}.json", uuid::Uuid::new_v4()));
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
        assert!(manager.deny("job-0").is_err());
        manager.job_sets[0].status = "awaiting_approval".into();
        manager.job_sets[0].jobs[0].status = "awaiting_approval".into();
        manager.job_sets[0].jobs[0].log = "Validation passed".into();
        manager.deny("job-0").unwrap();
        let mut manager = JobSetManager::load(path.clone()).unwrap();
        assert_eq!(manager.job_sets[0].status, "denied");
        assert_eq!(manager.job_sets[0].jobs[0].status, "denied");
        assert_eq!(
            manager.job_sets[0].jobs[0].phase.as_deref(),
            Some("validation")
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
        assert_eq!(retried.jobs[0].phase.as_deref(), Some("validation"));
        assert_eq!(retried.jobs[0].status, "running");
        assert_eq!(retried.jobs[1].status, "queued");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn restores_credentials_after_restart_and_retries_only_from_failed_job() {
        let path = env::temp_dir().join(format!("pharos-retry-{}.json", uuid::Uuid::new_v4()));
        let mut manager = JobSetManager::load(path.clone()).unwrap();
        let jobs = (0..3)
            .map(|index| AgentJobSubmission {
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
        assert_eq!(retried.status, "running");
        assert_eq!(retried.jobs[0].status, "completed");
        assert_eq!(retried.jobs[1].status, "running");
        assert_eq!(retried.jobs[1].phase.as_deref(), Some("validation"));
        assert_eq!(retried.jobs[2].status, "queued");
        assert_eq!(retried.jobs[0].log, "Previous successful step");
        assert_eq!(
            retried.jobs[1].log,
            "Retry requested by the operator; starting a new validation run."
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
    fn serves_authenticated_status_and_shutdown_requests() {
        let runtime = env::temp_dir().join(format!("pharos-agent-test-{}", uuid::Uuid::new_v4()));
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
        let retried = request(&endpoint, AgentAction::ListJobSets)
            .unwrap()
            .job_sets
            .unwrap();
        assert_eq!(retried[0].status, "running");
        assert_eq!(retried[0].jobs[0].status, "running");
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
