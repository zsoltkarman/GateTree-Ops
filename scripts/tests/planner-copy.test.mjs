import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const copySource=readFileSync(new URL('../../web/modules/deployments/planner-copy.js',import.meta.url),'utf8').replaceAll('export function ','function ');
const {copiedPlannerDraft}=new Function(copySource+'\nreturn {copiedPlannerDraft};')();
const source=readFileSync(new URL('../../web/modules/deployments/planner-plan.js',import.meta.url),'utf8').replaceAll('export function ','function ');
const {buildPlannerPlan,plannerPlanYaml}=new Function(source+'\nreturn {buildPlannerPlan,plannerPlanYaml};')();
const sourcePlan=()=>({planNumber:29,accountId:'account',plan:{plan_name:'Exporter staging',process:'Deploy staging',inputs:{
 jira:{issue:'APP-42'},cx:{request:'000001-000002',requester:'operator@example.test'},olam:{account:'deployment',strategy:'linear',targets:{roles:['exporter'],sites:['west'],hosts:['old-host'],job_mode:'per_host'},steps:[
 {name:'Deploy',extra_vars:{VERSION:'2026.11.1',ENVIRONMENT:'staging',COUNT:3,OPTIONS:{enabled:true}}},
 {name:'Validate exporter',type:'thanos_query',query:'metric{region="${DEPLOY_REGION}",instance=~"${DEPLOY_HOST_REGEX}",version="${EXPECTED_VERSION}"}',expected_version:'1.97.0'}
 ]}}}});
test('completed Plan copy preserves tickets, account, role, extra variables and query but requires fresh environment targets',()=>{
 const source=sourcePlan();const untouched=JSON.stringify(source);const d=copiedPlannerDraft(source);
 assert.equal(d.jiraNo,'APP-42');assert.equal(d.cxNo,'000001-000002');assert.equal(d.accountId,'account');assert.equal(d.cxRequester,'operator@example.test');
 assert.equal(d.processId,'');assert.equal(d.olamTargets,undefined);assert.deepEqual(d.targetSelection.roles,['exporter']);assert.deepEqual(d.targetSelection.sites,[]);
 assert.equal(d.steps[1].expectedVersion,'1.97.0');assert.equal(d.steps[1].query,source.plan.inputs.olam.steps[1].query);
 assert.equal(d.approveAll,undefined);assert.equal(d.jobs,undefined);assert.ok(!d.steps[0].extraVarsText.includes('ENVIRONMENT'));
 const processes=[{id:'prod',name:'Deploy production',definition:JSON.stringify({steps:[{driver:'olam',action:'deploy'}],details:{thanos:{url:'https://metrics.example.test'}}})}];const accounts=[{id:'account',name:'deployment'}];
 assert.throws(()=>buildPlannerPlan(d,processes,accounts),/Select a Process/);
 d.processId='prod';d.olamTargets={job_mode:'per_host',roles:['exporter'],sites:['east'],hosts:['new-host'],host_sites:{'new-host':'east'}};
 const plan=buildPlannerPlan(d,processes,accounts);const yaml=plannerPlanYaml(plan);
 assert.equal(plan.process,'Deploy production');assert.equal(plan.inputs.olam.steps[0].extra_vars.COUNT,3);assert.deepEqual(plan.inputs.olam.steps[0].extra_vars.OPTIONS,{enabled:true});
 assert.ok(yaml.includes('requester: "operator@example.test"'));assert.ok(yaml.includes('new-host'));assert.ok(!yaml.includes('old-host'));assert.ok(!yaml.includes('ENVIRONMENT'));
 assert.equal(JSON.stringify(source),untouched);assert.notEqual(copiedPlannerDraft(source).steps[0].id,d.steps[0].id);
});
test('generic Ansible alias copies into the same reusable Planner draft',()=>{
 const source=sourcePlan();source.plan.inputs.ansible=source.plan.inputs.olam;delete source.plan.inputs.olam;
 const d=copiedPlannerDraft(source);assert.equal(d.steps.length,2);assert.deepEqual(d.targetSelection.roles,['exporter']);
});
