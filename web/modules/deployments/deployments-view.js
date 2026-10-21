import { mountLocalAnsibleSource } from "./local-ansible-source.js";
import { showDeploymentHelp } from "../help/help-view.js";
import { escapeHtml } from "../../shared/ui.js";
import { deleteDeploymentResource, getDeploymentConfig, saveDeploymentResource, validateDeploymentResource } from "./deployments-api.js";

const NEW_CONFIG = "__new_process__";

export async function mountDeploymentConfigView(root, detail = {}, { onCatalogChange = () => {} } = {}) {
  const state = { processes: [], editors: new Map(), expanded: new Set(), selectedId: detail.resourceId ?? null, busy: null, onCatalogChange };
  const mountId = crypto.randomUUID(); root.dataset.configMountId = mountId;
  const alive = () => root.isConnected && root.dataset.configMountId === mountId;
  root.innerHTML = `<div class="deployment-config-module deployment-config-accordion"><div data-local-ansible-settings></div><section class="deployment-config-list" data-config-list><p class="deployment-config-loading">Loading deployment config…</p></section></div>`;
  mountLocalAnsibleSource(root.querySelector("[data-local-ansible-settings]"));
  try {
    state.processes = (await getDeploymentConfig()).processes;
    if (!alive()) return;
    if (detail.resourceId && state.processes.some(p => p.id === detail.resourceId)) state.expanded.add(detail.resourceId);
    if (detail.new) { editorState(state, NEW_CONFIG); state.expanded.add(NEW_CONFIG); }
    notifyCatalog(state); renderList(root, state);
    if (detail.new) focusEditor(root, NEW_CONFIG);
  } catch (cause) {
    if (alive()) root.querySelector("[data-config-list]").innerHTML = `<p class="deployment-config-error">${escapeHtml(cause)}</p>`;
  }
}

function editorState(state, id) {
  if (!state.editors.has(id)) {
    const resource = state.processes.find(p => p.id === id);
    const definition = resource ? normalizeProcessDefinition(resource.definition) : JSON.stringify({ name: "", steps: [], details: {} }, null, 2);
    state.editors.set(id, { draft: definition, original: definition, message: "", isError: false });
  }
  return state.editors.get(id);
}
function resourceMarkup(resource, state) {
  const id = resource?.id ?? NEW_CONFIG; const edit = editorState(state, id); const expanded = state.expanded.has(id);
  const revision = resource ? `Revision ${resource.revision}` : "Not saved";
  return `<article class="deployment-config-group" data-config-id="${escapeHtml(id)}">
    <header class="deployment-json-toolbar">
      <button type="button" class="deployment-config-toggle" data-toggle-config="${escapeHtml(id)}" aria-expanded="${expanded}" aria-controls="config-body-${escapeHtml(id)}">${icon("chevron")}<span><strong>${escapeHtml(resource?.name || "New Config")}</strong><small data-config-revision>${revision}${edit.draft !== edit.original ? " · Unsaved" : ""}</small></span></button>
      <nav><button type="button" class="deployment-delete" data-delete-config="${escapeHtml(id)}" title="${resource ? "Delete Config" : "Discard new Config"}" aria-label="${resource ? "Delete Config" : "Discard new Config"}" ${state.busy ? "disabled" : ""}>${icon("trash")}</button><button type="button" class="deployment-save" data-save-config="${escapeHtml(id)}" title="Save and validate Config" aria-label="Save and validate Config" ${state.busy ? "disabled" : ""}>${icon("save")}</button></nav>
    </header>
    <div class="deployment-config-body" id="config-body-${escapeHtml(id)}" ${expanded ? "" : "hidden"}>
      <textarea class="deployment-json-editor" data-config-editor="${escapeHtml(id)}" aria-label="${escapeHtml(resource?.name || "New Config")} JSON" spellcheck="false">${escapeHtml(edit.draft)}</textarea>
      <footer class="deployment-json-status"><span class="${edit.isError ? "is-error" : ""}" role="status">${escapeHtml(edit.message)}</span><code>${edit.draft.split("\n").length} lines</code></footer>
    </div>
  </article>`;
}
function renderList(root, state) {
  const list = root.querySelector("[data-config-list]"); const scrollTop = list.scrollTop;
  list.innerHTML = `<header class="deployment-config-list-heading"><strong>Config</strong><nav><button type="button" data-config-help title="Process Config help (F1)" aria-label="Process Config help">${icon("help")}</button><button type="button" data-add-config title="Add Config" aria-label="Add Config" ${state.busy ? "disabled" : ""}>${icon("plus")}</button></nav></header><div class="deployment-config-groups">${state.processes.map(p => resourceMarkup(p, state)).join("")}${state.editors.has(NEW_CONFIG) ? resourceMarkup(null, state) : ""}${!state.processes.length && !state.editors.has(NEW_CONFIG) ? '<p class="deployment-config-empty">No Process configs yet. Use + to add one.</p>' : ""}</div>`;
  bindEvents(root, state); list.scrollTop = scrollTop;
}
function focusEditor(root, id) { requestAnimationFrame(() => root.querySelector(`[data-config-editor="${CSS.escape(id)}"]`)?.focus()); }
function setExpanded(group, state, id, expanded) {
  if (expanded) state.expanded.add(id); else state.expanded.delete(id);
  group.querySelector("[data-toggle-config]").setAttribute("aria-expanded", String(expanded));
  group.querySelector(".deployment-config-body").hidden = !expanded;
}
function bindEvents(root, state) {
  const list = root.querySelector("[data-config-list]");
  list.querySelector("[data-config-help]").addEventListener("click", showDeploymentHelp);
  list.querySelector("[data-add-config]").addEventListener("click", () => { editorState(state, NEW_CONFIG); state.expanded.add(NEW_CONFIG); renderList(root, state); focusEditor(root, NEW_CONFIG); });
  list.querySelectorAll("[data-toggle-config]").forEach(button => button.addEventListener("click", () => {
    const id = button.dataset.toggleConfig;
    setExpanded(button.closest("[data-config-id]"), state, id, !state.expanded.has(id));
  }));
  list.querySelectorAll("[data-config-editor]").forEach(editor => {
    editor.addEventListener("input", () => {
      const id = editor.dataset.configEditor; const edit = editorState(state, id); edit.draft = editor.value;
      const group = editor.closest("[data-config-id]"); const resource = state.processes.find(p => p.id === id);
      group.querySelector("code").textContent = `${edit.draft.split("\n").length} lines`;
      group.querySelector("[data-config-revision]").textContent = `${resource ? `Revision ${resource.revision}` : "Not saved"}${edit.draft !== edit.original ? " · Unsaved" : ""}`;
    });
    editor.addEventListener("keydown", event => {
      if (event.key === "F1") { event.preventDefault(); showDeploymentHelp(); return; }
      if (event.key !== "Tab" || editor.readOnly) return;
      event.preventDefault(); editor.setRangeText("  ", editor.selectionStart, editor.selectionEnd, "end"); editor.dispatchEvent(new Event("input"));
    });
  });
  list.querySelectorAll("[data-save-config]").forEach(button => button.addEventListener("click", async () => {
    if (state.busy) return;
    const id = button.dataset.saveConfig; const edit = editorState(state, id); const replacingId = id === NEW_CONFIG ? null : id;
    state.expanded.add(id);
    await runAction(root, state, id, async () => {
      const validation = await validateDeploymentResource("process", edit.draft, replacingId);
      const saved = await saveDeploymentResource("process", validation.formatted, replacingId);
      const index = state.processes.findIndex(p => p.id === saved.id);
      if (index >= 0) state.processes[index] = saved; else state.processes.push(saved);
      state.processes.sort((a,b) => a.name.localeCompare(b.name));
      state.editors.delete(id); state.expanded.delete(id); state.expanded.add(saved.id);
      editorState(state, saved.id).message = `${saved.name} saved.`; state.selectedId = saved.id; notifyCatalog(state);
    });
  }));
  list.querySelectorAll("[data-delete-config]").forEach(button => button.addEventListener("click", async () => {
    if (state.busy) return;
    const id = button.dataset.deleteConfig; const resource = state.processes.find(p => p.id === id);
    if (resource && !window.confirm(`Delete process “${resource.name}”?`)) return;
    await runAction(root, state, id, async () => {
      if (resource) await deleteDeploymentResource("process", id);
      state.processes = state.processes.filter(p => p.id !== id); state.editors.delete(id); state.expanded.delete(id);
      if (state.selectedId === id) state.selectedId = null; notifyCatalog(state);
    });
  }));
}
function notifyCatalog(state) { state.onCatalogChange(state.processes, state.selectedId); }
async function runAction(root, state, id, action) {
  const list = root.querySelector("[data-config-list]"); const edit = editorState(state, id);
  state.busy = id; edit.message = ""; edit.isError = false;
  list.querySelectorAll("[data-save-config], [data-delete-config], [data-add-config]").forEach(button => { button.disabled = true; });
  const editor = list.querySelector(`[data-config-editor="${CSS.escape(id)}"]`); if (editor) editor.readOnly = true;
  try { await action(); } catch (cause) { edit.message = String(cause); edit.isError = true; }
  finally { state.busy = null; if (list.isConnected && root.contains(list)) renderList(root, state); }
}

function normalizeProcessDefinition(definition) {
  try {
    const process = JSON.parse(definition);
    if (!process.drivers || process.details) return JSON.stringify(process, null, 2);
    const steps = [];
    const details = {};
    for (const [driver, instance] of Object.entries(process.drivers)) {
      const oldDetails = instance?.details ?? {};
      details[driver] = {
        ...oldDetails,
        url: oldDetails.url ?? oldDetails.base_url ?? "",
        account: oldDetails.account ?? oldDetails.credential ?? oldDetails.authentication?.credential ?? "",
      };
      for (const configuredAction of instance?.actions ?? []) {
        const action = configuredAction?.action ?? "";
        const sourceName = configuredAction?.name ?? configuredAction?.id ?? action;
        steps.push({ name: readableStepName(sourceName), driver, action: normalizeActionName(action) });
      }
    }
    return JSON.stringify({ name: process.name ?? "", steps, details }, null, 2);
  } catch {
    return definition;
  }
}

function readableStepName(value) {
  return String(value).replace(/[-_]+/g, " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function normalizeActionName(action) {
  if (["validate_issue", "get_issue"].includes(action)) return "validate";
  if (["add_comment", "add_deployment_started_comment", "add_deployment_completed_comment", "add_deployment_failed_comment"].includes(action)) return "update";
  return action;
}

function icon(name) {
  const paths = { plus: "<path d='M12 5v14M5 12h14'/>", chevron: "<path d='m9 5 7 7-7 7'/>", help: "<circle cx='12' cy='12' r='9'/><path d='M9.8 9.5a2.3 2.3 0 1 1 3.2 2.1c-.7.3-1 1-1 1.7M12 16.7h.01'/>", save: "<path d='M5 4h12l2 2v14H5zM8 4v6h8V4M8 20v-6h8v6'/>", trash: "<path d='M5 7h14M9 7V4h6v3M7.5 7l.7 13h7.6l.7-13M10 11v5M14 11v5'/>" };
  return `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[name]}</svg>`;
}
