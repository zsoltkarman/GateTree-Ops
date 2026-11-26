import { escapeHtml } from "../../shared/ui.js";
import { saveQuerySet, deleteQuerySet } from "./deployments-api.js";

export const emptyQuerySet = () => ({ id: "", name: "", revision: 0, items: [], stableSamples: 3, intervalSeconds: 30, timeoutSeconds: 1200, maxAgeSeconds: 120 });
const emptyItem = () => ({ queryId: "", scope: "cluster", expectedVersion: "", minResults: 1 });
export function mountQuerySets(root, { sets = [], queries = [], save = saveQuerySet, remove = deleteQuerySet } = {}) {
  let draft = null, busy = false, message = "", error = false;
  const alive = () => root.isConnected;
  const options = item => queries.map(q => `<option value="${escapeHtml(q.id)}"${q.id === item.queryId ? " selected" : ""}>${escapeHtml(q.name)}</option>`).join("");
  function render() {
    root.innerHTML = `<section class="validation-query-library query-set-library"><header><span>Thanos Query Sets <small>${sets.length}</small></span><button type="button" data-set-add ${busy ? "disabled" : ""}>+ Add Query Set</button></header>
      <div class="validation-query-content"><div class="validation-query-list">${sets.map(set => `<article><div><strong>${escapeHtml(set.name)}</strong><small>${set.items.length} required checks · ${set.stableSamples} consecutive rounds</small></div><nav><button data-set-edit="${escapeHtml(set.id)}" ${busy ? "disabled" : ""}>Edit</button><button data-set-delete="${escapeHtml(set.id)}" ${busy ? "disabled" : ""}>Delete</button></nav></article>`).join("")}</div>
      ${draft ? `<form data-set-form autocomplete="off"><fieldset ${busy ? "disabled" : ""}><label>Name<input data-set-field="name" value="${escapeHtml(draft.name)}" maxlength="160" required></label>
        <div class="query-set-policy">${[["stableSamples","Consecutive healthy rounds",1,100],["intervalSeconds","Interval (seconds)",1,86400],["timeoutSeconds","Timeout (seconds)",1,86400],["maxAgeSeconds","Maximum result age (seconds)",1,3600]].map(([key,label,min,max]) => `<label>${label}<input type="number" data-set-field="${key}" value="${draft[key]}" min="${min}" max="${max}" required></label>`).join("")}</div>
        <p>All checks must pass in every round. Node checks follow the deployed host; cluster checks cover its region. Missing results and query failures block progress.</p>
        <div class="query-set-items">${draft.items.map((item,i) => `<div data-set-item="${i}"><label>Saved query<select data-item-field="queryId" required><option value="">Select a query</option>${options(item)}</select></label><label>Scope<select data-item-field="scope"><option value="cluster"${item.scope === "cluster" ? " selected" : ""}>Cluster</option><option value="node"${item.scope === "node" ? " selected" : ""}>Current node</option></select></label><label>Expected version<input data-item-field="expectedVersion" value="${escapeHtml(item.expectedVersion)}" placeholder="If required by query"></label><label>Minimum results<input type="number" data-item-field="minResults" value="${item.minResults}" min="1" max="10000" required></label><button type="button" data-item-remove="${i}" aria-label="Remove check ${i+1}">Remove</button></div>`).join("")}</div>
        <button type="button" data-item-add ${draft.items.length >= 32 ? "disabled" : ""}>+ Add check</button>
        <p>Queries must return 1 for success and explicitly check expected broker coverage and source metric freshness. Maximum result age checks response timestamps; aggregations can hide the age of source samples.</p>
        <footer><button type="button" data-set-cancel>Cancel</button><button type="submit">${busy ? "Saving…" : "Save Query Set"}</button></footer></fieldset></form>` : ""}
      ${message ? `<p role="status" class="${error ? "is-error" : ""}">${escapeHtml(message)}</p>` : ""}</div></section>`;
    root.querySelector("[data-set-add]").addEventListener("click", () => { draft = emptyQuerySet(); draft.items.push(emptyItem()); message = ""; render(); });
    root.querySelectorAll("[data-set-edit]").forEach(button => button.addEventListener("click", () => { draft = structuredClone(sets.find(s => s.id === button.dataset.setEdit)); message = ""; render(); }));
    root.querySelectorAll("[data-set-delete]").forEach(button => button.addEventListener("click", () => action(async () => {
      await remove(button.dataset.setDelete); sets = sets.filter(s => s.id !== button.dataset.setDelete); if (draft?.id === button.dataset.setDelete) draft = null;
      message = "Query Set deleted. Existing Plans keep their membership and policy.";
    })));
    const form = root.querySelector("[data-set-form]");
    if (!form) return;
    form.querySelectorAll("[data-set-field]").forEach(input => input.addEventListener("input", () => { draft[input.dataset.setField] = input.type === "number" ? Number(input.value) : input.value; }));
    form.querySelectorAll("[data-set-item]").forEach(row => row.querySelectorAll("[data-item-field]").forEach(input => input.addEventListener("change", () => { draft.items[Number(row.dataset.setItem)][input.dataset.itemField] = input.type === "number" ? Number(input.value) : input.value; })));
    form.querySelectorAll("[data-item-remove]").forEach(button => button.addEventListener("click", () => { draft.items.splice(Number(button.dataset.itemRemove),1); render(); }));
    form.querySelector("[data-item-add]").addEventListener("click", () => { draft.items.push(emptyItem()); render(); });
    form.querySelector("[data-set-cancel]").addEventListener("click", () => { draft = null; message = ""; render(); });
    form.addEventListener("submit", event => { event.preventDefault(); if (busy || !form.reportValidity()) return; return action(async () => {
      const saved = await save(structuredClone(draft)); sets = [...sets.filter(s => s.id !== saved.id),saved].sort((a,b) => a.name.localeCompare(b.name)); draft = null;
      message = "Query Set saved. Select it in a new Planner step. Existing Plans keep their membership and policy.";
    }); });
  }
  async function action(run) {
    if (busy) return; busy = true; error = false; message = ""; render();
    try { await run(); } catch (e) { error = true; message = String(e); }
    finally { busy = false; if (alive()) render(); }
  }
  render();
  return { updateQueries(next) { queries = next; if (alive()) render(); } };
}
