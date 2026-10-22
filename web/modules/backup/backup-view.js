import { escapeHtml } from "../../shared/ui.js";
import { exportBackup, importBackup } from "./backup-api.js";

export function mountBackupView(root) {
  render(root, { busy: false, message: "", isError: false });
}

function render(root, state) {
  root.innerHTML = `
    <div class="backup-module">
      <section class="backup-card">
        <span class="backup-card-icon">${icon("export")}</span>
        <div>
          <strong>Export workspace</strong>
          <p>Save an encrypted backup protected by this workspace’s password.</p>
        </div>
        <button data-action="export" ${state.busy ? "disabled" : ""}>Export encrypted backup</button>
      </section>
      <section class="backup-card">
        <span class="backup-card-icon">${icon("import")}</span>
        <div>
          <strong>Import workspace</strong>
          <p>Restore an encrypted backup using its original workspace password. Older JSON backups are also supported.</p>
        </div>
        <button data-action="import" ${state.busy ? "disabled" : ""}>Import backup</button>
      </section>
      ${state.message ? `<p class="backup-status${state.isError ? " is-error" : ""}">${escapeHtml(state.message)}</p>` : ""}
    </div>`;

  root.querySelector('[data-action="export"]').addEventListener("click", () => exportWorkspace(root, state));
  root.querySelector('[data-action="import"]').addEventListener("click", () => importWorkspace(root, state));
}

async function exportWorkspace(root, state) {
  let path;
  try {
    path = await window.__TAURI__.dialog.save({
      title: "Export GateTree-Ops workspace",
      defaultPath: "gatetree-ops-backup.gtbak",
      filters: [{ name: "Encrypted GateTree-Ops backup", extensions: ["gtbak"] }],
    });
  } catch (cause) {
    showError(root, state, cause);
    return;
  }
  if (!path) return;
  await run(root, state, async () => {
    const summary = await exportBackup(path);
    state.message = `Encrypted backup exported: ${summary.records} records from ${summary.tables} tables. Use this workspace’s password to restore it.`;
  });
}

async function importWorkspace(root, state) {
  let path;
  try {
    path = await window.__TAURI__.dialog.open({
      directory: false,
      multiple: false,
      title: "Import GateTree-Ops workspace",
      filters: [{ name: "GateTree-Ops backup", extensions: ["gtbak", "json"] }],
    });
  } catch (cause) {
    showError(root, state, cause);
    return;
  }
  if (!path) return;
  if (!window.confirm("Replace all current workspace content with this backup?")) return;
  let password = await requestBackupPassword(root);
  if (password === null) return;
  await run(root, state, async () => {
    try {
      const summary = await importBackup(path, password);
      state.message = `Imported ${summary.records} records into ${summary.tables} tables.`;
    } finally { password = ""; }
  });
}

function requestBackupPassword(root) {
  return new Promise(resolve => {
    const dialog = document.createElement("dialog");
    dialog.className = "queue-password-dialog";
    dialog.innerHTML = `<form method="dialog">
      <strong>Decrypt backup</strong>
      <small>Enter the workspace password used when this backup was exported. For an older unencrypted JSON backup, leave this blank.</small>
      <label><span>Backup workspace password</span><input type="password" autocomplete="off"></label>
      <div><button type="button" data-cancel-backup>Cancel</button><button type="submit" class="connection-primary-action">Decrypt and import</button></div>
    </form>`;
    root.append(dialog);
    const input = dialog.querySelector("input");
    dialog.querySelector("form").addEventListener("submit", event => { event.preventDefault(); dialog.close("import"); });
    dialog.querySelector("[data-cancel-backup]").addEventListener("click", () => dialog.close("cancel"));
    dialog.addEventListener("close", () => {
      const value = dialog.returnValue === "import" ? input.value : null;
      input.value = "";
      dialog.remove();
      resolve(value);
    }, { once: true });
    dialog.showModal();
    requestAnimationFrame(() => input.focus());
  });
}

function showError(root, state, cause) {
  state.message = String(cause);
  state.isError = true;
  render(root, state);
}

async function run(root, state, action) {
  state.busy = true;
  state.message = "";
  state.isError = false;
  render(root, state);
  try {
    await action();
  } catch (cause) {
    state.message = String(cause);
    state.isError = true;
  } finally {
    state.busy = false;
    render(root, state);
  }
}

function icon(name) {
  const paths = {
    export: "<path d='M5 5h14v14H5zM8 5v5h8V5M8 19v-6h8v6M12 2v7M9 5l3-3 3 3'/>",
    import: "<path d='M5 5h14v14H5zM8 5v5h8V5M8 19v-6h8v6M12 9V2M9 6l3 3 3-3'/>",
  };
  return `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[name]}</svg>`;
}
