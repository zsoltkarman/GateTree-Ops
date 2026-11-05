import { escapeHtml } from "../../shared/ui.js";
import { saveValidationQuery, deleteValidationQuery, getLocalDeploymentRoles } from "./deployments-api.js";

export function mountValidationQueries(root, { queries = [], save = saveValidationQuery, remove = deleteValidationQuery, loadRoles = getLocalDeploymentRoles } = {}) {
  const state = { queries: [...queries], draft: null, busy: false, message: "", error: false, open: false, roles: [], rolesLoading: false, rolesError: "" };
  let roleRequest = 0;
  const alive = () => root.isConnected;
  function render() {
    root.innerHTML = `<details class="validation-query-library" ${state.open ? "open" : ""}>
      <summary><span>Validation Queries <small>${state.queries.length}</small></span><button type="button" data-query-add ${state.busy ? "disabled" : ""}>+ Add Query</button></summary>
      <div class="validation-query-content">
        <div class="validation-query-list">${state.queries.map(query => `<article><div><strong>${escapeHtml(query.name)}</strong><code>${escapeHtml(query.role)}</code></div><nav><button type="button" data-query-edit="${escapeHtml(query.id)}" ${state.busy ? "disabled" : ""}>Edit</button><button type="button" data-query-delete="${escapeHtml(query.id)}" ${state.busy ? "disabled" : ""}>Delete</button></nav></article>`).join("")}</div>
        ${state.draft ? `<form data-query-form autocomplete="off"><label>Name<input name="name" value="${escapeHtml(state.draft.name)}" maxlength="160" required ${state.busy ? "disabled" : ""}></label><label>Role<select name="role" required ${state.busy || state.rolesLoading ? "disabled" : ""}><option value="">${state.rolesLoading ? "Loading roles…" : "Select role"}</option>${[...new Set([...state.roles, ...(state.draft.role ? [state.draft.role] : [])])].sort().map(role => `<option value="${escapeHtml(role)}"${role === state.draft.role ? " selected" : ""}>${escapeHtml(role)}</option>`).join("")}</select></label><label class="validation-query-promql" title="Variables: &#36;{DEPLOY_REGION}, &#36;{DEPLOY_HOST_REGEX}, &#36;{EXPECTED_VERSION}">PromQL<textarea name="query" rows="3" spellcheck="false" required ${state.busy ? "disabled" : ""}>${escapeHtml(state.draft.query)}</textarea></label>${state.rolesError ? `<p class="is-error" role="status">${escapeHtml(state.rolesError)}</p>` : ""}<footer><button type="button" data-query-cancel ${state.busy ? "disabled" : ""}>Cancel</button><button type="submit" ${state.busy || state.rolesLoading ? "disabled" : ""}>${state.busy ? "Saving…" : "Save Query"}</button></footer></form>` : ""}
        ${state.message ? `<p class="${state.error ? "is-error" : ""}" role="status">${escapeHtml(state.message)}</p>` : ""}
      </div></details>`;
    root.querySelector("details").addEventListener("toggle", event => { state.open = event.target.open; });
    root.querySelector("[data-query-add]").addEventListener("click", async event => {
      event.preventDefault(); event.stopPropagation?.();
      await openEditor({ id: "", name: "", role: "", query: "", revision: 0 });
      if (alive()) root.querySelector('[name="name"]')?.focus();
    });
    root.querySelectorAll("[data-query-edit]").forEach(button => button.addEventListener("click", () => openEditor({ ...state.queries.find(query => query.id === button.dataset.queryEdit) })));

    root.querySelectorAll("[data-query-delete]").forEach(button => button.addEventListener("click", async () => {
      if (state.busy) return;
      const query = state.queries.find(query => query.id === button.dataset.queryDelete);
      if (!window.confirm(`Delete saved query “${query.name}”? Existing Plans keep their copy.`)) return;
      await action(async () => { await remove(query.id); state.queries = state.queries.filter(item => item.id !== query.id); if (state.draft?.id === query.id) state.draft = null; state.message = "Query deleted. Existing Plans keep their copy."; });
    }));
    const form = root.querySelector("[data-query-form]");
    if (form) {
      form.querySelectorAll("input,select,textarea").forEach(field => field.addEventListener(field.tagName === "SELECT" ? "change" : "input", () => { state.draft[field.name] = field.value; }));
      form.querySelector("[data-query-cancel]").addEventListener("click", () => { state.draft = null; state.message = ""; render(); });
      form.addEventListener("submit", async event => {
        event.preventDefault(); if (state.busy || state.rolesLoading || !form.reportValidity()) return;
        await action(async () => { const saved = await save({ ...state.draft }); state.queries = [...state.queries.filter(query => query.id !== saved.id), saved].sort((a,b) => a.name.localeCompare(b.name)); state.draft = null; state.message = ""; });
      });
    }
  }
  async function openEditor(draft) {
    const request = ++roleRequest;
    state.draft = draft; state.message = ""; state.open = true; state.rolesLoading = true; state.rolesError = ""; render();
    try {
      const roles = await loadRoles();
      if (request !== roleRequest || state.draft !== draft || !alive()) return;
      state.roles = roles;
      if (!roles.length) state.rolesError = "No roles found in the local Ansible folder.";
    } catch (error) {
      if (request !== roleRequest || state.draft !== draft || !alive()) return;
      state.roles = []; state.rolesError = String(error);
    } finally {
      if (request === roleRequest && state.draft === draft && alive()) { state.rolesLoading = false; render(); }
    }
  }
  async function action(run) {
    state.busy = true; state.error = false; state.message = ""; state.open = true; render();
    try { await run(); } catch (error) { state.error = true; state.message = String(error); }
    finally { state.busy = false; if (alive()) render(); }
  }
  render();
}
