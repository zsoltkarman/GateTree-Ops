import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const source = readFileSync(new URL('../../web/modules/deployments/planner-targets.js', import.meta.url), 'utf8').replace(/^import .*;\n/gm, '').replaceAll('export function ', 'function ');
const {resolvePlannerTargets: resolve, plannerTargetInput} = new Function(source + '\nreturn {resolvePlannerTargets, plannerTargetInput};')();
const snapshot = { sites: [
  { name: 'a', group: 'env_a', hosts: [{name:'one',ip:'192.0.2.1',roles:['service','other']},{name:'two',ip:'192.0.2.2',roles:['other']}] },
  { name: 'b', group: 'env_b', hosts: [{name:'three',ip:'192.0.2.3',roles:['service']}] },
] };
test('Planner targets intersect selected roles and sites and preserve real inventory groups', () => {
  const result=resolve(snapshot,{roles:['service'],sites:['a','b']});
  assert.deepEqual(result.map(h=>h.name),['one','three']);
  assert.deepEqual(result.map(h=>h.group),['env_a','env_b']);
  assert.deepEqual(result[0].roles,['service']);
  assert.equal(result[0].ip,'192.0.2.1');
  assert.deepEqual(resolve(snapshot,{roles:['service'],sites:['a']}).map(h=>h.name),['one']);
});
test('Planner does not expand an empty selection to all inventory hosts', () => {
  assert.equal(resolve(snapshot,{roles:[],sites:['a','b']}).length,0);
  assert.equal(resolve(snapshot,{roles:['service'],sites:[]}).length,0);
  assert.equal(resolve(snapshot,{roles:['unknown'],sites:['a']}).length,0);
});

test('together mode uses groups for full sites and keeps partial host lists together', () => {
  const selection={jobMode:'together',roles:['service'],sites:['a','b'],excluded:[]};
  assert.deepEqual(plannerTargetInput(snapshot,selection),{job_mode:'together',roles:['service'],sites:['a','b'],inventory_groups:['env_a','env_b']});
  selection.excluded=[JSON.stringify(['a','one'])];
  assert.deepEqual(plannerTargetInput(snapshot,selection),{job_mode:'together',roles:['service'],sites:['a','b'],hosts:['three']});
  selection.excluded.push(JSON.stringify(['b','three']));
  assert.deepEqual(plannerTargetInput(snapshot,selection).hosts,[]);
});
test('unavailable inventory cannot leave usable stale targets in the Builder draft', () => {
  assert.deepEqual(plannerTargetInput(null,{roles:['service'],sites:['a'],excluded:[]}),{job_mode:'per_host',roles:[],sites:[],hosts:[]});
  const withoutGroup=structuredClone(snapshot);withoutGroup.sites[0].group=null;
  assert.deepEqual(plannerTargetInput(withoutGroup,{roles:['service'],sites:['a'],excluded:[]}).hosts,['one']);
});

test('per-host mode is the default and always saves resolved hosts rather than site groups',()=>{
 const selection={roles:['service'],sites:['a','b'],excluded:[]};
 const targets=plannerTargetInput(snapshot,selection);
 assert.deepEqual(targets,{job_mode:'per_host',roles:['service'],sites:['a','b'],hosts:['one','three']});
 selection.jobMode='together';assert.deepEqual(plannerTargetInput(snapshot,selection).inventory_groups,['env_a','env_b']);
 selection.jobMode='per_host';assert.deepEqual(plannerTargetInput(snapshot,selection),targets);
});
