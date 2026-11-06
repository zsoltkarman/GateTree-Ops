import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const source = readFileSync(new URL('../../web/modules/deployments/queue-view.js', import.meta.url),'utf8').replace(/^import .*;\n/gm,'').replaceAll('export ','');
function helpers(deps = {}) {
 const names = ['document','window','getDeploymentQueue','getDeploymentQueueLog','notifyQueue','patchQueue','render','escapeHtml','CSS'];
 const values = {document:{hidden:false},window:{},escapeHtml:String,CSS:{escape:String},...deps};
 return new Function(...names, source + '\nreturn {scheduleQueuePoll,queuePollDelay,queuePollVisible,newQueueControls,refreshActiveQueueLog,planLogMarkup,jobLogMarkup};')(...names.map(name=>values[name]));
}
const root = () => ({isConnected:true,dataset:{queueMountId:'m'},closest:()=>null,querySelector:()=>({})});
const state = () => ({mountId:'m',plans:[],adding:false,busyKey:null,resizingColumns:false});

test('idle polls slow down and hidden/disconnected panes do not poll',async()=>{
 let timer,delay,requests=0;const doc={hidden:false};
 const view=helpers({document:doc,window:{setTimeout:(callback,ms)=>{timer=callback;delay=ms}},getDeploymentQueue:async()=>{requests++;return {plans:[]}}});
 const r=root(),s=state();
 assert.equal(view.queuePollDelay(r,s),5000);
 s.plans=[{status:'running'}];assert.equal(view.queuePollDelay(r,s),1500);
 r.closest=()=>({hidden:true});view.scheduleQueuePoll(r,s);assert.equal(delay,5000);await timer();assert.equal(requests,0);
 r.closest=()=>null;doc.hidden=true;await timer();assert.equal(requests,0);
 r.isConnected=false;doc.hidden=false;await timer();assert.equal(requests,0);
});

test('polls request summaries, leave unchanged queues alone and discard stale mounts',async()=>{
 let timer,resolve,options,patched=0,notified=0;
 // Override the declared patch/notification functions to observe the poll seam.
 const code=source+'\npatchQueue=()=>{counters.patched++};notifyQueue=()=>{counters.notified++};refreshActiveQueueLog=async()=>{};return scheduleQueuePoll;';
 const counters={patched:0,notified:0};
 const poll=new Function('document','window','getDeploymentQueue','counters',code)({hidden:false},{setTimeout:callback=>{timer=callback}},args=>{options=args;return new Promise(r=>{resolve=r})},counters);
 const r=root(),s=state();poll(r,s);const first=timer();resolve({plans:[]});await first;
 assert.deepEqual(options,{includeLogs:false});assert.deepEqual(counters,{patched:0,notified:0});
 const next=timer();resolve({plans:[{id:'p',jobs:[],status:'running'}]});await next;
 assert.deepEqual(counters,{patched:1,notified:1});
 const stale=timer();r.dataset.queueMountId='new';resolve({plans:[]});await stale;
 assert.equal(s.plans[0].id,'p');assert.deepEqual(counters,{patched:1,notified:1});
});

test('controls are bound once while newly inserted controls are discovered',()=>{
 const a={},b={},r={querySelectorAll:()=>[a]},s={};const view=helpers();
 assert.deepEqual(view.newQueueControls(r,s,'button'),[a]);assert.deepEqual(view.newQueueControls(r,s,'button'),[]);
 r.querySelectorAll=()=>[a,b];assert.deepEqual(view.newQueueControls(r,s,'button'),[b]);
});

test('hidden log markup does not render or aggregate full logs',()=>{
 const view=helpers();const huge='secret-log-line\n'.repeat(10000);
 assert(!view.planLogMarkup({id:'p',jobs:[{log:huge}]},{}).includes('secret-log-line'));
 assert(!view.jobLogMarkup({id:'j',name:'Deploy',log:huge}).includes('secret-log-line'));
});

test('open log requests only its delta, preserves reading position and handles reset',async()=>{
 const requests=[];let response={reset:true,text:'first\n',totalBytes:6,hash:'one'};
 const pre={isConnected:true,textContent:'Loading',scrollTop:30,scrollHeight:300,clientHeight:100,append(node){this.textContent+=node.textContent}};
 const r=root();r.querySelector=()=>pre;
 const s={...state(),infoKey:'job-log:j',plans:[{id:'p',jobs:[{id:'j'}]}]};
 const view=helpers({document:{createTextNode:text=>({textContent:text})},getDeploymentQueueLog:async args=>{requests.push(args);return response}});
 await view.refreshActiveQueueLog(r,s);assert.equal(pre.textContent,'first\n');assert.equal(pre.scrollTop,30);
 response={reset:false,text:'next\n',totalBytes:11,hash:'two'};await view.refreshActiveQueueLog(r,s);
 assert.deepEqual(requests[1],{planId:'p',jobId:'j',offset:6,hash:'one'});assert.equal(pre.textContent,'first\nnext\n');assert.equal(pre.scrollTop,30);
 response={reset:true,text:'replacement',totalBytes:11,hash:'three'};await view.refreshActiveQueueLog(r,s);assert.equal(pre.textContent,'replacement');
 s.infoKey=null;await view.refreshActiveQueueLog(r,s);assert.equal(requests.length,3);
});

test('a delayed log response cannot overwrite a newly selected panel',async()=>{
 let resolve;const pre={isConnected:true,textContent:'current',scrollTop:0,scrollHeight:0,clientHeight:0};
 const r=root();r.querySelector=()=>pre;const s={...state(),infoKey:'job-log:j',plans:[{id:'p',jobs:[{id:'j'}]}]};
 const view=helpers({getDeploymentQueueLog:()=>new Promise(r=>{resolve=r})});
 const pending=view.refreshActiveQueueLog(r,s);s.infoKey=null;resolve({reset:true,text:'old',totalBytes:3,hash:'old'});await pending;
 assert.equal(pre.textContent,'current');
});
