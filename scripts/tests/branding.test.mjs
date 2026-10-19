import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
const root = new URL("../../", import.meta.url);
const source = await readFile(new URL("web/shared/branding.js", root), "utf8");
const { APP_NAME, migrateLegacyPreferences } = await import(`data:text/javascript;base64,${Buffer.from(source).toString("base64")}`);

test("old settings migrate without replacing new settings or touching original GateTree", () => {
  const map = new Map([["pharos.workspace.lastPath", "/workspace"], ["pharos.appearance.theme", "linen"], ["gatetree-ops.appearance.theme", "ocean"], ["gatetree.setting", "separate"]]);
  const storage = { get length() { return map.size; }, key: i => [...map.keys()][i], getItem: k => map.get(k) ?? null, setItem: (k,v) => map.set(k,v) };
  migrateLegacyPreferences(storage);
  assert.equal(map.get("gatetree-ops.workspace.lastPath"), "/workspace");
  assert.equal(map.get("gatetree-ops.appearance.theme"), "ocean");
  assert.equal(map.get("gatetree.setting"), "separate");
  const before = [...map];
  migrateLegacyPreferences(storage);
  assert.deepEqual([...map], before);
});

test("GUI and installer use the same product name and independent application identity", async () => {
  const config = JSON.parse(await readFile(new URL("src-tauri/tauri.conf.json", root), "utf8"));
  assert.equal(config.productName, APP_NAME);
  assert.equal(config.app.windows[0].title, APP_NAME);
  assert.equal(config.identifier, "com.zsoltkarman.gatetreeops");
  assert.deepEqual(config.bundle.externalBin, ["binaries/gatetree-ops-agent"]);
});
