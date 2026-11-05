import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const view=await readFile(new URL('../../web/modules/image-hub/image-hub-view.js',import.meta.url),'utf8');
const escape=value=>String(value).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
const functions=view.slice(view.indexOf('  function securityView()'),view.indexOf('  async function loadSecurityPage('));
function render(state) { return new Function('state','e','attr','imageHubUrl',functions+'\nreturn securityView();')(state,escape,escape,()=>null); }
function state() { return {securitySelection:{repositoryId:'repo',imageName:'sample',version:'1.2.3'},runtimeConfig:{slaps_realm:'TEST'},securityBusy:false,securityError:'',securityPage:{report:{realm:'TEST',decision:'unknown',fetchedAt:1},total:40,page:0,pageSize:2,findings:[{severity:'High',findingId:'GHSA-test',package:'example/module',installedVersion:'1.0',fixedVersion:'1.1',gatingDate:'IMMEDIATELY GATING'},{severity:'Low',findingId:'CVE-example',package:'long/package/"with quotes"',installedVersion:'2.0',fixedVersion:'2.1',details:'PRIVATE LONG DETAILS'}]}}; }
test('compact findings show six columns and only the requested page, without raw details',()=>{
 const html=render(state());
 assert.equal((html.match(/<th>/g)||[]).length,6);
 assert.equal((html.match(/<tr>/g)||[]).length,3);
 assert.match(html,/40 findings · Page 1 \/ 20/);
 assert.match(html,/GHSA-test/);assert.match(html,/example\/module/);
 assert.match(html,/&quot;with quotes&quot;/);
 assert.doesNotMatch(html,/PRIVATE LONG DETAILS/);
 assert.match(html,/security-prev" disabled/);assert.doesNotMatch(html,/security-next" disabled/);
 assert.match(html,/image-hub-findings-table/);
});
test('missing reports, authentication failures and empty downloaded reports are distinguishable',()=>{
 const missing=state();missing.securityPage={report:null,total:0,page:0,pageSize:2,findings:[]};
 assert.match(render(missing),/Run Sync Project to download/);
 const failed=state();failed.securityPage.report={decision:'error',error:'Refresh authentication'};failed.securityPage.findings=[];
 assert.match(render(failed),/Refresh authentication/);assert.match(render(failed),/No findings loaded/);
 const empty=state();empty.securityPage.findings=[];empty.securityPage.total=0;
 assert.match(render(empty),/No findings in the downloaded report/);
});
test('SLAPS clicks select the exact image/version for the local table and prevent browser navigation',()=>{
 const source=view.slice(view.indexOf('  function bindSecurityLinks()'),view.indexOf('  function bindReportLinks()'));
 const link={dataset:{hubSecurity:'r1',hubSecurityImage:'image-one',hubSecurityVersion:'2.3.4'}};
 const current={};let prevented=0,stopped=0,loaded=0;
 new Function('root','state','discard','render','loadSecurityPage',source+'\nreturn bindSecurityLinks();')({querySelectorAll:()=>[link]},current,()=>true,()=>{},()=>loaded++);
 link.onclick({preventDefault(){prevented++},stopPropagation(){stopped++}});
 assert.deepEqual(current.securitySelection,{repositoryId:'r1',imageName:'image-one',version:'2.3.4'});
 assert.equal(current.mode,'security');assert.equal(loaded,1);assert.equal(prevented,1);assert.equal(stopped,1);
});
test('a late response for another image cannot replace the selected report',async()=>{
 const source=view.slice(view.indexOf('  async function loadSecurityPage('),view.indexOf('  function bindSecurityLinks()'));
 const current={securitySelection:{repositoryId:'r1',imageName:'one',version:'1'},securityRevision:0,mode:'security'};
 const pending=[];
 const load=new Function('state','contentRoot','render','getImageHubSecurityReport',source+'\nreturn loadSecurityPage;')(current,{clientHeight:600,isConnected:true},()=>{},input=>new Promise(resolve=>pending.push({input,resolve})));
 const old=load();current.securitySelection={repositoryId:'r2',imageName:'two',version:'2'};const latest=load();
 assert.ok(pending[0].input.pageSize>=1 && pending[0].input.pageSize<=12);
 pending[1].resolve({report:{id:'two'}});await latest;
 pending[0].resolve({report:{id:'one'}});await old;
 assert.equal(current.securityPage.report.id,'two');assert.equal(current.securityBusy,false);
});
