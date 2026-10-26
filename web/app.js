import { openLocalTerminal, localTerminalTransport } from "./modules/local-terminal/local-terminal-api.js";
import { TERMINAL_THEMES } from "./modules/local-terminal/themes.js";
import { mountWorkspaceTabs } from "./modules/workspace-tabs/workspace-tabs.js";
import { mountSshTerminal } from "./modules/connections/terminal-view.js";
import { listAgentSshSessions, disconnectAgentSshSession } from "./modules/agent/agent-api.js";
import { createUpdateController } from "./modules/updates/update-controller.js";
import { APP_NAME, migrateLegacyPreferences } from "./shared/branding.js";
import { mountHelpView } from "./modules/help/help-view.js";
import { installIconTooltips } from "./shared/icon-tooltips.js";
import { monitorAgentCredentials } from "./modules/agent/credential-recovery.js";
import { mountWorkspaceGate } from "./modules/workspace/workspace-gate.js";
import { mountAppShell } from "./modules/shell/app-shell.js";
import { mountConnectionsView } from "./modules/connections/connections-view.js";
import { focusDetachedTerminalWindow, hasDetachedTerminalWindow, openDetachedTerminalWindow, mountDetachedTerminalWindow } from "./modules/connections/detached-terminal-window.js";
import { mountNotesView } from "./modules/notes/notes-view.js";
import { mountVaultsView } from "./modules/vaults/vaults-view.js";
import { mountBackupView } from "./modules/backup/backup-view.js";
import { mountAgentView } from "./modules/agent/agent-view.js";
import { mountAiSkillsView } from "./modules/ai-skills/ai-skills-view.js";
import { mountDeploymentConfigView } from "./modules/deployments/deployments-view.js";
import { mountDeploymentQueueView } from "./modules/deployments/queue-view.js";
import { getDeploymentQueue } from "./modules/deployments/queue-api.js";
import { getDeploymentConfig } from "./modules/deployments/deployments-api.js";
import { initializeTheme } from "./modules/appearance/theme-picker.js";
import {
  biometricStatus,
  disableBiometricUnlock,
  enableBiometricUnlock,
  lockWorkspace,
} from "./modules/workspace/workspace-api.js";

const root = document.querySelector("#app");
migrateLegacyPreferences();
document.title = APP_NAME;
initializeTheme();
installIconTooltips();

let stopCredentialMonitor = () => {};
let disposeShell = () => {};
const updates = createUpdateController();

function renderShell(session) {
  disposeShell();
  stopCredentialMonitor();
  let biometric = null;
  let shell = null, tabs = null;
  let disposed = false, snapshotRevision = 0;
  const plansById = new Map();
  const excludedSsh = new Set();
  const labels = { connections: "Connections", notes: "Notes", vaults: "Vaults", backup: "Backup", agent: "Agent", "ai-skills": "AI Skills", help: "Help", deployments: "Deployments" };
  const fail = cause => { if (!disposed) shell.showMessage(String(cause), true); };

  function queueChanged(plans) {
    if (disposed) return;
    const ids = new Set(plans.map(plan => plan.id));
    for (const id of plansById.keys()) {
      if (!ids.has(id)) { plansById.delete(id); tabs.remove(`plan:${id}`); }
    }
    for (const plan of plans) {
      plansById.set(plan.id, plan);
      tabs.update(`plan:${plan.id}`, { title: `#${plan.planNumber} · ${plan.name}`, status: plan.status });
    }
    const selected = tabs.get(tabs.activeId());
    shell.setDeploymentQueue(plans, selected?.planId ?? null);
  }
  async function syncSessions({ sshSessions, jobSets }) {
    const revision = ++snapshotRevision;
    const currentIds = new Set(sshSessions.map(item => item.id));
    for (const tab of tabs.entries()) {
      if (tab.type === "ssh" && !currentIds.has(tab.sessionId)) tabs.update(tab.id, { status: "closed" });
    }
    for (const plan of jobSets ?? []) {
      tabs.update(`plan:${plan.id}`, { title: `#${plan.planNumber} · ${plan.name}`, status: plan.status });
    }
    for (const session of sshSessions) {
      if (disposed || revision !== snapshotRevision) return;
      if (excludedSsh.has(session.id)) continue;
      if (tabs.get(`ssh:${session.id}`)) {
        tabs.update(`ssh:${session.id}`, { title: session.name, status: session.status });
      } else {
        if (await hasDetachedTerminalWindow(session)) continue;
        if (disposed || revision !== snapshotRevision || excludedSsh.has(session.id)) continue;
        openSsh(session, false);
      }
    }
  }
  function openSsh(session, select = true) {
    if (disposed) return;
    const id = `ssh:${session.id}`;
    excludedSsh.delete(session.id);
    const disconnect = async () => {
      try { await disconnectAgentSshSession(session.id); }
      catch (error) {
        const sessions = await listAgentSshSessions();
        if (sessions.some(item => item.id === session.id)) throw error;
      }
      excludedSsh.add(session.id);
    };
    tabs.open({ id, type: "ssh", title: session.name, host: session.host, port: session.port, status: session.status, sessionId: session.id,
      module: "connections", detail: { connectionId: session.connectionId }, onClose: disconnect,
      mount: pane => mountSshTerminal(pane, session, () => {
        excludedSsh.add(session.id); tabs.remove(id);
      }, {
        tabbed: true, onDisconnect: disconnect,
        onDetach: async () => { await openDetachedTerminalWindow(session); excludedSsh.add(session.id); },
        onDisconnected: () => tabs.update(id, { status: "closed" }),
      }),
    }, { select });
  }
  async function attachSsh(session) {
    const sessions = await listAgentSshSessions();
    if (disposed) return;
    const live = sessions.find(item => item.id === session.id);
    if (!live) {
      const id = `ssh:${session.id}`;
      tabs.update(id, { status: "closed" });
      if (tabs.get(id)) tabs.activate(id);
      else shell.navigate("connections", { connectionId: session.connectionId });
      shell.showMessage("This SSH session has closed. Open a new SSH session from Connections.");
      return;
    }
    if (await focusDetachedTerminalWindow(live)) return;
    openSsh(live);
  }
  function openPlan(planId) {
    if (disposed) return;
    const plan = plansById.get(planId);
    tabs.open({ id: `plan:${planId}`, type: "plan", planId, hiddenTab: true,
      title: plan ? `#${plan.planNumber} · ${plan.name}` : "Deployment Plan", status: plan?.status,
      module: "deployments", detail: { section: "queue", planId },
      mount: pane => { void mountDeploymentQueueView(pane, { section: "queue", planId }, { onQueueChange: queueChanged }); },
    });
  }
  function renderModule(module, content, detail) {
    if (module === "connections") {
      void mountConnectionsView(content, { ...(detail ?? {}), navigationRoot: shell.navigationHost("connections"), onOpenSsh: session => { void attachSsh(session).catch(fail); } });
    } else if (module === "notes") {
      mountNotesView(content, { ...(detail ?? {}), navigationRoot: shell.navigationHost("notes") });
    } else if (module === "vaults") {
      mountVaultsView(content, detail ?? {});
    } else if (module === "backup") {
      mountBackupView(content);
    } else if (module === "agent") {
      void mountAgentView(content, { ...(detail ?? {}), onFocusDeploymentPlan: openPlan,
        onQueueChange: queueChanged,
        onAttachSshSession: session => { void attachSsh(session).catch(fail); }, onSnapshot: snapshot => { void syncSessions(snapshot).catch(fail); } });
    } else if (module === "ai-skills") {
      mountAiSkillsView(content, detail ?? {}, { onCatalogChange: (libraries, skills, groups) => shell.setAiSkillsNavigation(libraries, skills, groups) });
    } else if (module === "help") {
      mountHelpView(content, detail ?? { section: "overview" }, { onNavigate: topic => shell.navigate("help", topic) });
    } else if (module === "deployments") {
      if (detail?.section === "config") {
        mountDeploymentConfigView(content, detail, { onCatalogChange: (processes, selectedId) => shell.setDeploymentNavigation(processes, selectedId) });
      } else if (detail?.section === "queue") {
        let openedPlan = null;
        void mountDeploymentQueueView(content, detail ?? { section: "queue" }, { onQueueChange: (plans, selectedId) => {
          queueChanged(plans);
          if (selectedId && selectedId !== openedPlan) { openedPlan = selectedId; openPlan(selectedId); }
        } });
        getDeploymentConfig().then(snapshot => { if (!disposed) shell.setDeploymentCatalog(snapshot.processes); }).catch(() => {});
      } else {
        content.replaceChildren();
        getDeploymentConfig().then(snapshot => { if (!disposed) shell.setDeploymentCatalog(snapshot.processes); }).catch(() => {});
        getDeploymentQueue().then(snapshot => queueChanged(snapshot.plans)).catch(() => {});
      }
    }
  }
  shell = mountAppShell(root, {
    onLock: async () => {
      stopCredentialMonitor();
      await lockWorkspace();
      showWorkspaceGate();
    },
    onBiometricToggle: toggleBiometric,
    onCheckUpdates: () => updates.check(),
    onOpenLocalTerminal: async themeId => {
      const palette = TERMINAL_THEMES.find(theme => theme.id === themeId);
      if (!palette) return;
      const session = await openLocalTerminal();
      if (disposed) { await localTerminalTransport.close(session.id); return; }
      const id = `local:${session.id}`;
      const terminalSession = { ...session, name: `Terminal · ${palette.name}` };
      let closing = null;
      const close = () => closing ??= localTerminalTransport.close(session.id).catch(error => { closing = null; throw error; });
      tabs.open({ id, type: "local", title: terminalSession.name, status: "connected", onClose: close,
        mount: pane => {
          const view = mountSshTerminal(pane, terminalSession, () => tabs.remove(id), {
            local: true, transport: localTerminalTransport,
            theme: { background: palette.background, foreground: palette.foreground, cursor: palette.foreground, selectionBackground: `${palette.foreground}40` },
            onDisconnect: close, onDisconnected: () => tabs.update(id, { status: "closed" }),
          });
          return { activate: view.activate, dispose() { view.dispose(); void close().catch(() => {}); } };
        },
      });
    },
    onNavigate: (module, _content, detail) => {
      if (module === "connections" && detail?.terminalSession) { void attachSsh(detail.terminalSession).catch(fail); return; }
      if (module === "deployments" && detail?.section === "queue" && detail.planId) { openPlan(detail.planId); return; }
      tabs.open({ id: "workspace", type: "workspace", hiddenTab: true, title: labels[module] || module, closable: false, module, detail,
        mount: pane => { renderModule(module, pane, detail); },
      }, { replace: true });
    },
  });
  tabs = mountWorkspaceTabs(root.querySelector('[data-role="workspace-tabs"]'), root.querySelector('[data-role="module-content"]'), {
    onActivate: tab => shell.selectNavigation(tab.module ?? null, tab.detail ?? null), onError: fail,
  });
  tabs.open({ id: "workspace", type: "workspace", hiddenTab: true, title: "Connections", closable: false,
    module: "connections", detail: null, mount: pane => { renderModule("connections", pane, null); },
  });
  let sessionTimer = null;
  const refreshSessions = async () => {
    try { const sshSessions = await listAgentSshSessions(); if (!disposed) await syncSessions({ sshSessions }); } catch { /* Agent may be stopped. */ }
    if (!disposed) sessionTimer = window.setTimeout(refreshSessions, 5000);
  };
  void refreshSessions();
  const navigationEvents = new AbortController();
  for (const module of ["connections", "notes"]) {
    const host = shell.navigationHost(module);
    host.addEventListener("click", event => {
      if (!event.target.closest("button")) return;
      const context = tabs.get("workspace");
      if (context?.module === module) {
        if (tabs.activeId() !== "workspace") tabs.activate("workspace");
        return;
      }
      event.preventDefault(); event.stopImmediatePropagation();
      const connection = event.target.closest("[data-connection-id]");
      const note = event.target.closest("[data-note-id]");
      const folder = event.target.closest("[data-folder-id]");
      shell.navigate(module, connection ? { connectionId: connection.dataset.connectionId }
        : note ? { noteId: note.dataset.noteId } : folder ? { folderId: folder.dataset.folderId } : null);
    }, { capture: true, signal: navigationEvents.signal });
  }
  disposeShell = () => { disposed = true; snapshotRevision++; window.clearTimeout(sessionTimer); navigationEvents.abort(); tabs.destroy(); shell.destroy(); };
  stopCredentialMonitor = monitorAgentCredentials((message, error) => shell.showMessage(message, error));
  const { biometricButton } = shell;

  const updateBiometricButton = () => {
    biometricButton.hidden = !biometric?.available;
    biometricButton.classList.toggle("is-active", Boolean(biometric?.configured));
    const action = biometric?.configured ? "Disable" : "Enable";
    const label = biometric?.label || "biometric";
    biometricButton.title = `${action} ${label} unlock`;
    biometricButton.setAttribute("aria-label", biometricButton.title);
  };

  biometricStatus(session.path).then((status) => {
    biometric = status;
    updateBiometricButton();
  }).catch(() => {});

  async function toggleBiometric() {
    biometricButton.disabled = true;
    try {
      if (biometric.configured) {
        await disableBiometricUnlock(session.path);
        biometric.configured = false;
      } else {
        biometric = await enableBiometricUnlock();
      }
      updateBiometricButton();
      shell.showMessage(`${biometric.label} unlock ${biometric.configured ? "enabled" : "disabled"}.`);
    } catch (cause) {
      shell.showMessage(String(cause), true);
    } finally {
      biometricButton.disabled = false;
    }
  }
}

function showWorkspaceGate() {
  disposeShell();
  disposeShell = () => {};
  mountWorkspaceGate(root, renderShell);
}

const detachedTerminal = new URLSearchParams(window.location.search).get("detachedTerminal");
if (detachedTerminal) {
  mountDetachedTerminalWindow(root, detachedTerminal);
} else {
  showWorkspaceGate();
  updates.start();
}

