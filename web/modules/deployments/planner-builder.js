import { mountPlannerSteps, emptyPlannerStep } from "./planner-steps.js";
import { buildPlannerPlan, plannerPlanYaml } from "./planner-plan.js";
import { createDeploymentPlan } from "./queue-api.js";
import { mountPlannerTargets } from "./planner-targets.js";
import { escapeHtml } from "../../shared/ui.js";
import { getDeploymentConfig } from "./deployments-api.js";
import { getVaults } from "../vaults/vaults-api.js";

export const emptyPlannerDraft = () => ({ planName: "", processId: "", cxNo: "", jiraNo: "", accountId: "", strategy: "linear", steps: [emptyPlannerStep()] });

export function mountPlannerBuilder(root, { draft = emptyPlannerDraft(), onClose = () => {}, loadProcesses = getDeploymentConfig, loadAccounts = getVaults, mountTargets = mountPlannerTargets, mountSteps = mountPlannerSteps, savePlan = createDeploymentPlan, onSaved = () => {} } = {}) {
  let disposed = false, saving = false, committed = false;
  const choices = { processes: [], entries: [] };
  root.innerHTML = `<section class="planner-builder queue-plan-editor" aria-labelledby="planner-builder-title">
    <header class="planner-builder-header">
      <div><p class="planner-builder-eyebrow">New deployment</p><h1 id="planner-builder-title">Planner Builder</h1></div>
      <button type="button" class="planner-builder-close" data-planner-close title="Close Planner Builder" aria-label="Close Planner Builder"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18"/></svg></button>
    </header>
    <div class="planner-builder-body">
      <section class="planner-details-card" aria-label="Plan details">
        <form class="planner-details-form" autocomplete="off">
          <label class="planner-field planner-field-wide"><span>Plan Name</span><input name="planName" data-planner-field="planName" value="${escapeHtml(draft.planName)}" placeholder="Enter a plan name" required /></label>
          <label class="planner-field"><span>Process</span><select name="processId" data-planner-field="processId" required disabled aria-busy="true"><option value="">Loading processes…</option></select><small data-planner-note="processId" role="status"></small></label>
          <label class="planner-field"><span>Account</span><select name="accountId" data-planner-field="accountId" required disabled aria-busy="true"><option value="">Loading accounts…</option></select><small data-planner-note="accountId" role="status"></small></label>
          <label class="planner-field"><span>CX No. <small class="planner-optional">Optional</small></span><input name="cxNo" data-planner-field="cxNo" value="${escapeHtml(draft.cxNo)}" placeholder="000000-000000" pattern="[0-9]{6}-[0-9]{6}" maxlength="13" title="Six digits, a hyphen, then six digits (000000-000000)" aria-describedby="planner-cx-error" spellcheck="false" /><small id="planner-cx-error" class="is-error" aria-live="polite"></small></label>
          <label class="planner-field"><span>Jira No. <small class="planner-optional">Optional</small></span><input name="jiraNo" data-planner-field="jiraNo" value="${escapeHtml(draft.jiraNo)}" placeholder="Jira issue key" spellcheck="false" /></label>
        </form>
      </section>
      <div data-planner-targets></div>
      <div data-planner-steps></div>
    </div>
    <footer class="planner-save-footer"><p data-planner-save-message role="status">Save creates a queued Plan. Start it separately from the Queue.</p><button type="button" data-planner-save>Save Plan</button></footer>
  </section>`;
  const disposeSteps = mountSteps(root.querySelector("[data-planner-steps]"), { draft });
  const targets = mountTargets(root.querySelector("[data-planner-targets]"), { draft });
  const form = root.querySelector("form");
  form.addEventListener("submit", event => event.preventDefault());
  root.querySelector("[data-planner-close]").addEventListener("click", onClose);
  form.querySelectorAll("[data-planner-field]").forEach(field => {
    field.addEventListener(field.tagName === "SELECT" ? "change" : "input", () => { draft[field.dataset.plannerField] = field.value; if (field.name === "processId") targets.processChanged(); });
  });

  const cxField = form.elements.namedItem("cxNo");
  const cxError = form.querySelector("#planner-cx-error");
  let cxTouched = false;
  const validateCx = () => {
    const invalid = cxField.value !== "" && !/^[0-9]{6}-[0-9]{6}$/.test(cxField.value);
    const message = invalid ? "Use six digits, a hyphen, then six digits: 000000-000000." : "";
    cxField.setCustomValidity(message);
    cxField.setAttribute("aria-invalid", String(invalid && cxTouched));
    cxError.textContent = cxTouched ? message : "";
  };
  cxField.addEventListener("input", validateCx);
  cxField.addEventListener("blur", () => { cxTouched = true; validateCx(); });
  validateCx();

  async function loadChoices(fieldName, load, key, label, emptyMessage) {
    const select = form.querySelector(`[data-planner-field="${fieldName}"]`);
    const note = form.querySelector(`[data-planner-note="${fieldName}"]`);
    try {
      const snapshot = await load();
      if (disposed || !root.contains(form)) return;
      choices[key] = snapshot[key];
      const items = [...snapshot[key]].sort((a, b) => a.name.localeCompare(b.name));
      select.innerHTML = `<option value="">${items.length ? label : "None available"}</option>${items.map(item => `<option value="${escapeHtml(item.id)}">${escapeHtml(item.name)}</option>`).join("")}`;
      select.disabled = !items.length;
      if (items.some(item => item.id === draft[fieldName])) select.value = draft[fieldName];
      else draft[fieldName] = "";
      if (fieldName === "processId") targets.processChanged();
      note.textContent = items.length ? "" : emptyMessage;
    } catch (error) {
      if (disposed || !root.contains(form)) return;
      select.innerHTML = '<option value="">Could not load options</option>';
      note.textContent = String(error);
      note.classList.add("is-error");
    } finally {
      if (!disposed && root.contains(form)) select.setAttribute("aria-busy", "false");
    }
  }
  // Independent reads: an unavailable Vault must not hide the Process choices.
  void loadChoices("processId", loadProcesses, "processes", "Select a process", "Add a process under Deployments → Config.");
  void loadChoices("accountId", loadAccounts, "entries", "Select an account", "Add an account reference under Vaults.");
  requestAnimationFrame(() => { if (!disposed && root.contains(form)) form.elements.namedItem("planName").focus(); });
  root.querySelector("[data-planner-save]").addEventListener("click", async () => {
    if (saving || committed) return;
    const message = root.querySelector("[data-planner-save-message]");
    message.classList.remove("is-error");
    if (!form.reportValidity()) { message.textContent = "Complete the required Plan details."; message.classList.add("is-error"); return; }
    let definition;
    try { definition = plannerPlanYaml(buildPlannerPlan(draft, choices.processes, choices.entries)); }
    catch (cause) { message.textContent = cause.message || String(cause); message.classList.add("is-error"); return; }
    saving = true;
    const controls = [...root.querySelectorAll("input,select,textarea,button")].map(control => [control, control.disabled]);
    controls.forEach(([control]) => { control.disabled = true; });
    const button = root.querySelector("[data-planner-save]"); button.textContent = "Saving…"; message.textContent = "Saving Plan to Queue…";
    try {
      const snapshot = await savePlan(definition);
      committed = true;
      if (disposed || !root.contains(form)) return;
      button.textContent = "Saved"; message.textContent = "Plan saved to Queue.";
      onSaved(snapshot);
    } catch (cause) {
      if (!disposed && root.contains(form)) { message.textContent = committed ? "Plan saved. Reopen the Queue to view it." : String(cause); message.classList.add("is-error"); }
    } finally {
      saving = false;
      if (!disposed && root.contains(form)) {
        controls.forEach(([control, disabled]) => { control.disabled = disabled; });
        button.disabled = committed; button.textContent = committed ? "Saved" : "Save Plan";
      }
    }
  });
  return () => { disposed = true; targets.dispose(); disposeSteps(); };
}
