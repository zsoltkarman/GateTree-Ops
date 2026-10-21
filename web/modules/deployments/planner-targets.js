import { escapeHtml } from "../../shared/ui.js";
import { getLocalDeploymentInventory, getLocalAnsibleSettings } from "./deployments-api.js";

export function resolvePlannerTargets(snapshot, selection) {
  const roles = new Set(selection.roles); const sites = new Set(selection.sites);
  return snapshot.sites.filter(site => sites.has(site.name)).flatMap(site => site.hosts
    .filter(host => host.roles.some(role => roles.has(role)))
    .map(host => ({ ...host, site: site.name, group: site.group, roles: host.roles.filter(role => roles.has(role)), key: JSON.stringify([site.name, host.name]) })));
}

export function plannerTargetInput(snapshot, selection) {
  const matches = snapshot ? resolvePlannerTargets(snapshot, selection) : [];
  const excluded = new Set(selection.excluded);
  const enabled = matches.filter(host => !excluded.has(host.key));
  const jobMode = selection.jobMode ?? "per_host";
  const groups = jobMode === "together" && matches.length > 0 && enabled.length === matches.length && matches.every(host => host.group);
  return {
    job_mode: jobMode,
    roles: snapshot ? [...selection.roles] : [],
    sites: snapshot ? [...selection.sites] : [],
    ...(groups ? { inventory_groups: [...new Set(matches.map(host => host.group))] } : { hosts: [...new Set(enabled.map(host => host.name))] }),
  };
}

export function mountPlannerTargets(root, { draft, loadInventory = getLocalDeploymentInventory, loadSettings = getLocalAnsibleSettings } = {}) {
  draft.targetSelection ??= { processId: draft.processId, roles: [], sites: [], excluded: [], revision: null };
  draft.targetSelection.jobMode ??= "per_host";
  draft.targetSelection.roles = draft.targetSelection.roles.slice(0, 1);
  let disposed = false, request = 0, snapshot = null, loading = false, message = "", sourcePath = undefined, roleQuery = "", siteQuery = "", openPicker = null;
  root.innerHTML = `<section class="planner-details-card planner-target-card" aria-label="Deployment targets"><div class="planner-target-content" data-target-content></div></section>`;
  const content = root.querySelector("[data-target-content]");
  const alive = () => !disposed && root.isConnected;
  const selected = () => draft.targetSelection;
  function syncDraft() {
    const selection = selected();
    draft.olamTargets = plannerTargetInput(snapshot, selection);
    if (snapshot) selection.revision = snapshot.revision;
  }
  function render() {
    root.querySelector(".planner-target-card").hidden = Boolean(sourcePath && !draft.processId && !message && !loading);
    syncDraft();
    if (!snapshot) {
      content.innerHTML = `<p class="planner-target-message${message ? " is-error" : ""}" role="status">${escapeHtml(loading ? "Reading local inventory…" : message || (sourcePath === undefined ? "Loading local Ansible settings…" : !sourcePath ? "Local Ansible folder is not configured. Set it under Deployments → Config to load the inventory." : !draft.processId ? "" : "No inventory loaded."))}</p>${message ? '<button type="button" data-target-refresh>Retry</button>' : ""}`;
      content.querySelector("[data-target-refresh]")?.addEventListener("click", refresh); return;
    }
    const selection = selected();
    const roles = [...new Set(snapshot.sites.flatMap(site => site.hosts.flatMap(host => host.roles)))].sort();
    const matches = resolvePlannerTargets(snapshot, selection); const enabled = matches.filter(host => !selection.excluded.includes(host.key));
    const deploymentJobs = selection.jobMode === "together" ? (enabled.length ? 1 : 0) : new Set(enabled.map(host => host.name)).size;
    const counts = site => site.hosts.filter(host => host.roles.some(role => selection.roles.includes(role))).length;
    content.innerHTML = `<div class="planner-inventory-meta"><div><span class="planner-environment-label">Environment</span><strong>${escapeHtml(snapshot.environment)}</strong><span title="Last inventory read">${escapeHtml(new Date(snapshot.readAt * 1000).toLocaleTimeString())}</span></div><label class="planner-job-mode"><span>Jobs</span><select data-job-mode aria-label="Deployment job grouping"><option value="per_host" ${selection.jobMode === "per_host" ? "selected" : ""}>Separate job per host</option><option value="together" ${selection.jobMode === "together" ? "selected" : ""}>All hosts in one job</option></select></label><button type="button" data-target-refresh ${loading ? "disabled" : ""}>${loading ? "Refreshing…" : "Refresh"}</button></div>
      ${message ? `<p class="planner-target-message is-error" role="alert">${escapeHtml(message)}</p>` : ""}
      ${snapshot.warnings.length ? `<details class="planner-inventory-warnings"><summary>Inventory notices (${snapshot.warnings.length})</summary>${snapshot.warnings.map(w => `<p>${escapeHtml(w)}</p>`).join("")}</details>` : ""}
      <div class="planner-target-pickers"><section><details class="planner-picker" data-picker="role" ${openPicker === "role" ? "open" : ""}><summary>Role <strong>${escapeHtml(selection.roles[0] || "Select role")}</strong></summary><div class="planner-picker-popover"><input type="search" data-role-search placeholder="Search roles…" aria-label="Search inventory roles" value="${escapeHtml(roleQuery)}"><div class="planner-target-options" data-role-options>${roles.filter(role => role.toLowerCase().includes(roleQuery.toLowerCase())).map(role => `<label><input type="radio" name="planner-target-role" data-target-role value="${escapeHtml(role)}" ${selection.roles.includes(role) ? "checked" : ""}><span>${escapeHtml(role)}</span></label>`).join("") || '<p>No matching roles.</p>'}</div></div></details></section>
      <section><details class="planner-picker" data-picker="site" ${openPicker === "site" ? "open" : ""}><summary>Sites <strong>${escapeHtml(selection.sites.length ? selection.sites.join(", ") : "Select sites")}</strong></summary><div class="planner-picker-popover"><input type="search" data-site-search placeholder="Search sites…" aria-label="Search inventory sites" value="${escapeHtml(siteQuery)}"><div class="planner-target-options">${snapshot.sites.filter(site => site.name.toLowerCase().includes(siteQuery.toLowerCase())).map(site => `<label><input type="checkbox" data-target-site value="${escapeHtml(site.name)}" ${selection.sites.includes(site.name) ? "checked" : ""} ${!counts(site) ? "disabled" : ""}><span>${escapeHtml(site.name)}</span><small>${counts(site)} hosts</small></label>`).join("") || '<p>No matching sites.</p>'}</div></div></details></section></div>
      <div class="planner-hosts-heading"><h3>Hosts <span>${enabled.length} / ${matches.length} selected</span><span class="planner-job-count">${deploymentJobs} ${deploymentJobs === 1 ? "job" : "jobs"} / step</span></h3>${matches.length ? '<button type="button" data-select-all-hosts>Select all</button>' : ""}</div>
      <div class="planner-host-table"><table><thead><tr><th aria-label="Selected"></th><th>Host</th><th>IP</th><th>Site</th><th>Role</th></tr></thead><tbody>${matches.map(host => `<tr><td><input type="checkbox" data-target-host value="${escapeHtml(host.key)}" aria-label="Include ${escapeHtml(host.name)}" ${selection.excluded.includes(host.key) ? "" : "checked"}></td><td>${escapeHtml(host.name)}</td><td>${escapeHtml(host.ip || "—")}</td><td>${escapeHtml(host.site)}</td><td>${escapeHtml(host.roles.join(", "))}</td></tr>`).join("") || '<tr><td colspan="5">Select a role and sites to preview the matching hosts.</td></tr>'}</tbody></table></div>
`;
    content.querySelector("[data-target-refresh]").addEventListener("click", refresh);
    content.querySelector("[data-job-mode]").addEventListener("change", event => { selection.jobMode = event.target.value; render(); });
    content.querySelectorAll("[data-picker]").forEach(picker => {
      picker.querySelector("summary").addEventListener("click", event => {
        event.preventDefault(); openPicker = openPicker === picker.dataset.picker ? null : picker.dataset.picker; render();
        content.querySelector(`[data-picker="${picker.dataset.picker}"] summary`).focus();
      });
      picker.addEventListener("keydown", event => {
        if (event.key === "Escape") { event.preventDefault(); openPicker = null; render(); content.querySelector(`[data-picker="${picker.dataset.picker}"] summary`).focus(); }
      });
    });
    for (const [selector, update] of [
      ["[data-role-search]", value => { roleQuery = value; }],
      ["[data-site-search]", value => { siteQuery = value; }],
    ]) {
      content.querySelector(selector).addEventListener("input", event => {
        const pos = event.target.selectionStart; update(event.target.value); render();
        const search = content.querySelector(selector); search.focus(); search.setSelectionRange(pos, pos);
      });
    }
    function updateList(field, value, checked) {
      selection[field] = field === "roles" ? (checked ? [value] : []) : checked ? [...new Set([...selection[field], value])] : selection[field].filter(v => v !== value);
      if (field === "roles") openPicker = null;
      if (field === "roles") selection.sites = selection.sites.filter(name => snapshot.sites.some(site => site.name === name && counts(site)));
      const keys = new Set(resolvePlannerTargets(snapshot, selection).map(host => host.key));
      selection.excluded = selection.excluded.filter(key => keys.has(key));
      render();
    }
    content.querySelectorAll("[data-target-role]").forEach(input => input.addEventListener("change", () => updateList("roles", input.value, input.checked)));
    content.querySelectorAll("[data-target-site]").forEach(input => input.addEventListener("change", () => updateList("sites", input.value, input.checked)));
    content.querySelectorAll("[data-target-host]").forEach(input => input.addEventListener("change", () => {
      selection.excluded = input.checked ? selection.excluded.filter(v => v !== input.value) : [...selection.excluded, input.value]; render();
    }));
    content.querySelector("[data-select-all-hosts]")?.addEventListener("click", () => { selection.excluded = []; render(); });
  }
  async function refresh() {
    const token = ++request; loading = false; message = "";
    if (!sourcePath || !draft.processId) { snapshot = null; render(); return; }
    loading = true; snapshot = null; render();
    try {
      const result = await loadInventory(draft.processId); if (!alive() || token !== request) return;
      snapshot = result;
      const selection = selected();
      if (selection.revision && selection.revision !== snapshot.revision) message = "Inventory changed since your previous preview. Review the selected hosts again.";
      const roles = new Set(snapshot.sites.flatMap(s => s.hosts.flatMap(h => h.roles)));
      selection.roles = selection.roles.filter(role => roles.has(role));
      selection.sites = selection.sites.filter(name => snapshot.sites.some(site => site.name === name && site.hosts.some(host => host.roles.some(role => selection.roles.includes(role)))));
      const keys = new Set(resolvePlannerTargets(snapshot, selection).map(h => h.key)); selection.excluded = selection.excluded.filter(key => keys.has(key));
    } catch (cause) { if (!alive() || token !== request) return; snapshot = null; message = String(cause); }
    finally { if (alive() && token === request) { loading = false; render(); } }
  }
  function folderChanged(settings) {
    settings = { ...settings, path: settings.path ?? null };
    if (sourcePath === settings.path) return;
    if (selected().sourcePath && selected().sourcePath !== settings.path) draft.targetSelection = { processId: draft.processId, jobMode: selected().jobMode, roles: [], sites: [], excluded: [], revision: null };
    sourcePath = settings.path; selected().sourcePath = sourcePath; void refresh();
  }
  void (async () => {
    try { const settings = await loadSettings(); if (alive()) folderChanged(settings); }
    catch (cause) { if (alive()) { sourcePath = null; message = `Cannot read local Ansible settings: ${cause}. Check Deployments → Config.`; render(); } }
  })();
  const outside = event => {
    if (!alive()) { document.removeEventListener("pointerdown", outside); return; }
    if (openPicker && !event.target.closest?.(".planner-picker")) { openPicker = null; root.querySelectorAll("[data-picker]").forEach(picker => { picker.open = false; }); }
  };
  document.addEventListener("pointerdown", outside);
  render();
  return { processChanged() {
    if (selected().processId !== draft.processId) { draft.targetSelection = { processId: draft.processId, jobMode: selected().jobMode, roles: [], sites: [], excluded: [], revision: null }; }
    void refresh();
  }, dispose() { disposed = true; request++; document.removeEventListener("pointerdown", outside); } };
}
