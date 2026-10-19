import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const rust = await readFile(new URL("src-tauri/agent-core/src/drivers/report.rs", root), "utf8");
const supported = [...rust.match(/const FIELDS:.*?= &\[([\s\S]*?)\];/)[1].matchAll(/"([a-z_]+)"/g)].map((m) => m[1]);
const source = (await readFile(new URL("web/modules/help/help-view.js", root), "utf8")).replace(/^import .*;\n/m, "");
const { fields, planReport, deploymentReport, cxPlanReport, cxDeploymentReport } = await import(`data:text/javascript;base64,${Buffer.from(source + "\nexport { fields, planReport, deploymentReport, cxPlanReport, cxDeploymentReport };").toString("base64")}`);

test("Help documents every supported report variable, without stale or duplicate fields", () => {
  assert.deepEqual(fields.map(([name]) => name).sort(), [...supported].sort());
});

test("copyable report steps use supported variables and explicit ZIP configuration", () => {
  for (const step of [planReport, deploymentReport, cxPlanReport, cxDeploymentReport]) {
    assert(["jira", "cx"].includes(step.driver));
    assert.equal(step.action, "update");
    assert.equal(step.report.attachments.length, 1);
    const attachment = step.report.attachments[0];
    assert.equal(attachment.compression, "zip");
    assert(attachment.filename_template.endsWith(".zip"));
    for (const [, name] of JSON.stringify(step).matchAll(/{{([^}]+)}}/g)) assert(supported.includes(name), name);
  }
  assert.equal(planReport.report.scope, "plan");
  assert.equal(planReport.report.attachments[0].source, "plan_log");
  assert(planReport.report.comment_template.includes("{{deployments}}"));
  assert(planReport.report.deployment_template.includes("{{job_id}}"));
  assert.equal(deploymentReport.report.scope, "deployment");
});

 test("CX help uses the same report contract and a CX request filename", () => {
  assert.equal(cxPlanReport.driver, "cx");
  assert.equal(cxPlanReport.requester, "operator@example.test");
  assert.equal(cxDeploymentReport.requester, "operator@example.test");
  assert(!Object.hasOwn(cxPlanReport.report, "requester"));
  assert.equal(cxPlanReport.report.scope, "plan");
  assert.equal(cxPlanReport.report.attachments[0].source, "plan_log");
  assert(cxPlanReport.report.attachments[0].filename_template.includes("{{request}}"));
  assert.equal(cxDeploymentReport.report.scope, "deployment");
});
