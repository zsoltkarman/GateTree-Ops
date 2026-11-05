import { escapeHtml as e } from "../../shared/ui.js";
import { getImageHub, saveImageHubConfigEntry, deleteImageHubConfigEntry, importImageHubConfig } from "./image-hub-api.js";

export async function mountImageHubConfigView(root) {
  const state = { config: {}, originalName: null, name: "", value: "", busy: false, message: "", error: false };
  root.innerHTML = '<p class="image-hub-loading">Loading Image Hub configuration…</p>';
  try {
    state.config = (await getImageHub()).config;
    if (root.isConnected) render();
  } catch (error) { root.innerHTML = `<p class="image-hub-message is-error" role="alert">${e(error)}</p>`; }

  function clearEditor() { state.originalName = null; state.name = ""; state.value = ""; }
  function render() {
    const entries = Object.entries(state.config).filter(([, value]) => typeof value === "string" && value !== "")
      .sort(([left], [right]) => left.localeCompare(right));
    root.innerHTML = `<section class="image-hub-workspace image-hub-config">
      <header class="image-hub-header"><div><h1>Image Hub</h1><p>Config</p></div><button type="button" data-import-config>Import JSON…</button></header>
      <div class="vaults-module">
        <form class="vault-entry-editor" data-config-entry-form>
          <label><span>Name</span><input data-config-name value="${e(state.name)}" maxlength="120" required autocomplete="off"></label>
          <label><span>Value</span><input data-config-value value="${e(state.value)}" required autocomplete="off" spellcheck="false"></label>
          <div class="vault-entry-form-actions">
            <button type="submit" class="vault-primary-action" title="${state.originalName === null ? "Add entry" : "Save entry"}" aria-label="${state.originalName === null ? "Add entry" : "Save entry"}"${state.name.trim() && state.value.trim() ? "" : " disabled"}>${icon(state.originalName === null ? "plus" : "save")}</button>
            <button type="button" data-config-cancel title="Cancel editing" aria-label="Cancel editing"${state.originalName === null ? " hidden" : ""}>${icon("close")}</button>
          </div>
        </form>
        <p class="vault-status${state.error ? " is-error" : ""}" role="${state.error ? "alert" : "status"}"${state.message ? "" : " hidden"}>${e(state.message)}</p>
        <section class="vault-entry-list">
          <div class="vault-entry-header"><span>Name</span><span>Value</span><span>Actions</span></div>
          ${entries.length ? entries.map(([name, value]) => `<div class="vault-entry-row">
            <strong>${e(name)}</strong><code>${e(value)}</code>
            <span class="vault-entry-actions"><button type="button" data-config-edit="${e(name)}" title="Edit entry" aria-label="Edit ${e(name)}">${icon("pencil")}</button><button type="button" data-config-delete="${e(name)}" title="Delete entry" aria-label="Delete ${e(name)}">${icon("trash")}</button></span>
          </div>`).join("") : '<p class="vault-empty">No configuration entries saved.</p>'}
        </section>
      </div>
    </section>`;
    bind();
  }

  async function run(action, message, clear = true) {
    if (state.busy) return;
    state.busy = true;
    const controls = [...root.querySelectorAll("button, input")]; controls.forEach(c => { c.disabled = true; });
    try {
      const config = await action();
      if (config === null) return;
      state.config = config;
      if (clear) clearEditor();
      state.message = message; state.error = false;
      if (root.isConnected) render();
    } catch (error) {
      state.message = String(error); state.error = true;
      if (root.isConnected) render();
    } finally { state.busy = false; controls.forEach(c => { c.disabled = false; }); }
  }

  function bind() {
    const nameInput = root.querySelector("[data-config-name]"), valueInput = root.querySelector("[data-config-value]");
    const updateDraft = () => {
      state.name = nameInput.value; state.value = valueInput.value;
      root.querySelector(".vault-primary-action").disabled = !state.name.trim() || !state.value.trim();
    };
    nameInput.addEventListener("input", updateDraft); valueInput.addEventListener("input", updateDraft);
    root.querySelector("[data-config-entry-form]").addEventListener("submit", event => {
      event.preventDefault(); updateDraft();
      void run(() => saveImageHubConfigEntry(state.originalName, state.name, state.value), "Configuration entry saved.");
    });
    root.querySelector("[data-config-cancel]").addEventListener("click", () => { clearEditor(); render(); });
    root.querySelectorAll("[data-config-edit]").forEach(button => button.addEventListener("click", () => {
      state.originalName = button.dataset.configEdit; state.name = state.originalName; state.value = state.config[state.originalName];
      state.message = ""; render(); root.querySelector("[data-config-name]").focus();
    }));
    root.querySelectorAll("[data-config-delete]").forEach(button => button.addEventListener("click", () => {
      const name = button.dataset.configDelete;
      if (!window.confirm(`Delete configuration entry “${name}”?`)) return;
      void run(() => deleteImageHubConfigEntry(name), "Configuration entry deleted.", state.originalName === name);
    }));
    root.querySelector("[data-import-config]").addEventListener("click", () => {
      void run(async () => {
        const path = await window.__TAURI__.dialog.open({ multiple: false, filters: [{ name: "Image Hub configuration", extensions: ["json"] }], title: "Import Image Hub configuration" });
        return path ? importImageHubConfig(path) : null;
      }, "Configuration imported.");
    });
  }
}

function icon(name) {
  const paths = {
    plus: "<path d='M12 5v14M5 12h14'/>",
    save: "<path d='M5 4h12l2 2v14H5zM8 4v6h8V4M8 20v-6h8v6'/>",
    close: "<path d='m7 7 10 10M17 7 7 17'/>",
    pencil: "<path d='m5 16-.7 3.7L8 19l10-10-3-3L5 16ZM13.5 7.5l3 3'/>",
    trash: "<path d='M5 7h14M9 7V4h6v3M7.5 7l.7 13h7.6l.7-13M10 11v5M14 11v5'/>",
  };
  return `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[name]}</svg>`;
}
