use crate::modules::workspace::WorkspaceState;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::State;

const BUFFER_LIMIT: usize = 4 * 1024 * 1024;
#[derive(Default)]
struct Output {
    bytes: VecDeque<u8>,
    start: u64,
    end: u64,
}
impl Output {
    fn append(&mut self, bytes: &[u8]) {
        self.bytes.extend(bytes);
        self.end += bytes.len() as u64;
        let excess = self.bytes.len().saturating_sub(BUFFER_LIMIT);
        self.bytes.drain(..excess);
        self.start += excess as u64;
    }
    fn chunk(
        &self,
        offset: Option<u64>,
        closed: bool,
    ) -> gatetree_ops_agent_core::SshTerminalChunk {
        let requested = offset.unwrap_or(self.start);
        let start = requested.clamp(self.start, self.end);
        let data: Vec<u8> = self
            .bytes
            .iter()
            .skip((start - self.start) as usize)
            .take(65536)
            .copied()
            .collect();
        let next_offset = start + data.len() as u64;
        gatetree_ops_agent_core::SshTerminalChunk {
            data,
            next_offset,
            truncated: requested < self.start,
            closed: closed && next_offset == self.end,
        }
    }
}
struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    output: Arc<Mutex<Output>>,
    closed: Arc<AtomicBool>,
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[derive(Default)]
pub struct LocalTerminals(Mutex<HashMap<String, Session>>);
impl LocalTerminals {
    pub fn clear(&self) {
        if let Ok(mut sessions) = self.0.lock() {
            sessions.clear();
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSession {
    id: String,
    shell: String,
}
fn size(columns: u16, rows: u16) -> PtySize {
    PtySize {
        cols: columns.clamp(2, 1000),
        rows: rows.clamp(2, 1000),
        pixel_width: 0,
        pixel_height: 0,
    }
}
fn unlocked(workspace: &WorkspaceState) -> Result<(), String> {
    workspace
        .with_connection(|_| Ok(()))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_local_terminal(
    workspace: State<'_, WorkspaceState>,
    terminals: State<'_, LocalTerminals>,
) -> Result<LocalSession, String> {
    unlocked(&workspace)?;
    spawn_terminal(&terminals)
}

fn spawn_terminal(terminals: &LocalTerminals) -> Result<LocalSession, String> {
    (|| -> Result<_, Box<dyn std::error::Error>> {
        #[cfg(not(windows))]
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/bin/sh".into());
        #[cfg(windows)]
        let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
        let pair = native_pty_system().openpty(size(120, 32))?;
        let mut command = CommandBuilder::new(&shell);
        #[cfg(not(windows))]
        command.arg("-l");
        command.env("TERM", "xterm-256color");
        if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
            command.cwd(home);
        }
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let session = Session {
            master: pair.master,
            writer,
            child,
            output: Arc::default(),
            closed: Arc::new(AtomicBool::new(false)),
        };
        let output = session.output.clone();
        let closed = session.closed.clone();
        std::thread::Builder::new()
            .name("local-terminal-output".into())
            .spawn(move || {
                let mut buffer = [0u8; 16384];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => {
                            if let Ok(mut out) = output.lock() {
                                out.append(&buffer[..count]);
                            } else {
                                break;
                            }
                        }
                    }
                }
                closed.store(true, Ordering::Release);
            })?;
        let id = uuid::Uuid::new_v4().to_string();
        terminals
            .0
            .lock()
            .map_err(|_| "Local terminal state unavailable")?
            .insert(id.clone(), session);
        Ok(LocalSession { id, shell })
    })()
    .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn read_local_terminal(
    workspace: State<'_, WorkspaceState>,
    terminals: State<'_, LocalTerminals>,
    session_id: String,
    offset: Option<u64>,
) -> Result<gatetree_ops_agent_core::SshTerminalChunk, String> {
    unlocked(&workspace)?;
    let mut sessions = terminals
        .0
        .lock()
        .map_err(|_| "Local terminal state unavailable")?;
    let session = sessions
        .get_mut(&session_id)
        .ok_or("Local terminal not found")?;
    let closed = session.closed.load(Ordering::Acquire);
    if closed {
        let _ = session.child.try_wait();
    }
    let output = session
        .output
        .lock()
        .map_err(|_| "Local terminal output unavailable")?;
    Ok(output.chunk(offset, closed))
}
#[tauri::command]
pub fn write_local_terminal(
    workspace: State<'_, WorkspaceState>,
    terminals: State<'_, LocalTerminals>,
    session_id: String,
    data: Vec<u8>,
) -> Result<(), String> {
    unlocked(&workspace)?;
    if data.len() > 65536 {
        return Err("Terminal input exceeds 64 KB".into());
    }
    let mut sessions = terminals
        .0
        .lock()
        .map_err(|_| "Local terminal state unavailable")?;
    let session = sessions
        .get_mut(&session_id)
        .ok_or("Local terminal not found")?;
    session
        .writer
        .write_all(&data)
        .and_then(|_| session.writer.flush())
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn resize_local_terminal(
    workspace: State<'_, WorkspaceState>,
    terminals: State<'_, LocalTerminals>,
    session_id: String,
    columns: u16,
    rows: u16,
) -> Result<(), String> {
    unlocked(&workspace)?;
    let sessions = terminals
        .0
        .lock()
        .map_err(|_| "Local terminal state unavailable")?;
    sessions
        .get(&session_id)
        .ok_or("Local terminal not found")?
        .master
        .resize(size(columns, rows))
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn close_local_terminal(
    terminals: State<'_, LocalTerminals>,
    session_id: String,
) -> Result<(), String> {
    terminals
        .0
        .lock()
        .map_err(|_| "Local terminal state unavailable")?
        .remove(&session_id);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn local_shell_accepts_input_resizes_and_closes() {
        let terminals = LocalTerminals::default();
        let opened = spawn_terminal(&terminals).unwrap();
        let closed = {
            let mut sessions = terminals.0.lock().unwrap();
            let session = sessions.get_mut(&opened.id).unwrap();
            session.master.resize(size(99, 27)).unwrap();
            session.closed.clone()
        };
        // Send a command whose expected output cannot be satisfied by terminal echo.
        {
            let mut sessions = terminals.0.lock().unwrap();
            let session = sessions.get_mut(&opened.id).unwrap();
            session
                .writer
                .write_all(b"printf '%s%s' GATETREE_ LOCAL_OK; stty size; exit\n")
                .unwrap();
            session.writer.flush().unwrap();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut text = String::new();
        while std::time::Instant::now() < deadline {
            let sessions = terminals.0.lock().unwrap();
            let session = sessions.get(&opened.id).unwrap();
            let output = session.output.lock().unwrap();
            text = String::from_utf8_lossy(&output.bytes.iter().copied().collect::<Vec<_>>())
                .into_owned();
            if text.contains("GATETREE_LOCAL_OK") && text.contains("27 99") {
                break;
            }
            drop(output);
            drop(sessions);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        terminals.clear();
        assert!(
            text.contains("GATETREE_LOCAL_OK"),
            "Shell command did not produce expected output"
        );
        assert!(text.contains("27 99"), "PTY resize did not reach shell");
        assert!(terminals.0.lock().unwrap().is_empty());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !closed.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(closed.load(Ordering::Acquire));
    }

    #[test]
    fn output_survives_lag_and_drains_before_closed() {
        let mut out = Output::default();
        out.append(&vec![b'a'; BUFFER_LIMIT + 10]);
        let first = out.chunk(Some(0), true);
        assert!(first.truncated);
        assert!(!first.closed);
        assert_eq!(first.next_offset, 65546);
        let last = out.chunk(Some(out.end - 3), true);
        assert_eq!(last.data, b"aaa");
        assert!(last.closed);
        assert_eq!(out.bytes.len(), BUFFER_LIMIT);
    }
}
