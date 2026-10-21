import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const source = await readFile(new URL("../../web/modules/quick-launcher/quick-launcher-model.js", import.meta.url), "utf8");
const { visibleLauncherCount, moveLauncherItem } = await import(`data:text/javascript;base64,${Buffer.from(source).toString("base64")}`);

test("launcher reserves overflow space, keeps order, and recovers all items on wider windows", () => {
  const widths = [80, 100, 65, 90];
  assert.equal(visibleLauncherCount(widths, 347), 4);
  assert.equal(visibleLauncherCount(widths, 346), 3);
  assert.equal(visibleLauncherCount(widths, 210), 1);
  assert.equal(visibleLauncherCount(widths, 30), 0);
  assert.equal(visibleLauncherCount(widths, 1000), 4);
  assert.equal(visibleLauncherCount([], 0), 0);
});

test("launcher reorder handles both drop sides, end drops, and stale ids without mutating inputs", () => {
  const ids = ["a", "b", "c", "d"];
  assert.deepEqual(moveLauncherItem(ids, "d", "b"), ["a", "d", "b", "c"]);
  assert.deepEqual(moveLauncherItem(ids, "a", "c", true), ["b", "c", "a", "d"]);
  assert.deepEqual(moveLauncherItem(ids, "a", null), ["b", "c", "d", "a"]);
  assert.deepEqual(moveLauncherItem(ids, "a", "a"), ids);
  assert.deepEqual(moveLauncherItem(ids, "missing", "c"), ids);
  assert.deepEqual(moveLauncherItem(ids, "a", "missing"), ids);
  assert.deepEqual(ids, ["a", "b", "c", "d"]);
});

test("an empty launcher renders its icon after workspace unlock", async () => {
  const view = (await readFile(new URL("../../web/modules/quick-launcher/quick-launcher-view.js", import.meta.url), "utf8"))
    .replace(/^import .*;\n/gm, "").replace("export function mountQuickLauncher", "function mountQuickLauncher");
  const icons = await readFile(new URL("../../web/shared/connection-icons.js", import.meta.url), "utf8");
  const { connectionIcon } = await import(`data:text/javascript;base64,${Buffer.from(icons).toString("base64")}`);
  const messages = [], clicks = new Map(), navigation = [];
  const host = { innerHTML: "", classList: { remove() {} }, addEventListener() {},
    querySelectorAll: () => [], querySelector: selector => selector === ".quick-launch-empty" ? { addEventListener: (event, fn) => clicks.set(event, fn) } : null };
  const target = { addEventListener() {} };
  const create = new Function("connectionIcon", "listQuickLaunchItems", "window", "document", "ResizeObserver", "requestAnimationFrame", "cancelAnimationFrame", `${view}\nreturn mountQuickLauncher;`);
  const mount = create(connectionIcon, async () => [], target, target, class { observe() {} disconnect() {} }, () => 1, () => {});
  const dispose = mount(host, { showMessage: (...args) => messages.push(args), onNavigate: (...args) => navigation.push(args) });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(messages, []);
  assert.match(host.innerHTML, /<svg/);
  assert.match(host.innerHTML, /Quick Launcher/);
  clicks.get("click")();
  assert.deepEqual(navigation, [["connections"]]);
  dispose();
});
