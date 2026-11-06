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
 const plan=buildPlannerPlan(draft(),processes,accounts,[{id:'saved'},{id:'library-q'}]);
 assert.equal(plan.process,'Process');assert.equal(plan.inputs.olam.account,'deployment_account');assert.equal(plan.inputs.cx.request,'000001-000002');assert.equal(plan.inputs.cx.requester,undefined);
 assert.deepEqual(plan.inputs.olam.steps.map(s=>s.name),['Phase one','Phase two']);const vars=plan.inputs.olam.steps[0].extra_vars;
 assert.equal(vars.VERSION,'3.9.2');assert.equal(vars.COUNT,"3");assert.equal(vars.ENABLED,"false");assert.equal(vars.OPTIONS,'{"items":[1,true]}');assert.equal(plan.inputs.olam.steps[1].extra_vars,undefined);
 assert.deepEqual(plan.inputs.olam.targets.inventory_groups,['env_a','env_b']);
 const yaml=plannerPlanYaml(plan);assert(yaml.includes('request: "000001-000002"'));assert(yaml.indexOf('Phase one')<yaml.indexOf('Phase two'));assert(yaml.includes('"COUNT":"3"'));
});
test('Builder rejects empty targets, multiple roles, missing step names, duplicate keys and missing variable names',()=>{
 for (const mutate of [d=>{d.olamTargets={roles:["service"],sites:[],hosts:[]}},d=>d.olamTargets.roles=['a','b'],d=>d.steps[0].name=' ',d=>d.steps=[],d=>d.strategy='invalid',d=>d.cxNo='12345-123456',d=>d.accountId='missing']) {
  const d=draft();mutate(d);assert.throws(()=>buildPlannerPlan(d,processes,accounts,[{id:'saved'},{id:'library-q'}]));
 }
 assert.throws(()=>stepExtraVars({variables:[variable('A','1'),variable(' A ','2')]},0),/duplicate/);
 assert.throws(()=>stepExtraVars({variables:[variable('','value')]},0));
 assert.equal(stepExtraVars({variables:[variable('VALUE','000123','number')]},0).VALUE,'000123');
 assert.equal(stepExtraVars({variables:[variable('VALUE','{','json')]},0).VALUE,'{');
});
test('Builder keeps literal keys and strings safe and allows empty tickets even with Jira/CX Process steps',()=>{
 const d=draft();d.jiraNo='';d.cxNo='';d.steps[0].name='Name: "quoted"\nnext';d.steps[0].variables=[variable('__proto__','literal'),variable('TEXT','a: b\nnext line')];
 const plan=buildPlannerPlan(d,processes,accounts,[{id:'saved'},{id:'library-q'}]);
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
 const yaml=plannerPlanYaml(buildPlannerPlan(d,processes,accounts,[{id:'saved'},{id:'library-q'}]));assert(yaml.includes('"COUNT":"000123"'));assert(yaml.includes('"ENABLED":"false"'));
});

test('container Process uses the same Planner and serializes generic Ansible inputs',()=>{
 const containerProcesses=[{id:'p',name:'Devcorp - Test',definition:JSON.stringify({steps:[{driver:'container',action:'deploy',workflow:['validate','approve','deploy']}]})}];
 const plan=buildPlannerPlan(draft(),containerProcesses,accounts,[{id:'saved'},{id:'library-q'}]);
 assert.equal(plan.inputs.olam,undefined);assert.equal(plan.inputs.ansible.account,'deployment_account');
 assert.deepEqual(plan.inputs.ansible.targets,draft().olamTargets);
 const yaml=plannerPlanYaml(plan);assert(yaml.includes('  ansible:\n'));assert(!yaml.includes('  olam:\n'));assert(yaml.includes('"COUNT":"3"'));
});

test('Thanos steps preserve literal PromQL and order without deploy variables or polling overrides',()=>{
 const d=draft();
 const query='count(up{job="exporter",region=~"a|b"} == 1) == bool 2';
 d.steps.splice(1,0,{name:'Verify running service',validationQueryId:'saved',type:'thanos_query',query,extraVarsText:'not a variable assignment'});
 const config=[{...processes[0],definition:JSON.stringify({steps:[{driver:'olam',action:'deploy'}],details:{thanos:{url:'https://metrics.example.test',timeout_seconds:300,interval_seconds:30}}})}];
 const plan=buildPlannerPlan(d,config,accounts,[{id:'saved'},{id:'library-q'}]); const step=plan.inputs.olam.steps[1];
 assert.deepEqual(step,{name:'Verify running service',type:'thanos_query',query});
 const yaml=plannerPlanYaml(plan);assert.ok(yaml.includes(`query: ${JSON.stringify(query)}`));
 assert.ok(yaml.indexOf('Phase one')<yaml.indexOf('Verify running service'));assert.ok(yaml.indexOf('Verify running service')<yaml.indexOf('Phase two'));
 assert.ok(!yaml.includes('timeout_seconds'));assert.ok(!yaml.includes('interval_seconds'));
});
test('Thanos steps require a query and connection and reject unresolved placeholders and unknown types',()=>{
 const d=draft();d.steps=[{name:'Check',validationQueryId:'saved',type:'thanos_query',query:''}];
 assert.throws(()=>buildPlannerPlan(d,processes,accounts,[{id:'saved'},{id:'library-q'}]),/select a saved Thanos query/);
 d.steps[0].query='vector(1)';assert.throws(()=>buildPlannerPlan(d,processes,accounts,[{id:'saved'},{id:'library-q'}]),/details.thanos/);
 d.steps[0].type='typo';assert.throws(()=>buildPlannerPlan(d,processes,accounts,[{id:'saved'},{id:'library-q'}]),/unknown step/);
 d.steps[0].type='thanos_query';d.steps[0].query='${UNRESOLVED}';assert.throws(()=>buildPlannerPlan(d,processes,accounts,[{id:'saved'},{id:'library-q'}]),/placeholders/);
});


test('Thanos query templates save expected version and require a preceding deployment with sites',()=>{
 const d=draft();
 const config=[{...processes[0],definition:JSON.stringify({steps:[{driver:'olam',action:'deploy'}],details:{thanos:{url:'https://metrics.example.test'}}})}];
 const step={name:'Verify',validationQueryId:'saved',type:'thanos_query',expectedVersion:' 1.97.0 ',query:'metric{region="${DEPLOY_REGION}",version="${EXPECTED_VERSION}"}'};
 d.steps.push(step);
 const plan=buildPlannerPlan(d,config,accounts,[{id:'saved'},{id:'library-q'}]);
 assert.equal(plan.inputs.olam.steps.at(-1).expected_version,'1.97.0');
 assert.ok(plannerPlanYaml(plan).includes('expected_version: "1.97.0"'));
 assert.equal(plan.inputs.olam.steps.at(-1).query,step.query);
 step.expectedVersion='';assert.throws(()=>buildPlannerPlan(d,config,accounts,[{id:'saved'},{id:'library-q'}]),/expected exporter version/);
 step.expectedVersion='1.97.0';d.olamTargets.sites=[];assert.throws(()=>buildPlannerPlan(d,config,accounts,[{id:'saved'},{id:'library-q'}]),/select deployment sites/);
 d.olamTargets.sites=['a'];d.steps=[step];assert.throws(()=>buildPlannerPlan(d,config,accounts,[{id:'saved'},{id:'library-q'}]),/preceding Deploy/);
});


test('per-host checks require the deployed host matcher and preserve host/site metadata',()=>{
 const d=draft();d.olamTargets={job_mode:'per_host',roles:['service'],sites:['a','b'],hosts:['one','two'],host_sites:{one:'a',two:'b'}};
 const config=[{...processes[0],definition:JSON.stringify({steps:[{driver:'olam',action:'deploy'}],details:{thanos:{url:'https://metrics.example.test'}}})}];
 const check={name:'Verify',validationQueryId:'saved',type:'thanos_query',expectedVersion:'1.97.0',query:'metric{region="${DEPLOY_REGION}",version="${EXPECTED_VERSION}"}'};
 d.steps.push(check);
 assert.throws(()=>buildPlannerPlan(d,config,accounts,[{id:'saved'},{id:'library-q'}]),/per-host validation requires/);
 check.query='metric{region="${DEPLOY_REGION}",instance=~"${DEPLOY_HOST_REGEX}",version="${EXPECTED_VERSION}"}';
 const plan=buildPlannerPlan(d,config,accounts,[{id:'saved'},{id:'library-q'}]);
 assert.deepEqual(plan.inputs.olam.targets.host_sites,{one:'a',two:'b'});
 assert.ok(plannerPlanYaml(plan).includes('host_sites: {"one":"a","two":"b"}'));
 d.olamTargets.job_mode='together';assert.throws(()=>buildPlannerPlan(d,config,accounts,[{id:'saved'},{id:'library-q'}]),/Separate job per host/);
});


test('Plan keeps the selected query text and never stores a live query-library reference',()=>{
 const d=draft();
 const config=[{...processes[0],definition:JSON.stringify({steps:[{driver:'olam',action:'deploy'}],details:{thanos:{url:'https://metrics.example.test'}}})}];
 d.steps.push({name:'Saved check',validationQueryId:'saved',type:'thanos_query',query:'vector(1)',validationQueryId:'library-q'});
 const plan=buildPlannerPlan(d,config,accounts,[{id:'saved'},{id:'library-q'}]);const yaml=plannerPlanYaml(plan);
 d.steps.at(-1).query='vector(0)';
 assert.equal(plan.inputs.olam.steps.at(-1).query,'vector(1)');
 assert.ok(!yaml.includes('library-q'));assert.ok(yaml.includes('query: "vector(1)"'));
});

test('parallel region limit survives YAML serialization and rejects invalid grouping and unmapped hosts',()=>{
 const d=draft(); d.parallelRegions=3;
 d.olamTargets={roles:['service'],sites:['a','b'],hosts:['one','two'],host_sites:{one:'a',two:'b'},job_mode:'per_host'};
 const plan=buildPlannerPlan(d,processes,accounts,[{id:'saved'},{id:'library-q'}]);
 assert.equal(plan.inputs.olam.parallel_regions,3); assert.ok(plannerPlanYaml(plan).includes('parallel_regions: 3'));
 for(const value of [0,17,1.5,'no']) { d.parallelRegions=value; assert.throws(()=>buildPlannerPlan(d,processes,accounts,[{id:'saved'},{id:'library-q'}]),/whole number/); }
 d.parallelRegions=3; d.olamTargets.job_mode='together'; assert.throws(()=>buildPlannerPlan(d,processes,accounts,[{id:'saved'},{id:'library-q'}]),/Separate job per host/);
 d.olamTargets.job_mode='per_host'; delete d.olamTargets.host_sites.one; assert.throws(()=>buildPlannerPlan(d,processes,accounts,[{id:'saved'},{id:'library-q'}]),/deployment region/);
});

test('role image version resolves at execution without a manually entered exporter version',()=>{
 const d=draft();d.olamTargets={job_mode:'per_host',roles:['collector'],sites:['a'],hosts:['host'],host_sites:{host:'a'}};
 const config=[{...processes[0],definition:JSON.stringify({steps:[{driver:'olam',action:'deploy'}],details:{thanos:{url:'https://metrics.example.test'}}})}];
 const check={name:'Thanos Query',validationQueryId:'saved',type:'thanos_query',query:'metric{region="${DEPLOY_REGION}",instance=~"${DEPLOY_HOST_REGEX}",version="${DEPLOY_IMAGE_VERSION}"}'};
 d.steps.push(check);
 const plan=buildPlannerPlan(d,config,accounts,[{id:'saved'},{id:'library-q'}]);
 assert.equal(plan.inputs.olam.steps.at(-1).query,check.query);
 assert.equal(plan.inputs.olam.steps.at(-1).expected_version,undefined);
 d.olamTargets.job_mode='together';assert.throws(()=>buildPlannerPlan(d,config,accounts,[{id:'saved'},{id:'library-q'}]),/Separate job per host/);
 d.olamTargets.job_mode='per_host';d.steps=[check];assert.throws(()=>buildPlannerPlan(d,config,accounts,[{id:'saved'},{id:'library-q'}]),/preceding Deploy/);
});

test('Planner requires a saved query selection and rejects missing or deleted library entries',()=>{
 const d=draft();d.steps=[{name:'Thanos Query',type:'thanos_query',query:'vector(1)'}];
 const config=[{...processes[0],definition:JSON.stringify({details:{thanos:{url:'https://metrics.example.test'}}})}];
 assert.throws(()=>buildPlannerPlan(d,config,accounts,[]),/select a saved Thanos query/);
 d.steps[0].validationQueryId='deleted';
 assert.throws(()=>buildPlannerPlan(d,config,accounts,[{id:'saved'}]),/select a saved Thanos query/);
 d.steps[0].validationQueryId='saved';
 const plan=buildPlannerPlan(d,config,accounts,[{id:'saved'}]);
 assert.equal(plan.inputs.olam.steps[0].query,'vector(1)');
 assert.ok(!Object.hasOwn(plan.inputs.olam.steps[0],'validationQueryId'));
});
