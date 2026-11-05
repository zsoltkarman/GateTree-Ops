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
test('the footer lists only active sync sources and keeps failures visible',()=>{
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
