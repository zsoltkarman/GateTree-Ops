import { escapeHtml } from "../../shared/ui.js";

export const emptyPlannerStep = () => ({ id: crypto.randomUUID(), name: "", extraVarsText: "" });
export function mountPlannerSteps(root, { draft, onChange = () => {} } = {}) {
  draft.strategy ||= "linear";
  if (!draft.steps?.length) draft.steps = [emptyPlannerStep()];
  for (const step of draft.steps) {
    step.extraVarsText ??= (step.variables ?? []).map(variable => `${variable.key}: ${JSON.stringify(String(variable.value ?? ""))}`).join("\n");
  }
  function render(focus) {
    root.innerHTML = `<section class="planner-details-card planner-steps-card">
      <header class="planner-details-heading"><h2>Steps</h2><nav><select data-planner-strategy aria-label="Execution strategy" title="Execution strategy"><option value="linear" ${draft.strategy === "linear" ? "selected" : ""}>Linear</option><option value="free" ${draft.strategy === "free" ? "selected" : ""}>Free</option></select><button type="button" data-add-step>+ Add Step</button></nav></header>

      <div class="planner-step-list">${draft.steps.map((step, index) => `<article class="planner-step" data-planner-step="${escapeHtml(step.id)}">
        <header><strong>${index + 1}.</strong><input data-step-name aria-label="Step ${index + 1} name" value="${escapeHtml(step.name)}" placeholder="Step name" required><nav>
          <button type="button" data-step-up title="Move step up" aria-label="Move step ${index + 1} up" ${index === 0 ? "disabled" : ""}>${icon("up")}</button>
          <button type="button" data-step-down title="Move step down" aria-label="Move step ${index + 1} down" ${index === draft.steps.length - 1 ? "disabled" : ""}>${icon("down")}</button>
          <button type="button" data-step-remove title="Remove step" aria-label="Remove step ${index + 1}" ${draft.steps.length === 1 ? "disabled" : ""}>${icon("remove")}</button>
        </nav></header>
        <textarea data-step-vars rows="3" aria-label="Step ${index + 1} extra variables" title="Optional extra variables: NAME: value or NAME=value, one per line" placeholder="VARIABLE: value&#10;OTHER_VARIABLE: value" spellcheck="false">${escapeHtml(step.extraVarsText)}</textarea>
      </article>`).join("")}</div></section>`;
    bind();
    if (focus) root.querySelector(focus)?.focus();
  }
  function bind() {
    root.querySelector("[data-add-step]").addEventListener("click", () => { const step = emptyPlannerStep(); draft.steps.push(step); render(`[data-planner-step="${CSS.escape(step.id)}"] [data-step-name]`); onChange(); });
    root.querySelector("[data-planner-strategy]").addEventListener("change", event => { draft.strategy = event.target.value; onChange(); });
    root.querySelectorAll("[data-planner-step]").forEach(el => {
      const index = draft.steps.findIndex(step => step.id === el.dataset.plannerStep); const step = draft.steps[index];
      el.querySelector("[data-step-name]").addEventListener("input", event => { step.name = event.target.value; onChange(); });
      function move(offset) { const next = index + offset; if (next < 0 || next >= draft.steps.length) return; [draft.steps[index], draft.steps[next]] = [draft.steps[next], draft.steps[index]]; render(`[data-planner-step="${CSS.escape(step.id)}"] [data-step-name]`); onChange(); }
      el.querySelector("[data-step-up]").addEventListener("click", () => move(-1));
      el.querySelector("[data-step-down]").addEventListener("click", () => move(1));
      el.querySelector("[data-step-remove]").addEventListener("click", () => { if (draft.steps.length <= 1) return; draft.steps.splice(index, 1); render(); onChange(); });
      const vars = el.querySelector("[data-step-vars]");
      vars.addEventListener("input", () => { step.extraVarsText = vars.value; onChange(); });
      vars.addEventListener("keydown", event => {
        if (event.key !== "Tab") return;
        event.preventDefault(); vars.setRangeText("  ", vars.selectionStart, vars.selectionEnd, "end");
        step.extraVarsText = vars.value; onChange();
      });
    });
  }
  render();
  return () => { root.replaceChildren(); };
}
function icon(name) {
  const path = { up: "m6 14 6-6 6 6", down: "m6 10 6 6 6-6", remove: "m6 6 12 12M18 6 6 18" };
  return `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="${path[name]}"/></svg>`;
}
