import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const queue=readFileSync(new URL('../../web/modules/deployments/queue-view.js',import.meta.url),'utf8').replace(/^import .*;\n/gm,'').replaceAll('export ','');
const shell=readFileSync(new URL('../../web/modules/shell/app-shell.js',import.meta.url),'utf8');
const nav=shell.slice(shell.indexOf('function queuePlanGroup('),shell.indexOf('export function mountAppShell('));
const escapeHtml=s=>String(s??'').replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
const {queuePlanGroup,queueNavigationMarkup}=new Function('escapeHtml','icon','planStatusIcon',nav+'\nreturn {queuePlanGroup,queueNavigationMarkup};')(escapeHtml,()=>'',()=> '');
const {queueMarkup,planMarkup,planInfoMarkup,jobMarkup}=new Function('escapeHtml',queue+'\nreturn {queueMarkup,planMarkup,planInfoMarkup,jobMarkup};')(escapeHtml);
const job={id:'j',sequence:1,name:'Deploy',status:'completed',driver:'olam',action:'deploy',details:'',target:'host',payload:'{}'};
const plan={id:'p',planNumber:1,name:'Release',processName:'Deploy',status:'completed',jobs:[job],definition:'original YAML'};

test('Archived is the last Queue navigation group and archived success is not duplicated under Success',()=>{
 const p={...plan,archived:true};assert.equal(queuePlanGroup(p),'archived');assert.equal(queuePlanGroup(plan),'success');
 const html=queueNavigationMarkup({plans:[p],section:'queue'},new Set(['archived','success']));
 assert(html.indexOf('data-queue-group="archived"')>html.indexOf('data-queue-group="success"'));
 assert.equal((html.match(/data-queue-plan="p"/g)||[]).length,1);
 assert(html.indexOf('data-queue-plan="p"')>html.indexOf('data-queue-group="archived"'));
});

test('normal Queue omits archived Plans and Archived view lists them without Add Plan',()=>{
 const plans=[plan,{...plan,id:'arch',archived:true}];
 let html=queueMarkup({plans,expandedPlans:new Set()});assert(html.includes('data-plan-group="p"'));assert(!html.includes('data-plan-group="arch"'));
 html=queueMarkup({plans,archiveView:true,expandedPlans:new Set()});assert(!html.includes('data-plan-group="p"'));assert(html.includes('data-plan-group="arch"'));assert(!html.includes('data-action="add"'));
 const empty=queueMarkup({plans:[plan],archiveView:true});assert(empty.includes('No archived Plans.'));
});

test('only completed nonempty successful Plans offer Archive; archived history remains readable and copyable',()=>{
 assert(planMarkup(plan,{}).includes('data-archive-plan="p"'));
 for(const p of [{...plan,status:'running'},{...plan,jobs:[]},{...plan,jobs:[{...job,status:'failed'}]}]) assert(!planMarkup(p,{}).includes('data-archive-plan='));
 const archived={...plan,archived:true};const html=planMarkup(archived,{});
 assert(!html.includes('data-archive-plan='));assert(html.includes('data-duplicate-plan="p"'));assert(html.includes('data-info="plan-log:p"'));
 assert.match(html,/<button[^>]*data-control-plan="p"[^>]*data-control-action="restart"[^>]* disabled>/);
 assert.match(jobMarkup(archived,job,0),/<button[^>]*data-control-action="restart"[^>]* disabled>/);
 assert.match(planInfoMarkup(archived,{}),/<textarea[^>]* readonly>/);assert.match(planInfoMarkup(archived,{}),/Archived Plan is read-only/);
});

test('Archive moves a successful Plan through the native command and refreshes the Queue once',async()=>{
 const listeners=[],button={dataset:{archivePlan:'p'},addEventListener:(_,fn)=>listeners.push(fn),disabled:false};
 const state={plans:[plan],mountId:'m'};let calls=0,rendered=0,notified=0;
 const root={isConnected:true,dataset:{queueMountId:'m'},querySelectorAll:selector=>selector==='[data-archive-plan]'?[button]:[]};
 const code=queue+'\nrender=()=>{counters.rendered++};notifyQueue=()=>{counters.notified++};return bindQueue;';
 const counters={rendered:0,notified:0};
 const bind=new Function('mountQueueColumns','archiveDeploymentPlan','counters',code)(()=>()=>{},async id=>{calls++;assert.equal(id,'p');return{plans:[{...plan,archived:true}]}},counters);
 bind(root,state);await listeners[0]();assert.equal(calls,1);assert(state.plans[0].archived);assert.deepEqual(counters,{rendered:1,notified:1});assert.equal(state.busyKey,null);
});

test('switching Queue groups always navigates and clears the Archived filter for active groups',()=>{
 const ids=['archived','running','queued','success'];
 const buttons=ids.map(id=>({dataset:{queueGroup:id},addEventListener(_,listener){this.click=listener;}}));
 const queueLinks={querySelectorAll:()=>buttons};const expanded=new Set(['running','queued']);const routes=[];
 const start=shell.indexOf('    queueLinks.querySelectorAll("[data-queue-group]")');
 const end=shell.indexOf('    queueLinks.querySelectorAll("[data-queue-plan]")',start);
 new Function('queueLinks','expandedQueueGroups','renderDeploymentCatalog','navigate',shell.slice(start,end))(queueLinks,expanded,()=>{},(module,detail)=>routes.push({module,detail}));
 for(const button of buttons)button.click();
 assert.deepEqual(routes,ids.map(id=>({module:'deployments',detail:{section:'queue',archived:id==='archived'}})));
 buttons[1].click();
 assert.equal(routes.at(-1).detail.archived,false);
});

test('multi-role Plans appear under a shared group without combining archived and active members',()=>{
 const group={id:'batch',name:'November staging'};
 const plans=[{...plan,group},{...plan,id:'second',group},{...plan,id:'old',group,archived:true}];
 const html=queueMarkup({plans,expandedPlans:new Set()});
 assert.equal((html.match(/data-plan-batch="batch"/g)||[]).length,1);
 assert.ok(html.includes('November staging'));assert.ok(html.includes('2 role Plans'));
 assert.ok(html.includes('data-plan-group="p"'));assert.ok(html.includes('data-plan-group="second"'));assert.ok(!html.includes('data-plan-group="old"'));
 const archived=queueMarkup({plans,archiveView:true,expandedPlans:new Set()});
 assert.ok(archived.includes('data-plan-group="old"'));assert.ok(!archived.includes('data-plan-group="second"'));
});
