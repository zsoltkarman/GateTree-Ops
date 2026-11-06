import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const source=readFileSync(new URL('../../web/modules/deployments/queue-view.js',import.meta.url),'utf8').replace(/^import .*;\n/gm,'').replaceAll('export ','');
const escapeHtml=value=>String(value??'').replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
const {jobGroupsMarkup,jobGroupKey,jobGroupExpanded}=new Function('escapeHtml',source+'\nreturn {jobGroupsMarkup,jobGroupKey,jobGroupExpanded};')(escapeHtml);
const job=(id,status,sequence)=>({id,status,sequence,name:id,driver:'olam',action:'deploy',target:'host',details:'',payload:'{}',log:''});
const plan={id:'p',status:'running',jobs:[job('completed','completed',1),job('running','running',2),job('queued','queued',3),job('approval','awaiting_approval',4),job('failed','failed',5),job('stopped','stopped',6),job('disabled','disabled',7)]};
const heading=(html,group)=>html.match(new RegExp(`<button[^>]*data-job-group="p:${group}"[^>]*>`))?.[0];
const item=(html,id)=>html.match(new RegExp(`<div[^>]*data-job-item="${id}"[^>]*>`))?.[0];

test('groups include every job exactly once with running/approval first and accurate counts',()=>{
 const before=JSON.stringify(plan),html=jobGroupsMarkup(plan,{});
 assert(html.indexOf('p:running')<html.indexOf('p:queued'));assert(html.indexOf('p:queued')<html.indexOf('p:completed'));
 assert.match(heading(html,'running'),/aria-label="Collapse Running: 2 jobs"/);
 assert.match(heading(html,'issues'),/aria-label="Collapse Failed \/ Stopped: 2 jobs"/);
 assert.match(heading(html,'queued'),/aria-label="Expand Queued: 1 jobs"/);
 assert.match(heading(html,'completed'),/aria-label="Expand Completed: 1 jobs"/);
 assert.equal((html.match(/data-job-item=/g)||[]).length,7);
 for(const j of plan.jobs)assert.equal((html.match(new RegExp(`data-job-item="${j.id}"`,'g'))||[]).length,1);
 assert.equal(JSON.stringify(plan),before,'display grouping must not reorder execution jobs');
});

test('default visibility opens active/problem jobs and collapses queued/completed/skipped jobs',()=>{
 const html=jobGroupsMarkup(plan,{});
 for(const id of ['running','approval','failed','stopped'])assert(!/ hidden[ >]/.test(item(html,id)),id);
 for(const id of ['queued','completed','disabled'])assert.match(item(html,id),/ hidden>/,id);
 assert.equal(jobGroupKey('warning'),'issues');assert.equal(jobGroupKey('denied'),'issues');assert.equal(jobGroupKey('unknown'),'issues');
});

test('group expansion is retained across refreshes and isolated per Plan',()=>{
 const state={jobGroups:new Map([['p:queued',true],['p:running',false]])};
 assert.equal(jobGroupExpanded(state,'p:queued'),true);assert.equal(jobGroupExpanded(state,'other:queued'),false);
 let html=jobGroupsMarkup(plan,state);assert.match(heading(html,'queued'),/aria-expanded="true"/);assert.match(item(html,'running'),/ hidden>/);
 const updated={...plan,jobs:[...plan.jobs,job('another','queued',8)]};html=jobGroupsMarkup(updated,state);
 assert.match(heading(html,'queued'),/Expand|Collapse/);assert.match(heading(html,'queued'),/aria-expanded="true"/);assert.match(heading(html,'queued'),/Queued: 2 jobs/);
 assert(!/ hidden[ >]/.test(item(html,'another')));
});

test('grouped controls keep original predecessor dependencies instead of display order',()=>{
 const p={id:'p',status:'stopped',jobs:[job('blocked','queued',1),job('done','completed',2)]};
 const html=jobGroupsMarkup(p,{});
 const completed=html.slice(html.indexOf('data-job-item="done"'));
 assert.match(completed,/<button[^>]*data-control-action="restart"[^>]* disabled>/);
 assert.match(completed,/<span>2<\/span>/,'original sequence number is retained');
});

test('a refreshed job changes groups without changing its identity or sequence',()=>{
 const running={id:'p',jobs:[job('same','running',9)],status:'running'};
 const first=jobGroupsMarkup(running,{});const next=jobGroupsMarkup({...running,jobs:[{...running.jobs[0],status:'completed'}]},{});
 assert.match(first,/data-job-item="same" data-job-group-membership="p:running"/);
 assert.match(next,/data-job-item="same" data-job-group-membership="p:completed" hidden/);
 assert.match(first,/<span>9<\/span>/);assert.match(next,/<span>9<\/span>/);
});

test('collapsed log groups do not request log data',async()=>{
 let requests=0;
 const {refreshActiveQueueLog}=new Function('getDeploymentQueueLog','CSS',source+'\nreturn {refreshActiveQueueLog};')(()=>{requests++},{escape:String});
 await refreshActiveQueueLog({querySelector:()=>({closest:()=>({hidden:true})})},{infoKey:'job-log:job'});
 assert.equal(requests,0);
});

test('group toggles update only their own jobs and remain bound once across refreshes',()=>{
 const listeners=[];const attrs={'aria-label':'Expand Queued: 2 jobs','aria-expanded':'false'};
 const button={dataset:{jobGroup:'p:queued'},addEventListener:(_,listener)=>listeners.push(listener),setAttribute:(name,value)=>{attrs[name]=value},getAttribute:name=>attrs[name]};
 const items=[{hidden:true},{hidden:true}];const state={infoKey:null};
 const root={querySelectorAll:selector=>selector==='[data-job-group]'?[button]:selector.startsWith('[data-job-group-membership=')?items:[]};
 const {bindQueue}=new Function('mountQueueColumns','CSS',source+'\nreturn {bindQueue};')(()=>()=>{},{escape:String});
 bindQueue(root,state);listeners[0]();assert.equal(state.jobGroups.get('p:queued'),true);assert(items.every(item=>!item.hidden));assert.equal(attrs['aria-expanded'],'true');assert.equal(attrs['aria-label'],'Collapse Queued: 2 jobs');
 bindQueue(root,state);assert.equal(listeners.length,1);listeners[0]();assert(items.every(item=>item.hidden));assert.equal(state.jobGroups.get('p:queued'),false);
});

test('a failed Thanos check brings its completed deployment into the issues group with a pair retry action',()=>{
 const deploy={...job('deploy','completed',1),target:'host-a',region:'a'};
 const check={...job('query','failed',2),driver:'thanos',action:'query',target:'host-a',region:'a'};
 const healthy={...job('healthy','running',3),target:'host-b',region:'b'};
 const p={id:'p',status:'running',parallelRegions:3,jobs:[deploy,check,healthy]};
 const html=jobGroupsMarkup(p,{});
 assert.match(heading(html,'issues'),/Failed \/ Stopped: 2 jobs/);
 assert.match(item(html,'deploy'),/data-job-group-membership="p:issues"/);
 assert(html.indexOf('data-job-item="deploy"')<html.indexOf('data-job-item="query"'));
 assert.match(html,/Thanos validation failed/);
 const retry=html.match(/<button[^>]*data-control-job="deploy"[^>]*data-control-action="restart"[^>]*>/)?.[0];
 assert.match(retry,/aria-label="Restart deploy \+ Thanos"/);assert(!/ disabled[ >]/.test(retry));
 assert.match(html,/class="queue-status is-completed">completed/,'the deployment keeps its true status');
 assert(!heading(html,'completed'),'the deployment is shown once, next to its failed check');
});

test('pair association never crosses another deployment or accepts a different host',()=>{
 const {deploymentPairChecks}=new Function(source+'\nreturn {deploymentPairChecks};')();
 const deploy=job('deploy','completed',1);const check={...job('check','failed',2),driver:'thanos',action:'query'};
 assert.equal(deploymentPairChecks({jobs:[deploy,check]},0).length,1);
 assert.equal(deploymentPairChecks({jobs:[deploy,{...check,target:'other-host'}]},0).length,0);
 assert.equal(deploymentPairChecks({jobs:[deploy,job('next','completed',2),check]},0).length,0);
 assert.equal(deploymentPairChecks({jobs:[deploy,{...check,status:'running'}]},0).length,0);
});
