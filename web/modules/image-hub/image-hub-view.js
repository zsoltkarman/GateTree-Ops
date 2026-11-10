import { inventoryRepositories } from "./inventory.js";
import { escapeHtml } from "../../shared/ui.js";
import { createModuleRoot } from "../../shared/module-root.js";
import { imageHubUrl, imageHubRepositoryUrl } from "./url-template.js";
import { syncButtonLabel, inventorySyncButtonLabel, sourceProgressLabel } from "./sync-progress.js";
import { getImageHub, saveImageHubFolder, saveImageHubRepository, deleteImageHubFolder, deleteImageHubRepository, saveImageHubProject, deleteImageHubProject, syncImageHubProject, syncImageHubInventory, openImageHubReport, getImageHubAnsibleImages, getImageHubSecurityReport } from "./image-hub-api.js";

const types = { cloudlab: "CloudLab", scm: "SCM", gitlab: "GitLab", github: "GitHub" };
const projectTypes = { scm: "SCM", gitlab: "GitLab", cloudlab: "CloudLab" };
const e = escapeHtml;
const attr = value => e(String(value ?? "")).replaceAll('"', "&quot;").replaceAll("'", "&#39;");

export async function mountImageHubView(contentRoot, options = {}) {
  const root = createModuleRoot(contentRoot, options.navigationRoot);
  const state = { folders: [], repositories: [], projects: [], latestImages: [], config: {}, runtimeConfig: {}, syncStatuses: [], securityReports: [], securitySelection: null, securityPage: null, securityBusy: false, securityError: "", securityRevision: 0, expandedImages: new Set(), ansibleImages: [], ansibleStatus: "idle", ansibleError: "", artifactRequests: new Set(), projectId: options.projectId ?? null, syncReport: null, folderId: options.folderId ?? null,
    query: "", repoTypeFilter: "", inventorySort: "folder", repositoryId: options.repositoryId ?? null, expanded: new Set(), mode: "list", dirty: false, busy: false, message: "", error: false };
  const mountId = crypto.randomUUID();
  contentRoot.dataset.imageHubMountId = mountId;
  let syncPollTimer = null;
  let closeContextMenu = () => {};
  root.innerHTML = '<p class="image-hub-loading">Loading Image Hub…</p>';
  try {
    Object.assign(state, await getImageHub());
    if (!contentRoot.isConnected) return;
    if (state.repositoryId) {
      const repo = state.repositories.find(r => r.id === state.repositoryId);
      if (repo) { state.folderId = repo.folderId; state.projectId = repo.projectId ?? null; state.mode = "repository"; state.draft = { ...repo }; }
    }
    const selectedProject = state.projects.find(p => p.id === state.projectId);
    if (selectedProject) { state.folderId = selectedProject.folderId; state.expanded.add(selectedProject.id); }
    expandAncestors(state.folderId);
    render(); scheduleSyncPoll();
    if (state.mode === "list") {
      if (state.expandedImages.size) void loadAnsibleImages();
    }
  } catch (error) { root.innerHTML = `<p class="image-hub-message is-error" role="alert">${e(error)}</p>`; }

  function expandAncestors(id) {
    const visited = new Set();
    while (id && !visited.has(id)) {
      visited.add(id); state.expanded.add(id);
      id = state.folders.find(f => f.id === id)?.parentId;
    }
  }
  function folderPath(id) {
    const names = [], seen = new Set();
    while (id && !seen.has(id)) {
      seen.add(id);
      const folder = state.folders.find(f => f.id === id);
      if (!folder) break;
      names.unshift(folder.name); id = folder.parentId;
    }
    return ["Image Hub", ...names].join(" / ");
  }
  function folderOptions(selected, excluded = null) {
    const walk = (parent, depth = 0, seen = new Set()) => state.folders.filter(f => f.parentId === parent && f.id !== excluded && !seen.has(f.id)).map(f => {
      const next = new Set([...seen, f.id]);
      return `<option value="${e(f.id)}"${selected === f.id ? " selected" : ""}>${e("  ".repeat(depth) + f.name)}</option>${walk(f.id, depth + 1, next)}`;
    }).join("");
    return `<option value="">Image Hub (root)</option>${walk(null)}`;
  }
  function tree(parent = null, seen = new Set()) {
    const folders = state.folders.filter(f => f.parentId === parent && !seen.has(f.id)).sort((a,b) => a.name.localeCompare(b.name, undefined, {numeric:true,sensitivity:"base"}));
    const repos = state.repositories.filter(r => r.folderId === parent && !r.projectId).sort((a,b) => a.name.localeCompare(b.name));
    const projects = state.projects.filter(p => p.folderId === parent).sort((a,b) => a.name.localeCompare(b.name));
    return folders.map(f => {
      const open = state.expanded.has(f.id), next = new Set([...seen, f.id]);
      return `<div class="image-hub-tree-folder">
        <div class="image-hub-folder-row${state.folderId === f.id ? " is-active" : ""}">
          <button data-hub-toggle="${e(f.id)}" aria-label="${open ? "Collapse" : "Expand"} ${e(f.name)}" aria-expanded="${open}" class="image-hub-toggle">${open ? "▾" : "▸"}</button>
          <button data-hub-folder="${e(f.id)}" title="${e(folderPath(f.id))}">${icon("folder")}<span>${e(f.name)}</span></button>
        </div>
        <div class="image-hub-tree-children"${open ? "" : " hidden"}>${tree(f.id, next)}</div>
      </div>`;
    }).join("") + projects.map(p => `<div class="image-hub-tree-project"><div class="image-hub-folder-row${state.projectId === p.id ? " is-active" : ""}">
      <button data-hub-project="${e(p.id)}" title="${e(p.name)}">${icon("project")}<span>${e(p.name)}</span></button>
    </div></div>`).join("") + repos.map(repositoryRow).join("");
  }
  function repositoryRow(r) {
    return `<button class="image-hub-repo-row${state.repositoryId === r.id ? " is-active" : ""}" data-hub-repository="${e(r.id)}" title="${e(r.localPath || r.name)}">${icon("repo")}<span>${e(r.name)}</span></button>`;
  }
  function repositoryForm() {
    const r = state.draft;
    return `<form class="image-hub-form" data-hub-form="repository">
      <h2>${r.id ? "Edit repository" : "Add repository"}</h2>
      <div class="image-hub-fields">
        <label>Name<input name="name" value="${e(r.name)}" required maxlength="240" autocomplete="off" autofocus></label>
        <label>Type<select name="repoType">${Object.entries(types).map(([key, label]) => `<option value="${key}"${r.repoType === key ? " selected" : ""}>${label}</option>`).join("")}</select></label>
        <label class="image-hub-wide">Folder<select name="folderId">${folderOptions(r.folderId)}</select></label>
        <label class="image-hub-wide">Project<select name="projectId"><option value="">No project</option>${state.projects.map(p => `<option value="${e(p.id)}"${r.projectId === p.id ? " selected" : ""}>${e(p.name)}</option>`).join("")}</select></label>
        <label class="image-hub-wide">Local path<span class="image-hub-path"><input name="localPath" value="${e(r.localPath)}" placeholder="/path/to/repository" autocomplete="off"><button type="button" data-hub-action="browse">Browse…</button></span></label>
        <label class="image-hub-wide">OCID<input name="ocid" value="${e(r.ocid)}" placeholder="ocid1.devopsrepository…" autocomplete="off" spellcheck="false"></label>
        <label class="image-hub-wide">Repository URL<input name="url" value="${e(r.url)}" placeholder="https://… or ssh://…" autocomplete="off" spellcheck="false"></label>
      </div>
      <p class="image-hub-hint">Name and type are required. Add the other details when available.</p>
      <div class="image-hub-form-actions"><button type="submit" class="connection-primary-action">Save repository</button><button type="button" data-hub-action="cancel">Cancel</button>${r.id ? '<button type="button" data-hub-action="delete-repository" class="image-hub-danger">Delete repository</button>' : ""}</div>
    </form>`;
  }
  function folderForm() {
    const f = state.draft;
    return `<form class="image-hub-form" data-hub-form="folder">
      <h2>${f.id ? "Edit folder" : "New folder"}</h2>
      <div class="image-hub-fields">
        <label class="image-hub-wide">Name<input name="name" value="${e(f.name)}" required maxlength="240" autocomplete="off" autofocus></label>
        <label class="image-hub-wide">Parent folder<select name="parentId">${folderOptions(f.parentId, f.id)}</select></label>
      </div>
      <div class="image-hub-form-actions"><button type="submit" class="connection-primary-action">Save folder</button><button type="button" data-hub-action="cancel">Cancel</button></div>
    </form>`;
  }
  function projectFolderForm() {
    const p = state.draft;
    return `<form class="image-hub-form" data-hub-form="project-folder">
      <h2>Move project: ${e(p.name)}</h2>
      <div class="image-hub-fields"><label class="image-hub-wide">Folder<select name="folderId">${folderOptions(p.folderId)}</select></label></div>
      <div class="image-hub-form-actions"><button type="submit" class="connection-primary-action">Move project</button><button type="button" data-hub-action="cancel">Cancel</button></div>
    </form>`;
  }
  function projectForm() {
    const p = state.draft;
    return `<form class="image-hub-form" data-hub-form="project">
      <h2>${p.id ? "Edit project" : "Add project"}</h2>
      <div class="image-hub-fields">
        <label>Name<input name="name" value="${e(p.name)}" required maxlength="240" autocomplete="off" autofocus></label>
        <label>Type<select name="projectType">${Object.entries(projectTypes).map(([key, label]) => `<option value="${key}"${p.projectType === key ? " selected" : ""}>${label}</option>`).join("")}</select></label>
        <label class="image-hub-wide">Folder<select name="folderId">${folderOptions(p.folderId)}</select></label>
        <label class="image-hub-wide">Local path<span class="image-hub-path"><input name="localPath" value="${e(p.localPath)}" required placeholder="/path/to/project" autocomplete="off"><button type="button" data-hub-action="browse">Browse…</button></span></label>
        <label class="image-hub-wide">Project URL / ID<input name="source" value="${e(p.source)}" placeholder="GitLab / CloudLab group URL or SCM project OCID" autocomplete="off" spellcheck="false"></label>
      </div>
      <p class="image-hub-hint">Saving discovers local repositories and clones missing remote repositories. SCM uses the existing OCI CLI session; GitLab and CloudLab use the host account configured in glab. For SCM, use the full project page URL to generate repository browser links automatically. Without a source, only local repositories are collected.</p>
      <div class="image-hub-form-actions"><button type="submit" class="connection-primary-action">Save and sync project</button><button type="button" data-hub-action="cancel">Cancel</button></div>
    </form>`;
  }
  function selectedSync() { return state.syncStatuses.find(status => status.projectId === state.projectId); }
  function syncRows(status) {
    return [["SCM","scm"],["SLAPS","slaps"],["AF","af"]].map(([label,key]) => {
      const source = status?.[key];
      return `<div class="image-hub-sync-source${source?.state === "error" ? " is-error" : ""}"><strong>${label}:</strong><span>${e(sourceProgressLabel(source))}</span>${source?.detail ? `<details><summary>Details</summary><p>${e(source.detail)}</p></details>` : ""}</div>`;
    }).join("") + (status?.error ? `<p class="image-hub-message is-error" role="alert">${e(status.error)}</p>` : "");
  }
  function syncResults() {
    if (state.mode !== "list") return "";
    if (state.projectId) return `<section class="image-hub-sync-status" data-hub-sync-status aria-label="Project sync status" aria-live="polite">${syncRows(selectedSync())}</section>`;
    if (options.manageProjects || !state.syncStatuses.length) return "";
    return `<section class="image-hub-sync-status" data-hub-sync-status aria-label="Inventory sync status" aria-live="polite">${inventorySyncRows()}</section>`;
  }
  function inventorySyncRows() {
    return [...state.projects].sort((a,b)=>a.name.localeCompare(b.name)).map(project => {
      const status = state.syncStatuses.find(status=>status.projectId===project.id);
      const partial = ["scm","slaps","af"].some(source=>status?.[source]?.state === "error");
      return `<details class="image-hub-inventory-sync"${status?.running || status?.error || partial ? " open" : ""}><summary>${e(project.name)}${status ? ` · ${status.running ? `${Math.max(0,Math.min(99,Number(status.percent)||0))}%` : status.error ? "Failed" : partial ? "Partial" : "Finished"}` : ""}</summary>${syncRows(status)}</details>`;
    }).join("");
  }
  function updateSyncWidgets() {
    const status = selectedSync();
    const footer = root.querySelector("[data-hub-sync-status]");
    if (footer) footer.innerHTML = state.projectId ? syncRows(status) : inventorySyncRows();
    const button = root.querySelector('[data-hub-action="sync-project"]');
    if (button) { button.textContent = syncButtonLabel(status); button.disabled = state.syncStatuses.some(status => status.running); }
    const allButton = root.querySelector('[data-hub-action="sync-inventory"]');
    if (allButton) { allButton.textContent = inventorySyncButtonLabel(state.syncStatuses,state.projects); allButton.disabled = !state.projects.length || state.syncStatuses.some(status=>status.running); }
  }
  function scheduleSyncPoll() {
    if (syncPollTimer || !state.syncStatuses.some(status => status.running)) return;
    syncPollTimer = window.setTimeout(async () => {
      syncPollTimer = null;
      if (!contentRoot.isConnected || contentRoot.dataset.imageHubMountId !== mountId || !root.querySelector(".image-hub-module")) return;
      try {
        const snapshot = await getImageHub();
        if (!contentRoot.isConnected || contentRoot.dataset.imageHubMountId !== mountId) return;
        const before = state.repositories.map(repo => repo.id).join(",");
        const wasRunning = state.syncStatuses.some(status => status.running);
        Object.assign(state, snapshot);
        const changed = before !== state.repositories.map(repo => repo.id).join(",");
        if (state.mode === "list" && (changed || !state.syncStatuses.some(status => status.running))) render();
        else { updateArtifactCells(); updateSyncWidgets(); if (state.mode === "security" && wasRunning && !state.syncStatuses.some(status => status.running)) void loadSecurityPage(state.securityPage?.page || 0); }
      } catch (error) {
        state.message = `Sync status could not be refreshed: ${String(error)}`; state.error = true;
        const message = root.querySelector(".image-hub-message");
        if (message) { message.textContent = state.message; message.hidden = false; message.classList.add("is-error"); }
        return;
      }
      scheduleSyncPoll();
    }, 800);
  }
  function list() {
    if (options.manageProjects) {
      const projects = [...state.projects].sort((a,b) => a.name.localeCompare(b.name,undefined,{numeric:true,sensitivity:"base"}));
      return `<div class="image-hub-list"><div class="image-hub-table-scroll"><table class="image-hub-table image-hub-project-table"><thead><tr><th>Project</th><th>Type</th><th>Local path</th><th>Actions</th></tr></thead><tbody>${projects.map(project => `<tr><td><strong>${e(project.name)}</strong><small>${e(folderPath(project.folderId))}</small></td><td>${e(projectTypes[project.projectType] ?? project.projectType)}</td><td><code>${e(project.localPath)}</code></td><td><div class="image-hub-toolbar"><button type="button" data-hub-action="edit-project" data-project-id="${e(project.id)}">Edit</button><button type="button" data-hub-action="move-project" data-project-id="${e(project.id)}">Move to folder…</button><button type="button" data-hub-action="sync-project" data-project-id="${e(project.id)}"${state.syncStatuses.some(status => status.running) ? " disabled" : ""}>Sync Project</button><button type="button" data-hub-action="delete-project" data-project-id="${e(project.id)}">Delete</button></div></td></tr>`).join("") || '<tr><td colspan="4">No projects yet. Add a project to collect its repositories.</td></tr>'}</tbody></table></div></div>`;
    }
    const repos = inventoryRepositories(state, { query: state.query, folderId: state.folderId, projectId: state.projectId, repoType: state.repoTypeFilter, sort: state.inventorySort });
    return `<div class="image-hub-list">
      <div class="image-hub-inventory-filters">
        <label class="image-hub-inventory-search">Search<input type="search" data-hub-filter="query" value="${e(state.query)}" placeholder="Repository, folder or project…" aria-label="Search inventory"></label>
        <label>Folder<select data-hub-filter="folder">${folderOptions(state.folderId).replace('Image Hub (root)', 'All folders')}</select></label>
        <label>Project<select data-hub-filter="project"><option value="">All projects</option>${[...state.projects].sort((a,b)=>a.name.localeCompare(b.name)).map(project=>`<option value="${e(project.id)}"${state.projectId===project.id?' selected':''}>${e(project.name)}</option>`).join("")}</select></label>
        <label>Type<select data-hub-filter="type"><option value="">All types</option>${Object.entries(types).map(([key,label])=>`<option value="${key}"${state.repoTypeFilter===key?' selected':''}>${label}</option>`).join("")}</select></label>
        <label>Sort<select data-hub-filter="sort"><option value="folder"${state.inventorySort==='folder'?' selected':''}>Folder / repository</option><option value="name"${state.inventorySort==='name'?' selected':''}>Repository name</option></select></label>
        <span class="image-hub-inventory-count" role="status">${repos.length} / ${state.repositories.length} repositories</span>
      </div>
      <div class="image-hub-table-scroll"><table class="image-hub-table image-hub-inventory-table">
        <thead><tr><th>Folder / Project</th><th>Repo name</th><th>Source</th><th>AF</th><th>SLAPS</th><th>Build Logs</th></tr></thead>
        <tbody>${repos.map(imageRows).join("") || '<tr><td colspan="6">No repositories match the current filters.</td></tr>'}</tbody>
      </table></div>
    </div>`;
  }
  function imageRows(repo) {
    const expanded = state.expandedImages.has(repo.id);
    const latest = `<tr class="image-hub-latest-row"><td class="image-hub-inventory-location"><span>${e(repo.inventoryFolder || "Root")}</span><small>${e(repo.inventoryProject || "")}</small></td><td><div class="image-hub-image-name"><button type="button" class="image-hub-image-toggle" title="Show or hide Ansible environments" data-hub-image-toggle="${e(repo.id)}" aria-expanded="${expanded}" aria-label="${expanded ? "Collapse" : "Expand"} Ansible environments for ${e(repo.name)}">${expanded ? "▾" : "▸"}</button><button type="button" data-hub-repository="${e(repo.id)}" title="Open repository">${e(repo.name)}</button></div></td><td>Latest</td><td data-hub-af="${e(repo.id)}">${artifactCell(repo)}</td><td data-hub-slaps="${e(repo.id)}">${reportLink(repo, "slaps")}</td><td>${reportLink(repo, "build")}</td></tr>`;
    return latest + (expanded ? ansibleRows(repo) : "");
  }
  function ansibleRows(repo) {
    const notice = text => `<tr class="image-hub-ansible-row"><td></td><td></td><td>Ansible</td><td colspan="3" class="image-hub-hint">${e(text)}</td></tr>`;
    if (state.ansibleStatus === "idle" || state.ansibleStatus === "loading") return notice("Loading Ansible versions…");
    if (state.ansibleStatus === "error") return notice(state.ansibleError);
    const exact = state.ansibleImages.filter(image => image.imageName === repo.name);
    const images = exact.length ? exact : state.ansibleImages.filter(image => image.role === repo.name || image.role === repo.name.replaceAll("-", "_"));
    if (!images.length) return notice("No matching image in ansible_dir.");
    return images.map(image => {
      const imageRepo = { ...repo, name: image.imageName };
      const version = ansibleVersionCell(repo, image);
      return `<tr class="image-hub-ansible-row"><td></td><td></td><td title="${e(image.imageName)} · ${e(image.role)} · ${e(image.source)}">Ansible · ${e(image.environment)}</td><td data-hub-ansible-af="${e(repo.id)}" data-hub-ansible-index="${state.ansibleImages.indexOf(image)}" title="${e(image.error || "Version configured in Ansible")}">${version}</td><td>${reportLink(imageRepo, "slaps", image.version || "")}</td><td>${reportLink(repo, "build", image.version || "")}</td></tr>`;
    }).join("");
  }
  function ansibleVersionCell(repo, image) {
    if (!image.version) return "—";
    const latest = state.latestImages.find(item => item.repositoryId === repo.id && item.imageName === image.imageName && item.status === "found" && item.version);
    const status = latest ? (latest.version === image.version ? "is-latest" : "is-outdated") : "";
    const title = latest ? (status === "is-latest" ? `Matches latest image: ${latest.version}` : `Differs from latest image: ${latest.version}`) : "Latest image version unavailable for comparison";
    const url = imageHubUrl(state.runtimeConfig.artifactPageUrlTemplate, state.runtimeConfig, { ...repo, name: image.imageName }, image.version);
    const version = url ? `<a class="image-hub-version-link" data-hub-report href="${e(url)}" target="_blank" rel="noopener noreferrer">${e(image.version)}</a>` : e(image.version);
    return `<span class="image-hub-ansible-version ${status}" title="${e(title)}">${version}</span>`;
  }
  async function loadAnsibleImages() {
    if (state.ansibleStatus === "loading" || state.ansibleStatus === "loaded") return;
    state.ansibleStatus = "loading";
    try {
      state.ansibleImages = await getImageHubAnsibleImages();
      state.ansibleStatus = "loaded";
    } catch (error) {
      state.ansibleStatus = "error"; state.ansibleError = String(error);
    }
    if (contentRoot.isConnected && state.mode === "list") render();
  }
  function reportLink(repo, kind, version = "") {
    const label = kind === "slaps" ? "SLAPS" : "Build Logs";
    const url = imageHubUrl(kind === "slaps" ? state.runtimeConfig.slapsUrlTemplate : state.runtimeConfig.buildLogsUrlTemplate, state.runtimeConfig, repo, version);
    if (kind === "slaps") {
      version ||= state.latestImages.find(image => image.repositoryId === repo.id && image.version && ["found","error"].includes(image.status))?.version || "";
      const scan = state.securityReports.find(scan => scan.projectId === repo.projectId && scan.imageName === repo.name && scan.version === version);
      const decision = scan?.decision || "not_downloaded";
      const findingCount = scan?.findingCount;
      const status = !scan?.error && !["pending", "error"].includes(decision) && Number.isInteger(findingCount) && findingCount >= 0
        ? (findingCount > 0 ? "has-findings" : "no-findings") : "not_downloaded";
      const findingsLabel = status === "not_downloaded" ? "" : ` · ${findingCount} findings`;
      const title = `${scan?.realm || state.runtimeConfig.slaps_realm || "SLAPS"} · ${decision}${findingsLabel} · ${scan?.error || (scan?.fetchedAt ? `Downloaded ${new Date(scan.fetchedAt * 1000).toLocaleString()}` : "Run Sync Project to download the report")}`;
      const data = `data-hub-security="${attr(repo.id)}" data-hub-security-image="${attr(repo.name)}" data-hub-security-version="${attr(version)}"`;
      const content = icon("shield");
      return url ? `<a class="image-hub-report-link image-hub-security-icon is-${attr(status)}" ${data} href="${attr(url)}" title="${attr(title)}" aria-label="View SLAPS findings for ${attr(repo.name)}">${content}</a>` : `<button type="button" class="image-hub-report-link image-hub-security-icon is-${attr(status)}" ${data} title="${attr(title)}" aria-label="View SLAPS findings for ${attr(repo.name)}">${content}</button>`;
    }
    if (!url) return `<span title="Configure ${label} URL in Image Hub → Config">—</span>`;
    return `<a class="image-hub-report-link" data-hub-report href="${e(url)}" target="_blank" rel="noopener noreferrer" title="${label}: ${e(repo.name)}${version ? ` · ${e(version)}` : ""}${!(kind === "slaps" ? state.runtimeConfig.slapsUrlTemplate : state.runtimeConfig.buildLogsUrlTemplate)?.includes("{version}") ? " · Repository overview" : ""}" aria-label="Open ${label} for ${e(repo.name)}">${icon(kind === "slaps" ? "shield" : "logs")}</a>`;
  }
  function artifactCell(repo) {
    if (!imageHubUrl(state.runtimeConfig.artifactFolderUrlTemplate, state.runtimeConfig, repo) || !state.runtimeConfig.artifactPageUrlTemplate) return '<span title="Configure artifact URLs in Image Hub → Config">—</span>';
    const image = state.latestImages.find(item => item.repositoryId === repo.id && item.imageName === repo.name);
    if (image?.status === "error" && image.version && image.url) {
      return `<a class="image-hub-version-link" data-hub-report href="${attr(image.url)}" target="_blank" rel="noopener noreferrer" title="${attr("Last known version. Latest lookup failed: " + (image.error || "Artifactory unavailable"))}">${e(image.version)}</a> <span class="image-hub-af-error" title="${attr(image.error || "Latest lookup failed")}">!</span>`;
    }
    if (image?.status === "found" && image.version && image.url) {
      return `<a class="image-hub-version-link" data-hub-report href="${e(image.url)}" target="_blank" rel="noopener noreferrer" title="${e(repo.name)}:${e(image.version)} · Checked ${e(new Date(image.checkedAt * 1000).toLocaleString())}">${e(image.version)}</a>`;
    }
    if (image?.status === "error") return `<span class="image-hub-af-error" title="${e(image.error || "Artifactory lookup failed")}">Error</span>`;
    if (state.syncStatuses.some(status => status.projectId === repo.projectId && status.running && status.af?.state === "running")) return '<span class="image-hub-hint">Loading…</span>';
    return `<span title="${image?.status === "not_found" ? "No versioned Docker image was found in the configured artifact repository." : "Image version has not been collected."}">—</span>`;
  }
  function updateArtifactCells() {
    if (!contentRoot.isConnected) return;
    root.querySelectorAll("[data-hub-af]").forEach(cell => {
      const repo = state.repositories.find(r => r.id === cell.dataset.hubAf);
      if (repo) cell.innerHTML = artifactCell(repo);
    });
    root.querySelectorAll("[data-hub-slaps]").forEach(cell => {
      const repo = state.repositories.find(repo => repo.id === cell.dataset.hubSlaps);
      if (repo) cell.innerHTML = reportLink(repo, "slaps");
    });
    root.querySelectorAll("[data-hub-ansible-af]").forEach(cell => {
      const repo = state.repositories.find(repo => repo.id === cell.dataset.hubAnsibleAf);
      const image = state.ansibleImages[Number(cell.dataset.hubAnsibleIndex)];
      if (repo && image) cell.innerHTML = ansibleVersionCell(repo, image);
    });
    bindReportLinks(); bindSecurityLinks();
  }
  function render() {
    const focusedFilter = root.querySelector("[data-hub-filter]:focus");
    const focusedKey = focusedFilter?.dataset.hubFilter;
    const selection = focusedKey === "query" ? focusedFilter.selectionStart : null;
    closeContextMenu();
    const selectedFolder = state.folders.find(f => f.id === state.folderId);
    const selectedProject = state.projects.find(p => p.id === state.projectId);
    root.innerHTML = `<div class="image-hub-module${options.manageProjects ? " is-project-settings" : options.navigationRoot ? " is-main-navigation-layout" : ""}">
      <aside class="image-hub-browser" aria-label="Image Hub inventory"${options.manageProjects ? " hidden" : ""}>
        <button class="image-hub-root${!state.folderId && !state.projectId && state.mode === "list" ? " is-active" : ""}" data-hub-folder="">${icon("repo")}<span>All repositories</span><small>${state.repositories.length}</small></button>
        <div class="image-hub-tree">${tree()}</div>
      </aside>
      <section class="image-hub-workspace">
        <header class="image-hub-header"><div><h1>${options.manageProjects ? "Projects" : "Inventory"}</h1><p>${options.manageProjects ? "Project sources and local repository discovery" : selectedProject ? e(selectedProject.name) : selectedFolder ? e(folderPath(state.folderId)) : "All repositories across every folder and project"}</p></div>
          ${options.manageProjects ? '<div class="image-hub-toolbar"><button type="button" data-hub-action="add-project">+ Add project</button></div>' : selectedProject ? `<div class="image-hub-toolbar"><button data-hub-action="sync-project"${state.syncStatuses.some(status => status.running) ? " disabled" : ""}>${e(syncButtonLabel(selectedSync()))}</button></div>` : selectedFolder ? '<div class="image-hub-toolbar"><button data-hub-action="edit-folder">Edit folder</button><button data-hub-action="delete-folder">Delete folder</button></div>' : state.mode === "list" ? `<div class="image-hub-toolbar"><button type="button" data-hub-action="sync-inventory"${!state.projects.length || state.syncStatuses.some(status=>status.running) ? " disabled" : ""}>${e(inventorySyncButtonLabel(state.syncStatuses,state.projects))}</button></div>` : ""}
        </header>
        <div class="image-hub-message${state.error ? " is-error" : ""}" role="${state.error ? "alert" : "status"}"${state.message ? "" : " hidden"}>${e(state.message)}</div>
        ${state.mode === "security" ? securityView() : state.mode === "repository" ? repositoryForm() : state.mode === "folder" ? folderForm() : state.mode === "project" ? projectForm() : state.mode === "project-folder" ? projectFolderForm() : list()}
        ${syncResults()}
      </section>
    </div>`;
    bind();
    if (options.navigationRoot) root.mountNavigation(".image-hub-browser");
    root.querySelector("[autofocus]")?.focus();
    if (focusedKey) {
      const filter = root.querySelector(`[data-hub-filter="${focusedKey}"]`);
      filter?.focus({preventScroll:true});
      if (selection !== null) { try { filter?.setSelectionRange(selection,selection); } catch (_) {} }
    }
  }
  function discard() { return !state.dirty || window.confirm("Discard unsaved Image Hub changes?"); }
  function navigate(folderId, repositoryId = null, projectId = null) {
    if (!discard()) return;
    state.folderId = folderId; state.repositoryId = repositoryId; state.projectId = projectId; state.dirty = false; state.message = ""; state.syncReport = null;
    if (projectId) state.expanded.add(projectId);
    expandAncestors(folderId);
    state.mode = repositoryId ? "repository" : "list";
    if (repositoryId) { state.draft = { ...state.repositories.find(r => r.id === repositoryId) }; state.projectId = state.draft.projectId ?? null; if (state.projectId) state.expanded.add(state.projectId); }
    render();
    if (state.mode === "list") {
      if (state.expandedImages.size) void loadAnsibleImages();
    }
  }
  function newItem(mode, folderId = state.folderId, projectId = null) {
    if (!discard()) return;
    state.folderId = folderId;
    state.projectId = projectId;
    expandAncestors(folderId);
    state.mode = mode; state.repositoryId = null; state.dirty = false; state.message = "";
    state.draft = mode === "folder" ? { id: null, name: "", parentId: state.folderId }
      : mode === "project" ? { id: null, name: "", folderId, localPath: "", projectType: "scm", source: "" }
      : { id: null, name: "", folderId: state.folderId, projectId, localPath: "", ocid: "", url: "", repoType: state.projects.find(p => p.id === projectId)?.projectType ?? "scm" };
    render();
  }
  async function mutate(action, after, success) {
    if (state.busy) return;
    state.busy = true;
    const controls = [...root.querySelectorAll("button, input, select")];
    controls.forEach(b => { b.disabled = true; });
    try {
      const result = await action();
      state.dirty = false;
      Object.assign(state, await getImageHub());
      if (!contentRoot.isConnected) return;
      after(result); state.message = success; state.error = false; render(); scheduleSyncPoll();
      if (state.mode === "list") {
          if (state.expandedImages.size) void loadAnsibleImages();
      }
    } catch (error) {
      if (!contentRoot.isConnected) return;
      state.message = String(error); state.error = true;
      const message = root.querySelector(".image-hub-message");
      message.textContent = state.message; message.hidden = false; message.classList.add("is-error"); message.setAttribute("role", "alert");
    } finally { state.busy = false; controls.forEach(b => { b.disabled = false; }); }
  }
  function securityView() {
    const selection = state.securitySelection;
    const page = state.securityPage;
    const report = page?.report;
    const url = imageHubUrl(state.runtimeConfig.slapsUrlTemplate, state.runtimeConfig, {name: selection.imageName}, selection.version);
    const cell = value => `<td title="${attr(value || "—")}"><span>${e(value || "—")}</span></td>`;
    const rows = (page?.findings || []).map(finding => `<tr>${cell(finding.severity)}${cell(finding.findingId)}${cell(finding.package)}${cell(finding.installedVersion)}${cell(finding.fixedVersion)}${cell(finding.gatingDate)}</tr>`).join("");
    const currentPage = page?.page || 0;
    const totalPages = page ? Math.max(1, Math.ceil(page.total / page.pageSize)) : 1;
    const notice = state.securityError || (!selection.version ? "Run Sync Project to collect the image version first." : !page?.report && !state.securityBusy ? "No local report. Run Sync Project to download SLAPS reports." : report?.error || (report?.decision === "pending" ? "The scan report is not available yet." : ""));
    return `<section class="image-hub-security-view"><div class="image-hub-security-heading"><div><h2>SLAPS findings</h2><p title="${attr(selection.imageName + ":" + selection.version)}">${e(selection.imageName)}:${e(selection.version || "—")} · ${e(report?.realm || state.runtimeConfig.slaps_realm || "—")} · ${e(report?.decision || "not downloaded")}</p></div><div class="image-hub-toolbar"><button data-hub-action="security-back">Back</button>${url ? `<a class="image-hub-version-link" data-hub-report href="${attr(url)}" title="Open SLAPS in browser">Open SLAPS ↗</a>` : ""}</div></div>
      ${state.securityBusy ? '<p role="status">Loading findings…</p>' : ""}${notice ? `<p class="image-hub-message is-error" role="alert">${e(notice)}</p>` : ""}
      <div class="image-hub-security-pagination"><span>${page?.total ?? 0} findings · Page ${currentPage + 1} / ${totalPages}</span><div><button data-hub-action="security-prev"${state.securityBusy || currentPage === 0 ? " disabled" : ""}>Previous</button><button data-hub-action="security-next"${state.securityBusy || currentPage + 1 >= totalPages ? " disabled" : ""}>Next</button></div></div>
      <table class="image-hub-table image-hub-findings-table"><colgroup><col style="width:10%"><col style="width:22%"><col style="width:26%"><col style="width:13%"><col style="width:13%"><col style="width:16%"></colgroup><thead><tr><th>Severity</th><th>Issue</th><th>Package</th><th>Installed</th><th>Fixed</th><th>Gating</th></tr></thead><tbody>${rows || `<tr><td colspan="6">${state.securityBusy ? "Loading…" : report?.decision === "error" || !report ? "No findings loaded." : "No findings in the downloaded report."}</td></tr>`}</tbody></table>
      ${report?.scanTime ? `<p class="image-hub-hint">Scanned: ${e(report.scanTime)} · Downloaded: ${e(new Date(report.fetchedAt * 1000).toLocaleString())}</p>` : ""}
    </section>`;
  }
  async function loadSecurityPage(page = 0) {
    const selection = state.securitySelection;
    if (!selection?.version) return;
    const revision = ++state.securityRevision;
    state.securityBusy = true; state.securityError = ""; render();
    try {
      const pageSize = Math.max(1, Math.min(12, Math.floor(((contentRoot.clientHeight || 600) - 320) / 32)));
      const result = await getImageHubSecurityReport({...selection, page, pageSize});
      if (revision === state.securityRevision) state.securityPage = result;
    } catch (error) { if (revision === state.securityRevision) state.securityError = String(error); }
    finally { if (revision === state.securityRevision) { state.securityBusy = false; if (contentRoot.isConnected && state.mode === "security") render(); } }
  }
  function bindSecurityLinks() {
    root.querySelectorAll("[data-hub-security]").forEach(link => link.onclick = event => {
      event.preventDefault(); event.stopPropagation();
      if (!discard()) return;
      state.securitySelection = {repositoryId: link.dataset.hubSecurity, imageName: link.dataset.hubSecurityImage, version: link.dataset.hubSecurityVersion};
      state.securityPage = null; state.securityError = ""; state.mode = "security";
      render(); void loadSecurityPage();
    });
  }
  function bindReportLinks() {
    root.querySelectorAll("[data-hub-report]").forEach(link => {
      if (link.dataset.hubReportBound) return;
      link.dataset.hubReportBound = "true";
      link.addEventListener("click", async event => {
      event.preventDefault();
      try { await openImageHubReport(link.href); }
      catch (error) {
        const message = root.querySelector(".image-hub-message");
        if (message) { message.textContent = String(error); message.hidden = false; message.classList.add("is-error"); message.setAttribute("role", "alert"); }
      }
      });
    });
  }
  function bind() {
    bindReportLinks(); bindSecurityLinks();
    root.querySelectorAll("[data-hub-filter]").forEach(control => {
      const update = () => {
        const key = control.dataset.hubFilter;
        const position = key === "query" ? control.selectionStart : null;
        if (key === "query") state.query = control.value;
        else if (key === "folder") state.folderId = control.value || null;
        else if (key === "project") state.projectId = control.value || null;
        else if (key === "type") state.repoTypeFilter = control.value;
        else state.inventorySort = control.value;
        render();
        const next = root.querySelector(`[data-hub-filter="${key}"]`);
        next?.focus({preventScroll:true});
        if (position !== null) { try { next?.setSelectionRange(position,position); } catch (_) {} }
      };
      if (control.dataset.hubFilter === "query") control.oninput = update; else control.onchange = update;
    });
    root.querySelectorAll("[data-hub-image-toggle]").forEach(button => button.onclick = event => {
      event.preventDefault();
      event.stopPropagation();
      const id = button.dataset.hubImageToggle;
      if (state.expandedImages.has(id)) state.expandedImages.delete(id); else state.expandedImages.add(id);
      render();
      if (state.expandedImages.has(id)) void loadAnsibleImages();
    });
    for (const area of root.querySelectorAll(".image-hub-browser, .image-hub-workspace")) {
      area.addEventListener("contextmenu", event => {
        if (event.target.closest("input, textarea, select, [contenteditable]")) return;
        event.preventDefault();
        event.stopPropagation();
        if (state.busy) return;
        const folder = event.target.closest("[data-hub-folder], [data-hub-toggle]")
          ?? event.target.closest(".image-hub-folder-row")?.querySelector("[data-hub-folder]");
        const repo = event.target.closest("[data-hub-repository]")
          ?? event.target.closest("tr")?.querySelector("[data-hub-repository]");
        const projectButton = event.target.closest("[data-hub-project]")
          ?? event.target.closest(".image-hub-tree-project")?.querySelector("[data-hub-project]");
        const clickedRepo = repo ? state.repositories.find(r => r.id === repo.dataset.hubRepository) : null;
        const projectId = projectButton ? projectButton.dataset.hubProject : clickedRepo ? clickedRepo.projectId ?? null
          : area.classList.contains("image-hub-browser") || folder ? null : state.projectId;
        const selectedProject = state.projects.find(p => p.id === projectId);
        const folderId = selectedProject ? selectedProject.folderId : folder ? folder.dataset.hubFolder || folder.dataset.hubToggle || null
          : repo ? state.repositories.find(r => r.id === repo.dataset.hubRepository)?.folderId ?? null
          : area.classList.contains("image-hub-browser") ? null : state.folderId;
        openContextMenu(event, folderId, projectId, clickedRepo?.id ?? null);
      });
    }
    root.querySelectorAll("[data-hub-folder]").forEach(b => b.onclick = () => navigate(b.dataset.hubFolder || null));
    root.querySelectorAll("[data-hub-project]").forEach(b => b.onclick = () => {
      const p = state.projects.find(p => p.id === b.dataset.hubProject);
      navigate(p.folderId, null, p.id);
    });
    root.querySelectorAll("[data-hub-repository]").forEach(b => b.onclick = async event => {
      if (event.target.closest("[data-hub-image-toggle]")) return;
      event.preventDefault();
      const r = state.repositories.find(r => r.id === b.dataset.hubRepository);
      try {
        const url = imageHubRepositoryUrl(state.runtimeConfig, r);
        if (!url) {
          if (!state.runtimeConfig.repositoryUrlTemplate) throw new Error("Set the SCM project page URL under Image Hub → Config → Projects → Edit → Project URL / ID, then run Sync Project to collect repository OCIDs. Alternatively set a repository HTTP(S) URL or repositoryUrlTemplate. Use the ▸ arrow to expand Ansible versions.");
          if (state.runtimeConfig.repositoryUrlTemplate.includes("{ocid}") && !r?.ocid) throw new Error("This repository has no OCID yet. Run Sync Project with a configured SCM source, or set its Repository URL (HTTP/HTTPS). Use the ▸ arrow to expand Ansible versions.");
          throw new Error("The repository URL template could not be resolved. Check repositoryUrlTemplate and its placeholders in Image Hub → Config.");
        }
        await openImageHubReport(url);
      } catch (error) {
        const message = root.querySelector(".image-hub-message");
        if (message) { message.textContent = String(error); message.hidden = false; message.classList.add("is-error"); message.setAttribute("role", "alert"); }
      }
    });
    root.querySelectorAll("[data-hub-toggle]").forEach(b => b.onclick = () => {
      const id = b.dataset.hubToggle;
      if (state.expanded.has(id)) state.expanded.delete(id); else state.expanded.add(id);
      captureDraft(); render();
    });
    root.querySelectorAll("[data-hub-action]").forEach(b => b.onclick = async () => {
      if (options.manageProjects && b.dataset.projectId) state.projectId = b.dataset.projectId;
      switch (b.dataset.hubAction) {
        case "add-project": newItem("project", null); break;
        case "sync-inventory":
          if (!discard()) return;
          state.mode = "list"; state.dirty = false; state.ansibleStatus = "idle"; state.message = ""; state.error = false;
          await mutate(() => syncImageHubInventory(), statuses => {
            const ids = new Set(statuses.map(status=>status.projectId));
            state.syncStatuses = [...state.syncStatuses.filter(status=>!ids.has(status.projectId)), ...statuses];
          }, "");
          break;
        case "security-back": state.securityRevision++; navigate(state.folderId, null, state.projectId); break;
        case "security-prev": void loadSecurityPage((state.securityPage?.page || 0) - 1); break;
        case "security-next": void loadSecurityPage((state.securityPage?.page || 0) + 1); break;
        case "cancel": navigate(state.folderId, null, state.projectId); break;
        case "move-project":
          if (!discard()) return;
          state.mode = "project-folder"; state.repositoryId = null; state.dirty = false;
          state.draft = { ...state.projects.find(p => p.id === state.projectId) }; render(); break;
        case "edit-project":
          if (!options.manageProjects || !discard()) return;
          state.mode = "project"; state.repositoryId = null; state.dirty = false;
          state.draft = { ...state.projects.find(p => p.id === state.projectId) }; render(); break;
        case "sync-project":
          if (!discard()) return;
          state.mode = "list"; state.dirty = false;
          state.ansibleStatus = "idle";
          state.message = ""; state.error = false;
          await mutate(() => syncImageHubProject(state.projectId), status => {
            state.syncStatuses = [...state.syncStatuses.filter(item => item.projectId !== status.projectId), status];
            state.expanded.add(state.projectId);
          }, "");
          break;
        case "delete-project": {
          const p = state.projects.find(p => p.id === state.projectId);
          if (!discard() || !window.confirm(`Delete the empty project “${p.name}”? Local files will be kept.`)) return;
          await mutate(() => deleteImageHubProject(p.id), () => { state.projectId = null; state.repositoryId = null; state.mode = "list"; state.syncReport = null; }, "Project removed."); break;
        }
        case "edit-folder":
          if (!discard()) return;
          state.mode = "folder"; state.repositoryId = null; state.dirty = false;
          state.draft = { ...state.folders.find(f => f.id === state.folderId) }; render(); break;
        case "delete-folder": {
          const f = state.folders.find(f => f.id === state.folderId);
          if (!discard() || !window.confirm(`Delete the empty folder “${f.name}”?`)) return;
          await mutate(() => deleteImageHubFolder(f.id), () => { state.folderId = f.parentId; state.mode = "list"; state.repositoryId = null; }, "Folder deleted."); break;
        }
        case "delete-repository": {
          const r = state.repositories.find(r => r.id === state.draft.id);
          if (!window.confirm(`Delete “${r.name}” from Image Hub? Local files will be kept.`)) return;
          await mutate(() => deleteImageHubRepository(r.id), () => { state.repositoryId = null; state.mode = "list"; }, "Repository removed."); break;
        }
        case "browse":
          try {
            const path = await window.__TAURI__.dialog.open({ directory: true, multiple: false, title: "Choose repository folder" });
            if (path) { root.querySelector('[name="localPath"]').value = path; state.dirty = true; }
          } catch (error) {
            const message = root.querySelector(".image-hub-message"); message.textContent = String(error); message.hidden = false; message.classList.add("is-error");
          }
          break;
      }
    });
    const form = root.querySelector("[data-hub-form]");
    form?.addEventListener("input", () => { state.dirty = true; });
    form?.addEventListener("change", () => { state.dirty = true; });
    form?.addEventListener("submit", async event => {
      event.preventDefault();
      if (state.busy) return;
      captureDraft();
      const input = { ...state.draft };
      if (state.mode === "folder") {
        await mutate(() => saveImageHubFolder(input), id => { state.folderId = id; state.repositoryId = null; state.mode = "list"; expandAncestors(id); }, "Folder saved.");
      } else if (state.mode === "project-folder") {
        await mutate(() => saveImageHubProject(input), id => {
          state.projectId = id; state.folderId = input.folderId; state.repositoryId = null; state.mode = "list";
          expandAncestors(input.folderId); state.expanded.add(id);
        }, "Project moved.");
      } else if (state.mode === "project") {
        const message = root.querySelector(".image-hub-message");
        message.textContent = "Saving project…"; message.hidden = false;
        await mutate(async () => {
          const id = await saveImageHubProject(input);
          state.draft.id = id;
          let report;
          try { report = await syncImageHubProject(id); }
          catch (error) { throw new Error(`Project saved. Background sync could not start: ${String(error)}`); }
          return { id, report };
        }, ({ id, report }) => {
          state.projectId = id; state.folderId = input.folderId; state.repositoryId = null; state.mode = "list";
          if (report?.runId) state.syncStatuses = [...state.syncStatuses.filter(item => item.projectId !== id), report];
          else if (report?.remoteError) { state.message = report.remoteError; state.error = true; }
          expandAncestors(input.folderId); state.expanded.add(id);
        }, "");
      } else {
        await mutate(() => saveImageHubRepository(input), id => { const repo = state.repositories.find(r => r.id === id); state.repositoryId = id; state.folderId = repo.folderId; state.projectId = repo.projectId ?? null; state.mode = "list"; expandAncestors(repo.folderId); if (state.projectId) state.expanded.add(state.projectId); }, "Repository saved.");
      }
    });
    form?.addEventListener("keydown", event => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") { event.preventDefault(); form.requestSubmit(); }
    });
  }
  function openContextMenu(event, folderId, projectId = null, repositoryId = null) {
    closeContextMenu();
    const menu = document.createElement("div");
    menu.className = "connection-context-menu image-hub-context-menu";
    menu.setAttribute("role", "menu");
    menu.setAttribute("aria-label", `Add to ${folderPath(folderId)}`);
    menu.innerHTML = options.manageProjects ? `<button type="button" role="menuitem" data-hub-create="project">${icon("project")}<span>Add project</span></button>` : `${projectId ? "" : `<button type="button" role="menuitem" data-hub-create="folder">${icon("folder-plus")}<span>New folder</span></button>`}
      <button type="button" role="menuitem" data-hub-create="repository">${icon("repo")}<span>Add repository</span></button>
      ${repositoryId ? `<button type="button" role="menuitem" data-hub-edit-repository>${icon("repo")}<span>Edit repository</span></button>` : ""}`;
    if (projectId) menu.innerHTML += `<button type="button" role="menuitem" data-hub-move-project>${icon("folder")}<span>Move to folder…</span></button>`;
    root.querySelector(".image-hub-module").append(menu);
    const bounds = event.target.getBoundingClientRect();
    const x = event.clientX || bounds.left, y = event.clientY || bounds.bottom;
    menu.style.left = `${Math.max(8, Math.min(x, window.innerWidth - menu.offsetWidth - 8))}px`;
    menu.style.top = `${Math.max(8, Math.min(y, window.innerHeight - menu.offsetHeight - 8))}px`;
    const controller = new AbortController();
    const previousFocus = document.activeElement;
    const observer = new MutationObserver(() => {
      if (!menu.isConnected) closeContextMenu();
    });
    closeContextMenu = () => {
      controller.abort(); observer.disconnect(); menu.remove(); closeContextMenu = () => {};
    };
    observer.observe(document.body, { childList: true, subtree: true });
    window.addEventListener("pointerdown", event => { if (!menu.contains(event.target)) closeContextMenu(); }, { capture: true, signal: controller.signal });
    window.addEventListener("blur", () => closeContextMenu(), { signal: controller.signal });
    window.addEventListener("scroll", () => closeContextMenu(), { capture: true, signal: controller.signal });
    const items = [...menu.querySelectorAll("button")];
    menu.addEventListener("keydown", event => {
      if (event.key === "Escape" || event.key === "Tab") {
        closeContextMenu(); previousFocus?.focus();
        if (event.key === "Escape") event.preventDefault();
      } else if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
        event.preventDefault();
        const index = items.indexOf(document.activeElement);
        items[event.key === "Home" ? 0 : event.key === "End" ? items.length - 1
          : (index + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length].focus();
      }
    });
    items.forEach(button => button.addEventListener("click", () => {
      closeContextMenu();
      if (button.hasAttribute("data-hub-move-project")) {
        if (!discard()) return;
        state.projectId = projectId; state.mode = "project-folder"; state.repositoryId = null; state.dirty = false;
        state.draft = { ...state.projects.find(p => p.id === projectId) }; render();
      } else if (button.hasAttribute("data-hub-edit-repository")) navigate(folderId, repositoryId, projectId);
      else newItem(button.dataset.hubCreate, folderId, projectId);
    }));
    items[0].focus({ preventScroll: true });
  }
  function captureDraft() {
    const form = root.querySelector("[data-hub-form]");
    if (!form) return;
    for (const [key, value] of new FormData(form)) state.draft[key] = ["folderId", "parentId", "projectId"].includes(key) ? value || null : value;
  }
}

function icon(name) {
  const paths = {
    folder: "<path d='M3 7h7l2-2h9v14H3z'/>",
    "folder-plus": "<path d='M3 7h7l2-2h9v14H3zM15 10v6m-3-3h6'/>",
    repo: "<path d='M6 3h12v18l-6-3-6 3zM9 7h6M9 10h6'/>",
    project: "<path d='M3 6h7l2 2h9v12H3zM3 6V4h7l2 2'/>",
    shield: "<path d='m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6zM8.5 11.5l2.5 2.5 4.5-5'/>",
    logs: "<path d='M6 3h8l4 4v14H6zM14 3v5h4M9 11h6M9 14h6M9 17h4'/>",
    plus: "<path d='M12 5v14M5 12h14'/>",
  };
  return `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[name]}</svg>`;
}
