import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const read = (path) => readFile(new URL(path, root), "utf8");
const moduleUrl = (source) => `data:text/javascript;base64,${Buffer.from(source).toString("base64")}`;
const previewUrl = moduleUrl(await read("web/modules/deployments/validation-preview.js"));
const { parseValidationPreview } = await import(previewUrl);
const escapeUrl = moduleUrl(`export const escapeHtml = value => String(value).replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"})[c]);`);
const dialogSource = (await read("web/modules/deployments/approval-dialog.js"))
  .replace("../../shared/ui.js", escapeUrl).replace("./validation-preview.js", previewUrl);
const { approvalDialogMarkup } = await import(moduleUrl(dialogSource));
const debug = (host, msg) => `ok: [${host}] => ${JSON.stringify({ msg })}\n`;
const output = debug("host-1", ["Deployment target", "Role name: kafka", "Environment: test", "Location: region-a", "Strategy: linear", "Running User: deploy", "Ansible ssh host: 192.0.2.10", "Ansible hostname: host-1", "IMAGE_VERSION: 2026.9.3", "Galaxy version: 0.12.251", "AF Docker host: registry.example:5000"])
  + debug("host-1", ["KAFKA_VERSION: 3.9.2", "JDK_VERSION: jdk-21.0.1"]);

test("preview arrays merge deployment settings and role variables by host", () => {
  const [target] = parseValidationPreview(output);
  assert.equal(target["Ansible hostname"], "host-1");
  assert.equal(target["Ansible ssh host"], "192.0.2.10");
  assert.equal(target.IMAGE_VERSION, "2026.9.3");
  assert.equal(target.KAFKA_VERSION, "3.9.2");
  assert.equal(target["AF Docker host"], "registry.example:5000");
});

test("escaped debug strings support wrapped values and older separate version tasks", () => {
  const log = debug("inventory-host", "Deployment target\\nRole name: kafka\\nAnsible ssh host: fe80::1\\nAnsible hostname: host-2\\nIMAGE_VERSION:\\n2026.9.3")
    + debug("inventory-host", ["Galaxy version: 0.12.251"]);
  const [target] = parseValidationPreview(log);
  assert.equal(target.IMAGE_VERSION, "2026.9.3");
  assert.equal(target["Ansible hostname"], "host-2");
  assert.equal(target["Ansible ssh host"], "fe80::1");
  assert.equal(target["Galaxy version"], "0.12.251");
});

test("multiple hosts retain their own addresses and versions", () => {
  const log = debug("b", ["Role name: kafka", "Ansible hostname: b", "Ansible ssh host: 10.0.0.2", "IMAGE_VERSION: v2"])
    + debug("a", ["Role name: kafka", "Ansible hostname: a", "Ansible ssh host: 10.0.0.1", "IMAGE_VERSION: v1"]);
  const targets = parseValidationPreview(log);
  assert.deepEqual(targets.map((host) => [host["Ansible hostname"], host["Ansible ssh host"], host.IMAGE_VERSION]), [["a", "10.0.0.1", "v1"], ["b", "10.0.0.2", "v2"]]);
  assert.deepEqual(parseValidationPreview('ok: [host] => {"msg": invalid}\n'), []);
});

const job = { id: "job", sequence: 1, name: "Kafka", target: "host-1", status: "awaiting_approval", log: output, payload: '{"IMAGE_VERSION":"2026.9.3"}' };
const plan = { id: "plan", planNumber: 2, name: "Deploy Kafka", jobs: [job] };

test("review displays validation, preview, payload, and both decisions, with escaped output", () => {
  const markup = approvalDialogMarkup(plan, { ...job, log: output + '<img src=x onerror="alert(1)">', payload: "<script>unsafe</script>" });
  for (const value of ["Deployment settings", "host-1", "192.0.2.10", "2026.9.3", "KAFKA_VERSION", "Validation output", "Deployment payload", 'data-decision="deny"', 'data-decision="approve"']) assert(markup.includes(value), value);
  assert(markup.includes("&lt;img"));
  assert(!markup.includes("<script>"));
  assert(markup.includes('<details open><summary>Validation output'));
  assert(approvalDialogMarkup(plan, { ...job, log: "" }).includes('data-decision="approve" disabled'));
});

const queueSource = await read("web/modules/deployments/queue-view.js");
const controlSource = queueSource.slice(queueSource.indexOf("async function controlItem("), queueSource.indexOf("function showQueueProgress("));
async function runDecision(decision, { stale = false, status = "awaiting_approval" } = {}) {
  const calls = { controls: [], unlocks: 0, reviews: 0, reads: 0 };
  const state = { plans: [], busyKey: null, message: "", error: false };
  const control = new Function("getDeploymentQueue", "reviewDeploymentApproval", "ensureAgentCredentials", "showQueueProgress", "controlDeploymentQueueItem", "notifyQueue", "render", `return ${controlSource}`)(
    async () => { calls.reads++; return { plans: [{ ...plan, jobs: [{ ...job, status, log: stale && calls.reads > 1 ? "new validation" : job.log }] }] }; },
    async () => { calls.reviews++; return decision; },
    async () => { calls.unlocks++; return true; },
    () => ({ close() {}, remove() {} }),
    async (...args) => { calls.controls.push(args); return { plans: [] }; },
    () => {}, () => {},
  );
  await control({}, state, "job", "job", "approve");
  assert.equal(state.busyKey, null);
  return { calls, state };
}

test("Not now never starts or denies a deployment", async () => {
  const { calls } = await runDecision(null);
  assert.equal(calls.controls.length, 0);
  assert.equal(calls.unlocks, 0);
});

test("Deny is sent to Agent without unlocking or launching deployment", async () => {
  const { calls } = await runDecision("deny");
  assert.deepEqual(calls.controls, [["job", "job", "deny"]]);
  assert.equal(calls.unlocks, 0);
});

test("Approve is sent only after review and session unlock", async () => {
  const { calls } = await runDecision("approve");
  assert.deepEqual(calls.controls, [["job", "job", "approve"]]);
  assert.equal(calls.unlocks, 1);
  assert.equal(calls.reviews, 1);
});

test("a changed validation result blocks both stale approval and stale denial", async () => {
  for (const decision of ["approve", "deny"]) {
    const { calls, state } = await runDecision(decision, { stale: true });
    assert.equal(calls.controls.length, 0);
    assert(state.error);
    assert.match(state.message, /validation changed/);
  }
});

test("a step no longer awaiting approval cannot be reviewed or approved", async () => {
  const { calls, state } = await runDecision("approve", { status: "running" });
  assert.equal(calls.controls.length, 0);
  assert.equal(calls.reviews, 0);
  assert(state.error);
});
