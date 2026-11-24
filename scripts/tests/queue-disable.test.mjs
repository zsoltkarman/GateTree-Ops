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

test('parallel region rows allow approvals and lane-specific retries without bypassing global or same-region dependencies',()=>{
 const a={...job,id:'a',region:'a',status:'failed'};const b={...job,id:'b',region:'b',status:'awaiting_approval'};
 const plan={parallelRegions:3,status:'running',jobs:[a,b]};
 assert(!/ disabled[ >]/.test(button(jobMarkup(plan,b,1),'approve')));
 b.status='failed';plan.status='failed';
 assert(!/ disabled[ >]/.test(button(jobMarkup(plan,b,1),'restart')));
 b.region='a';assert.match(button(jobMarkup(plan,b,1),'restart'),/ disabled>/);
 b.region='b';a.region=null;assert.match(button(jobMarkup(plan,b,1),'restart'),/ disabled>/);
});


test("OLAM monitoring failure offers status recheck instead of deployment restart", () => {
  const item = { ...job, status: "failed", action: "validate_deploy", olamMonitoringRetry: true };
  const plan = { jobs: [item], status: "failed", parallelRegions: 1 };
  let html = jobMarkup(plan, item, 0);
  assert.match(button(html, "recheck_olam"), /aria-label="Recheck OLAM status \(no new deployment\)"/);
  assert(!/ disabled[ >]/.test(button(html, "recheck_olam")));
  assert.equal(button(html, "restart"), undefined);
  assert.match(html, /queue-status is-warning">unknown<\/span>/);
  html = jobMarkup({ ...plan, archived: true }, item, 0);
  assert.match(button(html, "recheck_olam"), / disabled>/);
  const failed = { ...item, olamMonitoringRetry: false };
  assert.equal(button(jobMarkup({ ...plan, jobs: [failed] }, failed, 0), "recheck_olam"), undefined);
  const blocked = { ...plan, jobs: [{ ...job, status: "queued" }, item] };
  assert.match(button(jobMarkup(blocked, item, 1), "recheck_olam"), / disabled>/);
});
