const { invoke } = window.__TAURI__.core;

export const startAgent = () => invoke("start_agent");
export const stopAgent = () => invoke("stop_agent");
export const killAgent = () => invoke("kill_agent");
export const getAgentStatus = () => invoke("agent_status");
export const scanAgentSshHostKey = (host, port) => invoke("scan_agent_ssh_host_key", { host, port });
export const inspectAgentSshHostKey = (host, port) => invoke("inspect_agent_ssh_host_key", { host, port });
export const trustAgentSshHostKey = (host, port, expectedPublicKey) => invoke("trust_agent_ssh_host_key", { host, port, expectedPublicKey });
export const forgetAgentSshHostKey = (host, port) => invoke("forget_agent_ssh_host_key", { host, port });
export const listAgentSshSessions = () => invoke("list_agent_ssh_sessions");
export const listAgentJobSets = () => invoke("list_agent_job_sets");
export const connectAgentSshConnection = (connectionId, authentication, columns = 120, rows = 32) => invoke("connect_agent_ssh_connection", {
  connectionId,
  username: authentication.username ?? null,
  credential: authentication.credential ?? null,
  privateKey: authentication.privateKey ?? null,
  passphrase: authentication.passphrase ?? null,
  columns,
  rows,
});
export const disconnectAgentSshSession = (sessionId) => invoke("disconnect_agent_ssh_session", { sessionId });
export const readAgentSshTerminal = (sessionId, offset = null, maximumBytes = 65536) => invoke("read_agent_ssh_terminal", { sessionId, offset, maximumBytes });
export const writeAgentSshTerminal = (sessionId, data) => invoke("write_agent_ssh_terminal", { sessionId, data: Array.from(data) });
export const resizeAgentSshTerminal = (sessionId, columns, rows) => invoke("resize_agent_ssh_terminal", { sessionId, columns, rows });
export const listAgentActivity = () => invoke("list_agent_activity");
export const openUrlConnection = (connectionId) => invoke("open_url_connection", { connectionId });
export const focusUrlActivity = (activityId) => invoke("focus_url_activity", { activityId });
export const deleteAgentActivity = (activityId) => invoke("delete_agent_activity", { activityId });

export const restoreDeploymentCredentials = (keepassPassword) => invoke("restore_deployment_credentials", { keepassPassword });
