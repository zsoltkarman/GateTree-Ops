import { getAgentStatus, restoreDeploymentCredentials, startAgent } from "./agent-api.js";
import { getVaults } from "../vaults/vaults-api.js";
import { requestKeePassPassword } from "../../shared/keepass-password.js";

let activeSession = null;

export async function ensureAgentCredentials() {
  if (!activeSession) throw new Error("Unlock the workspace first.");
  return activeSession(true);
}

export function monitorAgentCredentials(showMessage) {
  const controller = new AbortController();
  let timer;
  let unlockedKey = null;
  let dismissedKey = null;
  let startupPending = true;
  let mismatchPid = null;
  let inFlight = null;
  const check = async (forJob = false) => {
    if (controller.signal.aborted) return false;
    if (!forJob && document.querySelector("dialog[open]")) return false;
    let status = await getAgentStatus();
    if (forJob && !status.running) status = await startAgent();
    if (controller.signal.aborted) return false;
    if (status.running && (status.protocolVersion !== status.expectedProtocolVersion || status.version !== status.expectedVersion)) {
      const message = "An older or incompatible Agent is running. In Agent, use Stop or Kill Agent, then Start.";
      if (mismatchPid !== status.pid) showMessage(message, true);
      mismatchPid = status.pid;
      if (forJob) throw new Error(message);
      return false;
    }
    if (!status.running && !startupPending) return false;
    const vault = await getVaults();
    if (!vault.databasePath || controller.signal.aborted) {
      if (forJob && !controller.signal.aborted) throw new Error("Choose a KeePass database in Vaults first.");
      return false;
    }
    const key = `${status.running ? status.pid : "startup"}:${vault.databasePath}`;
    if (unlockedKey === key) return true;
    if (dismissedKey === key) return false;
    let error = "";
    while (!controller.signal.aborted) {
      let password = await requestKeePassPassword(document.body, { signal: controller.signal, restoring: true, error });
      if (password === null || controller.signal.aborted) {
        dismissedKey = key;
        startupPending = false;
        return false;
      }
      try {
        await restoreDeploymentCredentials(password);
        const current = await getAgentStatus();
        // A restart during unlock must trigger a fresh prompt.
        if (status.running && current.pid !== status.pid) {
          status = current;
          error = "Agent restarted during KeePass unlock. Unlock again.";
          continue;
        }
        unlockedKey = `${current.pid}:${vault.databasePath}`;
        startupPending = false;
        if (!controller.signal.aborted) showMessage("KeePass unlocked for this session.");
        return !controller.signal.aborted;
      } catch (cause) {
        error = String(cause);
      } finally { password = null; }
    }
    return false;
  };
  const ensure = (forJob = false) => {
    if (!inFlight) inFlight = check(forJob).finally(() => { inFlight = null; });
    return inFlight;
  };
  activeSession = ensure;
  const poll = async () => {
    try { await ensure(); }
    catch (error) { if (!controller.signal.aborted) showMessage(String(error), true); }
    finally { if (!controller.signal.aborted) timer = window.setTimeout(poll, 1500); }
  };
  poll();
  return () => {
    controller.abort();
    window.clearTimeout(timer);
    if (activeSession === ensure) activeSession = null;
  };
}
