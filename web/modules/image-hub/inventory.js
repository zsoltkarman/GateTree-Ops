const compare = (a, b) => String(a ?? "").localeCompare(String(b ?? ""), undefined, { numeric: true, sensitivity: "base" });

export function inventoryFolderPath(folders, id) {
  const names = [], seen = new Set();
  while (id && !seen.has(id)) {
    seen.add(id);
    const folder = folders.find(folder => folder.id === id);
    if (!folder) break;
    names.unshift(folder.name); id = folder.parentId;
  }
  return names.join(" / ");
}

export function inventoryRepositories(snapshot, { query = "", folderId = null, projectId = null, repoType = "", sort = "folder" } = {}) {
  const folders = snapshot.folders ?? [], projects = snapshot.projects ?? [];
  const terms = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  const rows = (snapshot.repositories ?? []).map(repository => {
    const project = projects.find(project => project.id === repository.projectId);
    const id = repository.folderId ?? project?.folderId ?? null;
    return { ...repository, inventoryFolderId: id, inventoryFolder: inventoryFolderPath(folders, id) || "Root", inventoryProject: project?.name || "" };
  }).filter(repository => {
    if (projectId && repository.projectId !== projectId || repoType && repository.repoType !== repoType) return false;
    if (folderId) {
      let id = repository.inventoryFolderId; const seen = new Set();
      while (id && id !== folderId && !seen.has(id)) { seen.add(id); id = folders.find(folder => folder.id === id)?.parentId; }
      if (id !== folderId) return false;
    }
    const text = [repository.name, repository.inventoryFolder, repository.inventoryProject, repository.localPath, repository.repoType, repository.url, repository.ocid].join(" ").toLocaleLowerCase();
    return terms.every(term => text.includes(term));
  });
  return rows.sort((a, b) => sort === "name"
    ? compare(a.name, b.name) || compare(a.inventoryFolder, b.inventoryFolder) || compare(a.inventoryProject, b.inventoryProject) || compare(a.id,b.id)
    : compare(a.inventoryFolder,b.inventoryFolder) || compare(a.inventoryProject,b.inventoryProject) || compare(a.name,b.name) || compare(a.id,b.id));
}
