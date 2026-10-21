import { mountQueueColumns } from "./queue-columns.js";
import { emptyPlannerDraft, mountPlannerBuilder } from "./planner-builder.js";
import { reviewDeploymentApproval } from "./approval-dialog.js";
import { ensureAgentCredentials } from "../agent/credential-recovery.js";
import { escapeHtml } from "../../shared/ui.js";
import { controlDeploymentQueueItem, deleteDeploymentPlan, getDeploymentQueue, updateDeploymentPlan } from "./queue-api.js";

export async function mountDeploymentQueueView(root, detail = {}, { onQueueChange = () => {} } = {}) {
  const state = { plans: [], selectedId: detail.planId ?? null, expandedPlans: new Set(detail.planId ? [detail.planId] : []), adding: Boolean(detail.new), plannerDraft: emptyPlannerDraft(), disposeBuilder: null, message: "", error: false, busyKey: null, infoKey: null, onQueueChange, mountId: crypto.randomUUID() };
  root.dataset.queueMountId = state.mountId;
  root.innerHTML = '<div class="deployment-config-loading">Loading Queue…</div>';
  try {
    state.plans = (await getDeploymentQueue()).plans;
    if (!root.isConnected || root.dataset.queueMountId !== state.mountId) return;
    notifyQueue(state);
    render(root, state);
    scheduleQueuePoll(root, state);
  } catch (cause) {
    root.innerHTML = `<div class="deployment-config-error">${escapeHtml(cause)}</div>`;
  }
}

function render(root, state) {
  const logScroll = new Map([...root.querySelectorAll(".queue-log-panel")].map((panel) => {
    const pre = panel.querySelector("pre");
    return [panel.dataset.infoPanel, { top: pre.scrollTop, follow: pre.scrollHeight - pre.clientHeight - pre.scrollTop < 20 }];
  }));
  state.disposeColumns?.();
  state.disposeColumns = null;
  state.disposeBuilder?.();
  state.disposeBuilder = null;
  if (state.adding) {
    state.disposeBuilder = mountPlannerBuilder(root, { draft: state.plannerDraft, onSaved: snapshot => {
      state.plans = snapshot.plans; state.adding = false; state.plannerDraft = emptyPlannerDraft();
      state.selectedId = snapshot.createdPlanId; state.expandedPlans.add(snapshot.createdPlanId);
      const saved = state.plans.find(plan => plan.id === snapshot.createdPlanId);
      state.message = `Plan #${saved?.planNumber ?? ""} saved to Queue.`; state.error = false;
      render(root, state); notifyQueue(state);
    }, onClose: () => { state.adding = false; render(root, state); } });
  } else {
    root.innerHTML = queueMarkup(state);
    bindQueue(root, state);
  }
  root.querySelectorAll("[data-info-panel]").forEach((panel) => {
    panel.hidden = panel.dataset.infoPanel !== state.infoKey;
    const position = logScroll.get(panel.dataset.infoPanel);
    const pre = panel.querySelector("pre");
    if (position && pre) pre.scrollTop = position.follow ? pre.scrollHeight : position.top;
  });
}

function queueMarkup(state) {
  const visiblePlans = state.selectedId
    ? state.plans.filter((plan) => plan.id === state.selectedId)
    : state.plans;
  return `<section class="queue-module">
    ${state.selectedId && visiblePlans.length ? "" : `<header class="queue-toolbar"><strong>Queue</strong><button data-action="add" title="Add Plan" aria-label="Add Plan">${icon("plus")}</button></header>`}
    ${state.message ? `<p class="queue-message${state.error ? " is-error" : ""}">${escapeHtml(state.message)}</p>` : ""}
    <div class="queue-groups">
      ${visiblePlans.length ? visiblePlans.map((plan) => planMarkup(plan, state)).join("") : '<div class="queue-empty">No plans in the Queue.</div>'}
    </div>
  </section>`;
}

function planMarkup(plan, state) {
  const planAction = plan.status === "awaiting_approval" ? "stop" : plan.status === "stopped" ? "restart" : controlAction(plan.status);
  const expanded = Boolean(state.expandedPlans?.has(plan.id));
  return `<article class="queue-plan-group${expanded ? " is-expanded" : ""}" data-plan-group="${escapeHtml(plan.id)}">
    <header>
      <button type="button" class="queue-plan-toggle" data-toggle-plan="${escapeHtml(plan.id)}" aria-expanded="${expanded}" aria-controls="queue-plan-body-${escapeHtml(plan.id)}" aria-label="${expanded ? "Collapse" : "Expand"} Plan #${escapeHtml(plan.planNumber)}">
        ${icon("chevron")}<span class="queue-plan-heading"><strong>Plan #${escapeHtml(plan.planNumber)} · ${escapeHtml(plan.name)}</strong><span>${escapeHtml(plan.processName)}</span></span>
      </button>
      <div class="queue-plan-actions">
        ${state.selectedId ? `<button data-action="add" title="Add Plan" aria-label="Add Plan">${icon("plus")}</button>` : ""}
        <span class="queue-status is-${escapeHtml(plan.status)}">${escapeHtml(plan.status)}</span>
        <button class="queue-approve-all" data-control-plan="${escapeHtml(plan.id)}" data-control-action="${plan.approveAll ? "revoke_approve_all" : "approve_all"}" aria-pressed="${Boolean(plan.approveAll)}" title="${plan.approveAll ? "Turn off automatic approval for upcoming deployments" : "Automatically approve all deployments in this Plan"}" aria-label="${plan.approveAll ? "Revoke Approve all" : "Approve all"}">${icon("approve")}Approve all</button>
        <button data-info="plan-info:${escapeHtml(plan.id)}" title="Plan Info" aria-label="Plan Info">${icon("info")}</button>
        <button data-info="plan-log:${escapeHtml(plan.id)}" title="Plan Log" aria-label="Plan Log">${icon("log")}</button>
        <button data-control-plan="${escapeHtml(plan.id)}" data-control-action="${planAction}" title="${actionLabel(planAction)} Plan" aria-label="${actionLabel(planAction)} Plan">${icon(planAction)}</button>
        <button data-delete-plan="${escapeHtml(plan.id)}" title="Delete Plan" aria-label="Delete Plan">${icon("trash")}</button>
      </div>
    </header>
    <div class="queue-plan-body" id="queue-plan-body-${escapeHtml(plan.id)}" ${expanded ? "" : "hidden"}>
    ${planInfoMarkup(plan, state)}
    ${planLogMarkup(plan, state)}
    <div class="queue-job-table">
      <div class="queue-job-header">${["#", "Step name", "Target", "Details", "Status", "Actions"].map((label, index, labels) => `<span class="queue-column-heading">${index === 5 ? "" : label}${index < 5 ? `<span class="queue-column-resizer" data-resize-queue-column="${index}" role="separator" aria-orientation="vertical" aria-label="Resize ${label} and ${labels[index + 1]} columns" tabindex="0" title="Drag to resize · Double-click to reset · Arrow keys to adjust"></span>` : ""}</span>`).join("")}</div>
      ${plan.jobs.map((job, index) => jobMarkup(plan, job, index)).join("")}
    </div>
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
  const blocked = job.status === "disabled" || jobAction !== "stop" && plan.jobs.slice(0, index).some((previous) => !["completed", "disabled"].includes(previous.status));
  const blockedTitle = job.status === "disabled" ? "Enable this job before starting it" : "Waiting for previous enabled jobs to complete";
  return `<div class="queue-job-row${job.status === "disabled" ? " is-disabled" : ""}">
      <span>${job.sequence}</span><strong>${escapeHtml(job.name)}</strong><code title="${escapeHtml(job.target || "Plan level")}">${escapeHtml(job.target || "—")}</code><span class="queue-job-details" title="${escapeHtml(job.details)}">${jobDetailsMarkup(job.status === "disabled" ? "Skipped during execution" : job.details)}</span><span class="queue-status is-${escapeHtml(job.status)}">${escapeHtml(job.status)}</span>
      <div class="queue-row-actions">
        <button data-control-job="${escapeHtml(job.id)}" data-control-action="${job.status === "disabled" ? "enable" : "disable"}" title="${job.status === "disabled" ? "Enable Job" : "Disable Job"}" aria-label="${job.status === "disabled" ? "Enable Job" : "Disable Job"}" aria-pressed="${job.status === "disabled"}" ${["queued", "stopped", "disabled"].includes(job.status) ? "" : "disabled"}>${icon(job.status === "disabled" ? "enable" : "disable")}</button>
        <button data-control-job="${escapeHtml(job.id)}" data-control-action="${jobAction}" title="${blocked ? blockedTitle : `${actionLabel(jobAction)} Job`}" aria-label="${actionLabel(jobAction)} Job" ${blocked ? "disabled" : ""}>${icon(jobAction)}</button>
        <button data-info="job-info:${escapeHtml(job.id)}" title="Job Info" aria-label="Job Info">${icon("info")}</button>
        <button data-info="job-log:${escapeHtml(job.id)}" title="Job Log" aria-label="Job Log">${icon("log")}</button>
      </div>
    </div>
    ${jobInfoMarkup(job)}
    ${jobLogMarkup(job)}`;
}

function jobDetailsMarkup(details = "") {
  const match = /^(.*?)\s*·\s*(Job #\d+)$/u.exec(details);
  return `<span>${escapeHtml(match ? match[1] : details || "—")}</span>${match ? `<span class="queue-job-id">${escapeHtml(match[2])}</span>` : ""}`;
}

function jobInfoMarkup(job) {
  if (job.driver === "jira" && job.action === "update") {
    return `<div class="queue-info queue-job-info queue-job-payload" data-info-panel="job-info:${escapeHtml(job.id)}" hidden>
      <strong>Jira report configuration</strong><pre>${escapeHtml(job.payload || "{}")}</pre>
    </div>`;
  }
  if (["olam", "container"].includes(job.driver)) {
    return `<div class="queue-info queue-job-info queue-job-payload" data-info-panel="job-info:${escapeHtml(job.id)}" hidden>
      <strong>${job.driver === "container" ? "Ansible container" : "OLAM"} payload</strong>
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

function bindQueue(root, state) {
  state.disposeColumns = mountQueueColumns(root, { onResizing: value => { state.resizingColumns = value; } });
  const setExpanded = (id, expanded) => {
    if (expanded) state.expandedPlans.add(id); else state.expandedPlans.delete(id);
    const button = root.querySelector(`[data-toggle-plan="${CSS.escape(id)}"]`);
    button?.setAttribute("aria-expanded", String(expanded));
    button?.setAttribute("aria-label", `${expanded ? "Collapse" : "Expand"} ${button.querySelector("strong").textContent}`);
    const group = button?.closest(".queue-plan-group");
    group?.classList.toggle("is-expanded", expanded);
    const body = group?.querySelector(".queue-plan-body");
    if (body) body.hidden = !expanded;
  };
  root.querySelectorAll("[data-toggle-plan]").forEach(button => button.addEventListener("click", () => setExpanded(button.dataset.togglePlan, !state.expandedPlans.has(button.dataset.togglePlan))));
  root.querySelectorAll("button[title][aria-label]").forEach((button) => {
    button.dataset.tooltip = button.title;
    button.removeAttribute("title");
  });
  root.querySelector('[data-action="add"]').addEventListener("click", () => { state.adding = true; render(root, state); });
  root.querySelectorAll("[data-control-plan]").forEach((button) => button.addEventListener("click", () => controlItem(root, state, "plan", button.dataset.controlPlan, button.dataset.controlAction)));
  root.querySelectorAll("[data-control-job]").forEach((button) => button.addEventListener("click", () => controlItem(root, state, "job", button.dataset.controlJob, button.dataset.controlAction)));
  root.querySelectorAll("[data-info]").forEach((button) => button.addEventListener("click", () => {
    const planId = button.closest(".queue-plan-group")?.dataset.planGroup;
    const collapsed = planId && !state.expandedPlans.has(planId);
    state.infoKey = !collapsed && state.infoKey === button.dataset.info ? null : button.dataset.info;
    if (planId) setExpanded(planId, true);
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
    if (itemType === "job" && ["start", "restart"].includes(action)) {
      const job = state.plans.flatMap(plan => plan.jobs).find(job => job.id === id);
      if (job?.driver === "jira" && job.action === "update" && /unconfirmed/i.test(job.log ?? "")) {
        const confirmed = window.confirm(`The previous Jira report for ${job.target} has an unconfirmed result. Check the issue's attachments and comments first.

Continue only if the unconfirmed operation is missing. It may be sent again; already confirmed operations will be kept. Deployment steps will not be rerun.

Have you checked Jira and want to resend the missing report operation?`);
        if (!confirmed) return;
        action = "retry_unconfirmed_report";
      }
    }
    if (["start", "restart", "approve", "approve_all", "retry_unconfirmed_report"].includes(action)) {
      if (!await ensureAgentCredentials()) return;
    }
    if (reviewedJob) {
      const current = await getDeploymentQueue();
      const job = current.plans.flatMap((plan) => plan.jobs).find((job) => job.id === id);
      if (JSON.stringify(job) !== reviewedJob) throw new Error("The validation changed while you were reviewing it. Open Approve again to review the latest result.");
    }
    progressDialog = showQueueProgress(root, action === "retry_unconfirmed_report" ? "Retrying Jira report…" : action === "approve_all" ? "Enabling Approve all…" : action === "revoke_approve_all" ? "Turning off Approve all…" : action === "deny" ? "Denying deployment…" : action === "disable" ? "Disabling job…" : action === "enable" ? "Enabling job…" : "Preparing Agent credentials…");
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
    if (!root.isConnected || root.dataset.queueMountId !== state.mountId || !root.querySelector(".queue-module, .queue-plan-editor")) return;
    if (!state.adding && !state.busyKey && !state.resizingColumns) {
      try {
        const plans = (await getDeploymentQueue()).plans;
        if (!state.adding && !state.busyKey && !state.resizingColumns && root.dataset.queueMountId === state.mountId && JSON.stringify(plans) !== JSON.stringify(state.plans)) {
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
    chevron: "<path d='m9 5 7 7-7 7'/>",
    disable: "<circle cx='12' cy='12' r='9'/><path d='m6 6 12 12'/>",
    enable: "<circle cx='12' cy='12' r='9'/><path d='m7 12 3 3 7-7'/>",
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
