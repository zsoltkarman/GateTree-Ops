import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const source=readFileSync(new URL('../../web/modules/deployments/validation-queries.js',import.meta.url),'utf8').replace(/^import .*;\n/gm,'').replaceAll('export function ','function ');
const escapeHtml=value=>String(value).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
const mount=new Function('escapeHtml','saveValidationQuery','deleteValidationQuery','getLocalDeploymentRoles','window',source+'\nreturn mountValidationQueries;')(escapeHtml,()=>{},()=>{},async()=>['cache_exporter','exporter'],{confirm:()=>true});
class Element {
 constructor(dataset={}){this.dataset=dataset;this.listeners={};this.value='';}
 addEventListener(event,fn){this.listeners[event]=fn;}
 async fire(event,value){if(value!==undefined)this.value=value;return this.listeners[event]?.({target:this,preventDefault(){}});}
 focus(){}
}
class Root {
 isConnected=true;
 set innerHTML(html){
  this.html=html;this.nodes=new Map();
  for(const key of ['details','data-query-add','data-query-cancel'])if(key==='details'||html.includes(key))this.nodes.set(key,new Element());
  this.edit=[...html.matchAll(/data-query-edit="([^"]+)"/g)].map(m=>new Element({queryEdit:m[1]}));
  this.remove=[...html.matchAll(/data-query-delete="([^"]+)"/g)].map(m=>new Element({queryDelete:m[1]}));
  this.fields=['name','role','query'].map(name=>{const field=new Element();field.name=name;field.tagName=name==='role'?'SELECT':name==='query'?'TEXTAREA':'INPUT';return field;});
  if(html.includes('data-query-form')){
   const form=new Element();form.reportValidity=()=>true;form.querySelectorAll=()=>this.fields;form.querySelector=s=>this.querySelector(s);this.nodes.set('data-query-form',form);
  }
 }
 querySelector(selector){if(selector==='[name="name"]')return this.fields[0];return this.nodes.get(selector==='details'?'details':selector.slice(1,-1))??null;}
 querySelectorAll(selector){return selector==='[data-query-edit]'?this.edit:this.remove;}
}
test('query library UI adds, edits and deletes named role queries through the persistence API',async()=>{
 const root=new Root();const writes=[];const deletes=[];
 mount(root,{save:async query=>{writes.push({...query});return {...query,id:'saved',revision:1};},remove:async id=>deletes.push(id)});
 await root.querySelector('[data-query-add]').fire('click');
 assert.ok(root.html.includes('<select name="role"'));assert.ok(root.html.includes('cache_exporter'));
 for(const [index,value] of ['Exporter version','exporter','vector(1)'].entries())await root.fields[index].fire(index===1?'change':'input',value);
 await root.querySelector('[data-query-form]').fire('submit');
 assert.equal(writes[0].name,'Exporter version');assert.equal(writes[0].role,'exporter');assert.equal(writes[0].query,'vector(1)');
 assert.ok(!root.html.includes('Exporter version saved.'));
 await root.edit[0].fire('click');await root.fields[2].fire('input','vector(0)');
 await root.querySelector('[data-query-form]').fire('submit');assert.equal(writes[1].id,'saved');assert.equal(writes[1].query,'vector(0)');
 await root.remove[0].fire('click');assert.deepEqual(deletes,['saved']);assert.equal(root.edit.length,0);assert.ok(!root.html.includes('No saved queries yet.'));assert.ok(!root.html.includes('Reusable PromQL by role.'));
});
test('failed saves retain the editable query and display the persistence error',async()=>{
 const root=new Root();mount(root,{queries:[{id:'q',name:'Check',role:'exporter',query:'vector(1)',revision:1}],save:async()=>{throw new Error('Changed in another window');}});
 await root.edit[0].fire('click');await root.fields[2].fire('input','vector(0)');await root.querySelector('[data-query-form]').fire('submit');
 assert.ok(root.html.includes('Changed in another window'));assert.ok(root.html.includes('vector(0)'));assert.ok(root.querySelector('[data-query-form]'));
});


test('role dropdown preserves a saved role absent from the current local project',async()=>{
 const root=new Root();mount(root,{queries:[{id:'q',name:'Saved',role:'legacy_role',query:'vector(1)',revision:1}],loadRoles:async()=>['exporter']});
 await root.edit[0].fire('click');
 assert.ok(root.html.includes('value="legacy_role" selected'));assert.ok(root.html.includes('value="exporter"'));
});
