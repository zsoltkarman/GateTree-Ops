import { escapeHtml } from "../../shared/ui.js";
import {
  clearKeePassDatabasePath,
  createVaultEntry,
  deleteVaultEntry,
  getVaults,
  updateVaultEntry,
  validateAndSetKeePassDatabasePath,
} from "./vaults-api.js";

export async function mountVaultsView(root, detail = {}) {
  const state = {
    databasePath: "",
    databaseDraft: "",
    entries: [],
    editingId: null,
    name: "",
    entryPath: "",
    message: "",
    isError: false,
  };
  root.innerHTML = '<div class="vaults-loading">Loading vault references…</div>';
  try {
    const snapshot = await getVaults();
    state.databasePath = snapshot.databasePath;
    state.databaseDraft = snapshot.databasePath;
    state.entries = snapshot.entries;
    render(root, state);
    if (detail.entryId) {
      const row = root.querySelector(`[data-entry-id="${CSS.escape(detail.entryId)}"]`)?.closest(".vault-entry-row");
      if (row) {
        row.classList.add("is-search-target");
        row.scrollIntoView({ block: "center" });
      }
    }
  } catch (cause) {
    root.innerHTML = `<div class="vaults-error">${escapeHtml(cause)}</div>`;
  }
}

function render(root, state) {
  const configured = Boolean(state.databasePath);
  root.innerHTML = `
    <div class="vaults-module">
      <section class="keepass-database-card">
        <label for="keepass-database">KeePass database</label>
        <div class="keepass-database-control">
          <input id="keepass-database" data-role="database-path" value="${escapeHtml(state.databaseDraft)}" placeholder="Choose a .kdbx file" spellcheck="false" />
          <button data-action="choose-database" title="Choose KeePass database" aria-label="Choose KeePass database">${icon("folder")}</button>
          <button data-action="save-database" title="Save database path" aria-label="Save database path" ${state.databaseDraft.trim() ? "" : "disabled"}>${icon("save")}</button>
          <button data-action="clear-database" title="Clear database path" aria-label="Clear database path" ${configured ? "" : "disabled"}>${icon("trash")}</button>
        </div>
      </section>

      <form class="vault-entry-editor" data-role="entry-form">
        <label>
          <span>Name</span>
          <input data-role="entry-name" value="${escapeHtml(state.name)}" autocomplete="off" ${configured ? "" : "disabled"} />
        </label>
        <label>
          <span>KeePass entry path</span>
          <input data-role="entry-path" value="${escapeHtml(state.entryPath)}" autocomplete="off" spellcheck="false" ${configured ? "" : "disabled"} />
        </label>
        <div class="vault-entry-form-actions">
          <button class="vault-primary-action" type="submit" title="${state.editingId ? "Save reference" : "Add reference"}" aria-label="${state.editingId ? "Save reference" : "Add reference"}" ${configured && state.name.trim() && state.entryPath.trim() ? "" : "disabled"}>${icon(state.editingId ? "save" : "plus")}</button>
          <button type="button" data-action="cancel-entry" title="Cancel editing" aria-label="Cancel editing" ${state.editingId ? "" : "hidden"}>${icon("close")}</button>
        </div>
      </form>

      <p class="vault-status${state.isError ? " is-error" : ""}" role="status" ${state.message ? "" : "hidden"}>${escapeHtml(state.message)}</p>

      <section class="vault-entry-list">
        <div class="vault-entry-header"><span>Name</span><span>Path</span><span>Actions</span></div>
        ${state.entries.length ? state.entries.map((entry) => `
          <div class="vault-entry-row">
            <div class="vault-entry-value"><strong>${escapeHtml(entry.name)}</strong><button type="button" data-copy-entry="name" data-entry-id="${entry.id}" title="Copy name" aria-label="Copy name">${icon("copy")}</button></div>
            <div class="vault-entry-value"><code>${escapeHtml(entry.entryPath)}</code><button type="button" data-copy-entry="entryPath" data-entry-id="${entry.id}" title="Copy KeePass path" aria-label="Copy KeePass path">${icon("copy")}</button></div>
            <span class="vault-entry-actions">
              <button data-action="edit-entry" data-entry-id="${entry.id}" title="Edit reference" aria-label="Edit ${escapeHtml(entry.name)}">${icon("pencil")}</button>
              <button data-action="delete-entry" data-entry-id="${entry.id}" title="Delete reference" aria-label="Delete ${escapeHtml(entry.name)}">${icon("trash")}</button>
            </span>
          </div>`).join("") : '<p class="vault-empty">No KeePass references saved.</p>'}
      </section>
    </div>`;

  bindEvents(root, state);
}

function bindEvents(root, state) {
  root.querySelectorAll("[data-copy-entry]").forEach(button => {
    button.addEventListener("click", async () => {
      const entry = state.entries.find(item => item.id === button.dataset.entryId);
      if (!entry) return;
      const label = button.title;
      try {
        await copyVaultText(entry[button.dataset.copyEntry]);
        button.innerHTML = icon("check");
        button.title = "Copied";
        button.setAttribute("aria-label", "Copied");
        window.setTimeout(() => {
          if (!button.isConnected) return;
          button.innerHTML = icon("copy");
          button.title = label;
          button.setAttribute("aria-label", label);
        }, 1500);
      } catch (cause) {
        const status = root.querySelector(".vault-status");
        status.textContent = `Could not copy: ${String(cause)}`;
        status.classList.add("is-error");
        status.hidden = false;
      }
    });
  });
  const databaseInput = root.querySelector('[data-role="database-path"]');
  databaseInput.addEventListener("input", () => {
    state.databaseDraft = databaseInput.value;
    root.querySelector('[data-action="save-database"]').disabled = !state.databaseDraft.trim();
  });
  root.querySelector('[data-action="choose-database"]').addEventListener("click", async () => {
    const selected = await window.__TAURI__.dialog.open({
      directory: false,
      multiple: false,
      title: "Choose KeePass database",
      filters: [{ name: "KeePass database", extensions: ["kdbx"] }],
    });
    if (!selected) return;
    state.databaseDraft = selected;
    render(root, state);
  });
  root.querySelector('[data-action="save-database"]').addEventListener("click", async () => {
    const masterPassword = await requestKeePassPassword(root);
    if (masterPassword === null) return;
    const progressDialog = showValidationProgress(root);
    await runAction(root, state, async () => {
      try {
        state.databasePath = await validateAndSetKeePassDatabasePath(state.databaseDraft, masterPassword);
        state.databaseDraft = state.databasePath;
        state.message = `KeePass database saved. ${state.entries.length} entries validated.`;
      } finally {
        progressDialog.close();
        progressDialog.remove();
      }
    });
  });
  root.querySelector('[data-action="clear-database"]').addEventListener("click", async () => {
    if (!window.confirm("Clear the KeePass database path? Saved references will remain.")) return;
    await runAction(root, state, async () => {
      await clearKeePassDatabasePath();
      state.databasePath = "";
      state.databaseDraft = "";
      clearEditor(state);
      state.message = "KeePass database path cleared.";
    });
  });

  const nameInput = root.querySelector('[data-role="entry-name"]');
  const pathInput = root.querySelector('[data-role="entry-path"]');
  const updateDraft = () => {
    state.name = nameInput.value;
    state.entryPath = pathInput.value;
    root.querySelector(".vault-primary-action").disabled = !state.name.trim() || !state.entryPath.trim();
  };
  nameInput.addEventListener("input", updateDraft);
  pathInput.addEventListener("input", updateDraft);
  root.querySelector('[data-role="entry-form"]').addEventListener("submit", async (event) => {
    event.preventDefault();
    updateDraft();
    await runAction(root, state, async () => {
      if (state.editingId) {
        const updated = await updateVaultEntry(state.editingId, state.name, state.entryPath);
        const index = state.entries.findIndex((entry) => entry.id === updated.id);
        state.entries[index] = updated;
        state.message = "KeePass reference updated.";
      } else {
        state.entries.push(await createVaultEntry(state.name, state.entryPath));
        state.message = "KeePass reference added.";
      }
      state.entries.sort((left, right) => left.name.localeCompare(right.name));
      clearEditor(state, true);
    });
  });
  root.querySelector('[data-action="cancel-entry"]')?.addEventListener("click", () => {
    clearEditor(state);
    render(root, state);
  });
  root.querySelectorAll('[data-action="edit-entry"]').forEach((button) => {
    button.addEventListener("click", () => {
      const entry = state.entries.find((item) => item.id === button.dataset.entryId);
      state.editingId = entry.id;
      state.name = entry.name;
      state.entryPath = entry.entryPath;
      state.message = "";
      render(root, state);
      root.querySelector('[data-role="entry-name"]').focus();
    });
  });
  root.querySelectorAll('[data-action="delete-entry"]').forEach((button) => {
    button.addEventListener("click", async () => {
      const entry = state.entries.find((item) => item.id === button.dataset.entryId);
      if (!window.confirm(`Delete KeePass reference “${entry.name}”?`)) return;
      await runAction(root, state, async () => {
        await deleteVaultEntry(entry.id);
        state.entries = state.entries.filter((item) => item.id !== entry.id);
        if (state.editingId === entry.id) clearEditor(state);
        state.message = "KeePass reference deleted.";
      });
    });
  });
}

async function copyVaultText(value) {
  if (navigator.clipboard?.writeText) {
    try { await navigator.clipboard.writeText(value); return; } catch (_) {}
  }
  const previousFocus = document.activeElement;
  const field = document.createElement("textarea");
  field.value = value;
  field.style.position = "fixed";
  field.style.opacity = "0";
  document.body.append(field);
  try {
    field.select();
    if (!document.execCommand("copy")) throw new Error("Clipboard is unavailable. Select the text and copy it manually.");
  } finally {
    field.remove();
    previousFocus?.focus({ preventScroll: true });
  }
}

function requestKeePassPassword(root) {
  return new Promise((resolve) => {
    const dialog = document.createElement("dialog");
    dialog.className = "queue-password-dialog";
    dialog.innerHTML = `<form method="dialog">
      <strong>Validate KeePass database</strong>
      <small>The password is used only for this validation and is not saved.</small>
      <label><span>Master password</span><input type="password" autocomplete="current-password" required></label>
      <div><button value="cancel" formnovalidate>Cancel</button><button class="connection-primary-action" value="validate">Validate and Save</button></div>
    </form>`;
    root.append(dialog);
    const input = dialog.querySelector("input");
    dialog.addEventListener("close", () => {
      const value = dialog.returnValue === "validate" ? input.value : null;
      input.value = "";
      dialog.remove();
      resolve(value);
    }, { once: true });
    dialog.showModal();
    requestAnimationFrame(() => input.focus());
  });
}

function showValidationProgress(root) {
  const dialog = document.createElement("dialog");
  dialog.className = "queue-password-dialog queue-progress-dialog";
  dialog.innerHTML = `<div><span class="queue-progress-spinner"></span><strong>Validating KeePass database and entries…</strong></div>`;
  root.append(dialog);
  dialog.showModal();
  return dialog;
}

async function runAction(root, state, action) {
  state.message = "";
  state.isError = false;
  try {
    await action();
  } catch (cause) {
    state.message = String(cause);
    state.isError = true;
  }
  render(root, state);
}

function clearEditor(state, keepMessage = false) {
  state.editingId = null;
  state.name = "";
  state.entryPath = "";
  if (!keepMessage) state.message = "";
}

function icon(name) {
  const paths = {
    copy: "<rect x='8' y='8' width='11' height='11' rx='1.5'/><path d='M16 8V5H5v11h3'/>",
    check: "<path d='m5 12 4 4L19 6'/>",
    folder: "<path d='M3.5 7.5h6l2-2h9v13h-17z'/>",
    save: "<path d='M5 4h12l2 2v14H5zM8 4v6h8V4M8 20v-6h8v6'/>",
    trash: "<path d='M5 7h14M9 7V4h6v3M7.5 7l.7 13h7.6l.7-13M10 11v5M14 11v5'/>",
    plus: "<path d='M12 5v14M5 12h14'/>",
    close: "<path d='m7 7 10 10M17 7 7 17'/>",
    pencil: "<path d='m5 16-.7 3.7L8 19l10-10-3-3L5 16ZM13.5 7.5l3 3'/>",
  };
  return `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[name]}</svg>`;
}
