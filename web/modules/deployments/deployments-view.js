import { escapeHtml } from "../../shared/ui.js";
import {
  deleteDeploymentResource,
  getDeploymentConfig,
  saveDeploymentResource,
  validateDeploymentResource,
} from "./deployments-api.js";

export async function mountDeploymentConfigView(root, detail = {}, { onCatalogChange = () => {} } = {}) {
  const state = {
    section: "processes", processes: [], selectedId: null, draft: "", original: "",
    message: "", isError: false, busy: false, focusEditor: Boolean(detail.resourceId || detail.new), onCatalogChange,
  };
  root.innerHTML = '<div class="deployment-config-loading">Loading deployment config…</div>';
  try {
    const snapshot = await getDeploymentConfig();
    state.processes = snapshot.processes;
    selectResource(state, detail.new ? null : resources(state).find((resource) => resource.id === detail.resourceId) ?? resources(state)[0] ?? null);
    notifyCatalog(state);
    render(root, state);
  } catch (cause) {
    root.innerHTML = `<div class="deployment-config-error">${escapeHtml(cause)}</div>`;
  }
}

const resourceType = () => "process";
const resources = (state) => state[state.section];
const selectedResource = (state) => resources(state).find((resource) => resource.id === state.selectedId) ?? null;

function emptyDefinition() {
  return JSON.stringify({
    name: "",
    steps: [],
    details: {},
  }, null, 2);
}

function selectResource(state, resource) {
  state.selectedId = resource?.id ?? null;
  state.draft = resource ? normalizeProcessDefinition(resource.definition) : emptyDefinition();
  state.original = state.draft;
  state.message = "";
  state.isError = false;
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

function render(root, state) {
  const selected = selectedResource(state);
  const dirty = state.draft !== state.original;
  const label = "Config";
  root.innerHTML = `
    <div class="deployment-config-module">
      <section class="deployment-json-workspace">
        <header class="deployment-json-toolbar">
          <div><strong>${escapeHtml(selected?.name || `New ${label}`)}</strong><span>${selected ? `Revision ${selected.revision}` : "Not saved"}${dirty ? " · Unsaved" : ""}</span></div>
          <nav>
            <button class="deployment-delete" data-action="delete" title="Delete" aria-label="Delete" ${selected ? "" : "disabled"}>${icon("trash")}</button>
            <button class="deployment-save" data-action="save" title="Save and validate" aria-label="Save and validate" ${state.busy ? "disabled" : ""}>${icon("save")}</button>
          </nav>
        </header>
        <textarea class="deployment-json-editor" data-role="json-editor" aria-label="${label} JSON" spellcheck="false">${escapeHtml(state.draft)}</textarea>
        <footer class="deployment-json-status"><span class="${state.isError ? "is-error" : ""}">${escapeHtml(state.message)}</span><code>${state.draft.split("\n").length} lines</code></footer>
      </section>
    </div>`;
  bindEvents(root, state);
  if (state.focusEditor) {
    state.focusEditor = false;
    requestAnimationFrame(() => root.querySelector('[data-role="json-editor"]')?.focus());
  }
}

function bindEvents(root, state) {
  const editor = root.querySelector('[data-role="json-editor"]');
  editor.addEventListener("input", () => {
    state.draft = editor.value;
    root.querySelector(".deployment-json-status code").textContent = `${state.draft.split("\n").length} lines`;
    const selected = selectedResource(state);
    root.querySelector(".deployment-json-toolbar > div span").textContent = `${selected ? `Revision ${selected.revision}` : "Not saved"}${state.draft !== state.original ? " · Unsaved" : ""}`;
  });
  editor.addEventListener("keydown", (event) => {
    if (event.key !== "Tab") return;
    event.preventDefault();
    editor.setRangeText("  ", editor.selectionStart, editor.selectionEnd, "end");
    editor.dispatchEvent(new Event("input"));
  });
  root.querySelector('[data-action="save"]').addEventListener("click", async () => {
    syncEditor(editor, state);
    await runAction(root, state, async () => {
      const validation = await validateDeploymentResource(resourceType(state), state.draft, state.selectedId);
      state.draft = validation.formatted;
      const saved = await saveDeploymentResource(resourceType(state), state.draft, state.selectedId);
      const list = resources(state); const index = list.findIndex((resource) => resource.id === saved.id);
      if (index >= 0) list[index] = saved; else list.push(saved);
      list.sort((left, right) => left.name.localeCompare(right.name));
      selectResource(state, saved); state.message = `${saved.name} saved.`; notifyCatalog(state);
    });
  });
  root.querySelector('[data-action="delete"]').addEventListener("click", async () => {
    const selected = selectedResource(state);
    if (!selected || !window.confirm(`Delete ${resourceType(state)} “${selected.name}”?`)) return;
    await runAction(root, state, async () => {
      await deleteDeploymentResource(resourceType(state), selected.id);
      const list = resources(state); list.splice(list.findIndex((resource) => resource.id === selected.id), 1);
      selectResource(state, list[0] ?? null); state.message = `${selected.name} deleted.`; notifyCatalog(state);
    });
  });
}

function syncEditor(editor, state) { state.draft = editor.value; state.message = ""; state.isError = false; }
function notifyCatalog(state) { state.onCatalogChange(state.processes, state.selectedId); }

async function runAction(root, state, action) {
  state.busy = true; state.message = ""; state.isError = false;
  try { await action(); } catch (cause) { state.message = String(cause); state.isError = true; }
  finally { state.busy = false; render(root, state); }
}

function icon(name) {
  const paths = { save: "<path d='M5 4h12l2 2v14H5zM8 4v6h8V4M8 20v-6h8v6'/>", trash: "<path d='M5 7h14M9 7V4h6v3M7.5 7l.7 13h7.6l.7-13M10 11v5M14 11v5'/>" };
  return `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[name]}</svg>`;
}
