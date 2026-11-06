import { escapeHtml } from "../../shared/ui.js";
import { getDeploymentConfig, getLocalDeploymentInventory } from "./deployments-api.js";
import { getVaults } from "../vaults/vaults-api.js";
import { createDeploymentPlanBatch } from "./queue-api.js";
import { mountPlannerTargets } from "./planner-targets.js";
import { plannerPlanYaml } from "./planner-plan.js";
import { emptyMultiRoleDraft, emptyMultiRoleRow, buildMultiRolePlans } from "./multi-role-plan.js";

export function mountMultiRoleBuilder(root, { draft = emptyMultiRoleDraft(), onSaved = () => {}, onSavingChange = () => {}, loadProcesses = getDeploymentConfig, loadAccounts = getVaults, mountTargets = mountPlannerTargets, savePlans = createDeploymentPlanBatch } = {}) {
  let disposed = false, saving = false, committed = false;
  const choices = { processes: [], entries: [], validationQueries: [] }, controllers = new Map(), inventoryReads = new Map();
  const alive = () => !disposed && root.isConnected;
  root.innerHTML = `<div class="planner-builder-body planner-multi-body">
    <section class="planner-details-card" aria-label="Shared multi-role Plan details"><form class="planner-details-form" autocomplete="off">
      <label class="planner-field planner-field-wide"><span>Plan group name</span><input name="planName" data-multi-field="planName" value="${escapeHtml(draft.planName)}" maxlength="160" placeholder="e.g. November updates – ICSTG" required></label>
      <label class="planner-field"><span>Process</span><select name="processId" data-multi-field="processId" required disabled><option value="">Loading processes…</option></select><small data-multi-note="processId" role="status"></small></label>
      <label class="planner-field"><span>Account</span><select name="accountId" data-multi-field="accountId" required disabled><option value="">Loading accounts…</option></select><small data-multi-note="accountId" role="status"></small></label>
      <label class="planner-field"><span>CX No. <small class="planner-optional">Optional</small></span><input name="cxNo" data-multi-field="cxNo" value="${escapeHtml(draft.cxNo)}" placeholder="000000-000000" pattern="[0-9]{6}-[0-9]{6}" maxlength="13"></label>
      <label class="planner-field"><span>Jira No. <small class="planner-optional">Optional</small></span><input name="jiraNo" data-multi-field="jiraNo" value="${escapeHtml(draft.jiraNo)}" placeholder="Jira issue key"></label>
    </form></section>
    <header class="planner-multi-heading"><strong>Roles</strong><nav><label>Parallel regions / Plan<input type="number" data-multi-field="parallelRegions" min="1" max="16" value="${escapeHtml(String(draft.parallelRegions))}" aria-label="Parallel regions per role Plan"></label><select data-multi-field="strategy" aria-label="Execution strategy"><option value="linear"${draft.strategy === "linear" ? " selected" : ""}>Linear</option><option value="free"${draft.strategy === "free" ? " selected" : ""}>Free</option></select><button type="button" data-multi-add>+ Add Role</button></nav></header>
    <div class="planner-multi-rows" data-multi-rows></div>
    </div><footer class="planner-save-footer"><p data-multi-message role="status">Creates a separate queued Plan for each role. Start each Plan from the Queue.</p><button type="button" data-multi-save>Create Plans</button></footer>`;
  const form = root.querySelector("form"), list = root.querySelector("[data-multi-rows]"), message = root.querySelector("[data-multi-message]");
  form.addEventListener("submit", event => event.preventDefault());
  const sharedInventory = processId => {
    if (!inventoryReads.has(processId)) {
      const read = getLocalDeploymentInventory(processId).finally(() => { if (inventoryReads.get(processId) === read) inventoryReads.delete(processId); });
      inventoryReads.set(processId, read);
    }
    return inventoryReads.get(processId);
  };
  function processChanged() {
    for (const row of draft.rows) { row.processId = draft.processId; controllers.get(row.id)?.targets.processChanged(); }
  }
  root.querySelectorAll("[data-multi-field]").forEach(field => field.addEventListener(field.tagName === "SELECT" ? "change" : "input", () => {
    draft[field.dataset.multiField] = field.value;
    if (field.dataset.multiField === "processId") processChanged();
  }));
  function renderQuery(el, row) {
    const host = el.querySelector("[data-multi-query]");
    host.innerHTML = `<label class="planner-saved-query">Thanos query <small class="planner-optional">Optional</small><select data-multi-query-choice aria-label="Saved Thanos query"><option value="">None</option>${choices.validationQueries.map(q => `<option value="${escapeHtml(q.id)}"${q.id === row.validationQueryId ? " selected" : ""}>${escapeHtml(q.name)}</option>`).join("")}</select></label>
      ${row.query ? `<textarea data-step-query-preview rows="5" aria-label="Saved PromQL query" readonly spellcheck="false">${escapeHtml(row.query)}</textarea>` : ""}
      ${row.query.includes("${EXPECTED_VERSION}") ? `<label>Expected exporter version<input data-multi-expected value="${escapeHtml(row.expectedVersion)}" placeholder="e.g. 1.97.0"></label>` : ""}`;
    host.querySelector("[data-multi-query-choice]").addEventListener("change", event => {
      row.validationQueryId = event.target.value;
      row.query = choices.validationQueries.find(q => q.id === row.validationQueryId)?.query ?? "";
      renderQuery(el, row);
    });
    host.querySelector("[data-multi-expected]")?.addEventListener("input", event => { row.expectedVersion = event.target.value; });
  }
  function addRow(row) {
    const el = document.createElement("article"); el.className = "planner-details-card planner-multi-role"; el.dataset.multiRoleId = row.id;
    el.innerHTML = `<header class="planner-details-heading"><h2>Role</h2><button type="button" data-multi-remove>Remove</button></header><div data-multi-targets></div><div class="planner-step planner-multi-vars"><label>Extra variables <small class="planner-optional">Optional</small><textarea data-step-vars aria-label="Role extra variables" placeholder="VARIABLE: value" spellcheck="false">${escapeHtml(row.extraVarsText)}</textarea></label><div data-multi-query></div></div>`;
    list.append(el); row.processId = draft.processId;
    const targets = mountTargets(el.querySelector("[data-multi-targets]"), { draft: row, loadInventory: sharedInventory });
    controllers.set(row.id, { targets, el }); renderQuery(el, row);
    el.querySelector("[data-step-vars]").addEventListener("input", event => { row.extraVarsText = event.target.value; });
    el.querySelector("[data-multi-remove]").addEventListener("click", () => {
      if (saving || draft.rows.length === 1) return;
      targets.dispose(); controllers.delete(row.id); draft.rows = draft.rows.filter(item => item.id !== row.id); el.remove(); updateRemoveButtons();
    });
    updateRemoveButtons();
  }
  function updateRemoveButtons() { for (const { el } of controllers.values()) el.querySelector("[data-multi-remove]").disabled = draft.rows.length === 1; }
  draft.rows.forEach(addRow);
  root.querySelector("[data-multi-add]").addEventListener("click", () => {
    if (saving || draft.rows.length >= 100) return;
    const row = emptyMultiRoleRow(draft.processId); draft.rows.push(row); addRow(row);
  });
  async function load(fieldName, loader, key) {
    const select = root.querySelector(`[data-multi-field="${fieldName}"]`), note = root.querySelector(`[data-multi-note="${fieldName}"]`);
    try {
      const data = await loader(); if (!alive()) return;
      choices[key] = data[key];
      select.innerHTML = `<option value="">Select ${fieldName === "processId" ? "a process" : "an account"}</option>${[...choices[key]].sort((a,b) => a.name.localeCompare(b.name)).map(item => `<option value="${escapeHtml(item.id)}">${escapeHtml(item.name)}</option>`).join("")}`;
      if (!choices[key].some(item => item.id === draft[fieldName])) draft[fieldName] = "";
      select.value = draft[fieldName]; select.disabled = !choices[key].length || saving;
      if (!choices[key].length) note.textContent = fieldName === "processId" ? "Add a Process under Deployments → Config → Deploy." : "Add an account under Vaults.";
      if (fieldName === "processId") {
        choices.validationQueries = data.validationQueries ?? [];
        for (const row of draft.rows) renderQuery(controllers.get(row.id).el, row);
        processChanged();
      }
    } catch (error) { if (alive()) { note.textContent = String(error); note.classList.add("is-error"); } }
  }
  void Promise.allSettled([load("processId",loadProcesses,"processes"),load("accountId",loadAccounts,"entries")]);
  root.querySelector("[data-multi-save]").addEventListener("click", async () => {
    if (saving || committed) return;
    message.classList.remove("is-error");
    if (!form.reportValidity()) { message.textContent = "Complete the shared Plan details."; message.classList.add("is-error"); return; }
    let definitions;
    try { definitions = buildMultiRolePlans(draft,choices.processes,choices.entries,choices.validationQueries).map(plannerPlanYaml); }
    catch (error) { message.textContent = error.message; message.classList.add("is-error"); return; }
    saving = true; onSavingChange(true);
    const controls = [...root.querySelectorAll("input,select,textarea,button")].map(control => [control,control.disabled]); controls.forEach(([control]) => { control.disabled = true; });
    message.textContent = `Creating ${definitions.length} Plans…`;
    try {
      const snapshot = await savePlans(draft.groupId,draft.planName.trim(),definitions); committed = true;
      if (alive()) onSaved(snapshot);
    } catch (error) { if (alive()) { message.textContent = committed ? "Plans saved. Reopen the Queue to view them." : String(error); message.classList.add("is-error"); } }
    finally {
      saving = false; onSavingChange(false);
      if (alive()) { controls.forEach(([control,disabled]) => { control.disabled = disabled; }); root.querySelector("[data-multi-save]").disabled = committed; }
    }
  });
  return () => { disposed = true; for (const controller of controllers.values()) controller.targets.dispose(); controllers.clear(); };
}
