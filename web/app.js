import { monitorAgentCredentials } from "./modules/agent/credential-recovery.js";
import { mountWorkspaceGate } from "./modules/workspace/workspace-gate.js";
import { mountAppShell } from "./modules/shell/app-shell.js";
import { mountConnectionsView } from "./modules/connections/connections-view.js";
import { focusDetachedTerminalWindow, mountDetachedTerminalWindow } from "./modules/connections/detached-terminal-window.js";
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
initializeTheme();

let stopCredentialMonitor = () => {};

function renderShell(session) {
  stopCredentialMonitor();
  let biometric = null;
  let shell = null;
  shell = mountAppShell(root, {
    onLock: async () => {
    stopCredentialMonitor();
    await lockWorkspace();
    showWorkspaceGate();
    },
    onBiometricToggle: toggleBiometric,
    onNavigate: (module, content, detail) => {
      if (module === "connections") {
        mountConnectionsView(content, { ...(detail ?? {}), navigationRoot: shell.navigationHost("connections") });
      } else if (module === "notes") {
        mountNotesView(content, { navigationRoot: shell.navigationHost("notes") });
      } else if (module === "vaults") {
        mountVaultsView(content);
      } else if (module === "backup") {
        mountBackupView(content);
      } else if (module === "agent") {
        mountAgentView(content, {
          onFocusDeploymentPlan: (planId) => {
            shell.navigate("deployments", { section: "queue", planId });
          },
          onAttachSshSession: async (terminalSession) => {
            if (!await focusDetachedTerminalWindow(terminalSession)) {
              shell.navigate("connections", { terminalSession });
            }
          },
        });
      } else if (module === "ai-skills") {
        mountAiSkillsView(content, detail ?? {}, {
          onCatalogChange: (libraries, skills, groups) => shell.setAiSkillsNavigation(libraries, skills, groups),
        });
      } else if (module === "deployments") {
        if (detail?.section === "config") {
          mountDeploymentConfigView(content, detail, {
            onCatalogChange: (processes, selectedId) =>
              shell.setDeploymentNavigation(processes, selectedId),
          });
        } else if (detail?.section === "queue") {
          mountDeploymentQueueView(content, detail ?? { section: "queue" }, {
            onQueueChange: (plans, selectedPlanId) => shell.setDeploymentQueue(plans, selectedPlanId),
          });
          getDeploymentConfig().then((snapshot) => shell.setDeploymentCatalog(snapshot.processes)).catch(() => {});
        } else {
          content.replaceChildren();
          getDeploymentConfig().then((snapshot) => shell.setDeploymentCatalog(snapshot.processes)).catch(() => {});
          getDeploymentQueue().then((snapshot) => shell.setDeploymentQueue(snapshot.plans)).catch(() => {});
        }
      } else {
        content.replaceChildren();
      }
    },
  });
  stopCredentialMonitor = monitorAgentCredentials((message, error) => shell.showMessage(message, error));
  const { biometricButton } = shell;
  mountConnectionsView(root.querySelector('[data-role="module-content"]'), {
    navigationRoot: shell.navigationHost("connections"),
  });

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
  mountWorkspaceGate(root, renderShell);
}

const detachedTerminal = new URLSearchParams(window.location.search).get("detachedTerminal");
if (detachedTerminal) {
  mountDetachedTerminalWindow(root, detachedTerminal);
} else {
  showWorkspaceGate();
}

