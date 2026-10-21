import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const source=readFileSync(new URL('../../web/modules/deployments/planner-plan.js',import.meta.url),'utf8').replaceAll('export function ','function ');
const {buildPlannerPlan,plannerPlanYaml,stepExtraVars,parseExtraVarsText}=new Function(source+'\nreturn {buildPlannerPlan,plannerPlanYaml,stepExtraVars,parseExtraVarsText};')();
const processes=[{id:'p',name:'Process',definition:JSON.stringify({steps:[{driver:'jira'},{driver:'cx'},{driver:'olam'}]})}];
const accounts=[{id:'a',name:'deployment_account'}];
const variable=(key,value,type='text')=>({key,value,type});
const draft=()=>({planName:'Upgrade',processId:'p',accountId:'a',cxNo:'000001-000002',jiraNo:'APP-42',strategy:'linear',olamTargets:{roles:['service'],sites:['a','b'],inventory_groups:['env_a','env_b']},steps:[{name:'Phase one',variables:[variable('VERSION','3.9.2'),variable('COUNT','3','number'),variable('ENABLED','false','boolean'),variable('OPTIONS','{"items":[1,true]}','json')]},{name:'Phase two',variables:[]}]});
test('Builder maps IDs to Process/Account references, keeps step order and all extra vars as text',()=>{
 const plan=buildPlannerPlan(draft(),processes,accounts);
 assert.equal(plan.process,'Process');assert.equal(plan.inputs.olam.account,'deployment_account');assert.equal(plan.inputs.cx.request,'000001-000002');assert.equal(plan.inputs.cx.requester,undefined);
 assert.deepEqual(plan.inputs.olam.steps.map(s=>s.name),['Phase one','Phase two']);const vars=plan.inputs.olam.steps[0].extra_vars;
 assert.equal(vars.VERSION,'3.9.2');assert.equal(vars.COUNT,"3");assert.equal(vars.ENABLED,"false");assert.equal(vars.OPTIONS,'{"items":[1,true]}');assert.equal(plan.inputs.olam.steps[1].extra_vars,undefined);
 assert.deepEqual(plan.inputs.olam.targets.inventory_groups,['env_a','env_b']);
 const yaml=plannerPlanYaml(plan);assert(yaml.includes('request: "000001-000002"'));assert(yaml.indexOf('Phase one')<yaml.indexOf('Phase two'));assert(yaml.includes('"COUNT":"3"'));
});
test('Builder rejects empty targets, multiple roles, missing step names, duplicate keys and missing variable names',()=>{
 for (const mutate of [d=>{d.olamTargets={roles:["service"],sites:[],hosts:[]}},d=>d.olamTargets.roles=['a','b'],d=>d.steps[0].name=' ',d=>d.steps=[],d=>d.strategy='invalid',d=>d.cxNo='12345-123456',d=>d.accountId='missing']) {
  const d=draft();mutate(d);assert.throws(()=>buildPlannerPlan(d,processes,accounts));
 }
 assert.throws(()=>stepExtraVars({variables:[variable('A','1'),variable(' A ','2')]},0),/duplicate/);
 assert.throws(()=>stepExtraVars({variables:[variable('','value')]},0));
 assert.equal(stepExtraVars({variables:[variable('VALUE','000123','number')]},0).VALUE,'000123');
 assert.equal(stepExtraVars({variables:[variable('VALUE','{','json')]},0).VALUE,'{');
});
test('Builder keeps literal keys and strings safe and allows empty tickets even with Jira/CX Process steps',()=>{
 const d=draft();d.jiraNo='';d.cxNo='';d.steps[0].name='Name: "quoted"\nnext';d.steps[0].variables=[variable('__proto__','literal'),variable('TEXT','a: b\nnext line')];
 const plan=buildPlannerPlan(d,processes,accounts);
 assert.equal(plan.inputs.jira,undefined);assert.equal(plan.inputs.cx,undefined);assert.equal(plan.inputs.olam.steps[0].extra_vars.__proto__,'literal');assert.equal({}.literal,undefined);
 const yaml=plannerPlanYaml(plan);assert(yaml.includes('Name: \\"quoted\\"\\nnext'));assert(yaml.includes('"__proto__":"literal"'));
});

test('textarea accepts pasted assignments while retaining numbers, booleans, URLs and quoted values as text',()=>{
 const parsed=parseExtraVarsText('# Comment\r\nVERSION: 3.9.2\r\nCOUNT=000123\r\nENABLED: false\r\nURL: https://host/path?a=b\r\nMESSAGE: "a: b"\r\n- EMPTY: \r\nJSON: {"a":true}',0);
 assert.deepEqual({...parsed},{VERSION:'3.9.2',COUNT:'000123',ENABLED:'false',URL:'https://host/path?a=b',MESSAGE:'a: b',EMPTY:'',JSON:'{"a":true}'});
 assert.throws(()=>parseExtraVarsText('A: 1\nA=2',1),/Step 2: duplicate/);
 assert.throws(()=>parseExtraVarsText('A: 1\ninvalid',0),/line 2/);
 assert.deepEqual({...stepExtraVars({extraVarsText:'VALUE: new',variables:[variable('VALUE','old')]},0)},{VALUE:'new'});
 const d=draft();d.steps[0]={name:'Pasted variables',extraVarsText:'COUNT: 000123\nENABLED=false'};
 const yaml=plannerPlanYaml(buildPlannerPlan(d,processes,accounts));assert(yaml.includes('"COUNT":"000123"'));assert(yaml.includes('"ENABLED":"false"'));
});

test('container Process uses the same Planner and serializes generic Ansible inputs',()=>{
 const containerProcesses=[{id:'p',name:'Devcorp - Test',definition:JSON.stringify({steps:[{driver:'container',action:'deploy',workflow:['validate','approve','deploy']}]})}];
 const plan=buildPlannerPlan(draft(),containerProcesses,accounts);
 assert.equal(plan.inputs.olam,undefined);assert.equal(plan.inputs.ansible.account,'deployment_account');
 assert.deepEqual(plan.inputs.ansible.targets,draft().olamTargets);
 const yaml=plannerPlanYaml(plan);assert(yaml.includes('  ansible:\n'));assert(!yaml.includes('  olam:\n'));assert(yaml.includes('"COUNT":"3"'));
});
