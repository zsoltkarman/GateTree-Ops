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
          <p>Save all workspace content to a JSON file.</p>
        </div>
        <button data-action="export" ${state.busy ? "disabled" : ""}>Export JSON</button>
      </section>
      <section class="backup-card">
        <span class="backup-card-icon">${icon("import")}</span>
        <div>
          <strong>Import workspace</strong>
          <p>Replace workspace content from a PHAROS JSON backup.</p>
        </div>
        <button data-action="import" ${state.busy ? "disabled" : ""}>Import JSON</button>
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
      title: "Export PHAROS workspace",
      defaultPath: "pharos-backup.json",
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
  } catch (cause) {
    showError(root, state, cause);
    return;
  }
  if (!path) return;
  await run(root, state, async () => {
    const summary = await exportBackup(path);
    state.message = `Exported ${summary.records} records from ${summary.tables} tables.`;
  });
}

async function importWorkspace(root, state) {
  let path;
  try {
    path = await window.__TAURI__.dialog.open({
      directory: false,
      multiple: false,
      title: "Import PHAROS workspace",
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
  } catch (cause) {
    showError(root, state, cause);
    return;
  }
  if (!path) return;
  if (!window.confirm("Replace all current workspace content with this backup?")) return;
  await run(root, state, async () => {
    const summary = await importBackup(path);
    state.message = `Imported ${summary.records} records into ${summary.tables} tables.`;
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
