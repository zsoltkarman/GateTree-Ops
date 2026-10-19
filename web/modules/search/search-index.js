export const SEARCH_CATEGORIES = ["Connections", "Skills", "Notes", "Folders", "Deployments", "Vaults", "Agent", "Modules"];

const normalize = (value) => String(value ?? "").normalize("NFD").replace(/\p{M}/gu, "").toLocaleLowerCase();

export function buildSearchIndex(data) {
  const entries = [];
  const add = (category, type, item, title, subtitle, body, keywords, module, detail, metadata = {}) => entries.push({
    id: `${type}:${item.id}`, category, type, title: title || "Untitled", subtitle: subtitle || "",
    body: body || "", keywords: keywords || "", module, detail, location: category, ...metadata,
  });
  const folders = new Map((data.connectionFolders ?? []).map((folder) => [folder.id, folder]));
  const pathOf = (id) => {
    const names = [], seen = new Set();
    while (folders.has(id) && !seen.has(id)) {
      seen.add(id);
      const folder = folders.get(id);
      names.unshift(folder.name);
      id = folder.parentId;
    }
    return names.join(" / ");
  };
  for (const item of data.connections ?? []) {
    const target = `${item.username ? `${item.username}@` : ""}${item.target}${item.port ? `:${item.port}` : ""}`;
    add("Connections", item.connectionType.toUpperCase(), item, item.name,
      [target, pathOf(item.folderId), item.tag ? `Tags: ${item.tag}` : ""].filter(Boolean).join(" · "),
      "", item.tag, "connections", { connectionId: item.id }, { location: pathOf(item.folderId), target, tags: item.tag, connectionType: item.connectionType, icon: item.icon });
  }
  for (const folder of folders.values()) {
    add("Folders", "Connection folder", folder, folder.name, pathOf(folder.id), "", "", "connections", { folderId: folder.id }, { location: pathOf(folder.id) });
  }
  const noteFolders = new Map((data.notes?.folders ?? []).map((folder) => [folder.id, folder.name]));
  for (const note of data.notes?.notes ?? []) {
    add("Notes", "Note", note, note.title, noteFolders.get(note.folderId) || "Notes", note.body, "", "notes", { noteId: note.id }, { location: noteFolders.get(note.folderId) || "Notes" });
  }
  for (const folder of data.notes?.folders ?? []) {
    add("Folders", "Note folder", folder, folder.name, "Notes", "", "", "notes", { folderId: folder.id }, { location: `Notes / ${folder.name}` });
  }
  for (const skill of data.skills?.skills ?? []) {
    const groups = (data.skills.groups ?? []).filter((group) => group.skillIds.includes(skill.id)).map((group) => group.label);
    add("Skills", "Skill", skill, skill.name, [skill.description, ...groups, skill.path].filter(Boolean).join(" · "),
      data.skillDocuments?.get(skill.id)?.content, "", "ai-skills", { section: "skills", skillId: skill.id }, { location: skill.path, description: skill.description });
  }
  for (const library of data.skills?.libraries ?? []) {
    add("Folders", "Skill library", library, library.name, library.path, "", "", "ai-skills", { section: "config", libraryId: library.id }, { location: library.path });
  }
  for (const process of data.deployments?.processes ?? []) {
    add("Deployments", "Config", process, process.name, "Deployment configuration", process.definition, "", "deployments", { section: "config", resourceId: process.id }, { location: "Deployments / Config" });
  }
  for (const plan of data.queue?.plans ?? []) {
    add("Deployments", "Plan", plan, plan.name, `Plan #${plan.planNumber} · ${plan.processName} · ${plan.status}`,
      [plan.definition, ...(plan.jobs ?? []).map((job) => [job.name, job.target, job.details, job.log].filter(Boolean).join(" "))].join("\n"),
      "", "deployments", { section: "queue", planId: plan.id }, { location: "Deployments / Queue", description: `Plan #${plan.planNumber} · ${plan.processName}`, status: plan.status });
  }
  for (const entry of data.vaults?.entries ?? []) {
    add("Vaults", "Reference", entry, entry.name, entry.entryPath, "", "", "vaults", { entryId: entry.id }, { location: entry.entryPath });
  }
  for (const activity of data.activities ?? []) {
    add("Agent", "Activity", activity, activity.name, [activity.target, activity.status].filter(Boolean).join(" · "), "", "", "agent", { activityId: activity.id }, { location: "Agent / Activity", target: activity.target, status: activity.status });
  }
  for (const session of data.sessions ?? []) {
    add("Agent", "SSH session", session, session.name, `${session.username}@${session.host}:${session.port} · ${session.status}`, "", "", "connections", { connectionId: session.connectionId }, { location: "Agent / SSH", target: `${session.username}@${session.host}:${session.port}`, status: session.status });
  }
  for (const [id, title] of [["connections", "Connections"], ["notes", "Notes"], ["ai-skills", "AI Skills"], ["deployments", "Deployments"], ["vaults", "Vaults"], ["agent", "Agent"], ["backup", "Backup"]]) {
    add("Modules", "Module", { id }, title, "Open module", "", "", id, id === "ai-skills" ? { section: "skills" } : null);
  }
  return entries;
}

export function searchEntries(entries, query, category = "All") {
  const terms = normalize(query).trim().split(/\s+/u).filter(Boolean);
  if (!terms.length) return [];
  return entries.flatMap((entry) => {
    if (category !== "All" && entry.category !== category) return [];
    const fields = [entry.title, entry.keywords, entry.subtitle, entry.body, entry.type].map(normalize);
    if (!terms.every((term) => fields.some((field) => field.includes(term)))) return [];
    const score = terms.reduce((total, term) => total + (fields[0] === term ? 100 : fields[0].startsWith(term) ? 60 : fields[0].includes(term) ? 40 : fields[1].includes(term) ? 30 : fields[2].includes(term) ? 15 : 5), 0);
    const bodyTerm = terms.find((term) => fields[3].includes(term));
    const text = bodyTerm ? entry.body : entry.subtitle;
    const position = bodyTerm ? normalize(text).indexOf(bodyTerm) : 0;
    const start = Math.max(0, position - 45);
    const excerpt = `${start ? "…" : ""}${text.slice(start, start + 170).replace(/\s+/gu, " ")}${text.length > start + 170 ? "…" : ""}`;
    return [{ ...entry, score, excerpt, matchedBody: Boolean(bodyTerm) }];
  }).sort((a, b) => b.score - a.score || a.title.localeCompare(b.title));
}
