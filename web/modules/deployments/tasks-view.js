import { escapeHtml as escapeText } from "../../shared/ui.js";
import { getDeploymentQueue } from "./queue-api.js";
import { getDeploymentTasks, saveDeploymentTask, deleteDeploymentTask, linkDeploymentTaskPlan } from "./tasks-api.js";
import { taskProgress, applyTaskItem } from "./tasks-model.js";
import { emptyPlannerDraft } from "./planner-builder.js";
import { getLocalDeploymentRoles } from "./deployments-api.js";

const escapeHtml = value => escapeText(value).replaceAll('"', "&quot;").replaceAll("'", "&#39;");

export async function mountDeploymentTasksView(root, detail = {}, { onOpenPlan = () => {}, onCreatePlan = () => {}, onQueueChange = () => {} } = {}) {
  const host = document.createElement("div");
  root.replaceChildren(host);
  root = host;
  const mountId = crypto.randomUUID();
  root.dataset.tasksMountId = mountId;
  let tasks = [], plans = [], selectedId = detail.taskId ?? null, editing = detail.new ? emptyTask() : null;
  let busy = false, message = "", error = false, deleteId = null;
  let localRoles = [], rolesLoading = false, rolesError = "";
  const linkChoices = new Map(), linkErrors = new Map();
  const alive = () => root.isConnected && root.dataset.tasksMountId === mountId;
  root.innerHTML = '<div class="deployment-config-loading">Loading Tasks…</div>';

  async function refresh() {
    const [nextTasks, queue] = await Promise.all([getDeploymentTasks(), getDeploymentQueue({ includeLogs: false })]);
    if (!alive()) return;
    tasks = nextTasks; plans = queue.plans;
    onQueueChange(plans);
    if (selectedId && !tasks.some(task => task.id === selectedId)) selectedId = null;
  }
  async function editTask(task) {
    editing = task; message = ""; rolesLoading = true; rolesError = ""; render();
    try {
      const snapshot = await getLocalDeploymentRoles();
      if (!alive() || editing !== task) return;
      localRoles = snapshot;
    } catch (cause) { if (alive() && editing === task) { localRoles = []; rolesError = String(cause); } }
    finally { if (alive() && editing === task) { rolesLoading = false; render(); } }
  }
  async function mutate(action, success, onError = () => {}) {
    if (busy) return;
    busy = true; message = ""; error = false; render();
    try {
      await action();
      if (!alive()) return;
      editing = null; deleteId = null; message = success;
      await refresh();
    } catch (cause) { if (alive()) { message = String(cause); error = true; onError(cause); } }
    finally { busy = false; if (alive()) render(); }
  }
  function render() {
    const selected = tasks.find(task => task.id === selectedId);
    root.innerHTML = `<section class="deployment-tasks">
      <header class="task-toolbar"><div><h1>Tasks</h1><p>Track service deployments and validation by Jira.</p></div><nav><button data-task-refresh ${busy || editing ? "disabled" : ""}>Refresh</button><button data-task-new ${busy || editing ? "disabled" : ""}>+ New Task</button></nav></header>
      <p class="queue-message${error ? " is-error" : ""}" role="status">${escapeHtml(message)}</p>
      ${editing ? editorMarkup(editing, busy, localRoles, rolesLoading, rolesError) : tasks.length ? `<div class="task-table-scroll"><table class="task-table task-overview"><thead><tr><th>Task</th><th>Services</th><th>Planned</th><th>Deployed</th><th>Validated</th><th>Ready</th></tr></thead><tbody>${tasks.map(task => {
        const progress = taskProgress(task, plans);
        const expanded = task.id === selectedId;
        return `<tr class="task-overview-row"><td><button data-task-select="${escapeHtml(task.id)}" aria-expanded="${expanded}" aria-controls="task-detail-${escapeHtml(task.id)}"><span aria-hidden="true">${expanded ? "▾" : "▸"}</span><strong>${escapeHtml(task.jiraKey)}</strong><span>${escapeHtml(task.name)}</span>${progress.attention ? `<small class="is-error">${progress.attention} need attention</small>` : ""}</button></td><td>${progress.total}</td><td>${progress.planned}</td><td>${progress.deployed}</td><td>${progress.validated}</td><td>${progress.ready}/${progress.total}</td></tr>${expanded ? `<tr id="task-detail-${escapeHtml(task.id)}"><td colspan="6" class="task-expanded">${taskMarkup(task, plans, busy, deleteId === task.id, linkChoices, linkErrors)}</td></tr>` : ""}`;
      }).join("")}</tbody></table></div>` : '<div class="queue-empty">No Tasks yet. Create a Task and add the roles you want to deploy.</div>'}
    </section>`;
    root.querySelector("[data-task-new]").onclick = () => { void editTask(emptyTask()); };
    root.querySelector("[data-task-refresh]").onclick = () => { void mutate(async () => { await refresh(); }, "Tasks refreshed."); };
    root.querySelectorAll("[data-task-select]").forEach(button => { button.onclick = () => { selectedId = selectedId === button.dataset.taskSelect ? null : button.dataset.taskSelect; deleteId = null; render(); }; });
    if (editing) bindEditor();
    else if (selected) bindTask(selected);
  }
  function bindEditor() {
    const form = root.querySelector("[data-task-form]");
    form.oninput = event => {
      if (event.target.name === "jiraKey" || event.target.name === "name") editing[event.target.name] = event.target.value;
    };
    form.onchange = event => {
      const row = event.target.closest("[data-service-index]");
      if (!row || event.target.name !== "role") return;
      const item = editing.items[Number(row.dataset.serviceIndex)];
      if (item.planId || item.role === event.target.value) return;
      item.role = event.target.value;
      render();
    };
    form.onsubmit = event => {
      event.preventDefault();
      if (!form.reportValidity()) return;
      void mutate(async () => {
        selectedId = await saveDeploymentTask(editing);
      }, "Task saved.");
    };
    root.querySelector("[data-task-cancel]").onclick = () => { editing = null; render(); };
    root.querySelector("[data-service-add]").onclick = () => { editing.items.push(emptyItem()); render(); root.querySelector('[data-service-index]:last-child input')?.focus(); };
    root.querySelectorAll("[data-service-remove]").forEach(button => { button.onclick = () => { editing.items.splice(Number(button.dataset.serviceRemove), 1); render(); }; });
  }
  function bindTask(task) {
    root.querySelector("[data-task-edit]").onclick = () => { void editTask(structuredClone(task)); };
    root.querySelector("[data-task-delete]").onclick = () => {
      if (deleteId !== task.id) { deleteId = task.id; render(); return; }
      void mutate(() => deleteDeploymentTask(task.id, task.revision), "Task deleted. Queue Plans were kept.");
    };
    root.querySelectorAll("[data-task-plan]").forEach(button => { button.onclick = () => onOpenPlan(button.dataset.taskPlan); });
    root.querySelectorAll("[data-task-create]").forEach(button => { button.onclick = () => {
      const item = task.items.find(item => item.id === button.dataset.taskCreate);
      const draft = emptyPlannerDraft(); applyTaskItem(draft, task, item); onCreatePlan(draft);
    }; });
    root.querySelectorAll("[data-task-link]").forEach(button => { button.onclick = () => {
      const itemId = button.dataset.taskLink;
      const select = root.querySelector(`[data-task-link-choice="${CSS.escape(itemId)}"]`);
      const planId = select.value;
      if (!planId) { message = "Select a Queue Plan first."; error = true; render(); return; }
      linkChoices.set(itemId, planId); linkErrors.delete(itemId);
      void mutate(async () => {
        await linkDeploymentTaskPlan(itemId, planId);
        linkChoices.delete(itemId); linkErrors.delete(itemId);
      }, "Plan link saved.", cause => linkErrors.set(itemId, String(cause)));
    }; });
    root.querySelectorAll("[data-task-link-choice]").forEach(select => { select.onchange = () => {
      const itemId = select.dataset.taskLinkChoice;
      linkChoices.set(itemId, select.value); linkErrors.delete(itemId);
      root.querySelector(`[data-task-link="${CSS.escape(itemId)}"]`).disabled = busy || !select.value;
      root.querySelector(`[data-task-link-error="${CSS.escape(itemId)}"]`).textContent = "";
    }; });
    root.querySelectorAll("[data-task-unlink]").forEach(button => { button.onclick = () => { void mutate(() => linkDeploymentTaskPlan(button.dataset.taskUnlink, null), "Plan unlinked. Its Queue history was kept."); }; });
  }
  async function poll() {
    if (!alive()) return;
    if (!busy && !editing && !document.hidden && !root.closest("[hidden]") && document.activeElement?.tagName !== "SELECT") {
      try {
        const before = JSON.stringify([tasks, plans]);
        await refresh();
        if (alive() && !busy && !editing && before !== JSON.stringify([tasks, plans])) render();
      }
      catch (cause) { if (alive()) { message = `Refresh failed: ${cause}`; error = true; render(); } }
    }
    if (alive()) setTimeout(poll, 5000);
  }
  try { await refresh(); if (alive()) { if (editing) await editTask(editing); else render(); setTimeout(poll, 5000); } }
  catch (cause) { if (alive()) { message = String(cause); error = true; render(); } }
}

const emptyItem = () => ({ id: "", role: "", planId: null });
const emptyTask = () => ({ id: "", jiraKey: "", name: "", revision: 0, items: [emptyItem()] });

function editorMarkup(task, busy, localRoles, loading, rolesError) {
  return `<form class="task-editor planner-details-card" data-task-form><fieldset ${busy ? "disabled" : ""}><legend>${task.id ? "Edit Task" : "New Task"}</legend>
    <div class="task-fields"><label class="planner-field"><span>Jira key</span><input name="jiraKey" value="${escapeHtml(task.jiraKey)}" placeholder="GPDM-25741" maxlength="100" required ${task.items.some(item => item.planId) ? "readonly" : ""}></label>
    <label class="planner-field"><span>Task name <small>Optional</small></span><input name="name" value="${escapeHtml(task.name)}" maxlength="200" placeholder="Defaults to Jira key"></label></div>
    <p>${escapeHtml(loading ? "Reading local Ansible roles…" : rolesError || "Select the roles to track under this Jira. Ansible supplies image versions for deployment approval.")}</p>
    <div class="task-service-editor">${task.items.map((item, index) => {
      const roles = [...new Set([...localRoles, ...(item.role ? [item.role] : [])])].sort();
      return `<div data-service-index="${index}"><label class="planner-field"><span>Service / role</span><select name="role" required ${item.planId || loading || !roles.length ? "disabled" : ""}><option value="">${loading ? "Loading roles…" : roles.length ? "Select a role" : "No local roles available"}</option>${roles.map(role => `<option value="${escapeHtml(role)}"${item.role === role ? " selected" : ""}>${escapeHtml(role)}${!localRoles.includes(role) ? " (not in local project)" : ""}</option>`).join("")}</select></label><button type="button" data-service-remove="${index}" ${item.planId || task.items.length === 1 ? "disabled" : ""}>Remove</button></div>`;
    }).join("")}</div>
    <footer><button type="button" data-service-add ${task.items.length >= 100 ? "disabled" : ""}>+ Add service</button><span></span><button type="button" data-task-cancel>Cancel</button><button type="submit">Save Task</button></footer>
  </fieldset></form>`;
}

function taskMarkup(task, plans, busy, deleting, linkChoices, linkErrors) {
  const progress = taskProgress(task, plans);
  const badge = state => `<span class="task-status${state.done ? " is-completed" : ""}">${escapeHtml(state.label)}${state.total ? ` · ${state.completed}/${state.total}` : ""}</span>`;
  return `<header class="task-heading"><div><small>${escapeHtml(task.jiraKey)}</small><h2>${escapeHtml(task.name)}</h2></div><nav><button data-task-edit ${busy ? "disabled" : ""}>Edit</button><button data-task-delete ${busy ? "disabled" : ""}>${deleting ? "Confirm delete Task" : "Delete"}</button></nav></header>
    <p class="task-hint">Ready requires all deployment and Thanos validation jobs to complete. Progress shows completed/total jobs for the linked Plan's selected targets.</p>
    <div class="task-table-scroll"><table class="task-table"><thead><tr><th>Service / role</th><th>Deployment</th><th>Validation</th><th>Plan</th></tr></thead><tbody>${progress.items.map(item => {
      const state = item.progress, plan = state.plan;
      return `<tr><td><strong>${escapeHtml(item.role)}</strong><small class="${state.attention ? "is-error" : ""}">${escapeHtml(state.label)}</small></td><td>${badge(state.deployment)}</td><td>${badge(state.validation)}</td><td><div class="task-plan-actions">${plan ? `<button data-task-plan="${escapeHtml(plan.id)}">Open #${plan.planNumber}</button><span>${escapeHtml(plan.status)}${plan.archived ? " · archived" : ""}</span><button data-task-unlink="${escapeHtml(item.id)}" ${busy ? "disabled" : ""}>Unlink</button>` : `<div class="task-plan-link-controls"><button type="button" data-task-create="${escapeHtml(item.id)}" title="Create Plan" aria-label="Create Plan" ${busy ? "disabled" : ""}>${taskActionIcon("create")}</button><select data-task-link-choice="${escapeHtml(item.id)}" aria-label="Existing Plan for ${escapeHtml(item.role)}" ${busy ? "disabled" : ""}><option value="">Existing Queue Plan…</option>${plans.filter(plan => !plan.archived).map(plan => `<option value="${escapeHtml(plan.id)}"${linkChoices.get(item.id) === plan.id ? " selected" : ""}>#${plan.planNumber} · ${escapeHtml(plan.name)}</option>`).join("")}</select><button type="button" data-task-link="${escapeHtml(item.id)}" title="Save link" aria-label="Save link" ${busy || !plans.some(plan => !plan.archived && plan.id === linkChoices.get(item.id)) ? "disabled" : ""}>${taskActionIcon("link")}</button></div><p class="is-error" data-task-link-error="${escapeHtml(item.id)}" role="alert">${escapeHtml(linkErrors.get(item.id) ?? "")}</p>`}</div></td></tr>`;
    }).join("")}</tbody></table></div>
    <p class="task-hint">Linking requires matching Jira and role. Image versions come from Ansible. Start or retry from the Queue. Tasks are stored in this workspace; Jira is not updated.</p>`;
}

function taskActionIcon(action) {
  const path = action === "create"
    ? "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8zM14 2v6h6M12 12v6M9 15h6"
    : "M10 13a5 5 0 0 0 7 0l3-3a5 5 0 0 0-7-7l-2 2M14 11a5 5 0 0 0-7 0l-3 3a5 5 0 0 0 7 7l2-2";
  return `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="${path}"/></svg>`;
}
