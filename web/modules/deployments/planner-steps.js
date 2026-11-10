import { escapeHtml } from "../../shared/ui.js";

export const emptyPlannerStep = () => ({ id: crypto.randomUUID(), name: "", type: "deploy", query: "", expectedVersion: "", extraVarsText: "" });
export function mountPlannerSteps(root, { draft, onChange = () => {}, validationQueries = () => [] } = {}) {
  draft.strategy ||= "linear";
  draft.parallelRegions ??= 1;
  if (!draft.steps?.length) draft.steps = [emptyPlannerStep()];
  for (const step of draft.steps) {
    step.type ||= "deploy"; step.query ??= ""; step.expectedVersion ??= step.expected_version ?? "";
    step.extraVarsText ??= (step.variables ?? []).map(variable => `${variable.key}: ${JSON.stringify(String(variable.value ?? ""))}`).join("\n");
  }
  function render(focus) {
    const queries = validationQueries();
    for (const step of draft.steps) {
      if (step.type === "thanos_query" && step.validationQueryId) {
        const current = queries.find(query => query.id === step.validationQueryId);
        step.query = current?.query ?? "";
      }
      if (step.type === "thanos_query" && !step.validationQueryId && step.query) {
        step.validationQueryId = queries.find(query => query.query === step.query)?.id ?? "";
      }
    }
    root.innerHTML = `<section class="planner-details-card planner-steps-card">
      <header class="planner-details-heading"><h2>Steps</h2><nav><label class="planner-parallel-regions">Parallel regions<input type="number" min="1" max="16" step="1" data-parallel-regions aria-label="Parallel regions" title="Maximum regions running together. Deploy and Thanos stay sequential within each region. Requires Separate job per host." value="${escapeHtml(String(draft.parallelRegions))}"></label><select data-planner-strategy aria-label="Execution strategy" title="Execution strategy"><option value="linear" ${draft.strategy === "linear" ? "selected" : ""}>Linear</option><option value="free" ${draft.strategy === "free" ? "selected" : ""}>Free</option></select><button type="button" data-add-step>+ Add Step</button></nav></header>

      <div class="planner-step-list">${draft.steps.map((step, index) => `<article class="planner-step" data-planner-step="${escapeHtml(step.id)}">
        <header><strong>${index + 1}.</strong><select data-step-type aria-label="Step ${index + 1} type"><option value="deploy"${step.type === "deploy" ? " selected" : ""}>Deploy</option><option value="thanos_query"${step.type === "thanos_query" ? " selected" : ""}>Thanos Query</option></select><input data-step-name aria-label="Step ${index + 1} name" value="${escapeHtml(step.name)}" placeholder="Step name" required><nav>
          <button type="button" data-step-up title="Move step up" aria-label="Move step ${index + 1} up" ${index === 0 ? "disabled" : ""}>${icon("up")}</button>
          <button type="button" data-step-down title="Move step down" aria-label="Move step ${index + 1} down" ${index === draft.steps.length - 1 ? "disabled" : ""}>${icon("down")}</button>
          <button type="button" data-step-remove title="Remove step" aria-label="Remove step ${index + 1}" ${draft.steps.length === 1 ? "disabled" : ""}>${icon("remove")}</button>
        </nav></header>
        ${step.type === "thanos_query" ? `<label class="planner-saved-query">Saved query<select data-step-saved-query aria-label="Step ${index + 1} saved validation query"><option value="" disabled${!queries.some(query => query.id === step.validationQueryId) ? " selected" : ""}>${queries.length ? "Select a saved query" : "Add queries in Config → Thanos Queries"}</option>${queries.map(query => `<option value="${escapeHtml(query.id)}"${step.validationQueryId === query.id ? " selected" : ""}>${escapeHtml(query.name)}</option>`).join("")}</select></label>${step.query.includes("${EXPECTED_VERSION}") ? `<label>Expected exporter version<input data-step-expected-version aria-label="Step ${index + 1} expected exporter version" value="${escapeHtml(step.expectedVersion)}" placeholder="e.g. 1.97.0"></label>` : ""}${step.query ? `<textarea data-step-query-preview rows="5" aria-label="Step ${index + 1} saved PromQL query" readonly spellcheck="false">${escapeHtml(step.query)}</textarea>` : ""}<p class="planner-query-hint">Manage queries in Deployments → Config → Thanos Queries.</p>` : `<textarea data-step-vars rows="3" aria-label="Step ${index + 1} extra variables" title="Optional extra variables: NAME: value or NAME=value, one per line" placeholder="VARIABLE: value&#10;OTHER_VARIABLE: value" spellcheck="false">${escapeHtml(step.extraVarsText)}</textarea>`}
      </article>`).join("")}</div></section>`;
    bind();
    if (focus) root.querySelector(focus)?.focus();
  }
  function bind() {
    root.querySelector("[data-parallel-regions]").addEventListener("input", event => { draft.parallelRegions = event.target.value; onChange(); });
    root.querySelector("[data-add-step]").addEventListener("click", () => { const step = emptyPlannerStep(); draft.steps.push(step); render(`[data-planner-step="${CSS.escape(step.id)}"] [data-step-name]`); onChange(); });
    root.querySelector("[data-planner-strategy]").addEventListener("change", event => { draft.strategy = event.target.value; onChange(); });
    root.querySelectorAll("[data-planner-step]").forEach(el => {
      const index = draft.steps.findIndex(step => step.id === el.dataset.plannerStep); const step = draft.steps[index];
      el.querySelector("[data-step-name]").addEventListener("input", event => { step.name = event.target.value; onChange(); });
      function move(offset) { const next = index + offset; if (next < 0 || next >= draft.steps.length) return; [draft.steps[index], draft.steps[next]] = [draft.steps[next], draft.steps[index]]; render(`[data-planner-step="${CSS.escape(step.id)}"] [data-step-name]`); onChange(); }
      el.querySelector("[data-step-up]").addEventListener("click", () => move(-1));
      el.querySelector("[data-step-down]").addEventListener("click", () => move(1));
      el.querySelector("[data-step-remove]").addEventListener("click", () => { if (draft.steps.length <= 1) return; draft.steps.splice(index, 1); render(); onChange(); });
      el.querySelector("[data-step-type]").addEventListener("change", event => {
        step.type = event.target.value; render(`[data-planner-step="${CSS.escape(step.id)}"] [data-step-name]`); onChange();
      });
      const saved = el.querySelector("[data-step-saved-query]");
      if (saved) saved.addEventListener("change", () => {
        step.validationQueryId = saved.value;
        const selected = validationQueries().find(query => query.id === saved.value);
        step.query = selected?.query ?? "";
        if (selected && !step.name.trim()) step.name = selected.name;
        render(); onChange();
      });
      const expected = el.querySelector("[data-step-expected-version]");
      if (expected) expected.addEventListener("input", () => { step.expectedVersion = expected.value; onChange(); });
      const vars = el.querySelector("[data-step-vars]");
      if (!vars) return;
      vars.addEventListener("input", () => { step.extraVarsText = vars.value; onChange(); });
      vars.addEventListener("keydown", event => {
        if (event.key !== "Tab") return;
        event.preventDefault(); vars.setRangeText("  ", vars.selectionStart, vars.selectionEnd, "end");
        step.extraVarsText = vars.value; onChange();
      });
    });
  }
  render();
  const dispose = () => { root.replaceChildren(); };
  dispose.refresh = () => render();
  return dispose;
}
function icon(name) {
  const path = { up: "m6 14 6-6 6 6", down: "m6 10 6 6 6-6", remove: "m6 6 12 12M18 6 6 18" };
  return `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="${path[name]}"/></svg>`;
}
