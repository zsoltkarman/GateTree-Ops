import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const plannerSource=readFileSync(new URL('../../web/modules/deployments/planner-plan.js',import.meta.url),'utf8').replaceAll('export function ','function ');
const {buildPlannerPlan}=new Function(plannerSource+'\nreturn {buildPlannerPlan};')();
const source=readFileSync(new URL('../../web/modules/deployments/multi-role-plan.js',import.meta.url),'utf8').replace(/^import .*;\n/gm,'').replaceAll('export ','');
const {buildMultiRolePlans}=new Function('buildPlannerPlan',source+'\nreturn {buildMultiRolePlans};')(buildPlannerPlan);
const processes=[{id:'p',name:'Deploy staging',definition:JSON.stringify({steps:[{driver:'olam',action:'deploy'}],details:{thanos:{url:'https://metrics.example.test'}}})}];
const accounts=[{id:'a',name:'deployment'}];
const query='metric{region="${DEPLOY_REGION}",instance=~"${DEPLOY_HOST_REGEX}",version="${DEPLOY_IMAGE_VERSION}"}';
const queries=[{id:'check',name:'Version',query}];
const row=(role,host)=>({extraVarsText:'',validationQueryId:'',query:'',olamTargets:{job_mode:'per_host',roles:[role],sites:['west'],hosts:[host],host_sites:{[host]:'west'}}});
const draft=()=>({planName:'November staging',processId:'p',accountId:'a',jiraNo:'APP-1',cxNo:'000001-000002',strategy:'linear',parallelRegions:3,rows:[row('collector','consumer'),row('maintainer','redis')]});
test('multi-role Plans share tickets and account while keeping targets, variables and optional queries isolated',()=>{
 const d=draft();d.rows[0].extraVarsText='IMAGE_VERSION: 2026.11.1';d.rows[1].extraVarsText='IMAGE_VERSION: 2026.11.2';
 d.rows[0].validationQueryId='check';d.rows[0].query=query;
 const plans=buildMultiRolePlans(d,processes,accounts,queries);
 assert.equal(plans.length,2);
 assert.deepEqual(plans.map(p=>p.plan_name),['November staging · collector','November staging · maintainer']);
 for(const p of plans){assert.equal(p.inputs.jira.issue,'APP-1');assert.equal(p.inputs.cx.request,'000001-000002');assert.equal(p.inputs.olam.account,'deployment');assert.equal(p.inputs.olam.parallel_regions,3);}
 assert.deepEqual(plans[0].inputs.olam.targets.hosts,['consumer']);assert.deepEqual(plans[1].inputs.olam.targets.hosts,['redis']);
 assert.equal(plans[0].inputs.olam.steps[0].extra_vars.IMAGE_VERSION,'2026.11.1');assert.equal(plans[1].inputs.olam.steps[0].extra_vars.IMAGE_VERSION,'2026.11.2');
 assert.equal(plans[0].inputs.olam.steps[1].query,query);assert.equal(plans[1].inputs.olam.steps.length,1);
 d.rows[0].query='vector(0)';assert.equal(plans[0].inputs.olam.steps[1].query,query);
});
test('every role may omit Thanos; bad or duplicate role targets and unregistered queries reject the entire batch',()=>{
 const d=draft();assert.ok(buildMultiRolePlans(d,processes,accounts,queries).every(p=>p.inputs.olam.steps.length===1));
 d.rows[1].olamTargets.roles=['collector'];assert.throws(()=>buildMultiRolePlans(d,processes,accounts,queries),/already included/);
 d.rows[1]=row('maintainer','redis');d.rows[1].olamTargets.hosts=[];assert.throws(()=>buildMultiRolePlans(d,processes,accounts,queries),/maintainer.*target/);
 d.rows[1]=row('maintainer','redis');d.rows[1].validationQueryId='missing';assert.throws(()=>buildMultiRolePlans(d,processes,accounts,queries),/existing saved Thanos query/);
 d.rows[1].validationQueryId='';d.rows[1].extraVarsText='broken';assert.throws(()=>buildMultiRolePlans(d,processes,accounts,queries),/maintainer/);
});
