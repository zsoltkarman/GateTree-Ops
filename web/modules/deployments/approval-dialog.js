import { escapeHtml } from "../../shared/ui.js";
import { parseValidationPreview, PREVIEW_SETTINGS } from "./validation-preview.js";

export function approvalDialogMarkup(plan, job) {
  const targets = parseValidationPreview(job.log);
  const settings = PREVIEW_SETTINGS.map(([label, field]) => {
    const values = [...new Set(targets.map((target) => target[field]).filter(Boolean))];
    return values.length ? `<div><dt>${escapeHtml(label)}</dt><dd>${escapeHtml(values.join(", "))}</dd></div>` : "";
  }).join("");
  const knownFields = new Set([...PREVIEW_SETTINGS.map(([, field]) => field), "Ansible hostname", "Ansible ssh host", "IMAGE_VERSION"]);
  const variables = targets.map((target) => {
    const fields = Object.entries(target).filter(([field]) => !knownFields.has(field));
    return fields.length ? `<h4>${escapeHtml(target["Ansible hostname"])}</h4><dl>${fields.map(([field, value]) => `<div><dt>${escapeHtml(field)}</dt><dd>${escapeHtml(value)}</dd></div>`).join("")}</dl>` : "";
  }).join("");
  return `<header>
      <h2 id="queue-approval-title">Review validation</h2>
      <p>Plan #${escapeHtml(plan.planNumber)} · ${escapeHtml(plan.name)} · Step ${escapeHtml(job.sequence)}: ${escapeHtml(job.name)}</p>
      <p>Review this step before starting deployment. Deny stops the Plan.</p>
    </header>
    <div class="queue-approval-content">
      ${settings ? `<section><h3>Deployment settings</h3><dl>${settings}</dl></section>` : ""}
      <section><h3>Validation targets</h3>${targets.length ? `<div class="queue-approval-targets"><table><thead><tr><th>Hostname</th><th>SSH / IP</th><th>Image version</th></tr></thead><tbody>${targets.map((target) => `<tr><td>${escapeHtml(target["Ansible hostname"])}</td><td>${escapeHtml(target["Ansible ssh host"] || "—")}</td><td>${escapeHtml(target.IMAGE_VERSION || "—")}</td></tr>`).join("")}</tbody></table></div>` : `<p class="queue-approval-notice">No deployment preview block was found. Review the validation output for target ${escapeHtml(job.target || "—")}.</p>`}</section>
      ${variables ? `<details class="queue-approval-variables"><summary>Role variables</summary>${variables}</details>` : ""}
      <details open><summary>Validation output</summary><pre>${escapeHtml(job.log || "No validation output is available.")}</pre></details>
      <details><summary>Deployment payload</summary><pre>${escapeHtml(job.payload || "{}")}</pre></details>
    </div>
    <footer>
      <button type="button" data-decision="cancel">Not now</button>
      <span></span>
      <button type="button" class="queue-approval-deny" data-decision="deny">Deny</button>
      <button type="button" class="connection-primary-action" data-decision="approve" ${!job.log?.trim() ? "disabled" : ""}>Approve deployment</button>
    </footer>`;
}

export function reviewDeploymentApproval(root, plan, job) {
  return new Promise((resolve) => {
    const dialog = document.createElement("dialog");
    dialog.className = "queue-password-dialog queue-approval-dialog";
    dialog.setAttribute("aria-labelledby", "queue-approval-title");
    dialog.innerHTML = approvalDialogMarkup(plan, job);
    root.append(dialog);
    dialog.querySelectorAll("[data-decision]").forEach((button) => {
      button.addEventListener("click", () => dialog.close(button.dataset.decision));
    });
    // Navigation can remove the mounted Queue while a review is open.
    const observer = new MutationObserver(() => {
      if (!dialog.isConnected) finish(null);
    });
    let finished = false;
    function finish(decision) {
      if (finished) return;
      finished = true;
      observer.disconnect();
      dialog.remove();
      resolve(decision);
    }
    observer.observe(document.body, { childList: true, subtree: true });
    dialog.addEventListener("close", () => finish(["approve", "deny"].includes(dialog.returnValue) ? dialog.returnValue : null), { once: true });
    dialog.showModal();
    dialog.querySelector('[data-decision="cancel"]').focus();
  });
}
