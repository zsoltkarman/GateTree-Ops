// Convert the stored source Plan, never its completed jobs or resolved payloads.
export function copiedPlannerDraft({ plan, planNumber, accountId = "" }) {
  const deployment = plan.inputs?.olam ?? plan.inputs?.ansible ?? plan.inputs?.container;
  if (!deployment?.steps?.length) throw new Error("This Plan has no deployment steps to copy.");
  const roles = deployment.targets?.roles ?? [];
  if (roles.length !== 1) throw new Error("Planner Builder can copy Plans with one deployment role.");
  const jobMode = deployment.targets.job_mode ?? (deployment.targets.inventory_groups?.length ? "together" : "per_host");
  const steps = deployment.steps.map(step => {
    const type = step.type || "deploy";
    if (!["deploy", "thanos_query"].includes(type)) throw new Error("This Plan contains an unsupported Planner step type.");
    const copy = { id: crypto.randomUUID(), name: step.name, type, query: step.query ?? "", expectedVersion: step.expected_version ?? "", extraVarsText: "" };
    if (type === "thanos_query" && step.validation_query_id) copy.validationQueryId = step.validation_query_id;
    if (type === "deploy") {
      const extra = JSON.parse(JSON.stringify(step.extra_vars ?? {}));
      if (!extra || typeof extra !== "object" || Array.isArray(extra)) throw new Error("Deployment extra_vars must be a key-value object.");
      // The selected Process supplies the new environment; do not carry an old override.
      delete extra.ENVIRONMENT;
      const text = Object.entries(extra).map(([key,value]) => `${key}: ${JSON.stringify(value)}`).join("\n");
      copy.extraVarsText = text; copy.originalExtraVarsText = text; copy.originalExtraVars = extra;
    }
    return copy;
  });
  return {
    planName: `${plan.plan_name} · Copy`, processId: "", accountId,
    jiraNo: plan.inputs?.jira?.issue ?? "", cxNo: plan.inputs?.cx?.request ?? "",
    cxRequester: plan.inputs?.cx?.requester ?? "", strategy: deployment.strategy || "linear", parallelRegions: deployment.parallel_regions ?? 1, steps,
    copySource: { planNumber },
    targetSelection: { processId: "", jobMode, roles: [...roles], sites: [], excluded: [], revision: null },
  };
}
