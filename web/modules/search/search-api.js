import { listConnectionFolders, listConnectionItems } from "../connections/connections-api.js";
import { listNotes } from "../notes/notes-api.js";
import { getAiSkills, readAiSkill } from "../ai-skills/ai-skills-api.js";
import { getDeploymentConfig } from "../deployments/deployments-api.js";
import { getDeploymentQueue } from "../deployments/queue-api.js";
import { getVaults } from "../vaults/vaults-api.js";
import { listAgentActivity, listAgentSshSessions } from "../agent/agent-api.js";
import { buildSearchIndex } from "./search-index.js";

export async function loadSearchIndex({ onProgress = () => {}, signal } = {}) {
  const sources = [
    ["connectionFolders", "Connection folders", listConnectionFolders], ["connections", "Connections", listConnectionItems],
    ["notes", "Notes", listNotes], ["skills", "Skills", getAiSkills],
    ["deployments", "Deployment config", getDeploymentConfig], ["queue", "Deployment queue", getDeploymentQueue],
    ["vaults", "Vault references", getVaults], ["activities", "Agent activity", listAgentActivity],
    ["sessions", "SSH sessions", listAgentSshSessions],
  ];
  const settled = await Promise.allSettled(sources.map(([, , read]) => Promise.resolve().then(read)));
  if (signal?.aborted) return;
  const data = { skillDocuments: new Map() }, unavailable = [];
  settled.forEach((result, index) => {
    if (result.status === "fulfilled") data[sources[index][0]] = result.value;
    else unavailable.push(sources[index][1]);
  });
  onProgress({ entries: buildSearchIndex(data), unavailable, loading: Boolean(data.skills?.skills.length) });
  const pending = [...(data.skills?.skills ?? [])];
  await Promise.all(Array.from({ length: Math.min(4, pending.length) }, async () => {
    while (pending.length && !signal?.aborted) {
      const skill = pending.shift();
      try {
        data.skillDocuments.set(skill.id, await readAiSkill(skill.id));
      } catch {
        if (!unavailable.includes("Some skill contents")) unavailable.push("Some skill contents");
      }
    }
  }));
  if (!signal?.aborted) onProgress({ entries: buildSearchIndex(data), unavailable, loading: false });
}
