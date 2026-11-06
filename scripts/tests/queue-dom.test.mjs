import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const source=readFileSync(new URL('../../web/modules/deployments/queue-dom.js',import.meta.url),'utf8').replaceAll('export ','');
const {reconcileQueue}=new Function(source+'\nreturn {reconcileQueue};')();
let mutations=0;
class Node {
 constructor(name,attributes={},children=[]) {
  this.nodeName=name;this.nodeType=name==='#text'?3:1;this.attrs={...attributes};this.childNodes=[];this.parentNode=null;this.nodeValue=null;
  this.dataset=new Proxy({}, {get:(_,key)=>this.attrs['data-'+String(key).replace(/[A-Z]/g,c=>'-'+c.toLowerCase())],set:(_,key,value)=>{this.setAttribute('data-'+String(key).replace(/[A-Z]/g,c=>'-'+c.toLowerCase()),value);return true}});
  this.classList={contains:name=>(this.attrs.class??'').split(' ').includes(name)};
  for(const child of children)this.insertBefore(child,null);
 }
 get attributes(){return Object.entries(this.attrs).map(([name,value])=>({name,value}));}
 get firstChild(){return this.childNodes[0]??null;}
 get nextSibling(){const items=this.parentNode?.childNodes??[];return items[items.indexOf(this)+1]??null;}
 getAttribute(name){return this.attrs[name]??null;}
 hasAttribute(name){return name in this.attrs;}
 setAttribute(name,value){this.attrs[name]=value;mutations++;}
 removeAttribute(name){delete this.attrs[name];mutations++;}
 insertBefore(node,cursor){
  node.remove();const index=cursor===null?this.childNodes.length:this.childNodes.indexOf(cursor);assert(index>=0);
  this.childNodes.splice(index,0,node);node.parentNode=this;mutations++;
 }
 remove(){if(this.parentNode){const items=this.parentNode.childNodes;items.splice(items.indexOf(this),1);this.parentNode=null;mutations++;}}
 cloneNode(deep){const copy=new Node(this.nodeName,this.attrs,deep?this.childNodes.map(child=>child.cloneNode(true)):[]);copy.nodeValue=this.nodeValue;return copy;}
}
const text=value=>{const node=new Node('#text');node.nodeValue=value;return node};
const el=(name,attributes={},...children)=>new Node(name,attributes,children);
const control=(action='start')=>el('BUTTON',{'data-control-job':'job','data-control-action':action,'aria-label':action},text(action));

test('a status refresh preserves button identity, listeners, payload draft and open log node',()=>{
 const button=control();button.listener=()=> 'clicked';
 const log=el('PRE',{'data-queue-log':'job-log:job'},text('large live log'));const selection=log.childNodes[0];
 const editor=el('TEXTAREA',{'data-plan-payload':'plan'},text('saved YAML'));editor.value='unsaved local edits';
 const current=el('SECTION',{class:'queue-module',style:'--queue-columns:custom'},button,log,editor);
 const incoming=el('SECTION',{class:'queue-module'},control('stop'),el('PRE',{'data-queue-log':'job-log:job'},text('Loading')),el('TEXTAREA',{'data-plan-payload':'plan',readonly:''},text('server YAML')));
 reconcileQueue(current,incoming);
 assert.equal(current.childNodes[0],button);assert.equal(button.listener(),'clicked');assert.equal(button.getAttribute('data-control-action'),'stop');
 assert.equal(current.childNodes[1],log);assert.equal(log.childNodes[0],selection);assert.equal(selection.nodeValue,'large live log');
 assert.equal(current.childNodes[2],editor);assert.equal(editor.value,'unsaved local edits');assert(editor.hasAttribute('readonly'));
 assert.equal(current.attrs.style,'--queue-columns:custom');
});

test('keyed insertion, reordering and removal keep surviving controls attached once',()=>{
 const a=el('ARTICLE',{'data-plan-group':'a'},control());const b=el('ARTICLE',{'data-plan-group':'b'},control());
 const current=el('DIV',{},a,b);
 reconcileQueue(current,el('DIV',{},el('ARTICLE',{'data-plan-group':'b'},control('stop')),el('ARTICLE',{'data-plan-group':'c'},control())));
 assert.equal(current.childNodes.length,2);assert.equal(current.childNodes[0],b);assert.equal(a.parentNode,null);
 assert.equal(current.childNodes[1].dataset.planGroup,'c');assert.equal(b.childNodes[0].dataset.controlAction,'stop');
});

test('conditional header controls do not replace neighbouring action buttons',()=>{
 const run=control('stop');const progress=el('SPAN',{class:'queue-plan-progress'},text('1/2'));
 const current=el('DIV',{},progress,run);
 reconcileQueue(current,el('DIV',{},el('BUTTON',{'data-duplicate-plan':'p'},text('Copy')),el('SPAN',{class:'queue-plan-progress'},text('2/2')),control('restart')));
 assert.equal(current.childNodes[1],progress);assert.equal(current.childNodes[2],run);assert.equal(run.dataset.controlAction,'restart');
});

test('an identical refresh makes no attribute or structural mutations',()=>{
 const current=el('DIV',{},el('BUTTON',{'data-control-job':'job','data-control-action':'stop','data-tooltip':'Stop'},text('Stop')));
 const incoming=el('DIV',{},el('BUTTON',{'data-control-job':'job','data-control-action':'stop',title:'Stop'},text('Stop')));
 mutations=0;reconcileQueue(current,incoming);assert.equal(mutations,0);
});

test('a job moving between status groups retains its row, controls and open log DOM',()=>{
 const button=control('stop');const log=el('PRE',{'data-queue-log':'job-log:job'},text('live log'));
 const item=el('DIV',{'data-job-item':'job','data-job-group-membership':'p:running'},button,log);
 const current=el('DIV',{},el('BUTTON',{'data-job-group':'p:running'},text('Running')),item);
 const incoming=el('DIV',{},el('BUTTON',{'data-job-group':'p:completed'},text('Completed')),el('DIV',{'data-job-item':'job','data-job-group-membership':'p:completed',hidden:''},control('restart'),el('PRE',{'data-queue-log':'job-log:job'},text('Loading'))));
 reconcileQueue(current,incoming);
 assert.equal(current.childNodes[1],item);assert.equal(item.childNodes[0],button);assert.equal(item.childNodes[1],log);
 assert.equal(item.dataset.jobGroupMembership,'p:completed');assert(item.hasAttribute('hidden'));
 assert.equal(button.dataset.controlAction,'restart');assert.equal(log.childNodes[0].nodeValue,'live log');
});
