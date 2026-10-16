import { escapeHtml } from "../../shared/ui.js";
import {
  deleteAgentActivity,
  disconnectAgentSshSession,
  focusUrlActivity,
  getAgentStatus,
  listAgentActivity,
  listAgentJobSets,
  listAgentSshSessions,
  startAgent,
  stopAgent,
  killAgent,
} from "./agent-api.js";

export async function mountAgentView(root, options = {}) {
  const state = {
    status: null, activities: [], sshSessions: [], jobSets: [], busy: false, error: "",
    mountId: crypto.randomUUID(), onAttachSshSession: options.onAttachSshSession,
    onFocusDeploymentPlan: options.onFocusDeploymentPlan,
  };
  root.dataset.agentMountId = state.mountId;
  root.innerHTML = '<div class="agent-loading">Loading Agent status…</div>';
  await refresh(root, state);
  scheduleStatusPoll(root, state);
}

function render(root, state) {
  const running = Boolean(state.status?.running);
  const statusLabel = running && state.status?.pid
    ? `${state.status.label} - PID: ${state.status.pid}`
    : (state.status?.label ?? "Unknown");
  root.innerHTML = `
    <div class="agent-module">
      <section class="agent-controls">
        <div class="agent-summary">
          <span class="agent-state${running ? " is-running" : ""}"><i></i>${escapeHtml(statusLabel)}</span>
          <span class="agent-version">Agent: ${escapeHtml(state.status?.version ?? "—")} · Protocol: ${escapeHtml(state.status?.protocolVersion ?? "—")} · GUI expects: ${escapeHtml(state.status?.expectedVersion ?? "—")} / protocol ${escapeHtml(state.status?.expectedProtocolVersion ?? "—")}${versionWarning(state.status)}</span>
        </div>
        <div>
          <button data-action="start-agent" ${running || state.busy ? "disabled" : ""}>Start</button>
          <button data-action="stop-agent" ${!running || state.busy ? "disabled" : ""}>Stop</button>
          <button data-action="kill-agent" ${state.busy ? "disabled" : ""}>Kill Agent</button>
        </div>
      </section>
      ${state.error ? `<p class="agent-error">${escapeHtml(state.error)}</p>` : ""}
      <section class="agent-activity-table">
        <div class="agent-table-header"><span>Name</span><span>Type</span><span>Target</span><span>Opened</span><span>Status</span><span></span></div>
        ${jobSetRows(state)}
        ${sessionRows(state)}
        ${activityRows(state)}
        ${state.jobSets.length || state.sshSessions.length || state.activities.length ? "" : '<p class="agent-empty">No active sessions or remembered activity.</p>'}
      </section>
    </div>`;
  bindEvents(root, state);
}

function jobSetRows(state) {
  return state.jobSets.map((jobSet) => `
    <div class="agent-table-row agent-plan-row">
      <strong>${escapeHtml(jobSet.name)}</strong>
      <span>Deploy</span>
      <code>#${escapeHtml(jobSet.planNumber)}</code>
      <span></span>
      <span class="agent-activity-status">${escapeHtml(jobSet.status)}</span>
      <span class="agent-row-actions"><button data-action="focus-plan" data-plan-id="${escapeHtml(jobSet.id)}" title="Open Deployment Plan">Focus</button></span>
    </div>`).join("");
}

function sessionRows(state) {
  return state.sshSessions.map((session) => {
    const target = `${session.username}@${session.host}:${session.port}`;
    return `
      <div class="agent-table-row">
        <strong>${escapeHtml(session.name)}</strong>
        <span>SSH</span>
        <code title="${escapeHtml(target)}">${escapeHtml(target)}</code>
        <time>${formatUnixTimestamp(session.connectedAt)}</time>
        <span class="agent-activity-status" title="Keepalive every ${session.keepaliveIntervalSeconds}s; disconnect after ${session.keepaliveMaxFailures} failed replies. ${formatBytes(session.bufferedBytes)} terminal output retained.">${escapeHtml(session.status)} · ${formatBytes(session.bufferedBytes)}</span>
        <span class="agent-row-actions">
          <button data-action="attach-session" data-session-id="${session.id}" title="Open SSH terminal">Open</button>
          <button class="agent-delete-action" data-action="disconnect-session" data-session-id="${session.id}" title="Disconnect SSH session" aria-label="Disconnect SSH session">${disconnectIcon()}</button>
        </span>
      </div>`;
  }).join("");
}

function activityRows(state) {
  return state.activities.map((activity) => `
    <div class="agent-table-row">
      <strong>${escapeHtml(activity.name)}</strong>
      <span>${escapeHtml(activity.resourceType.toUpperCase())}</span>
      <code title="${escapeHtml(activity.target)}">${escapeHtml(activity.target)}</code>
      <time>${formatTimestamp(activity.openedAt)}</time>
      <span class="agent-activity-status">${escapeHtml(activity.status)}</span>
      <span class="agent-row-actions">
        ${state.status?.canFocusBrowser && activity.resourceType === "url" ? `<button data-action="focus-activity" data-activity-id="${activity.id}" title="Focus Chrome">Focus</button>` : ""}
        <button class="agent-delete-action" data-action="delete-activity" data-activity-id="${activity.id}" title="Remove" aria-label="Remove">${trashIcon()}</button>
      </span>
    </div>`).join("");
}

function bindEvents(root, state) {
  root.querySelector('[data-action="kill-agent"]').addEventListener("click", () => run(root, state, async () => {
    state.status = await killAgent();
    state.sshSessions = [];
    state.jobSets = [];
  }));
  root.querySelector('[data-action="start-agent"]').addEventListener("click", () => run(root, state, async () => {
    state.status = await startAgent();
    state.sshSessions = await listSessionsIfRunning(state.status);
    state.jobSets = await listJobSetsIfRunning(state.status);
  }));
  root.querySelector('[data-action="stop-agent"]').addEventListener("click", () => run(root, state, async () => {
    state.status = await stopAgent();
    state.sshSessions = [];
    state.jobSets = [];
  }));
  root.querySelectorAll('[data-action="focus-plan"]').forEach((button) => {
    button.addEventListener("click", () => state.onFocusDeploymentPlan?.(button.dataset.planId));
  });
  root.querySelectorAll('[data-action="attach-session"]').forEach((button) => {
    button.addEventListener("click", () => {
      const session = state.sshSessions.find((item) => item.id === button.dataset.sessionId);
      if (session) state.onAttachSshSession?.(session);
    });
  });
  root.querySelectorAll('[data-action="disconnect-session"]').forEach((button) => {
    button.addEventListener("click", () => run(root, state, async () => {
      await disconnectAgentSshSession(button.dataset.sessionId);
      state.sshSessions = state.sshSessions.filter((session) => session.id !== button.dataset.sessionId);
    }));
  });
  root.querySelectorAll('[data-action="focus-activity"]').forEach((button) => {
    button.addEventListener("click", () => run(root, state, () => focusUrlActivity(button.dataset.activityId)));
  });
  root.querySelectorAll('[data-action="delete-activity"]').forEach((button) => {
    button.addEventListener("click", () => run(root, state, async () => {
      await deleteAgentActivity(button.dataset.activityId);
      state.activities = state.activities.filter((activity) => activity.id !== button.dataset.activityId);
    }));
  });
}

async function refresh(root, state) {
  await run(root, state, async () => {
    [state.status, state.activities] = await Promise.all([getAgentStatus(), listAgentActivity()]);
    [state.sshSessions, state.jobSets] = await Promise.all([
      listSessionsIfRunning(state.status), listJobSetsIfRunning(state.status),
    ]);
  });
}

function scheduleStatusPoll(root, state) {
  window.setTimeout(async () => {
    if (root.dataset.agentMountId !== state.mountId || !root.querySelector(".agent-module")) return;
    if (!state.busy) {
      try {
        const status = await getAgentStatus();
        const [sshSessions, jobSets] = await Promise.all([
          listSessionsIfRunning(status), listJobSetsIfRunning(status),
        ]);
        if (!sameStatus(status, state.status)
          || JSON.stringify(sshSessions) !== JSON.stringify(state.sshSessions)
          || JSON.stringify(jobSets) !== JSON.stringify(state.jobSets)) {
          state.status = status;
          state.sshSessions = sshSessions;
          state.jobSets = jobSets;
          state.error = "";
          render(root, state);
        }
      } catch (cause) {
        state.error = String(cause);
        render(root, state);
      }
    }
    scheduleStatusPoll(root, state);
  }, 1500);
}

function listSessionsIfRunning(status) {
  return compatible(status) ? listAgentSshSessions() : Promise.resolve([]);
}

function listJobSetsIfRunning(status) {
  return compatible(status) ? listAgentJobSets() : Promise.resolve([]);
}

function compatible(status) {
  return status?.running && status.protocolVersion === status.expectedProtocolVersion;
}

function versionWarning(status) {
  if (!status?.running) return "";
  const actual = String(status.version ?? "").split(".").map(Number);
  const expected = String(status.expectedVersion ?? "").split(".").map(Number);
  let older = status.protocolVersion < status.expectedProtocolVersion;
  for (let i = 0; i < Math.max(actual.length, expected.length); i++) {
    if ((actual[i] ?? 0) === (expected[i] ?? 0)) continue;
    older ||= (actual[i] ?? 0) < (expected[i] ?? 0);
    break;
  }
  if (older) return ' · <strong class="agent-error">Older Agent running — Stop or Kill Agent, then Start to update.</strong>';
  if (status.protocolVersion !== status.expectedProtocolVersion || status.version !== status.expectedVersion) {
    return ' · <strong class="agent-error">Agent version differs from GUI — restart the Agent.</strong>';
  }
  return "";
}

function sameStatus(left, right) {
  return Boolean(left?.running) === Boolean(right?.running)
    && left?.pid === right?.pid
    && left?.version === right?.version
    && left?.protocolVersion === right?.protocolVersion;
}

async function run(root, state, action) {
  state.busy = true;
  state.error = "";
  try {
    await action();
  } catch (cause) {
    state.error = String(cause);
  } finally {
    state.busy = false;
    render(root, state);
  }
}

function formatTimestamp(value) {
  const date = new Date(`${value.replace(" ", "T")}Z`);
  return formatDate(date, value);
}

function formatUnixTimestamp(value) {
  return formatDate(new Date(Number(value) * 1000), String(value));
}

function formatDate(date, fallback) {
  return Number.isNaN(date.getTime()) ? fallback : new Intl.DateTimeFormat(undefined, {
    year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  }).format(date);
}

function formatBytes(value) {
  const bytes = Number(value) || 0;
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function trashIcon() {
  return '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 7h16M9 7V4h6v3m3 0-1 13H7L6 7m4 4v5m4-5v5" /></svg>';
}

function disconnectIcon() {
  return '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3.5" y="4.5" width="17" height="15" rx="3" /><path d="m8 9 8 6m0-6-8 6" /></svg>';
}
