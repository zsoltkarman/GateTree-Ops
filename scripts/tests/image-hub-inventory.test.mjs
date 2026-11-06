import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const source=readFileSync(new URL('../../web/modules/image-hub/inventory.js',import.meta.url),'utf8');
const {inventoryRepositories,inventoryFolderPath}=await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
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
