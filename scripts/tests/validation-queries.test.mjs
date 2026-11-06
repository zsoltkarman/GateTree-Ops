import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const source=readFileSync(new URL('../../web/modules/deployments/validation-queries.js',import.meta.url),'utf8').replace(/^import .*;\n/gm,'').replaceAll('export function ','function ');
const escapeHtml=value=>String(value).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
const mount=new Function('escapeHtml','saveValidationQuery','deleteValidationQuery','window',source+'\nreturn mountValidationQueries;')(escapeHtml,()=>{},()=>{},{confirm:()=>true});
class Element {
 constructor(dataset={}){this.dataset=dataset;this.listeners={};this.value='';}
 addEventListener(event,fn){this.listeners[event]=fn;}
 async fire(event,value){if(value!==undefined)this.value=value;return this.listeners[event]?.({target:this,preventDefault(){}});}
 focus(){}
 showModal(){this.open=true;}
}
class Root {
 isConnected=true;
 set innerHTML(html){
  this.html=html;this.nodes=new Map();
  for(const key of ['details','data-query-add','data-query-cancel','data-query-delete-dialog'])if(key==='details'||html.includes(key))this.nodes.set(key,new Element());
  this.edit=[...html.matchAll(/data-query-edit="([^"]+)"/g)].map(m=>new Element({queryEdit:m[1]}));
  this.remove=[...html.matchAll(/data-query-delete="([^"]+)"/g)].map(m=>new Element({queryDelete:m[1]}));
  this.fields=['name','query'].map(name=>{const field=new Element();field.name=name;field.tagName=name==='query'?'TEXTAREA':'INPUT';return field;});
  if(html.includes('data-query-form')){
   const form=new Element();form.reportValidity=()=>true;form.querySelectorAll=()=>this.fields;form.querySelector=s=>this.querySelector(s);this.nodes.set('data-query-form',form);
  }
 }
 querySelector(selector){if(selector==='[name="name"]')return this.fields[0];return this.nodes.get(selector==='details'?'details':selector.slice(1,-1))??null;}
 querySelectorAll(selector){return selector==='[data-query-edit]'?this.edit:this.remove;}
}
test('query library UI adds, edits and deletes named queries without a role through the persistence API',async()=>{
 const root=new Root();const writes=[];const deletes=[];
 mount(root,{save:async query=>{writes.push({...query});return {...query,id:'saved',revision:1};},remove:async id=>deletes.push(id)});
 await root.querySelector('[data-query-add]').fire('click');
 assert.ok(!root.html.includes('name="role"'));
 for(const [index,value] of ['Exporter version','vector(1)'].entries())await root.fields[index].fire('input',value);
 await root.querySelector('[data-query-form]').fire('submit');
 assert.equal(writes[0].name,'Exporter version');assert.ok(!Object.hasOwn(writes[0],'role'));assert.equal(writes[0].query,'vector(1)');
 assert.ok(!root.html.includes('Exporter version saved.'));
 await root.edit[0].fire('click');await root.fields[1].fire('input','vector(0)');
 await root.querySelector('[data-query-form]').fire('submit');assert.equal(writes[1].id,'saved');assert.equal(writes[1].query,'vector(0)');
 await root.remove[0].fire('click');assert.deepEqual(deletes,[]);
 const dialog=root.querySelector('[data-query-delete-dialog]');assert.ok(dialog.open);assert.ok(root.html.includes('Are you sure?'));dialog.returnValue='delete';await dialog.fire('close');assert.deepEqual(deletes,['saved']);assert.equal(root.edit.length,0);assert.ok(!root.html.includes('No saved queries yet.'));assert.ok(!root.html.includes('Reusable PromQL by role.'));
});
test('failed saves retain the editable query and display the persistence error',async()=>{
 const root=new Root();mount(root,{queries:[{id:'q',name:'Check',role:'exporter',query:'vector(1)',revision:1}],save:async()=>{throw new Error('Changed in another window');}});
 await root.edit[0].fire('click');await root.fields[1].fire('input','vector(0)');await root.querySelector('[data-query-form]').fire('submit');
 assert.ok(root.html.includes('Changed in another window'));assert.ok(root.html.includes('vector(0)'));assert.ok(root.querySelector('[data-query-form]'));
});


test('legacy role metadata is hidden and omitted when editing a saved query',async()=>{
 const root=new Root();const writes=[];
 mount(root,{queries:[{id:'q',name:'Saved',role:'legacy_role',query:'vector(1)',revision:1}],save:async query=>{writes.push(query);return query;}});
 assert.ok(!root.html.includes('legacy_role'));
 await root.edit[0].fire('click');
 assert.ok(!root.html.includes('name="role"'));
 await root.querySelector('[data-query-form]').fire('submit');
 assert.ok(!Object.hasOwn(writes[0],'role'));
});

 test('cancel and Escape leave saved queries untouched',async()=>{
 const root=new Root();const deletes=[];
 mount(root,{queries:[{id:'q',name:'Cache',role:'cache_exporter',query:'vector(1)',revision:1}],remove:async id=>deletes.push(id)});
 for(const result of ['cancel','']){
  await root.remove[0].fire('click');const dialog=root.querySelector('[data-query-delete-dialog]');
  dialog.returnValue=result;await dialog.fire('close');
  assert.deepEqual(deletes,[]);assert.equal(root.edit.length,1);assert.equal(root.querySelector('[data-query-delete-dialog]'),null);
 }
 });
