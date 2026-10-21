export function parseExtraVarsText(text, index) {
  const result = Object.create(null);
  for (const [lineIndex, line] of text.split(/\r?\n/).entries()) {
    if (!line.trim() || line.trimStart().startsWith("#")) continue;
    const match = line.match(/^\s*(?:-\s+)?([^:=]+?)\s*[:=](.*)$/);
    if (!match || !match[1].trim()) throw new Error(`Step ${index + 1}, line ${lineIndex + 1}: use NAME: value or NAME=value.`);
    const key = match[1].trim(); let value = match[2].trim();
    if (Object.hasOwn(result, key)) throw new Error(`Step ${index + 1}: duplicate variable “${key}”.`);
    if (value.startsWith('"') && value.endsWith('"') && value.length >= 2) {
      try { value = JSON.parse(value); } catch (_) { value = value.slice(1, -1); }
    } else if (value.startsWith("'") && value.endsWith("'") && value.length >= 2) value = value.slice(1, -1).replaceAll("''", "'");
    result[key] = String(value);
  }
  return result;
}

export function stepExtraVars(step, index) {
  if (typeof step.extraVarsText === "string") return parseExtraVarsText(step.extraVarsText, index);
  const result = Object.create(null);
  for (const variable of step.variables ?? []) {
    const key = variable.key.trim(); const raw = String(variable.value ?? "");
    if (!key && !raw.trim()) continue;
    if (!key) throw new Error(`Step ${index + 1}: every variable needs a name.`);
    if (Object.hasOwn(result, key)) throw new Error(`Step ${index + 1}: duplicate variable “${key}”.`);
    result[key] = raw;
  }
  return result;
}

export function buildPlannerPlan(draft, processes, accounts) {
  const name = draft.planName.trim(); if (!name) throw new Error("Enter a Plan Name.");
  const process = processes.find(item => item.id === draft.processId); if (!process) throw new Error("Select a Process.");
  const account = accounts.find(item => item.id === draft.accountId); if (!account) throw new Error("Select an Account.");
  const jira = draft.jiraNo.trim(); const cx = draft.cxNo.trim();
  if (cx && !/^[0-9]{6}-[0-9]{6}$/.test(cx)) throw new Error("CX No. must use the format 000000-000000.");
  let config;
  try { config = JSON.parse(process.definition); } catch (_) { throw new Error("The selected Process config is invalid. Check Deployments → Config."); }
  if (!["linear", "free"].includes(draft.strategy)) throw new Error("Select an execution strategy.");
  const targets = draft.olamTargets;
  if (!targets || targets.roles.length !== 1) throw new Error("Select one role under Deployment targets.");
  if (targets.job_mode !== undefined && !["per_host", "together"].includes(targets.job_mode)) throw new Error("Select a deployment job grouping.");
  const hosts = targets.hosts ?? []; const groups = targets.inventory_groups ?? [];
  if ((!hosts.length && !groups.length) || (hosts.length && groups.length)) throw new Error("Select at least one deployment target.");
  if (!draft.steps?.length) throw new Error("Add at least one deployment step.");
  const steps = draft.steps.map((step, index) => {
    if (!step.name.trim()) throw new Error(`Enter a name for Step ${index + 1}.`);
    const extra = stepExtraVars(step, index);
    return { name: step.name.trim(), ...(Object.keys(extra).length ? { extra_vars: extra } : {}) };
  });
  const inputs = {
    ...(jira ? { jira: { issue: jira } } : {}),
    ...(cx ? { cx: { request: cx } } : {}),
    [config.steps?.some(step => step.driver === "container") ? "ansible" : "olam"]: { strategy: draft.strategy, account: account.name, targets, steps },
  };
  return { plan_name: name, process: process.name, inputs };
}

// All values use JSON quoting, which is valid YAML and preserves literal strings,
// leading zeroes, colons and multiline text.
export function plannerPlanYaml(plan) {
  const q = JSON.stringify; const key = plan.inputs.ansible ? "ansible" : "olam"; const olam = plan.inputs[key];
  const lines = [`plan_name: ${q(plan.plan_name)}`, `process: ${q(plan.process)}`, "inputs:"];
  if (plan.inputs.jira) lines.push("  jira:", `    issue: ${q(plan.inputs.jira.issue)}`);
  if (plan.inputs.cx) lines.push("  cx:", `    request: ${q(plan.inputs.cx.request)}`);
  lines.push(`  ${key}:`, `    strategy: ${q(olam.strategy)}`, `    account: ${q(olam.account)}`, "    targets:");
  for (const [key, value] of Object.entries(olam.targets)) if (Array.isArray(value) ? value.length : value != null) lines.push(`      ${key}: ${q(value)}`);
  lines.push("    steps:");
  for (const step of olam.steps) {
    lines.push(`      - name: ${q(step.name)}`);
    if (step.extra_vars) lines.push(`        extra_vars: ${q(step.extra_vars)}`);
  }
  return `${lines.join("\n")}\n`;
}
