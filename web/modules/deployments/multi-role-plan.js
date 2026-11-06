import { buildPlannerPlan } from "./planner-plan.js";

export const emptyMultiRoleRow = (processId = "") => ({ id: crypto.randomUUID(), processId, extraVarsText: "", validationQueryId: "", query: "", expectedVersion: "" });
export const emptyMultiRoleDraft = () => ({ groupId: crypto.randomUUID(), planName: "", processId: "", accountId: "", jiraNo: "", cxNo: "", strategy: "linear", parallelRegions: 1, rows: [emptyMultiRoleRow()] });

export function buildMultiRolePlans(draft, processes, accounts, queries) {
  const name = draft.planName.trim();
  if (!name || name.length > 160) throw new Error("Enter a Plan group name (maximum 160 characters).");
  if (!draft.rows?.length || draft.rows.length > 100) throw new Error("Add between 1 and 100 roles.");
  const roles = new Set();
  return draft.rows.map((row, index) => {
    const role = row.olamTargets?.roles?.[0];
    if (!role || row.olamTargets.roles.length !== 1) throw new Error(`Role ${index + 1}: select one role and its deployment targets.`);
    if (roles.has(role)) throw new Error(`Role ${role} is already included. Select each role once.`);
    roles.add(role);
    const steps = [{ name: "Deploy", extraVarsText: row.extraVarsText ?? "" }];
    if (row.validationQueryId) {
      const query = queries.find(query => query.id === row.validationQueryId);
      if (!query) throw new Error(`Role ${role}: select an existing saved Thanos query.`);
      steps.push({ name: "Thanos Query", type: "thanos_query", validationQueryId: query.id, query: row.query || query.query, expectedVersion: row.expectedVersion ?? "" });
    }
    try {
      return buildPlannerPlan({ ...draft, planName: `${name} · ${role}`, steps, olamTargets: row.olamTargets }, processes, accounts, queries);
    } catch (error) { throw new Error(`Role ${role}: ${error.message}`); }
  });
}
