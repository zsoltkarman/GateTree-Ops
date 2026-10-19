const { invoke } = window.__TAURI__.core;
export const openLocalTerminal = () => invoke('open_local_terminal');
export const localTerminalTransport = {
  read: (sessionId, offset) => invoke('read_local_terminal', { sessionId, offset }),
  write: (sessionId, data) => invoke('write_local_terminal', { sessionId, data: Array.from(data) }),
  resize: (sessionId, columns, rows) => invoke('resize_local_terminal', { sessionId, columns, rows }),
  close: sessionId => invoke('close_local_terminal', { sessionId }),
};
