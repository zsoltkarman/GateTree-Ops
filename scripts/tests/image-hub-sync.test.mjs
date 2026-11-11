import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const view=await readFile(new URL('../../web/modules/image-hub/image-hub-view.js',import.meta.url),'utf8');
const helpers=await readFile(new URL('../../web/modules/image-hub/sync-progress.js',import.meta.url),'utf8');
const {syncButtonLabel,sourceProgressLabel}=await import(`data:text/javascript;base64,${Buffer.from(helpers).toString('base64')}`);
const escape=value=>String(value).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
test('source counters distinguish completed work, failures and missing configuration',()=>{
 assert.equal(syncButtonLabel({running:true,percent:42}),'Sync Project 42%');
 assert.equal(syncButtonLabel({running:false,percent:100}),'Sync Project');
 assert.equal(sourceProgressLabel({state:'synced',completed:9,total:9,errors:0}),'Synced 9/9');
 assert.equal(sourceProgressLabel({state:'error',completed:1,total:11,errors:1}),'Partial 0/11');
 assert.equal(sourceProgressLabel({state:'not_configured'}),'Not configured');
 assert.equal(sourceProgressLabel({state:'running',completed:3,total:11}),'Syncing 3/11');
});
test('the compact status bar lists active sync sources and retains failure details',()=>{
 const source=view.slice(view.indexOf('  function selectedSync()'),view.indexOf('  function updateSyncWidgets()'));
 const state={projectId:'p1',mode:'list',syncStatuses:[{projectId:'p1',scm:{state:'synced',completed:9,total:9},af:{state:'synced',completed:9,total:9},slaps:{state:'error',completed:1,total:4,errors:1,detail:'Scan failed <details>'},buildLogs:{state:'error',completed:1,total:4,errors:1,detail:'Access denied <details>'}}]};
 const html=new Function('state','e','sourceProgressLabel',source+'\nreturn syncResults();')(state,escape,sourceProgressLabel);
 assert.equal((html.match(/class="image-hub-sync-source/g)||[]).length,3);
 for(const label of ['SCM:','SLAPS:','AF:'])assert.ok(html.includes(label));
 assert.ok(!html.includes('Build Logs:'));assert.ok(!html.includes('Access denied'));
 assert.match(html,/Synced 9\/9/);assert.match(html,/Partial 0\/4/);assert.match(html,/Scan failed &lt;details&gt;/);
});
test('render interpolates the actual percentage into the sync button',()=>{
 const source=view.slice(view.indexOf('  function render()'),view.indexOf('  function discard()'));
 const state={folders:[],projects:[{id:'p1',name:'Project'}],projectId:'p1',folderId:null,syncStatuses:[{projectId:'p1',running:true,percent:42}],mode:'list',error:false,message:'',repositories:[]};
 const root={innerHTML:'',querySelector:()=>null};
 new Function('state','root','options','closeContextMenu','folderPath','e','tree','list','syncResults','bind','syncButtonLabel','selectedSync','icon',source+'\nrender();')(state,root,{},()=>{},()=>'',escape,()=>'',()=>'',()=>'',()=>{},syncButtonLabel,()=>state.syncStatuses[0],()=>'');
 assert.match(root.innerHTML,/Sync Project 42%/);assert.match(root.innerHTML,/data-hub-action="sync-project" disabled/);assert.ok(!root.innerHTML.includes('${'));
});
test('background polling refreshes latest versions and stops when the task finishes',async()=>{
 const source=view.slice(view.indexOf('  function scheduleSyncPoll()'),view.indexOf('  function list()'));
 let callback,reads=0,renders=0;
 const state={syncStatuses:[{projectId:'p1',running:true}],repositories:[{id:'r1'}],mode:'list'};
 const root={querySelector:()=>({})};const contentRoot={isConnected:true,dataset:{imageHubMountId:'mount'}};
 const schedule=new Function('state','root','contentRoot','mountId','window','getImageHub','render','updateArtifactCells','updateSyncWidgets','let syncPollTimer=null;\n'+source+'\nreturn scheduleSyncPoll;')(state,root,contentRoot,'mount',{setTimeout:fn=>{callback=fn;return 1}},async()=>{reads++;return{syncStatuses:[{projectId:'p1',running:false,percent:100}],repositories:[{id:'r1'}],latestImages:[{version:'2.0.0'}]}},()=>renders++,()=>{},()=>{});
 schedule();await callback();assert.equal(reads,1);assert.equal(renders,1);assert.equal(state.latestImages[0].version,'2.0.0');assert.equal(state.syncStatuses[0].percent,100);
 contentRoot.dataset.imageHubMountId='another-view';state.syncStatuses[0].running=true;schedule();await callback();assert.equal(reads,1);
});

test('an AF lookup failure still displays the last known version with an explicit refresh error',()=>{
 const source=view.slice(view.indexOf('  function artifactCell('),view.indexOf('  function updateArtifactCells('));
 const state={runtimeConfig:{artifactFolderUrlTemplate:'https://artifacts.example.test/api/{repo}',artifactPageUrlTemplate:'https://artifacts.example.test/{repo}/{version}'},latestImages:[{repositoryId:'r1',imageName:'sample',status:'error',version:'1.2.3',url:'https://artifacts.example.test/sample/1.2.3',error:'Network unavailable'}],syncStatuses:[]};
 const html=new Function('state','imageHubUrl','e','attr',source+'\nreturn artifactCell({id:"r1",name:"sample"});')(state,()=> 'https://artifacts.example.test',escape,escape);
 assert.match(html,/1.2.3/);assert.match(html,/Last known version/);assert.match(html,/Network unavailable/);assert.doesNotMatch(html,/Checked /);
});

test('inventory sync shows batch progress across current projects and ignores unrelated older runs',async()=>{
 const {inventorySyncButtonLabel}=await import(`data:text/javascript;base64,${Buffer.from(helpers).toString('base64')}`);
 const projects=[{id:'a'},{id:'b'},{id:'c'}];
 assert.equal(inventorySyncButtonLabel([],projects),'Sync all projects');
 assert.equal(inventorySyncButtonLabel([{projectId:'a',runId:'batch',running:false,percent:30,error:'Failed'},{projectId:'b',runId:'batch',running:true,percent:20},{projectId:'c',runId:'batch',running:true,percent:0},{projectId:'old',runId:'old',running:true,percent:99}],projects),'Sync all projects 40%');
 assert.equal(inventorySyncButtonLabel([{projectId:'a',runId:'batch',running:false,percent:100}],projects),'Sync all projects');
});

test('the complete inventory has an enabled Sync all projects button and disables it while syncing',async()=>{
 const {inventorySyncButtonLabel}=await import(`data:text/javascript;base64,${Buffer.from(helpers).toString('base64')}`);
 const source=view.slice(view.indexOf('  function render()'),view.indexOf('  function discard()'));
 const state={folders:[],projects:[{id:'p1',name:'Project'}],projectId:null,folderId:null,syncStatuses:[],mode:'list',error:false,message:'',repositories:[]};
 const root={innerHTML:'',querySelector:()=>null};
 const render=new Function('state','root','options','closeContextMenu','folderPath','e','tree','list','syncResults','bind','inventorySyncButtonLabel','icon',source+'\nreturn render;')(state,root,{},()=>{},()=>'',escape,()=>'',()=>'',()=>'',()=>{},inventorySyncButtonLabel,()=>'');
 render();assert.match(root.innerHTML,/data-hub-action="sync-inventory">Sync all projects/);
 state.syncStatuses=[{projectId:'p1',runId:'batch',running:true,percent:42}];render();
 assert.match(root.innerHTML,/data-hub-action="sync-inventory" disabled>Sync all projects 42%/);
 state.projects=[];state.syncStatuses=[];render();assert.match(root.innerHTML,/data-hub-action="sync-inventory" disabled/);
});

test('inventory status bar summarizes projects while keeping source details in a collapsed disclosure',()=>{
 const source=view.slice(view.indexOf('  function selectedSync()'),view.indexOf('  function updateSyncWidgets()'));
 const state={projectId:null,mode:'list',projects:[{id:'b',name:'Bravo'},{id:'a',name:'Alpha <x>'}],syncStatuses:[{projectId:'a',running:false,scm:{state:'synced',completed:2,total:2},slaps:{state:'error',completed:1,total:2,errors:1,detail:'Refresh authentication'},af:{state:'synced',completed:2,total:2}},{projectId:'b',running:true,percent:25}]};
 const html=new Function('state','options','e','sourceProgressLabel',source+'\nreturn syncResults();')(state,{},escape,sourceProgressLabel);
 assert(html.indexOf('Alpha &lt;x&gt;')<html.indexOf('Bravo'));
 assert.match(html,/Alpha &lt;x&gt; · Partial/);assert.match(html,/Refresh authentication/);
 assert.match(html,/1\/2 finished/);assert.match(html,/1 with issues/);
 assert.match(html,/data-hub-sync-details><summary>Details<\/summary>/);
 assert.equal((html.match(/<details class="image-hub-inventory-sync" open>/g)||[]).length,2);
 assert.equal((html.match(/class="image-hub-sync-source/g)||[]).length,6);
});

test('full inventory sync starts one native background batch and retains the full-list selection',async()=>{
 const source=view.slice(view.indexOf('  function bind()'),view.indexOf('  function openContextMenu('));
 const button={dataset:{hubAction:'sync-inventory'}};let requests=0;
 const state={projectId:null,folderId:null,mode:'list',query:'cache',expandedImages:new Set(),syncStatuses:[{projectId:'a',runId:'old',running:false}]};
 const root={querySelectorAll:selector=>selector==='[data-hub-action]'?[button]:[],querySelector:()=>null};
 const statuses=[{projectId:'a',runId:'batch',running:true},{projectId:'b',runId:'batch',running:true}];
 const bind=new Function('root','state','options','bindReportLinks','bindSecurityLinks','discard','mutate','syncImageHubInventory',source+'\nreturn bind;')(root,state,{},()=>{},()=>{},()=>true,async(action,after)=>after(await action()),async()=>{requests++;return statuses});
 bind();await button.onclick();
 assert.equal(requests,1);assert.equal(state.projectId,null);assert.equal(state.folderId,null);assert.equal(state.query,'cache');assert.deepEqual(state.syncStatuses,statuses);
});
