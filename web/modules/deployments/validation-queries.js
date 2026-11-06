import { escapeHtml } from "../../shared/ui.js";
import { saveValidationQuery, deleteValidationQuery } from "./deployments-api.js";

export function mountValidationQueries(root, { queries = [], save = saveValidationQuery, remove = deleteValidationQuery } = {}) {
  const state = { queries: [...queries], draft: null, pendingDelete: null, busy: false, message: "", error: false };
  const alive = () => root.isConnected;
  function render() {
    root.innerHTML = `<section class="validation-query-library">
      <header><span>Thanos Queries <small>${state.queries.length}</small></span><button type="button" data-query-add ${state.busy ? "disabled" : ""}>+ Add Query</button></header>
      <div class="validation-query-content">
        <div class="validation-query-list">${state.queries.map(query => `<article><div><strong>${escapeHtml(query.name)}</strong></div><nav><button type="button" data-query-edit="${escapeHtml(query.id)}" ${state.busy ? "disabled" : ""}>Edit</button><button type="button" data-query-delete="${escapeHtml(query.id)}" ${state.busy ? "disabled" : ""}>Delete</button></nav></article>`).join("")}</div>
        ${state.draft ? `<form data-query-form autocomplete="off"><label>Name<input name="name" value="${escapeHtml(state.draft.name)}" maxlength="160" required ${state.busy ? "disabled" : ""}></label><label class="validation-query-promql" title="Variables: &#36;{DEPLOY_REGION}, &#36;{DEPLOY_HOST_REGEX}, &#36;{EXPECTED_VERSION}">PromQL<textarea name="query" rows="9" spellcheck="false" required ${state.busy ? "disabled" : ""}>${escapeHtml(state.draft.query)}</textarea></label><footer><button type="button" data-query-cancel ${state.busy ? "disabled" : ""}>Cancel</button><button type="submit" ${state.busy ? "disabled" : ""}>${state.busy ? "Saving…" : "Save Query"}</button></footer></form>` : ""}
        ${state.message ? `<p class="${state.error ? "is-error" : ""}" role="status">${escapeHtml(state.message)}</p>` : ""}
      </div></section>${state.pendingDelete ? `<dialog class="queue-password-dialog" data-query-delete-dialog aria-labelledby="query-delete-title"><form method="dialog"><strong id="query-delete-title">Are you sure?</strong><p>Delete saved query “${escapeHtml(state.pendingDelete.name)}”?</p><small>Existing Plans keep their copy.</small><div><button value="cancel" autofocus>Cancel</button><button value="delete" class="connection-primary-action">Delete Query</button></div></form></dialog>` : ""}`;
    root.querySelector("[data-query-add]").addEventListener("click", async event => {
      event.preventDefault(); event.stopPropagation?.();
      await openEditor({ id: "", name: "", query: "", revision: 0 });
      if (alive()) root.querySelector('[name="name"]')?.focus();
    });
    root.querySelectorAll("[data-query-edit]").forEach(button => button.addEventListener("click", () => openEditor({ ...state.queries.find(query => query.id === button.dataset.queryEdit) })));

    root.querySelectorAll("[data-query-delete]").forEach(button => button.addEventListener("click", () => {
      if (state.busy) return;
      state.pendingDelete = state.queries.find(query => query.id === button.dataset.queryDelete);
      render();
    }));
    const deleteDialog = root.querySelector("[data-query-delete-dialog]");
    if (deleteDialog) {
      const query = state.pendingDelete;
      deleteDialog.addEventListener("close", async () => {
        state.pendingDelete = null;
        if (!alive()) return;
        if (deleteDialog.returnValue !== "delete") { render(); return; }
        await action(async () => { await remove(query.id); state.queries = state.queries.filter(item => item.id !== query.id); if (state.draft?.id === query.id) state.draft = null; state.message = "Query deleted. Existing Plans keep their copy."; });
      }, { once: true });
      deleteDialog.showModal();
    }
    const form = root.querySelector("[data-query-form]");
    if (form) {
      form.querySelectorAll("input,textarea").forEach(field => field.addEventListener("input", () => { state.draft[field.name] = field.value; }));
      form.querySelector("[data-query-cancel]").addEventListener("click", () => { state.draft = null; state.message = ""; render(); });
      form.addEventListener("submit", async event => {
        event.preventDefault(); if (state.busy || !form.reportValidity()) return;
        await action(async () => { const saved = await save({ ...state.draft }); state.queries = [...state.queries.filter(query => query.id !== saved.id), saved].sort((a,b) => a.name.localeCompare(b.name)); state.draft = null; state.message = ""; });
      });
    }
  }
  function openEditor(draft) {
    const { id, name, query, revision } = draft;
    state.draft = { id, name, query, revision }; state.message = ""; render();
  }
  async function action(run) {
    state.busy = true; state.error = false; state.message = ""; render();
    try { await run(); } catch (error) { state.error = true; state.message = String(error); }
    finally { state.busy = false; if (alive()) render(); }
  }
  render();
}
