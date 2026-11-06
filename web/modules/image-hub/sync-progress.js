export function syncButtonLabel(status) {
  return status?.running ? `Sync Project ${Math.max(0, Math.min(99, Number(status.percent) || 0))}%` : "Sync Project";
}
export function inventorySyncButtonLabel(statuses = [], projects = []) {
  const ids = new Set(projects.map(project => project.id));
  const active = statuses.find(status => status.running && ids.has(status.projectId));
  if (!active) return "Sync all projects";
  const batch = statuses.filter(status => ids.has(status.projectId) && status.runId === active.runId);
  const percent = Math.floor(batch.reduce((sum,status) => sum + (status.running ? Math.max(0,Math.min(99,Number(status.percent)||0)) : 100),0) / Math.max(1,batch.length));
  return `Sync all projects ${percent}%`;
}
export function sourceProgressLabel(source) {
  if (!source || source.state === "pending") return "Not synced";
  if (source.state === "not_configured") return "Not configured";
  const completed = Math.max(0, Number(source.completed) || 0);
  const total = Math.max(0, Number(source.total) || 0);
  const errors = Math.max(0, Number(source.errors) || 0);
  if (source.state === "running") return total ? `Syncing ${completed}/${total}` : "Syncing…";
  if (source.state === "error") return `Partial ${Math.max(0, completed - errors)}/${total}`;
  return `Synced ${Math.max(0, completed - errors)}/${total}`;
}
