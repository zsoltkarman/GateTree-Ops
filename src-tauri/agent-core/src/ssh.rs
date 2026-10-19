use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use russh::{
    ChannelMsg, Disconnect,
    client::{self, AuthResult, KeyboardInteractiveAuthResponse},
    keys::{
        HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey,
        agent::{
            AgentIdentity,
            client::{AgentClient, AgentStream},
        },
    },
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::runtime::{Builder, Runtime};
use uuid::Uuid;
use zeroize::Zeroizing;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_TERMINAL_BUFFER: usize = 4 * 1024 * 1024;
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(20);
const KEEPALIVE_MAX_FAILURES: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshHostKey {
    pub public_key: String,
    pub fingerprint: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SshAuthentication {
    Password {
        password: String,
    },
    PrivateKey {
        private_key: String,
        passphrase: Option<String>,
    },
    KeyboardInteractive {
        response: String,
    },
    SshAgent,
    OpenSsh,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConnectRequest {
    pub connection_id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub authentication: SshAuthentication,
    pub trusted_host_key: String,
    pub columns: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshSessionStatus {
    pub id: String,
    pub connection_id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub status: String,
    pub connected_at: u64,
    pub buffered_bytes: usize,
    pub keepalive_interval_seconds: u64,
    pub keepalive_max_failures: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshTerminalChunk {
    pub data: Vec<u8>,
    pub closed: bool,
    pub next_offset: u64,
    pub truncated: bool,
}

#[derive(Default)]
struct TerminalBuffer {
    bytes: VecDeque<u8>,
    start_offset: u64,
    end_offset: u64,
}

struct ManagedSession {
    status: SshSessionStatus,
    backend: SessionBackend,
    output: Arc<Mutex<TerminalBuffer>>,
    closed: Arc<AtomicBool>,
}

enum SessionBackend {
    Russh {
        handle: client::Handle<StrictHostKeyClient>,
        writer: russh::ChannelWriteHalf<client::Msg>,
    },
    OpenSsh {
        child: Box<dyn portable_pty::Child + Send + Sync>,
        master: Box<dyn MasterPty + Send>,
        writer: Mutex<Box<dyn Write + Send>>,
    },
}

pub(crate) struct SshSessionManager {
    runtime: Runtime,
    sessions: HashMap<String, ManagedSession>,
}

impl SshSessionManager {
    pub(crate) fn new() -> Result<Self, SshManagerError> {
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .thread_name("gatetree-ops-ssh")
            .build()?;
        Ok(Self {
            runtime,
            sessions: HashMap::new(),
        })
    }

    pub(crate) fn scan_host_key(
        &self,
        host: String,
        port: u16,
    ) -> Result<SshHostKey, SshManagerError> {
        validate_host(&host, port)?;
        self.runtime.block_on(async move {
            let captured = Arc::new(Mutex::new(None));
            let handler = HostKeyProbe {
                captured: Arc::clone(&captured),
            };
            let config = Arc::new(client_config());
            let _ = tokio::time::timeout(
                CONNECT_TIMEOUT,
                client::connect(config, (host.as_str(), port), handler),
            )
            .await
            .map_err(|_| SshManagerError::Timeout)?;
            let key = captured
                .lock()
                .map_err(|_| SshManagerError::StatePoisoned)?
                .clone()
                .ok_or(SshManagerError::HostKeyUnavailable)?;
            host_key_details(&key)
        })
    }

    pub(crate) fn connect(
        &mut self,
        request: SshConnectRequest,
    ) -> Result<SshSessionStatus, SshManagerError> {
        validate_request(&request)?;
        if matches!(request.authentication, SshAuthentication::OpenSsh) {
            return self.connect_openssh(request);
        }
        let SshConnectRequest {
            connection_id,
            name,
            host,
            port,
            username,
            authentication,
            trusted_host_key,
            columns,
            rows,
        } = request;
        let trusted_key = PublicKey::from_openssh(&trusted_host_key)
            .map_err(|error| SshManagerError::InvalidHostKey(error.to_string()))?;
        let handler = StrictHostKeyClient {
            trusted_key: Arc::new(trusted_key),
        };
        let mut handle = self.runtime.block_on(async {
            tokio::time::timeout(
                CONNECT_TIMEOUT,
                client::connect(Arc::new(client_config()), (host.as_str(), port), handler),
            )
            .await
            .map_err(|_| SshManagerError::Timeout)?
            .map_err(SshManagerError::from)
        })?;

        let authenticated = self
            .runtime
            .block_on(async { authenticate(&mut handle, &username, authentication).await })?;
        if !authenticated {
            let _ = self.runtime.block_on(handle.disconnect(
                Disconnect::ByApplication,
                "authentication failed",
                "en",
            ));
            return Err(SshManagerError::AuthenticationFailed);
        }

        let channel = self.runtime.block_on(async {
            let channel = handle.channel_open_session().await?;
            channel
                .request_pty(
                    false,
                    "xterm-256color",
                    u32::from(columns),
                    u32::from(rows),
                    0,
                    0,
                    &[],
                )
                .await?;
            channel.request_shell(false).await?;
            Ok::<_, russh::Error>(channel)
        })?;
        let (mut reader, writer) = channel.split();
        let output = Arc::new(Mutex::new(TerminalBuffer::default()));
        let reader_output = Arc::clone(&output);
        let closed = Arc::new(AtomicBool::new(false));
        let reader_closed = Arc::clone(&closed);
        self.runtime.spawn(async move {
            while let Some(message) = reader.wait().await {
                match message {
                    ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                        append_terminal_output(&reader_output, &data);
                    }
                    ChannelMsg::Eof | ChannelMsg::Close => break,
                    _ => {}
                }
            }
            reader_closed.store(true, Ordering::Release);
        });

        let id = Uuid::new_v4().to_string();
        let status = SshSessionStatus {
            id: id.clone(),
            connection_id,
            name,
            host,
            port,
            username,
            status: "connected".into(),
            connected_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            buffered_bytes: 0,
            keepalive_interval_seconds: KEEPALIVE_INTERVAL.as_secs(),
            keepalive_max_failures: KEEPALIVE_MAX_FAILURES,
        };
        self.sessions.insert(
            id,
            ManagedSession {
                status: status.clone(),
                backend: SessionBackend::Russh { handle, writer },
                output,
                closed,
            },
        );
        Ok(status)
    }

    fn connect_openssh(
        &mut self,
        request: SshConnectRequest,
    ) -> Result<SshSessionStatus, SshManagerError> {
        let pair = native_pty_system().openpty(PtySize {
            rows: request.rows,
            cols: request.columns,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut command = CommandBuilder::new(openssh_executable());
        let port = request.port.to_string();
        command.args([
            "-tt",
            "-o",
            "ServerAliveInterval=20",
            "-o",
            "ServerAliveCountMax=3",
            "-p",
            port.as_str(),
        ]);
        command.args(["-l", request.username.trim()]);
        command.arg(request.host.as_str());
        command.env("TERM", "xterm-256color");

        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let output = Arc::new(Mutex::new(TerminalBuffer::default()));
        let reader_output = Arc::clone(&output);
        let closed = Arc::new(AtomicBool::new(false));
        let reader_closed = Arc::clone(&closed);
        thread::Builder::new()
            .name("gatetree-ops-openssh-reader".into())
            .spawn(move || {
                let mut buffer = [0_u8; 16 * 1024];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => append_terminal_output(&reader_output, &buffer[..count]),
                    }
                }
                reader_closed.store(true, Ordering::Release);
            })?;

        let id = Uuid::new_v4().to_string();
        let status = SshSessionStatus {
            id: id.clone(),
            connection_id: request.connection_id,
            name: request.name,
            host: request.host,
            port: request.port,
            username: request.username,
            status: "connected".into(),
            connected_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            buffered_bytes: 0,
            keepalive_interval_seconds: KEEPALIVE_INTERVAL.as_secs(),
            keepalive_max_failures: KEEPALIVE_MAX_FAILURES,
        };
        self.sessions.insert(
            id,
            ManagedSession {
                status: status.clone(),
                backend: SessionBackend::OpenSsh {
                    child,
                    master: pair.master,
                    writer: Mutex::new(writer),
                },
                output,
                closed,
            },
        );
        Ok(status)
    }

    pub(crate) fn list(&mut self) -> Vec<SshSessionStatus> {
        self.sessions.retain(|_, session| match &session.backend {
            SessionBackend::Russh { handle, .. } => !handle.is_closed(),
            SessionBackend::OpenSsh { .. } => !session.closed.load(Ordering::Acquire),
        });
        let mut sessions = self
            .sessions
            .values()
            .map(|session| {
                let mut status = session.status.clone();
                status.buffered_bytes = session
                    .output
                    .lock()
                    .map(|output| output.bytes.len())
                    .unwrap_or_default();
                status
            })
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| right.connected_at.cmp(&left.connected_at));
        sessions
    }

    pub(crate) fn disconnect(&mut self, id: &str) -> Result<(), SshManagerError> {
        let session = self
            .sessions
            .remove(id)
            .ok_or(SshManagerError::SessionNotFound)?;
        match session.backend {
            SessionBackend::Russh { handle, .. } => self
                .runtime
                .block_on(handle.disconnect(
                    Disconnect::ByApplication,
                    "GateTree-Ops session closed",
                    "en",
                ))
                .map_err(SshManagerError::from),
            SessionBackend::OpenSsh { mut child, .. } => {
                child.kill()?;
                let _ = child.wait();
                Ok(())
            }
        }
    }

    pub(crate) fn write_terminal(&self, id: &str, data: Vec<u8>) -> Result<(), SshManagerError> {
        let session = self
            .sessions
            .get(id)
            .ok_or(SshManagerError::SessionNotFound)?;
        if session.closed.load(Ordering::Acquire) {
            return Err(SshManagerError::TerminalClosed);
        }
        match &session.backend {
            SessionBackend::Russh { writer, .. } => self
                .runtime
                .block_on(writer.data_bytes(data))
                .map_err(SshManagerError::from),
            SessionBackend::OpenSsh { writer, .. } => {
                let mut writer = writer.lock().map_err(|_| SshManagerError::StatePoisoned)?;
                writer.write_all(&data)?;
                writer.flush()?;
                Ok(())
            }
        }
    }

    pub(crate) fn resize_terminal(
        &self,
        id: &str,
        columns: u16,
        rows: u16,
    ) -> Result<(), SshManagerError> {
        validate_terminal_size(columns, rows)?;
        let session = self
            .sessions
            .get(id)
            .ok_or(SshManagerError::SessionNotFound)?;
        match &session.backend {
            SessionBackend::Russh { writer, .. } => self
                .runtime
                .block_on(writer.window_change(u32::from(columns), u32::from(rows), 0, 0))
                .map_err(SshManagerError::from),
            SessionBackend::OpenSsh { master, .. } => master
                .resize(PtySize {
                    rows,
                    cols: columns,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(SshManagerError::from),
        }
    }

    pub(crate) fn read_terminal(
        &self,
        id: &str,
        offset: Option<u64>,
        maximum_bytes: usize,
    ) -> Result<SshTerminalChunk, SshManagerError> {
        let session = self
            .sessions
            .get(id)
            .ok_or(SshManagerError::SessionNotFound)?;
        let output = session
            .output
            .lock()
            .map_err(|_| SshManagerError::StatePoisoned)?;
        let requested_offset = offset.unwrap_or(output.start_offset);
        let truncated = requested_offset < output.start_offset;
        let read_offset = requested_offset.clamp(output.start_offset, output.end_offset);
        let start = usize::try_from(read_offset - output.start_offset).unwrap_or(usize::MAX);
        let count = maximum_bytes
            .clamp(1, 64 * 1024)
            .min(output.bytes.len().saturating_sub(start));
        let data = output
            .bytes
            .iter()
            .skip(start)
            .take(count)
            .copied()
            .collect();
        Ok(SshTerminalChunk {
            data,
            closed: session.closed.load(Ordering::Acquire),
            next_offset: read_offset + count as u64,
            truncated,
        })
    }

    pub(crate) fn disconnect_all(&mut self) {
        for (_, session) in self.sessions.drain() {
            match session.backend {
                SessionBackend::Russh { handle, .. } => {
                    let _ = self.runtime.block_on(handle.disconnect(
                        Disconnect::ByApplication,
                        "GateTree-Ops Agent stopped",
                        "en",
                    ));
                }
                SessionBackend::OpenSsh { mut child, .. } => {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn openssh_executable() -> &'static str {
    "/usr/bin/ssh"
}

#[cfg(not(target_os = "macos"))]
fn openssh_executable() -> &'static str {
    "ssh"
}

fn client_config() -> client::Config {
    client::Config {
        inactivity_timeout: Some(Duration::from_secs(60)),
        keepalive_interval: Some(KEEPALIVE_INTERVAL),
        keepalive_max: KEEPALIVE_MAX_FAILURES,
        ..Default::default()
    }
}

async fn authenticate(
    handle: &mut client::Handle<StrictHostKeyClient>,
    username: &str,
    authentication: SshAuthentication,
) -> Result<bool, SshManagerError> {
    let result = match authentication {
        SshAuthentication::Password { password } => {
            let password = Zeroizing::new(password);
            handle
                .authenticate_password(username, password.as_str())
                .await?
        }
        SshAuthentication::PrivateKey {
            private_key,
            passphrase,
        } => {
            let private_key = Zeroizing::new(private_key);
            let passphrase = passphrase.map(Zeroizing::new);
            let key = PrivateKey::from_openssh(private_key.as_bytes())
                .map_err(|error| SshManagerError::InvalidPrivateKey(error.to_string()))?;
            let key = if key.is_encrypted() {
                let passphrase = passphrase
                    .as_deref()
                    .ok_or(SshManagerError::PrivateKeyPassphraseRequired)?;
                key.decrypt(passphrase.as_bytes())
                    .map_err(|error| SshManagerError::InvalidPrivateKey(error.to_string()))?
            } else {
                key
            };
            let hash = handle.best_supported_rsa_hash().await?.flatten();
            handle
                .authenticate_publickey(username, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await?
        }
        SshAuthentication::KeyboardInteractive { response } => {
            let response = Zeroizing::new(response);
            let mut result = handle
                .authenticate_keyboard_interactive_start(username, None::<String>)
                .await?;
            for _ in 0..4 {
                result = match result {
                    KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                        handle
                            .authenticate_keyboard_interactive_respond(
                                prompts.into_iter().map(|_| response.to_string()).collect(),
                            )
                            .await?
                    }
                    KeyboardInteractiveAuthResponse::Success => return Ok(true),
                    KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
                };
            }
            return Err(SshManagerError::TooManyAuthenticationPrompts);
        }
        SshAuthentication::SshAgent => {
            return authenticate_with_ssh_agent(handle, username).await;
        }
        SshAuthentication::OpenSsh => return Err(SshManagerError::InvalidRequest),
    };
    Ok(matches!(result, AuthResult::Success))
}

async fn authenticate_with_ssh_agent(
    handle: &mut client::Handle<StrictHostKeyClient>,
    username: &str,
) -> Result<bool, SshManagerError> {
    let mut agent = connect_platform_ssh_agent().await?;
    let identities = agent
        .request_identities()
        .await
        .map_err(|error| SshManagerError::SshAgentUnavailable(error.to_string()))?;
    let hash = handle.best_supported_rsa_hash().await?.flatten();
    for identity in identities {
        let result = match identity {
            AgentIdentity::PublicKey { key, .. } => handle
                .authenticate_publickey_with(username, key, hash, &mut agent)
                .await
                .map_err(|error| SshManagerError::SshAgentUnavailable(error.to_string()))?,
            AgentIdentity::Certificate { certificate, .. } => handle
                .authenticate_certificate_with(username, certificate, hash, &mut agent)
                .await
                .map_err(|error| SshManagerError::SshAgentUnavailable(error.to_string()))?,
        };
        if matches!(result, AuthResult::Success) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(unix)]
async fn connect_platform_ssh_agent()
-> Result<AgentClient<Box<dyn AgentStream + Send + Unpin>>, SshManagerError> {
    AgentClient::connect_env()
        .await
        .map(AgentClient::dynamic)
        .map_err(|error| SshManagerError::SshAgentUnavailable(error.to_string()))
}

#[cfg(target_os = "windows")]
async fn connect_platform_ssh_agent()
-> Result<AgentClient<Box<dyn AgentStream + Send + Unpin>>, SshManagerError> {
    if let Ok(agent) = AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
        return Ok(agent.dynamic());
    }
    AgentClient::connect_pageant()
        .await
        .map(AgentClient::dynamic)
        .map_err(|error| SshManagerError::SshAgentUnavailable(error.to_string()))
}

fn validate_request(request: &SshConnectRequest) -> Result<(), SshManagerError> {
    validate_host(&request.host, request.port)?;
    validate_terminal_size(request.columns, request.rows)?;
    if request.connection_id.trim().is_empty()
        || request.name.trim().is_empty()
        || request.username.trim().is_empty()
    {
        return Err(SshManagerError::InvalidRequest);
    }
    if !matches!(request.authentication, SshAuthentication::OpenSsh)
        && request.trusted_host_key.trim().is_empty()
    {
        return Err(SshManagerError::HostKeyRequired);
    }
    Ok(())
}

fn validate_terminal_size(columns: u16, rows: u16) -> Result<(), SshManagerError> {
    if columns < 2 || rows == 0 {
        Err(SshManagerError::InvalidTerminalSize)
    } else {
        Ok(())
    }
}

fn append_terminal_output(output: &Arc<Mutex<TerminalBuffer>>, data: &[u8]) {
    let Ok(mut output) = output.lock() else {
        return;
    };
    output.end_offset = output.end_offset.saturating_add(data.len() as u64);
    let overflow = output
        .bytes
        .len()
        .saturating_add(data.len())
        .saturating_sub(MAX_TERMINAL_BUFFER);
    if overflow > 0 {
        let drop_count = overflow.min(output.bytes.len());
        output.bytes.drain(..drop_count);
        output.start_offset = output.start_offset.saturating_add(drop_count as u64);
    }
    if data.len() > MAX_TERMINAL_BUFFER {
        let retained = &data[data.len() - MAX_TERMINAL_BUFFER..];
        output.bytes.clear();
        output.bytes.extend(retained);
        output.start_offset = output.end_offset - retained.len() as u64;
    } else {
        output.bytes.extend(data);
    }
}

fn validate_host(host: &str, port: u16) -> Result<(), SshManagerError> {
    if host.trim().is_empty() || host.contains(['\r', '\n']) || port == 0 {
        Err(SshManagerError::InvalidTarget)
    } else {
        Ok(())
    }
}

fn host_key_details(key: &PublicKey) -> Result<SshHostKey, SshManagerError> {
    Ok(SshHostKey {
        public_key: key
            .to_openssh()
            .map_err(|error| SshManagerError::InvalidHostKey(error.to_string()))?,
        fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
    })
}

#[derive(Clone)]
struct StrictHostKeyClient {
    trusted_key: Arc<PublicKey>,
}

impl client::Handler for StrictHostKeyClient {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKey) -> Result<bool, Self::Error> {
        Ok(key == self.trusted_key.as_ref())
    }
}

#[derive(Clone)]
struct HostKeyProbe {
    captured: Arc<Mutex<Option<PublicKey>>>,
}

impl client::Handler for HostKeyProbe {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKey) -> Result<bool, Self::Error> {
        if let Ok(mut captured) = self.captured.lock() {
            *captured = Some(key.clone());
        }
        Ok(false)
    }
}

#[derive(Debug, Error)]
pub(crate) enum SshManagerError {
    #[error("SSH target is invalid")]
    InvalidTarget,
    #[error("SSH connection request is incomplete")]
    InvalidRequest,
    #[error("A trusted SSH host key is required")]
    HostKeyRequired,
    #[error("The SSH server did not provide a host key")]
    HostKeyUnavailable,
    #[error("SSH host key is invalid: {0}")]
    InvalidHostKey(String),
    #[error("SSH private key is invalid or its passphrase is incorrect: {0}")]
    InvalidPrivateKey(String),
    #[error("The SSH private key requires a passphrase")]
    PrivateKeyPassphraseRequired,
    #[error("SSH authentication failed")]
    AuthenticationFailed,
    #[error("SSH agent authentication is unavailable: {0}")]
    SshAgentUnavailable(String),
    #[error("SSH keyboard-interactive authentication sent too many prompts")]
    TooManyAuthenticationPrompts,
    #[error("SSH connection timed out")]
    Timeout,
    #[error("SSH session was not found")]
    SessionNotFound,
    #[error("SSH terminal size is invalid")]
    InvalidTerminalSize,
    #[error("SSH terminal is closed")]
    TerminalClosed,
    #[error("SSH session state is unavailable")]
    StatePoisoned,
    #[error("Could not start the SSH runtime: {0}")]
    Runtime(#[from] std::io::Error),
    #[error("Could not start the local OpenSSH session: {0}")]
    OpenSsh(#[from] anyhow::Error),
    #[error("SSH error: {0}")]
    Russh(#[from] russh::Error),
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
    };

    use russh::{
        Channel, ChannelId, Pty,
        keys::{Algorithm, PrivateKey},
        server::{self, Msg, Session},
    };
    use tokio::net::TcpListener;

    use super::*;

    const USERNAME: &str = "gatetree-ops-test";
    const PASSWORD: &str = "test-only-password";

    #[derive(Clone)]
    struct TestHandler;

    impl server::Handler for TestHandler {
        type Error = russh::Error;

        async fn auth_password(
            &mut self,
            user: &str,
            password: &str,
        ) -> Result<server::Auth, Self::Error> {
            Ok(if user == USERNAME && password == PASSWORD {
                server::Auth::Accept
            } else {
                server::Auth::reject()
            })
        }

        async fn channel_open_session(
            &mut self,
            _channel: Channel<Msg>,
            reply: server::ChannelOpenHandle,
            _session: &mut Session,
        ) -> Result<(), Self::Error> {
            reply.accept().await;
            Ok(())
        }

        async fn pty_request(
            &mut self,
            channel: ChannelId,
            _term: &str,
            _col_width: u32,
            _row_height: u32,
            _pix_width: u32,
            _pix_height: u32,
            _modes: &[(Pty, u32)],
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            session.channel_success(channel)?;
            Ok(())
        }

        async fn shell_request(
            &mut self,
            channel: ChannelId,
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            session.channel_success(channel)?;
            Ok(())
        }

        async fn data(
            &mut self,
            channel: ChannelId,
            data: &[u8],
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            session.data(channel, data.to_vec())?;
            Ok(())
        }
    }

    struct TestServer {
        address: std::net::SocketAddr,
        public_key: String,
        stop: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl TestServer {
        fn start() -> Self {
            let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
            let public_key = host_key.public_key().to_openssh().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let server_stop = Arc::clone(&stop);
            let (address_tx, address_rx) = mpsc::sync_channel(1);
            let thread = thread::spawn(move || {
                let runtime = Builder::new_current_thread().enable_all().build().unwrap();
                runtime.block_on(async move {
                    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                    address_tx.send(listener.local_addr().unwrap()).unwrap();
                    let config = Arc::new(server::Config {
                        auth_rejection_time: Duration::ZERO,
                        auth_rejection_time_initial: Some(Duration::ZERO),
                        keys: vec![host_key],
                        ..Default::default()
                    });
                    while !server_stop.load(Ordering::Acquire) {
                        if let Ok(Ok((socket, _))) =
                            tokio::time::timeout(Duration::from_millis(50), listener.accept()).await
                        {
                            let config = Arc::clone(&config);
                            tokio::spawn(async move {
                                if let Ok(session) =
                                    server::run_stream(config, socket, TestHandler).await
                                {
                                    let _ = session.await;
                                }
                            });
                        }
                    }
                });
            });
            Self {
                address: address_rx.recv().unwrap(),
                public_key,
                stop,
                thread: Some(thread),
            }
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                thread.join().unwrap();
            }
        }
    }

    #[test]
    fn openssh_requires_an_explicit_username_instead_of_using_the_local_account() {
        let mut request = SshConnectRequest {
            connection_id: "connection".into(),
            name: "Host".into(),
            host: "192.0.2.1".into(),
            port: 22,
            username: String::new(),
            authentication: SshAuthentication::OpenSsh,
            trusted_host_key: String::new(),
            columns: 80,
            rows: 24,
        };
        assert!(validate_request(&request).is_err());
        request.username = "   ".into();
        assert!(validate_request(&request).is_err());
        request.username = "remote-user".into();
        assert!(validate_request(&request).is_ok());
    }

    #[test]
    fn scans_connects_lists_and_disconnects_a_real_russh_session() {
        let server = TestServer::start();
        let mut manager = SshSessionManager::new().unwrap();
        let scanned = manager
            .scan_host_key(server.address.ip().to_string(), server.address.port())
            .unwrap();
        assert_eq!(scanned.public_key, server.public_key);
        assert!(scanned.fingerprint.starts_with("SHA256:"));

        let session = manager
            .connect(SshConnectRequest {
                connection_id: "connection-1".into(),
                name: "Test host".into(),
                host: server.address.ip().to_string(),
                port: server.address.port(),
                username: USERNAME.into(),
                authentication: SshAuthentication::Password {
                    password: PASSWORD.into(),
                },
                trusted_host_key: scanned.public_key,
                columns: 80,
                rows: 24,
            })
            .unwrap();
        assert_eq!(manager.list(), vec![session.clone()]);
        manager
            .write_terminal(&session.id, b"echo test\r".to_vec())
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        let output = loop {
            let chunk = manager.read_terminal(&session.id, None, 1024).unwrap();
            if !chunk.data.is_empty() {
                break chunk;
            }
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(output.data, b"echo test\r");
        assert!(output.next_offset > 0);
        let no_duplicate = manager
            .read_terminal(&session.id, Some(output.next_offset), 1024)
            .unwrap();
        assert!(no_duplicate.data.is_empty());
        let replay = manager.read_terminal(&session.id, None, 1024).unwrap();
        assert_eq!(replay.data, b"echo test\r");
        manager.resize_terminal(&session.id, 120, 40).unwrap();
        manager.disconnect(&session.id).unwrap();
        assert!(manager.list().is_empty());
    }
}
