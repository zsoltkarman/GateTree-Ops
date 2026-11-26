import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const source=readFileSync(new URL('../../web/modules/deployments/query-sets.js',import.meta.url),'utf8').replace(/^import .*;\n/gm,'').replaceAll('export ','');
const escapeHtml=value=>String(value).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
const { mountQuerySets, emptyQuerySet }=new Function('escapeHtml','saveQuerySet','deleteQuerySet',source+'\nreturn {mountQuerySets,emptyQuerySet};')(escapeHtml,()=>{},()=>{});
class Element {
 constructor(dataset={},type='text'){this.dataset=dataset;this.type=type;this.listeners={};this.value='';}
 addEventListener(name,fn){this.listeners[name]=fn;}
 async fire(name,value){if(value!==undefined)this.value=value;return this.listeners[name]?.({target:this,preventDefault(){}});}
}
class Root {
 isConnected=true;
 set innerHTML(html){
  this.html=html;this.controls=new Map();
  for(const name of ['set-add','set-cancel','item-add'])if(html.includes('data-'+name))this.controls.set(`[data-${name}]`,new Element());
  this.edits=[...html.matchAll(/data-set-edit="([^"]+)"/g)].map(m=>new Element({setEdit:m[1]}));
  this.deletes=[...html.matchAll(/data-set-delete="([^"]+)"/g)].map(m=>new Element({setDelete:m[1]}));
  this.removes=[...html.matchAll(/data-item-remove="([^"]+)"/g)].map(m=>new Element({itemRemove:m[1]}));
  this.fields=['name','stableSamples','intervalSeconds','timeoutSeconds','maxAgeSeconds'].map(key=>new Element({setField:key},key==='name'?'text':'number'));
  this.rows=[...html.matchAll(/data-set-item="([^"]+)"/g)].map(m=>{const row=new Element({setItem:m[1]});row.fields=['queryId','scope','expectedVersion','minResults'].map(key=>new Element({itemField:key},key==='minResults'?'number':'text'));row.querySelectorAll=()=>row.fields;return row;});
  if(html.includes('data-set-form')){const form=new Element();form.reportValidity=()=>true;form.querySelector=s=>this.controls.get(s);form.querySelectorAll=s=>({'[data-set-field]':this.fields,'[data-set-item]':this.rows,'[data-item-remove]':this.removes}[s]??[]);this.controls.set('[data-set-form]',form);}
 }
 querySelector(s){return this.controls.get(s);}
 querySelectorAll(s){return {'[data-set-edit]':this.edits,'[data-set-delete]':this.deletes}[s]??[];}
}
test('Query Set editor persists scopes, cardinality and a shared policy, and reloads saved sets',async()=>{
 const root=new Root(),saved=[];mountQuerySets(root,{queries:[{id:'q',name:'Under-replicated'}],save:async set=>{saved.push(set);return {...set,id:'set',revision:1};}});
 await root.querySelector('[data-set-add]').fire('click');
 await root.fields[0].fire('input','Kafka health');
 await root.rows[0].fields[0].fire('change','q');await root.rows[0].fields[1].fire('change','cluster');await root.rows[0].fields[3].fire('change','3');
 await root.querySelector('[data-set-form]').fire('submit');
 assert.equal(saved[0].items[0].queryId,'q');assert.equal(saved[0].items[0].minResults,3);assert.equal(saved[0].stableSamples,3);assert.equal(saved[0].timeoutSeconds,1200);
 assert.ok(root.html.includes('Kafka health'));await root.edits[0].fire('click');
 assert.ok(root.html.includes('value="Kafka health"'));assert.ok(root.html.includes('Under-replicated'));
});
test('failed set saves preserve edits and expose errors; cancelling performs no write',async()=>{
 const root=new Root();let calls=0;mountQuerySets(root,{save:async()=>{calls++;throw Error('Query was deleted');}});
 await root.querySelector('[data-set-add]').fire('click');await root.fields[0].fire('input','Kafka');
 await root.querySelector('[data-set-form]').fire('submit');assert.equal(calls,1);assert.ok(root.html.includes('Query was deleted'));assert.ok(root.html.includes('value="Kafka"'));
 await root.querySelector('[data-set-cancel]').fire('click');assert.equal(root.querySelector('[data-set-form]'),undefined);assert.equal(calls,1);
});
test('editor refreshes query choices and adds/removes required checks',async()=>{
 const root=new Root();const view=mountQuerySets(root);
 await root.querySelector('[data-set-add]').fire('click');await root.querySelector('[data-item-add]').fire('click');assert.equal(root.rows.length,2);
 await root.removes[0].fire('click');assert.equal(root.rows.length,1);
 view.updateQueries([{id:'new',name:'Newly saved query'}]);assert.ok(root.html.includes('Newly saved query'));
 assert.equal(emptyQuerySet().items.length,0);
});
