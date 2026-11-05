export function syncButtonLabel(status) {
  return status?.running ? `Sync Project ${Math.max(0, Math.min(99, Number(status.percent) || 0))}%` : "Sync Project";
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
