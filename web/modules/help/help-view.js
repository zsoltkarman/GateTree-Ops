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
  ["job_id", "Deployment", "OLAM or local container deployment job ID."],
  ["validation_job_id", "Deployment", "OLAM or local container validation job ID, if available."],
  ["job_url", "Deployment", "Link to OLAM output; empty for local container jobs."],
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
const containerProcess = {
  "name": "Container deployment example",
  "details": {
    "container": {
      "engine": "auto",
      "image": "registry.example.test/ansible-deployer:your-version",
      "repository": "local_ansible_folder",
      "deployment_playbook": "playbooks/deploy_role.yml",
      "role_variable": "ROLE_NAME",
      "vault_password_file": "~/.ansible_vault_pass",
      "payload_defaults": {
        "ENVIRONMENT": "staging",
        "ansible_ssh_common_args": "-q -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ServerAliveInterval=50 -o ServerAliveCountMax=3"
      }
    }
  },
  "steps": [
    {
      "name": "Deploy Ansible",
      "driver": "container",
      "action": "deploy",
      "workflow": [
        "validate",
        "approve",
        "deploy"
      ]
    }
  ]
};

const reports = { container: containerProcess, plan: planReport, deployment: deploymentReport, cxPlan: cxPlanReport, cxDeployment: cxDeploymentReport };

function markup() {
  return `<article class="report-help-document">
    <header><p class="report-help-breadcrumb">Help / Deployments / Config</p><h1>Deployment Config</h1><p>Configure the report in a Jira or CX <code>update</code> step under <code>report</code>. Connection settings belong in <code>details.jira</code> or <code>details.cx</code>.</p></header>
    <section><h2>Local container Ansible</h2>
      <p>Use the same Planner and select a Process with <code>driver: "container"</code>. The image is read from this Process config, never from a hardcoded version. Docker or Podman must be running and able to pull the image (log in to the registry first if required).</p>
      <p>Choose the local project containing the configured deployment playbook under Deployments / Ansible / Config. The Agent mounts it read-only at <code>/workspace</code>. Planner Account supplies SSH and become credentials. Ansible Vault uses the password file already inside the image; <code>~/</code> refers to the image user's home.</p>
      <p><code>ansible_config</code> is optional: omit it to retain the image environment and Ansible's normal config discovery. Optional <code>validation_playbook</code> overrides the built-in SSH ping and target preview. The selected workflow runs validation, pauses for approval, then deploys. Stop removes the running container; failed validation never deploys.</p>
      <p>Plans use <code>inputs.ansible</code>; existing <code>inputs.olam</code> Plans remain supported. Use the inventory layout <code>inventory/ENVIRONMENT/SITE</code>; existing legacy layouts remain supported. Host grouping and extra variables work as in OLAM.</p>
      <div class="report-help-code-heading"><strong>Container deployment example · container Process</strong><button type="button" data-copy-report="container">Copy config JSON</button></div>
      <pre>${escapeHtml(JSON.stringify(containerProcess, null, 2))}</pre>
      <p>Replace the example image, version, deployment playbook, role variable and Vault password file with values from your environment. The playbook must exist in the selected Local Ansible folder.</p>
    </section>
    <section><h2>Copy a completed Plan</h2>
      <p>Use <strong>Copy to new Plan</strong> in a completed Plan’s header to open a prefilled Planner. Jira, CX, account, role, steps, variables and Thanos query/version are copied. Choose the destination Process and new deployment targets, review the account and variables, then Save Plan. The destination Process supplies ENVIRONMENT. The copy gets a new Plan number and starts queued, with no inherited approvals or job results. Start it separately; the original Plan keeps its history.</p>
    </section>
    <section><h2>Saved validation queries</h2>
      <p>Under <strong>Deployments → Config → Thanos Queries</strong>, add a name and PromQL template. In a Planner Thanos step, select a saved query and enter an expected exporter version if its template requires one. Every query attempt uses the latest saved Config text, including retries and polls of running checks. Host, region and version values still come from that deployment. Completed jobs keep their results and logs; editing a query does not rerun Deploy. Deleting the referenced query or removing required placeholders fails the check. Legacy Plans with no identifiable saved query require selecting one again. Queries are saved in the encrypted workspace and its backups. Connection and polling settings remain in Process Config.</p>
    </section>
    <section><h2>Thanos Query steps</h2>
      <p>Choose <strong>Thanos Query</strong> in a Planner step and enter its name, PromQL query and optional <strong>Expected exporter version</strong>. Connection, timeout and polling interval belong to the selected Process Config under <code>details.thanos</code>, not to each step.</p>
      <pre>${escapeHtml(JSON.stringify({thanos:{url:"https://thanos.example.test",auth:"none",timeout_seconds:300,interval_seconds:30}}, null, 2))}</pre>
      <p>Add the <code>thanos</code> object above inside the existing <code>details</code> object. For authenticated endpoints, use <code>auth: "basic"</code> or <code>"bearer"</code> and <code>account: "your-vault-reference"</code>. Credentials come from Vaults; do not put tokens in the URL or query.</p>
      <p>The query must return an instant scalar or vector with <strong>1 for success</strong>. All returned values must equal 1. Zero, other values, no data and partial-data warnings remain unverified and retry until the timeout. Authentication and invalid-query errors fail immediately. Stop cancels polling. The query is responsible for checking the intended hosts, freshness and expected version; a build-info metric alone is not an image-tag check. Use <code>\${DEPLOY_REGION}</code>, <code>\${EXPECTED_VERSION}</code> and <code>\${DEPLOY_HOST_REGEX}</code> inside double-quoted label values. The region comes from selected deployment sites; configure <code>details.thanos.region_map</code> when site names and metric regions differ (for example <code>{"west":"west_staging"}</code>). Expected version comes from the step field, not the image tag. Unknown or missing placeholders are rejected.</p>
      <p>Example: <code>vector(1)</code> tests connectivity and the success path; it does not validate a deployment. The Process must have exactly one deploy step when mixing Deploy and Thanos Query steps in Planner. With <strong>Separate job per host</strong>, each deployment is followed immediately by its Thanos checks, before the next host. Include <code>instance=~"\${DEPLOY_HOST_REGEX}"</code>: it matches that host with an optional scheme and port. The host’s site supplies its region. Zero/no data retries while later hosts remain queued; timeout or a fatal error fails the Plan. <strong>All hosts in one job</strong> keeps batch checks after the combined deployment. Sites with all hosts excluded are omitted. Existing saved Plans keep their frozen job order; recreate the Plan to use per-host checks. Query output is saved in Queue logs. Existing Jira/CX update steps can include the Plan log as evidence; adding a query does not add external writes automatically.</p>
    </section>
    <section><h2>Local Ansible folder</h2>
      <p>Choose the Ansible project folder under Deployments / Config. Select the project containing <code>inventory</code>, not an individual site. The path is saved on this computer and survives restart; it is not part of the shared Process or workspace backup.</p>
      <p>The Planner uses <code>details.container.payload_defaults.ENVIRONMENT</code> or <code>details.olam.payload_defaults.ENVIRONMENT</code> from the selected Process. Roles, sites, host names and IP addresses come from its static inventory. Refresh rereads the local files. This is a planning preview; OLAM validation remains the source for approval. Choose one role and one or more sites, review the hosts, then add deployment steps and their extra variables. Save Plan writes the Plan to Queue in Queued state; Start is a separate action. Jira and CX numbers are optional; without a ticket, that system’s validation and update jobs are omitted from the Plan. Enter extra variables in each step’s text area, one NAME: value or NAME=value per line. New or edited extra variables are stored as text. Unchanged copied variables retain their original types.</p>
    </section>
    <section><h2>OLAM Plan targets</h2>
      <p>In Planner Builder, <strong>Jobs</strong> selects <strong>Separate job per host</strong> (default) or <strong>All hosts in one job</strong>. The selected mode is saved as <code>targets.job_mode: per_host</code> or <code>together</code>. Three hosts and two deployment steps produce six deployment jobs in Per host mode and two in Together mode. Together also works with a partial host selection.</p>
      <p>In Plan YAML, put deployment inputs under <code>inputs.olam</code>. Existing <code>targets.hosts</code> lists keep creating a separate Queue job per host, role and Plan step. To pass several inventory groups together to one OLAM job, use <code>targets.inventory_groups</code> instead:</p>
      <pre>targets:
  roles:
    - service_role
  inventory_groups:
    - group_one
    - group_two</pre>
      <p>The complete group list becomes <code>ansible_limit</code>. OLAM/Ansible resolves the machines from its inventory. Each role and Plan step still gets its own job and configured workflow; validation and deployment use the same group list. Use either <code>hosts</code> or <code>inventory_groups</code>, not both. Group names are explicit: <code>sites</code> does not automatically translate site labels into inventory group names.</p>
    </section>
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
      <p>The step reports preceding completed OLAM or local container deployments with a deployment job ID. Disabled rows are excluded. If a deployment fails, the linear Plan stops before a later report step.</p>
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
