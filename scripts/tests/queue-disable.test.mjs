import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const source = (await readFile(new URL("../../web/modules/deployments/queue-view.js", import.meta.url), "utf8"))
  .replace(/^import .*;\n/gm, "");
const helpers = `const escapeHtml = value => String(value ?? "").replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"})[c]);`;
const { jobMarkup } = await import(`data:text/javascript;base64,${Buffer.from(helpers + source + "\nexport { jobMarkup };").toString("base64")}`);
const job = { id: "job", sequence: 2, name: "Deploy", target: "host", driver: "olam", status: "queued", details: "Deployment pending" };
const button = (html, action) => html.match(new RegExp(`<button[^>]*data-control-action="${action}"[^>]*>`))?.[0];

test("disabled rows show enable and cannot be started", () => {
  const disabled = { ...job, status: "disabled" };
  const html = jobMarkup({ jobs: [disabled] }, disabled, 0);
  assert.match(html, /queue-job-row is-disabled/);
  assert.match(html, /Skipped during execution/);
  assert.match(button(html, "enable"), /aria-label="Enable Job"/);
  assert(!/ disabled[ >]/.test(button(html, "enable")));
  assert.match(button(html, "start"), / disabled>/);
});

test("a disabled predecessor allows the next queued job to start", () => {
  const html = jobMarkup({ jobs: [{ ...job, status: "disabled" }, job] }, job, 1);
  assert(!/ disabled[ >]/.test(button(html, "start")));
  assert(!/ disabled[ >]/.test(button(html, "disable")));
  const blocked = jobMarkup({ jobs: [{ ...job, status: "queued" }, job] }, job, 1);
  assert.match(button(blocked, "start"), / disabled>/);
});

test("active and completed jobs cannot be disabled", () => {
  for (const status of ["running", "awaiting_approval", "completed", "failed"]) {
    const item = { ...job, status };
    const html = jobMarkup({ jobs: [item] }, item, 0);
    assert.match(button(html, "disable"), / disabled>/);
  }
});
