import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const source = await readFile(new URL("../../web/modules/search/search-index.js", import.meta.url), "utf8");
const { buildSearchIndex, searchEntries } = await import(`data:text/javascript;base64,${Buffer.from(source).toString("base64")}`);

const data = {
  connectionFolders: [{ id: "platform", name: "Platform", parentId: null }],
  connections: [{ id: "host", folderId: "platform", connectionType: "ssh", name: "Production DB", target: "db.test", username: "admin", port: 2222, tag: "critical, Budapest" }],
  notes: { folders: [{ id: "runbooks", name: "Runbooks" }], notes: [{ id: "note", folderId: "runbooks", title: "Helyreállítás", body: `${"Intro ".repeat(50)}Unique recovery instructions` }] },
  skills: { libraries: [{ id: "lib", name: "Local tools", path: "/skills" }], skills: [{ id: "skill", name: "Database backup", description: "Save snapshots", path: "/skills/backup" }], groups: [{ id: "ops", label: "Operations", skillIds: ["skill"] }] },
  skillDocuments: new Map([["skill", { content: "Full content contains incremental archiving" }]]),
  deployments: { processes: [{ id: "process", name: "Rollout", definition: '{"region":"Frankfurt"}' }] },
  queue: { plans: [{ id: "plan", planNumber: 42, name: "Nightly release", processName: "Rollout", status: "pending", definition: "", jobs: [{ name: "Deploy", target: "host-42", log: "Worker retry-marker" }] }] },
  vaults: { entries: [{ id: "vault", name: "DB credential", entryPath: "Production/Platform" }] },
  activities: [{ id: "activity", name: "Dashboard", target: "https://metrics.test", status: "open" }],
  sessions: [{ id: "session", name: "DB shell", connectionId: "host", username: "admin", host: "db.test", port: 2222, status: "connected" }],
};
const entries = buildSearchIndex(data);
const find = (query, category = "All") => searchEntries(entries, query, category);

test("connections match names, tags, hosts, users, ports and folder paths", () => {
  for (const query of ["Production", "critical", "db.test", "admin", "2222", "Platform", "ssh"]) {
    assert(find(query, "Connections").some((entry) => entry.detail.connectionId === "host"), query);
  }
  assert.deepEqual(find("critical", "Connections")[0].detail, { connectionId: "host" });
});

test("notes and full skill contents are searchable with match excerpts", () => {
  assert.equal(find("recovery", "Notes")[0].detail.noteId, "note");
  assert.match(find("recovery", "Notes")[0].excerpt, /Unique recovery instructions/);
  assert(find("recovery", "Notes")[0].excerpt.startsWith("…"));
  assert.equal(find("archiving", "Skills")[0].detail.skillId, "skill");
  assert.equal(find("Operations", "Skills")[0].detail.skillId, "skill");
});

test("case and accents are ignored and every word must match", () => {
  assert.equal(find("HELYREALLITAS", "Notes")[0].title, "Helyreállítás");
  assert.equal(find("production BUDAPEST", "Connections").length, 1);
  assert.equal(find("production missing-token", "Connections").length, 0);
  assert.equal(find("  ").length, 0);
});

test("exact and prefix names rank above tags and body-only matches", () => {
  const base = { category: "Notes", type: "Note", subtitle: "", keywords: "", body: "" };
  const ranked = searchEntries([
    { ...base, title: "Other", body: "backup" },
    { ...base, title: "Other", keywords: "backup" },
    { ...base, title: "Backup guide" },
    { ...base, title: "Backup" },
  ], "backup");
  assert.deepEqual(ranked.map((entry) => entry.title), ["Backup", "Backup guide", "Other", "Other"]);
  assert(ranked[2].keywords);
});

test("deployment definitions, job logs, vault paths and activity resolve to the right destination", () => {
  assert.deepEqual(find("Frankfurt")[0].detail, { section: "config", resourceId: "process" });
  assert.deepEqual(find("retry-marker")[0].detail, { section: "queue", planId: "plan" });
  assert.deepEqual(find("Production/Platform", "Vaults")[0].detail, { entryId: "vault" });
  assert.deepEqual(find("metrics.test")[0].detail, { activityId: "activity" });
  assert.deepEqual(find("DB shell")[0].detail, { connectionId: "host" });
});

test("folder and module results navigate without requiring a content match", () => {
  assert.deepEqual(find("Platform", "Folders")[0].detail, { folderId: "platform" });
  assert.deepEqual(find("Runbooks", "Folders")[0].detail, { folderId: "runbooks" });
  assert.deepEqual(find("Local tools", "Folders")[0].detail, { section: "config", libraryId: "lib" });
  assert.equal(find("Backup", "Modules")[0].module, "backup");
});

test("partial or empty snapshots are safe and folder cycles terminate", () => {
  assert.equal(buildSearchIndex({}).length, 7);
  const cyclic = buildSearchIndex({ connectionFolders: [{ id: "a", parentId: "b", name: "A" }, { id: "b", parentId: "a", name: "B" }] });
  assert.equal(cyclic.filter((entry) => entry.category === "Folders").length, 2);
});

test("compact rows and preview use separate folder, target and tag values", () => {
  const connection = find("critical", "Connections")[0];
  assert.equal(connection.location, "Platform");
  assert.equal(connection.target, "admin@db.test:2222");
  assert.equal(connection.tags, "critical, Budapest");
  assert.equal(connection.matchedBody, false);
  assert.equal(find("archiving", "Skills")[0].matchedBody, true);
  assert.equal(find("recovery", "Notes")[0].location, "Runbooks");
  assert.equal(find("Local tools", "Folders")[0].location, "/skills");
});
