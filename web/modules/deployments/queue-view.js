import { reviewDeploymentApproval } from "./approval-dialog.js";
import { ensureAgentCredentials } from "../agent/credential-recovery.js";
import { escapeHtml } from "../../shared/ui.js";
import { controlDeploymentQueueItem, createDeploymentPlan, deleteDeploymentPlan, getDeploymentQueue, updateDeploymentPlan } from "./queue-api.js";

export async function mountDeploymentQueueView(root, detail = {}, { onQueueChange = () => {} } = {}) {
  const state = { plans: [], selectedId: detail.planId ?? null, adding: Boolean(detail.new), draft: "", message: "", error: false, busy: false, busyKey: null, infoKey: null, onQueueChange, mountId: crypto.randomUUID() };
  root.dataset.queueMountId = state.mountId;
  root.innerHTML = '<div class="deployment-config-loading">Loading Queue…</div>';
  try {
    state.plans = (await getDeploymentQueue()).plans;
    notifyQueue(state);
    render(root, state);
    scheduleQueuePoll(root, state);
  } catch (cause) {
    root.innerHTML = `<div class="deployment-config-error">${escapeHtml(cause)}</div>`;
  }
}

function render(root, state) {
  root.innerHTML = state.adding ? editorMarkup(state) : queueMarkup(state);
  if (state.adding) bindEditor(root, state); else bindQueue(root, state);
}

function editorMarkup(state) {
  return `<section class="queue-plan-editor">
    <header class="deployment-json-toolbar">
      <div><strong>New Plan</strong><span>YAML input</span></div>
      <nav>
        <button data-action="cancel" title="Cancel" aria-label="Cancel">${icon("close")}</button>
        <button class="deployment-save" data-action="save" title="Add to Queue" aria-label="Add to Queue" ${state.busy ? "disabled" : ""}>${icon("save")}</button>
      </nav>
    </header>
    <textarea class="deployment-json-editor" data-role="plan-editor" aria-label="Plan YAML" spellcheck="false">${escapeHtml(state.draft)}</textarea>
    <footer class="deployment-json-status"><span class="${state.error ? "is-error" : ""}">${escapeHtml(state.message)}</span><code>YAML</code></footer>
  </section>`;
}

function queueMarkup(state) {
  const visiblePlans = state.selectedId
    ? state.plans.filter((plan) => plan.id === state.selectedId)
    : state.plans;
  return `<section class="queue-module">
    <header class="queue-toolbar"><strong>${escapeHtml(visiblePlans[0]?.name && state.selectedId ? visiblePlans[0].name : "Queue")}</strong><button data-action="add" title="Add Plan" aria-label="Add Plan">${icon("plus")}</button></header>
    ${state.message ? `<p class="queue-message${state.error ? " is-error" : ""}">${escapeHtml(state.message)}</p>` : ""}
    <div class="queue-groups">
      ${visiblePlans.length ? visiblePlans.map((plan) => planMarkup(plan, state)).join("") : '<div class="queue-empty">No plans in the Queue.</div>'}
    </div>
  </section>`;
}

function planMarkup(plan, state) {
  const planAction = plan.status === "awaiting_approval" ? "stop" : controlAction(plan.status);
  return `<article class="queue-plan-group">
    <header>
      <div><strong>Plan #${escapeHtml(plan.planNumber)} · ${escapeHtml(plan.name)}</strong><span>${escapeHtml(plan.processName)}</span></div>
      <div class="queue-plan-actions">
        <span class="queue-status is-${escapeHtml(plan.status)}">${escapeHtml(plan.status)}</span>
        <button data-info="plan-info:${escapeHtml(plan.id)}" title="Plan Info" aria-label="Plan Info">${icon("info")}</button>
        <button data-info="plan-log:${escapeHtml(plan.id)}" title="Plan Log" aria-label="Plan Log">${icon("log")}</button>
        <button data-control-plan="${escapeHtml(plan.id)}" data-control-action="${planAction}" title="${actionLabel(planAction)} Plan" aria-label="${actionLabel(planAction)} Plan">${icon(planAction)}</button>
        <button data-delete-plan="${escapeHtml(plan.id)}" title="Delete Plan" aria-label="Delete Plan">${icon("trash")}</button>
      </div>
    </header>
    ${planInfoMarkup(plan, state)}
    ${planLogMarkup(plan, state)}
    <div class="queue-job-table">
      <div class="queue-job-header"><span>#</span><span>Step name</span><span>Target</span><span>Details</span><span>Status</span><span></span></div>
      ${plan.jobs.map((job, index) => jobMarkup(plan, job, index)).join("")}
    </div>
  </article>`;
}

function planInfoMarkup(plan, state) {
  const infoKey = `plan-info:${plan.id}`;
  const running = plan.status === "running";
  return `<div class="queue-info queue-plan-payload" data-info-panel="${escapeHtml(infoKey)}" ${state.infoKey === infoKey ? "" : "hidden"}>
    <div class="queue-payload-heading">
      <div><strong>Plan #${escapeHtml(plan.planNumber)} Payload</strong><span>${running ? "Stop the Plan before editing" : "Saving regenerates all Queue jobs"}</span></div>
      <nav class="queue-payload-actions">
        <button data-copy-plan="${escapeHtml(plan.id)}" title="Copy Plan" aria-label="Copy Plan">${icon("copy")}</button>
        <button data-save-plan="${escapeHtml(plan.id)}" title="Save Payload" aria-label="Save Payload" ${running ? "disabled" : ""}>${icon("save")}</button>
      </nav>
    </div>
    <textarea data-plan-payload="${escapeHtml(plan.id)}" aria-label="Plan YAML payload" spellcheck="false" ${running ? "readonly" : ""}>${escapeHtml(plan.definition)}</textarea>
  </div>`;
}

function planLogMarkup(plan, state) {
  const infoKey = `plan-log:${plan.id}`;
  const log = plan.jobs
    .filter((job) => job.log)
    .map((job) => `[${job.sequence}] ${job.name}${job.target ? ` — ${job.target}` : ""}\n${job.log}`)
    .join("\n\n");
  return `<div class="queue-info queue-log-panel" data-info-panel="${escapeHtml(infoKey)}" ${state.infoKey === infoKey ? "" : "hidden"}>
    <strong>Plan log</strong>
    <pre>${escapeHtml(log || "No log output yet.")}</pre>
  </div>`;
}

function jobMarkup(plan, job, index) {
  const jobAction = controlAction(job.status);
  const blocked = jobAction !== "stop" && plan.jobs.slice(0, index).some((previous) => previous.status !== "completed");
  const blockedTitle = "Waiting for all previous jobs to complete successfully";
  return `<div class="queue-job-row">
      <span>${job.sequence}</span><strong>${escapeHtml(job.name)}</strong><code title="${escapeHtml(job.target || "Plan level")}">${escapeHtml(job.target || "—")}</code><span class="queue-job-details" title="${escapeHtml(job.details)}">${escapeHtml(job.details || "—")}</span><span class="queue-status is-${escapeHtml(job.status)}">${escapeHtml(job.status)}</span>
      <div class="queue-row-actions">
        <button data-control-job="${escapeHtml(job.id)}" data-control-action="${jobAction}" title="${blocked ? blockedTitle : `${actionLabel(jobAction)} Job`}" aria-label="${actionLabel(jobAction)} Job" ${blocked ? "disabled" : ""}>${icon(jobAction)}</button>
        <button data-info="job-info:${escapeHtml(job.id)}" title="Job Info" aria-label="Job Info">${icon("info")}</button>
        <button data-info="job-log:${escapeHtml(job.id)}" title="Job Log" aria-label="Job Log">${icon("log")}</button>
      </div>
    </div>
    ${jobInfoMarkup(job)}
    ${jobLogMarkup(job)}`;
}

function jobInfoMarkup(job) {
  if (job.driver === "olam") {
    return `<div class="queue-info queue-job-info queue-job-payload" data-info-panel="job-info:${escapeHtml(job.id)}" hidden>
      <strong>OLAM payload</strong>
      <pre>${escapeHtml(job.payload || "{}")}</pre>
    </div>`;
  }
  return `<div class="queue-info queue-job-info" data-info-panel="job-info:${escapeHtml(job.id)}" hidden>
    <dl><div><dt>Sequence</dt><dd>${job.sequence}</dd></div><div><dt>Target</dt><dd>${escapeHtml(job.target || "Plan level")}</dd></div><div><dt>Details</dt><dd>${escapeHtml(job.details || "—")}</dd></div><div><dt>Driver</dt><dd>${escapeHtml(job.driver)}</dd></div><div><dt>Action</dt><dd>${escapeHtml(job.action)}</dd></div><div><dt>Status</dt><dd>${escapeHtml(job.status)}</dd></div></dl>
  </div>`;
}

function jobLogMarkup(job) {
  return `<div class="queue-info queue-job-info queue-log-panel" data-info-panel="job-log:${escapeHtml(job.id)}" hidden>
    <strong>${escapeHtml(job.name)} log</strong>
    <pre>${escapeHtml(job.log || "No log output yet.")}</pre>
  </div>`;
}

function bindEditor(root, state) {
  const editor = root.querySelector('[data-role="plan-editor"]');
  requestAnimationFrame(() => editor.focus());
  editor.addEventListener("input", () => { state.draft = editor.value; });
  editor.addEventListener("keydown", (event) => {
    if (event.key !== "Tab") return;
    event.preventDefault();
    editor.setRangeText("  ", editor.selectionStart, editor.selectionEnd, "end");
  });
  root.querySelector('[data-action="cancel"]').addEventListener("click", () => { state.adding = false; render(root, state); });
  root.querySelector('[data-action="save"]').addEventListener("click", async () => {
    state.busy = true; state.message = ""; state.error = false; render(root, state);
    try {
      const snapshot = await createDeploymentPlan(state.draft);
      state.plans = snapshot.plans; state.selectedId = snapshot.plans[0]?.id ?? null; state.adding = false; notifyQueue(state);
    } catch (cause) {
      state.message = String(cause); state.error = true;
    } finally {
      state.busy = false; render(root, state);
    }
  });
}

function bindQueue(root, state) {
  root.querySelector('[data-action="add"]').addEventListener("click", () => { state.adding = true; state.draft = ""; render(root, state); });
  root.querySelectorAll("[data-control-plan]").forEach((button) => button.addEventListener("click", () => controlItem(root, state, "plan", button.dataset.controlPlan, button.dataset.controlAction)));
  root.querySelectorAll("[data-control-job]").forEach((button) => button.addEventListener("click", () => controlItem(root, state, "job", button.dataset.controlJob, button.dataset.controlAction)));
  root.querySelectorAll("[data-info]").forEach((button) => button.addEventListener("click", () => {
    state.infoKey = state.infoKey === button.dataset.info ? null : button.dataset.info;
    root.querySelectorAll("[data-info-panel]").forEach((panel) => { panel.hidden = panel.dataset.infoPanel !== state.infoKey; });
  }));
  root.querySelectorAll("[data-plan-payload]").forEach((editor) => editor.addEventListener("keydown", (event) => {
    if (event.key !== "Tab" || editor.readOnly) return;
    event.preventDefault();
    editor.setRangeText("  ", editor.selectionStart, editor.selectionEnd, "end");
  }));
  root.querySelectorAll("[data-copy-plan]").forEach((button) => button.addEventListener("click", async () => {
    const editor = root.querySelector(`[data-plan-payload="${CSS.escape(button.dataset.copyPlan)}"]`);
    if (!editor) return;
    try {
      await copyText(editor.value);
      button.title = "Copied";
      button.setAttribute("aria-label", "Copied");
      window.setTimeout(() => {
        button.title = "Copy Plan";
        button.setAttribute("aria-label", "Copy Plan");
      }, 1500);
    } catch (cause) {
      window.alert(String(cause));
    }
  }));
  root.querySelectorAll("[data-save-plan]").forEach((button) => button.addEventListener("click", async () => {
    const editor = root.querySelector(`[data-plan-payload="${CSS.escape(button.dataset.savePlan)}"]`);
    if (!editor || state.busyKey) return;
    state.busyKey = `plan:${button.dataset.savePlan}`;
    button.disabled = true;
    try {
      state.plans = (await updateDeploymentPlan(button.dataset.savePlan, editor.value)).plans;
      notifyQueue(state);
      render(root, state);
    } catch (cause) {
      window.alert(String(cause));
      button.disabled = false;
    } finally {
      state.busyKey = null;
    }
  }));
  root.querySelectorAll("[data-delete-plan]").forEach((button) => button.addEventListener("click", async () => {
    const plan = state.plans.find((item) => item.id === button.dataset.deletePlan);
    if (!plan || !window.confirm(`Delete plan “${plan.name}”?`)) return;
    try { state.plans = (await deleteDeploymentPlan(plan.id)).plans; state.selectedId = null; notifyQueue(state); render(root, state); }
    catch (cause) { window.alert(String(cause)); }
  }));
}

async function controlItem(root, state, itemType, id, action) {
  if (state.busyKey) return;
  state.busyKey = `${itemType}:${id}`;
  state.message = "";
  state.error = false;
  let progressDialog = null;
  let reviewedJob = null;
  try {
    if (itemType === "job" && action === "approve") {
      state.plans = (await getDeploymentQueue()).plans;
      const plan = state.plans.find((plan) => plan.jobs.some((job) => job.id === id));
      const job = plan?.jobs.find((job) => job.id === id);
      if (!job || job.status !== "awaiting_approval") throw new Error("This step is no longer awaiting approval.");
      reviewedJob = JSON.stringify(job);
      const decision = await reviewDeploymentApproval(root, plan, job);
      if (!decision) return;
      action = decision;
    }
    if (["start", "restart", "approve"].includes(action)) {
      if (!await ensureAgentCredentials()) return;
    }
    if (reviewedJob) {
      const current = await getDeploymentQueue();
      const job = current.plans.flatMap((plan) => plan.jobs).find((job) => job.id === id);
      if (JSON.stringify(job) !== reviewedJob) throw new Error("The validation changed while you were reviewing it. Open Approve again to review the latest result.");
    }
    progressDialog = showQueueProgress(root, action === "deny" ? "Denying deployment…" : "Preparing Agent credentials…");
    state.plans = (await controlDeploymentQueueItem(itemType, id, action)).plans;
    notifyQueue(state);
    render(root, state);
  } catch (cause) {
    state.message = String(cause);
    state.error = true;
    render(root, state);
  } finally {
    progressDialog?.close();
    progressDialog?.remove();
    state.busyKey = null;
  }
}

function showQueueProgress(root, message) {
  const dialog = document.createElement("dialog");
  dialog.className = "queue-password-dialog queue-progress-dialog";
  dialog.innerHTML = `<div><span class="queue-progress-spinner"></span><strong>${escapeHtml(message)}</strong></div>`;
  root.append(dialog);
  dialog.showModal();
  return dialog;
}


function scheduleQueuePoll(root, state) {
  window.setTimeout(async () => {
    if (root.dataset.queueMountId !== state.mountId || !root.querySelector(".queue-module, .queue-plan-editor")) return;
    if (!state.adding && !state.busyKey) {
      try {
        const plans = (await getDeploymentQueue()).plans;
        if (JSON.stringify(plans) !== JSON.stringify(state.plans)) {
          state.plans = plans;
          notifyQueue(state);
          render(root, state);
        }
      } catch (_) {}
    }
    scheduleQueuePoll(root, state);
  }, 1500);
}

function controlAction(status) {
  if (status === "awaiting_approval") return "approve";
  if (status === "running") return "stop";
  if (status === "failed" || status === "warning" || status === "completed" || status === "denied") return "restart";
  return "start";
}

function actionLabel(action) { return action[0].toUpperCase() + action.slice(1); }

function notifyQueue(state) { state.onQueueChange(state.plans, state.selectedId); }

async function copyText(value) {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(value);
      return;
    } catch (_) {}
  }
  const field = document.createElement("textarea");
  field.value = value;
  field.style.position = "fixed";
  field.style.opacity = "0";
  document.body.append(field);
  field.select();
  const copied = document.execCommand("copy");
  field.remove();
  if (!copied) throw new Error("Could not copy the Plan.");
}

function icon(name) {
  const paths = {
    plus: "<path d='M12 5v14M5 12h14'/>",
    close: "<path d='m6 6 12 12M18 6 6 18'/>",
    save: "<path d='M5 4h12l2 2v14H5zM8 4v6h8V4M8 20v-6h8v6'/>",
    copy: "<rect x='8' y='8' width='11' height='11' rx='1.5'/><path d='M16 8V5H5v11h3'/>",
    trash: "<path d='M5 7h14M9 7V4h6v3M7.5 7l.7 13h7.6l.7-13'/>",
    start: "<path d='m8 5 11 7-11 7z'/>",
    stop: "<rect x='6' y='6' width='12' height='12' rx='1'/>",
    restart: "<path d='M20 11a8 8 0 1 0-2.34 5.66M20 4v7h-7'/>",
    approve: "<path d='m5 12 4 4L19 6'/>",
    info: "<circle cx='12' cy='12' r='9'/><path d='M12 11v6M12 7h.01'/>",
    log: "<path d='M6 3h9l3 3v15H6zM15 3v4h4M9 11h6M9 15h6M9 19h4'/>",
  };
  return `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[name]}</svg>`;
}
