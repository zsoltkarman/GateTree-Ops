import { escapeHtml } from "../../shared/ui.js";

const fields = [
  ["issue", "Plan / deployment", "Jira issue key or CX request number for the current update."],
  ["request", "Plan / deployment", "CX request number from inputs.cx.request; empty for Jira updates."],
  ["plan_name", "Plan / deployment", "Plan name."],
  ["plan_number", "Plan / deployment", "Queue Plan number."],
  ["role", "Deployment", "Role from the deployment payload (playbook_name)."],
  ["step", "Deployment", "Deployment Queue row name, including the Plan step name."],
  ["target", "Deployment", "Selected target hostname or FQDN."],
  ["ip", "Deployment", "Ansible ssh host from validation output; empty when unavailable."],
  ["datetime", "Plan / deployment", "UTC deployment completion time; report generation time for the Plan summary. Older jobs may show unknown."],
  ["status", "Plan / deployment", "Deployment status; completed for the Plan report."],
  ["job_id", "Deployment", "OLAM deployment job ID."],
  ["validation_job_id", "Deployment", "OLAM validation job ID, if available."],
  ["job_url", "Deployment", "Link to the OLAM deployment output."],
  ["deployments", "Plan", "Summary rendered using deployment_template, once per completed deployment."],
];

const planReport = {
  name: "Update Jira", driver: "jira", action: "update",
  report: {
    scope: "plan",
    comment_template: "Plan #{{plan_number}}: {{plan_name}}\n\n{{deployments}}",
    deployment_template: "{{role}} · {{step}} · {{target}} / {{ip}} · {{status}} · Job #{{job_id}} · {{datetime}}",
    attachments: [{ source: "plan_log", compression: "zip", filename_template: "{{issue}}-plan-{{plan_number}}.zip" }],
  },
};
const deploymentReport = {
  name: "Update Jira", driver: "jira", action: "update",
  report: {
    scope: "deployment",
    comment_template: "Following role has been deployed:\nRole name: {{role}}\nTarget: {{target}} / {{ip}}\nDatetime: {{datetime}}\nStatus: {{status}}\nJOB ID: {{job_id}}\n{{job_url}}",
    attachments: [{ source: "deployment_log", compression: "zip", filename_template: "{{issue}}-{{target}}-{{job_id}}.zip" }],
  },
};

const cxPlanReport = {
  ...planReport, name: "Update CX", driver: "cx", requester: "operator@example.test",
  report: {
    ...planReport.report,
    attachments: [{ source: "plan_log", compression: "zip", filename_template: "{{request}}-plan-{{plan_number}}.zip" }],
  },
};
const cxDeploymentReport = {
  ...deploymentReport, name: "Update CX", driver: "cx", requester: "operator@example.test",
  report: {
    ...deploymentReport.report,
    attachments: [{ source: "deployment_log", compression: "zip", filename_template: "{{request}}-{{target}}-{{job_id}}.zip" }],
  },
};
const reports = { plan: planReport, deployment: deploymentReport, cxPlan: cxPlanReport, cxDeployment: cxDeploymentReport };

function markup() {
  return `<article class="report-help-document">
    <header><p class="report-help-breadcrumb">Help / Deployments / Config</p><h1>Deployment Config</h1><p>Configure the report in a Jira or CX <code>update</code> step under <code>report</code>. Connection settings belong in <code>details.jira</code> or <code>details.cx</code>.</p></header>
    <section><h2>One summary and one ZIP per Plan</h2>
      <p>Place this step after the deployments in Process Config <code>steps</code>. One deployment or many: the issue receives one summary comment and one ZIP attachment.</p>
      <div class="report-help-code-heading"><strong>Jira update step</strong><button type="button" data-copy-report="plan">Copy step JSON</button></div>
      <pre>${escapeHtml(JSON.stringify(planReport, null, 2))}</pre>
      <p>The ZIP contains one UTF-8 <code>.log</code> file, with a separate section for each preceding enabled job. <code>compression: "zip"</code> always compresses; the filename must end in <code>.zip</code>.</p>
    </section>
    <section><h2>CX: one summary and one ZIP per Plan</h2>
      <p>CX uses the same report templates and attachment options. Configure <code>details.cx.url</code> (the requests API base), <code>account</code> (KeePass username/password), and put <code>requester</code> alongside <code>action</code>/<code>report</code> in each CX update step. Priority: Plan <code>inputs.cx.requester</code>, then the update step, then the legacy <code>details.cx.requester</code> default. At least one nonempty value is required. The request number comes from <code>inputs.cx.request</code>. Existing CX certificate behavior is preserved; set <code>verify_tls_certificate: true</code> to require certificate verification.</p>
      <div class="report-help-code-heading"><strong>CX update step</strong><button type="button" data-copy-report="cxPlan">Copy step JSON</button></div>
      <pre>${escapeHtml(JSON.stringify(cxPlanReport, null, 2))}</pre>
      <p>The Agent uploads the ZIP, then posts the configured summary. It does not change the CX status, close the request, or post a comment for each deployment when using Plan scope.</p>
    </section>
    <section><h2>Available template variables</h2>
      <p>Click a variable to copy it. In a Plan report, use deployment variables inside <code>deployment_template</code>; <code>{{deployments}}</code> inserts that list into <code>comment_template</code>.</p>
      <div class="report-help-table"><table><thead><tr><th>Variable</th><th>Context</th><th>Value</th></tr></thead><tbody>
        ${fields.map(([name, scope, description]) => `<tr><td><button type="button" data-copy-field="${name}" aria-label="Copy {{${name}}}"><code>{{${name}}}</code></button></td><td>${escapeHtml(scope)}</td><td>${escapeHtml(description)}</td></tr>`).join("")}
      </tbody></table></div>
      <p>The same variables can be used in <code>filename_template</code>, using the context of the selected scope. An unknown variable causes a configuration error.</p>
    </section>
    <section><h2>Attachment options</h2>
      <div class="report-help-table"><table><thead><tr><th>Setting</th><th>Meaning</th></tr></thead><tbody>
        <tr><td><code>source: "plan_log"</code></td><td>Logs from preceding enabled Queue rows. Requires Plan scope.</td></tr>
        <tr><td><code>source: "deployment_log"</code></td><td>Selected deployment output. Requires deployment scope.</td></tr>
        <tr><td><code>source: "job_log"</code></td><td>Selected deployment row's log, including its validation. Requires deployment scope.</td></tr>
        <tr><td><code>compression: "zip"</code></td><td>ZIP attachment; use a .zip filename. Requires a ZIP-capable GUI and Agent.</td></tr>
        <tr><td><code>compression: "none"</code></td><td>Plain text attachment, also used when compression is omitted. Use a .log filename.</td></tr>
      </tbody></table></div>
      <p>For a comment only, omit <code>attachments</code>. For attachments only, omit <code>comment_template</code>. Configure at least one.</p>
    </section>
    <details><summary>Per-deployment reports</summary>
      <p><code>scope: "deployment"</code> creates a separate comment and attachment for each completed deployment. Use Plan scope when you want a single report.</p>
      <div class="report-help-code-heading"><strong>Per-deployment Jira step</strong><button type="button" data-copy-report="deployment">Copy step JSON</button></div>
      <pre>${escapeHtml(JSON.stringify(deploymentReport, null, 2))}</pre>
      <div class="report-help-code-heading"><strong>Per-deployment CX step</strong><button type="button" data-copy-report="cxDeployment">Copy step JSON</button></div>
      <pre>${escapeHtml(JSON.stringify(cxDeploymentReport, null, 2))}</pre>
    </details>
    <section><h2>When reports run</h2>
      <p>The step reports preceding completed deployments with an OLAM deployment job ID. Disabled rows are excluded. If a deployment fails, the linear Plan stops before a later report step.</p>
      <p>Process edits apply to existing Plans after a full Restart Plan, which also starts their deployment steps again. Retrying only the update job reuses confirmed writes (remote IDs or CX HTTP acknowledgements). An uncertain write result requires checking the ticket before retrying.</p>
    </section>
    <p class="report-help-copy-status" role="status" aria-live="polite"></p>
  </article>`;
}

async function copy(text, root) {
  if (navigator.clipboard?.writeText) {
    try { await navigator.clipboard.writeText(text); return; } catch (_) {}
  }
  const field = document.createElement("textarea");
  field.value = text;
  field.className = "report-help-copy-buffer";
  root.append(field);
  try {
    field.select();
    if (!document.execCommand("copy")) throw new Error("Could not copy to clipboard.");
  } finally { field.remove(); }
}

export function mountHelpView(root, detail = { module: "deployments", section: "config" }, { onNavigate = () => {} } = {}) {
  if (detail.module !== "deployments" || detail.section !== "config") {
    const deployments = detail.module === "deployments";
    root.innerHTML = `<div class="report-help-page"><article class="report-help-document">
      <header><h1>${deployments ? "Deployments Help" : "Help"}</h1><p>Select a topic from the Help menu.</p></header>
      <section><h2>${deployments ? "Config" : "Deployments"}</h2>
        <p>${deployments ? "Process Config steps, Jira / CX report templates, variables and compressed log attachments." : "Configuration and deployment report reference."}</p>
        <button type="button" data-open-help-topic>${deployments ? "Open Config Help" : "Open Deployments Help"}</button>
      </section></article></div>`;
    root.querySelector("[data-open-help-topic]").addEventListener("click", () => onNavigate(deployments ? { module: "deployments", section: "config" } : { module: "deployments" }));
    return;
  }
  root.innerHTML = `<div class="report-help-page">${markup()}</div>`;
  root.querySelectorAll("[data-copy-field], [data-copy-report]").forEach((button) => {
    button.addEventListener("click", async () => {
      const text = button.dataset.copyField
        ? `{{${button.dataset.copyField}}}`
        : JSON.stringify(reports[button.dataset.copyReport], null, 2);
      const status = root.querySelector(".report-help-copy-status");
      try {
        await copy(text, root);
        status.textContent = button.dataset.copyField ? `Copied ${text}` : "Step JSON copied. Paste it into the Process Config steps array.";
      } catch (error) { status.textContent = String(error); }
      button.focus();
    });
  });
}

export function showDeploymentHelp() {
  const existing = document.querySelector(".report-help-modal");
  if (existing) { existing.focus(); return; }
  const previousFocus = document.activeElement;
  const dialog = document.createElement("dialog");
  dialog.className = "report-help-modal";
  dialog.setAttribute("aria-labelledby", "report-help-title");
  dialog.innerHTML = `<header><h2 id="report-help-title">Process Config help</h2><button type="button" data-close-help aria-label="Close Help" title="Close Help"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18"/></svg></button></header><div data-help-content></div>`;
  document.body.append(dialog);
  mountHelpView(dialog.querySelector("[data-help-content]"));
  dialog.querySelector("[data-close-help]").addEventListener("click", () => dialog.close());
  dialog.addEventListener("close", () => {
    dialog.remove();
    if (previousFocus?.isConnected) previousFocus.focus();
  }, { once: true });
  dialog.showModal();
}
