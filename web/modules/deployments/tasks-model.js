function jobProgress(jobs, emptyLabel) {
  const completed = jobs.filter(job => job.status === "completed").length;
  let label = emptyLabel;
  if (jobs.length) {
    if (completed === jobs.length) label = "Completed";
    else if (jobs.some(job => ["failed", "denied", "warning", "stopped"].includes(job.status))) label = "Needs attention";
    else if (jobs.some(job => ["running", "awaiting_approval"].includes(job.status))) label = "In progress";
    else if (jobs.some(job => ["disabled", "skipped"].includes(job.status))) label = "Incomplete";
    else label = completed ? "In progress" : "Pending";
  }
  return { label, completed, total: jobs.length, done: jobs.length > 0 && completed === jobs.length };
}

export function taskItemProgress(item, plans) {
  const plan = plans.find(plan => plan.id === item.planId);
  const jobs = plan?.jobs ?? [];
  const deployment = jobProgress(jobs.filter(job => ["olam", "container"].includes(job.driver) && ["deploy", "validate_deploy"].includes(job.action)), "Not planned");
  const validation = jobProgress(jobs.filter(job => job.driver === "thanos" && job.action === "query"), "Not configured");
  const attention = plan && (["failed", "denied", "warning", "stopped"].includes(plan.status) || [deployment, validation].some(progress => ["Needs attention", "Incomplete"].includes(progress.label)));
  const done = Boolean(plan?.status === "completed" && deployment.done && validation.done);
  const label = !plan ? "Not planned" : attention ? "Needs attention" : done ? "Ready" : ["running", "awaiting_approval"].includes(plan.status) ? "In progress" : deployment.done ? "Deployed" : "Planned";
  return { plan, deployment, validation, done, attention: Boolean(attention), label };
}

export function taskProgress(task, plans) {
  const items = task.items.map(item => ({ ...item, progress: taskItemProgress(item, plans) }));
  return {
    items, total: items.length,
    planned: items.filter(item => item.progress.plan).length,
    deployed: items.filter(item => item.progress.deployment.done).length,
    validated: items.filter(item => item.progress.validation.done).length,
    ready: items.filter(item => item.progress.done).length,
    attention: items.filter(item => item.progress.attention).length,
  };
}

// Task references do not override Ansible deployment variables.
export function applyTaskItem(draft, task, item) {
  draft.taskItemId = item.id;
  draft.taskRole = item.role;
  draft.jiraNo = task.jiraKey;
  draft.planName ||= `${task.jiraKey} · ${item.role}`;
  if (draft.targetSelection) {
    draft.targetSelection.roles = [item.role];
    draft.targetSelection.sites = [];
    draft.targetSelection.excluded = [];
  }
}
