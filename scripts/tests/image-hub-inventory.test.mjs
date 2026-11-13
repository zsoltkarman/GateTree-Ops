import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const source=readFileSync(new URL('../../web/modules/image-hub/inventory.js',import.meta.url),'utf8');
const {inventoryRepositories,inventoryFolderPath,inventorySlapsStatus}=await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const snapshot={folders:[{id:'apps',name:'Applications',parentId:null},{id:'nested',name:'Exporters',parentId:'apps'},{id:'other',name:'Other',parentId:null}],projects:[{id:'p',name:'Monitoring',folderId:'nested'}],repositories:[
 {id:'r3',name:'cache-10',folderId:'nested',projectId:'p',repoType:'scm',localPath:'/dev/cache-10'},
 {id:'r2',name:'cache-2',folderId:null,projectId:'p',repoType:'scm',localPath:'/dev/cache-2'},
 {id:'r1',name:'alpha',folderId:'other',projectId:null,repoType:'github',localPath:'/dev/alpha'},
 {id:'root',name:'zeta',folderId:null,projectId:null,repoType:'gitlab',localPath:'/dev/zeta'}
]};

test('default inventory includes every repository across projects, root and nested folders without duplicates',()=>{
 const before=JSON.stringify(snapshot);const rows=inventoryRepositories(snapshot);
 assert.equal(rows.length,4);assert.equal(new Set(rows.map(row=>row.id)).size,4);
 assert.deepEqual(rows.map(row=>row.name),['cache-2','cache-10','alpha','zeta']);
 assert.equal(rows[0].inventoryFolder,'Applications / Exporters');assert.equal(rows[0].inventoryProject,'Monitoring');
 assert.equal(JSON.stringify(snapshot),before);
});
test('folder filtering includes descendants and inherits the project folder when a repo has no folder',()=>{
 assert.deepEqual(inventoryRepositories(snapshot,{folderId:'apps'}).map(row=>row.id),['r2','r3']);
 assert.deepEqual(inventoryRepositories(snapshot,{folderId:'nested'}).map(row=>row.id),['r2','r3']);
 assert.deepEqual(inventoryRepositories(snapshot,{folderId:'other'}).map(row=>row.id),['r1']);
});
test('search, project and type filters combine, and name sorting is natural',()=>{
 assert.deepEqual(inventoryRepositories(snapshot,{query:'MONITORING cache-2',projectId:'p',repoType:'scm'}).map(row=>row.id),['r2']);
 assert.equal(inventoryRepositories(snapshot,{query:'exporters',repoType:'github'}).length,0);
 assert.equal(inventoryRepositories(snapshot,{query:'/dev/alpha'})[0].id,'r1');
 assert.deepEqual(inventoryRepositories(snapshot,{sort:'name'}).map(row=>row.name),['alpha','cache-2','cache-10','zeta']);
});
test('cyclic or missing folder references do not loop or hide repositories from the combined list',()=>{
 const cyclic={...snapshot,folders:[{id:'apps',name:'Applications',parentId:'nested'},{id:'nested',name:'Exporters',parentId:'apps'}]};
 assert.equal(inventoryRepositories(cyclic).length,4);
 assert.equal(inventoryRepositories(cyclic,{folderId:'absent'}).length,0);
 assert.equal(inventoryFolderPath(cyclic.folders,'absent'),'');
});

const view=readFileSync(new URL('../../web/modules/image-hub/image-hub-view.js',import.meta.url),'utf8');
const escape=value=>String(value??'').replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
test('the inventory uses the same latest/image report table for the root and project filters',()=>{
 const body=view.slice(view.indexOf('  function list()'),view.indexOf('  function imageRows('));
 const state={...snapshot,query:'',repoTypeFilter:'',inventorySort:'folder',folderId:null,projectId:null};
 const render=new Function('state','options','inventoryRepositories','e','folderOptions','imageRows','types','projectTypes','folderPath',body+'\nreturn list;')(state,{},inventoryRepositories,escape,()=>'<option>All</option>',repo=>`<tr data-repo="${repo.id}"></tr>`,{scm:'SCM',github:'GitHub'},{scm:'SCM'},()=> 'Folder');
 let html=render();assert.equal((html.match(/data-repo=/g)||[]).length,4);
 for(const label of ['Folder / Project','Repo name','AF','SLAPS','Build Logs','All projects','Search inventory']) assert(html.includes(label),label);
 state.projectId='p';html=render();assert.equal((html.match(/data-repo=/g)||[]).length,2);
});
test('project management offers add/edit/sync controls in Config and no repository rows',()=>{
 const body=view.slice(view.indexOf('  function list()'),view.indexOf('  function imageRows('));
 const render=new Function('state','options','e','projectTypes','folderPath',body+'\nreturn list;')({...snapshot,syncStatuses:[]},{manageProjects:true},escape,{scm:'SCM'},()=> 'Folder');
 const html=render();assert(html.includes('data-hub-action="edit-project"'));assert(html.includes('data-project-id="p"'));assert(html.includes('data-hub-action="sync-project"'));assert(!html.includes('cache-10'));
});
test('right-click project creation exists only in Config; Inventory retains folder/repository actions',()=>{
 const body=view.slice(view.indexOf('  function openContextMenu('),view.indexOf('    root.querySelector(".image-hub-module").append(menu);'))+'return menu.innerHTML; }';
 const render=manageProjects=>new Function('options','document','closeContextMenu','folderPath','icon','e',body+'\nreturn openContextMenu;')({manageProjects},{createElement:()=>({setAttribute(){}})},()=>{},()=> 'Root',()=>'',escape)({},null,null,null);
 assert(!render(false).includes('data-hub-create="project"'));assert(render(false).includes('data-hub-create="folder"'));assert(render(false).includes('data-hub-create="repository"'));
 assert(render(true).includes('data-hub-create="project"'));assert(!render(true).includes('data-hub-create="repository"'));
});

test('Config edit selects the correct project and Inventory cannot open the project editor',async()=>{
 const body=view.slice(view.indexOf('  function bind()'),view.indexOf('  function openContextMenu('));
 for(const manageProjects of [true,false]) {
  const state={...snapshot,expandedImages:new Set(),projectId:null,mode:'list'};
  const button={dataset:{hubAction:'edit-project',projectId:'p'}};let renders=0;
  const root={querySelectorAll:selector=>selector==='[data-hub-action]'?[button]:[],querySelector:()=>null};
  const bind=new Function('root','state','options','bindReportLinks','bindSecurityLinks','discard','render',body+'\nreturn bind;')(root,state,{manageProjects},()=>{},()=>{},()=>true,()=>renders++);
  bind();await button.onclick();
  assert.equal(renders,manageProjects?1:0);
  if(manageProjects){assert.equal(state.mode,'project');assert.equal(state.projectId,'p');assert.deepEqual(state.draft,snapshot.projects[0]);assert.notEqual(state.draft,snapshot.projects[0]);}
 }
});

test('moving a project saves its selected folder without starting repository sync',async()=>{
 const body=view.slice(view.indexOf('  function bind()'),view.indexOf('  function openContextMenu('));
 const state={...snapshot,projectId:null,mode:'list',expanded:new Set(),draft:null};
 const button={dataset:{hubAction:'move-project',projectId:'p'}};
 const handlers={};const form={addEventListener:(name,handler)=>{handlers[name]=handler;}};
 const root={querySelectorAll:selector=>selector==='[data-hub-action]'?[button]:[],querySelector:selector=>selector==='[data-hub-form]'?form:null};
 let saved,expanded,rendered=0;
 const bind=new Function('root','state','options','bindReportLinks','bindSecurityLinks','discard','render','captureDraft','mutate','saveImageHubProject','expandAncestors',body+'\nreturn bind;')(root,state,{manageProjects:true},()=>{},()=>{},()=>true,()=>rendered++,()=>{state.draft.folderId='other';},async(action,after)=>after(await action()),async input=>{saved={...input};return input.id;},id=>{expanded=id;});
 bind();await button.onclick();
 assert.equal(rendered,1);assert.equal(state.mode,'project-folder');assert.equal(state.projectId,'p');
 assert.notEqual(state.draft,snapshot.projects[0]);
 await handlers.submit({preventDefault(){}});
 assert.deepEqual(saved,{...snapshot.projects[0],folderId:'other'});
 assert.equal(state.mode,'list');assert.equal(state.folderId,'other');assert.equal(expanded,'other');assert(state.expanded.has('p'));
});

const scannedSnapshot = () => ({...snapshot,
 latestImages: [{repositoryId:'r2',imageName:'cache-2',version:'2',status:'found'}, {repositoryId:'r3',imageName:'cache-10',version:'3',status:'found'}],
 securityReports: [{projectId:'p',imageName:'cache-2',version:'2',decision:'unknown',findingCount:0}, {projectId:'p',imageName:'cache-10',version:'3',decision:'approved',findingCount:3}]
});
test('green SLAPS filtering uses zero findings on the latest exact image and combines with existing filters',()=>{
 const data=scannedSnapshot(), before=JSON.stringify(data);
 assert.deepEqual(inventoryRepositories(data,{slapsStatus:'no-findings',folderId:'apps',projectId:'p',repoType:'scm',query:'cache'}).map(r=>r.id),['r2']);
 assert.equal(inventoryRepositories(data,{slapsStatus:'no-findings',query:'cache-10'}).length,0);
 assert.equal(inventoryRepositories(data).length,4);
 assert.equal(inventorySlapsStatus(data,data.repositories[0]),'has-findings');
 assert.equal(JSON.stringify(data),before);
});
test('red SLAPS filtering selects latest images with findings and excludes unavailable scans',()=>{
 const data=scannedSnapshot();
 assert.deepEqual(inventoryRepositories(data,{slapsStatus:'has-findings',projectId:'p',query:'cache'}).map(r=>r.id),['r3']);
 data.securityReports[1].decision='pending';
 assert.equal(inventoryRepositories(data,{slapsStatus:'has-findings'}).length,0);
 data.securityReports[1].decision='error';
 assert.equal(inventoryRepositories(data,{slapsStatus:'has-findings'}).length,0);
 assert.equal(inventoryRepositories(data).length,4);
});
test('pending, failed, missing, wrong-version and other-project reports cannot make a latest image green',()=>{
 for(const report of [null,{decision:'pending',findingCount:0},{decision:'error',findingCount:0},{decision:'unknown',findingCount:0,error:'Unavailable'},{decision:'approved'},{decision:'unknown',findingCount:-1},{decision:'unknown',findingCount:0.5},{decision:'unknown',findingCount:'0'},{decision:'unknown',findingCount:0,version:'old'},{decision:'unknown',findingCount:0,projectId:'other'}]) {
  const data=scannedSnapshot();data.securityReports=report?[{projectId:'p',imageName:'cache-2',version:'2',...report}]:[];
  assert.equal(inventoryRepositories(data,{slapsStatus:'no-findings'}).length,0,JSON.stringify(report));
 }
 const data=scannedSnapshot();data.latestImages[0].version='new';
 assert.equal(inventoryRepositories(data,{slapsStatus:'no-findings'}).length,0);
 data.latestImages=[];data.securityReports[0].version='';
 assert.equal(inventoryRepositories(data,{slapsStatus:'no-findings'}).length,0);
});
test('the green filter retains last-known AF versions and updates membership when a scan arrives',()=>{
 const data=scannedSnapshot();data.latestImages[0].status='error';
 assert.deepEqual(inventoryRepositories(data,{slapsStatus:'no-findings'}).map(r=>r.id),['r2']);
 data.securityReports[1].findingCount=0;
 assert.deepEqual(inventoryRepositories(data,{slapsStatus:'no-findings'}).map(r=>r.id),['r2','r3']);
 data.latestImages[0].imageName='different-image';
 assert.deepEqual(inventoryRepositories(data,{slapsStatus:'no-findings'}).map(r=>r.id),['r3']);
});
test('Inventory exposes and applies All, Green and Red latest-image SLAPS filters',()=>{
 const body=view.slice(view.indexOf('  function list()'),view.indexOf('  function imageRows('));
 const state={...scannedSnapshot(),query:'',repoTypeFilter:'',slapsFilter:'no-findings',inventorySort:'folder',folderId:null,projectId:null};
 const list=new Function('state','options','inventoryRepositories','e','folderOptions','imageRows','types','projectTypes','folderPath',body+'\nreturn list;')(state,{},inventoryRepositories,escape,()=>'',repo=>`<tr data-repo="${repo.id}"></tr>`,{}, {},()=> 'Folder');
 const html=list();assert.match(html,/SLAPS \(latest\)/);assert.match(html,/value="no-findings" selected/);assert.match(html,/1 \/ 4 repositories/);assert.doesNotMatch(html,/data-repo="r3"/);
 state.slapsFilter='has-findings';
 const redHtml=list();assert.match(redHtml,/value="has-findings" selected/);assert.match(redHtml,/Red · Findings/);assert.match(redHtml,/data-repo="r3"/);assert.doesNotMatch(redHtml,/data-repo="r2"/);
 const bindBody=view.slice(view.indexOf('  function bind()'),view.indexOf('  function openContextMenu('));
 let renders=0;const control={dataset:{hubFilter:'slaps'},value:'',focus(){}};
 const root={querySelectorAll:selector=>selector==='[data-hub-filter]'?[control]:[],querySelector:selector=>selector==='[data-hub-filter="slaps"]'?control:null};
 const bind=new Function('root','state','options','bindReportLinks','bindSecurityLinks','render',bindBody+'\nreturn bind;')(root,state,{},()=>{},()=>{},()=>renders++);
 bind();control.onchange();assert.equal(state.slapsFilter,'');assert.equal(renders,1);assert.equal(inventoryRepositories(state).length,4);
});

test('active green filtering rerenders when sync downloads new reports without changing repository IDs',async()=>{
 const body=view.slice(view.indexOf('  function scheduleSyncPoll()'),view.indexOf('  function list()'));
 const state={...scannedSnapshot(),syncStatuses:[{running:true}],mode:'list',slapsFilter:'no-findings'};
 let scheduled, renders=0;
 const snapshot={repositories:state.repositories,securityReports:[...state.securityReports.slice(0,1),{...state.securityReports[1],findingCount:0}],syncStatuses:[{running:true}]};
 const schedule=new Function('state','window','contentRoot','mountId','root','getImageHub','render','updateArtifactCells','updateSyncWidgets',`let syncPollTimer=null; ${body}\nreturn scheduleSyncPoll;`)(state,{setTimeout:callback=>{scheduled=callback;return 1;}},{isConnected:true,dataset:{imageHubMountId:'test'}},'test',{querySelector:selector=>selector==='.image-hub-module'?{}:null},async()=>snapshot,()=>renders++,()=>assert.fail('Filtered membership needs a full rerender'),()=>{});
 schedule();await scheduled();
 assert.equal(renders,1);
 assert.deepEqual(inventoryRepositories(state,{slapsStatus:state.slapsFilter}).map(r=>r.id),['r2','r3']);
});
