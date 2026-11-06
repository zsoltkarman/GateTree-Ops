// Reconcile keyed controls instead of replacing the live Queue. Event handlers,
// focus, text selection, payload drafts and open log nodes retain their identity.
function key(node) {
  if (node.nodeType !== 1) return null;
  const data = node.dataset;
  for (const name of ["planBatch", "planGroup", "jobGroup", "jobItem", "jobRow", "infoPanel", "queueLog", "togglePlan", "duplicatePlan", "archivePlan", "archivedPlan", "info", "copyPlan", "savePlan", "deletePlan", "planPayload", "resizeQueueColumn", "action"]) {
    if (data[name] !== undefined) return `${name}:${data[name]}`;
  }
  if (data.controlJob || data.controlPlan) {
    const group = ["disable", "enable"].includes(data.controlAction) ? "enabled" : ["approve_all", "revoke_approve_all"].includes(data.controlAction) ? "approval" : "run";
    return `${data.controlJob ? "job" : "plan"}:${data.controlJob || data.controlPlan}:${group}`;
  }
  for (const name of ["queue-plan-progress", "queue-status", "queue-plan-actions", "queue-job-details", "queue-module", "queue-toolbar", "queue-groups", "queue-plan-body", "queue-job-table"]) if (node.classList.contains(name)) return name;
  return null;
}

function compatible(a, b) { return a.nodeType === b.nodeType && a.nodeName === b.nodeName; }

export function reconcileQueue(current, incoming) {
  if (current.nodeType !== 1) {
    if (current.nodeValue !== incoming.nodeValue) current.nodeValue = incoming.nodeValue;
    return;
  }
  for (const attribute of [...current.attributes]) {
    if (current.dataset.infoPanel !== undefined && attribute.name === "hidden") continue;
    if (!incoming.hasAttribute(attribute.name) && attribute.name !== "data-tooltip" && attribute.name !== "style") current.removeAttribute(attribute.name);
  }
  for (const attribute of [...incoming.attributes]) {
    if (current.dataset.infoPanel !== undefined && attribute.name === "hidden") continue;
    if (attribute.name === "title" && current.dataset.tooltip !== undefined) {
      if (current.dataset.tooltip !== attribute.value) current.dataset.tooltip = attribute.value;
    } else if (current.getAttribute(attribute.name) !== attribute.value) current.setAttribute(attribute.name, attribute.value);
  }
  // Logs update through their incremental endpoint; edits stay local until Save.
  if (current.hasAttribute("data-queue-log") || current.hasAttribute("data-plan-payload")) return;
  const previous = [...current.childNodes];
  const keyed = new Map(previous.map(node => [key(node), node]).filter(([key]) => key !== null));
  const unkeyed = new Map();
  for (const node of previous) if (key(node) === null) {
    const type = `${node.nodeType}:${node.nodeName}`;
    if (!unkeyed.has(type)) unkeyed.set(type, { nodes: [], index: 0 });
    unkeyed.get(type).nodes.push(node);
  }
  const retained = new Set();
  let cursor = current.firstChild;
  for (const fresh of [...incoming.childNodes]) {
    const freshKey = key(fresh);
    const queue = unkeyed.get(`${fresh.nodeType}:${fresh.nodeName}`);
    let existing = freshKey !== null ? keyed.get(freshKey) : queue?.nodes[queue.index++];
    if (existing && !compatible(existing, fresh)) existing = null;
    if (existing) {
      retained.add(existing);
      if (existing !== cursor) current.insertBefore(existing, cursor);
      reconcileQueue(existing, fresh);
      cursor = existing.nextSibling;
    } else {
      const added = fresh.cloneNode(true);
      current.insertBefore(added, cursor);
      retained.add(added);
    }
  }
  for (const old of previous) if (!retained.has(old)) old.remove();
}
