import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
const base = new URL('../../web/modules/workspace-tabs/', import.meta.url);
const modelSource = await readFile(new URL('tab-model.js', base), 'utf8');
const modelUrl = `data:text/javascript;base64,${Buffer.from(modelSource).toString('base64')}`;
const { moveTab, nextTabAfterClose } = await import(modelUrl);
const source = (await readFile(new URL('workspace-tabs.js', base), 'utf8')).replace(/^import .*;\n/gm, '');
const prelude = `import { moveTab, nextTabAfterClose } from '${modelUrl}'; const escapeHtml = value => String(value ?? ''); const connectionIcon = () => '<svg></svg>';\n`;
const { mountWorkspaceTabs } = await import(`data:text/javascript;base64,${Buffer.from(prelude+source).toString('base64')}`);

class Element {
  children = []; hidden = false; parent = null; scrollLeft = 0; dataset = {}; attributes = {};
  setAttribute(name, value) { this.attributes[name] = value; }
  addEventListener() {}
  querySelectorAll() { return []; }
  contains() { return false; }
  append(child) { child.parent = this; this.children.push(child); }
  replaceChildren() { this.children = []; }
  remove() { if (this.parent) this.parent.children = this.parent.children.filter(child => child !== this); }
}
function fixture({ overview = true } = {}) {
  const bar = new Element(), panels = new Element(), strip = new Element(), more = new Element(), menu = new Element();
  bar.querySelector = selector => selector === '[role="tablist"]' ? strip : selector === '[role="menu"]' ? menu : more;
  bar.classList = { add() {} };
  globalThis.document = { activeElement: null, createElement: () => new Element(), addEventListener() {} };
  globalThis.requestAnimationFrame = callback => callback();
  const errors = [];
  const tabs = mountWorkspaceTabs(bar, panels, { onError: error => errors.push(error) });
  if (overview) tabs.open({ id: 'overview', title: 'Overview', type: 'overview', closable: false });
  return { tabs, errors, panels, bar, strip, menu };
}

test('reordering keeps Overview first and closing selects an adjacent surviving tab', () => {
  const tabs = ['overview', 'ssh:a', 'ssh:b', 'plan:1'].map(id => ({id}));
  assert.deepEqual(moveTab(tabs, 'plan:1', 'ssh:a').map(tab => tab.id), ['overview','plan:1','ssh:a','ssh:b']);
  assert.equal(moveTab(tabs, 'overview', 'plan:1'), tabs);
  assert.deepEqual(moveTab(tabs, 'ssh:a', 'plan:1', true).map(tab => tab.id), ['overview','ssh:b','plan:1','ssh:a']);
  assert.equal(nextTabAfterClose(tabs,'ssh:a'),'ssh:b');
  assert.equal(nextTabAfterClose(tabs,'plan:1'),'ssh:b');
});

test('SSH and Plan panes mount once and preserve state across tab switches', () => {
  const { tabs } = fixture();
  let mounted = 0, disposed = 0, disconnects = 0;
  const first = tabs.open({ id:'ssh:a',title:'Host A',type:'ssh',onClose:()=>disconnects++,mount:pane=>{ mounted++;pane.buffer='retained terminal output';return {dispose:()=>disposed++}; }});
  tabs.open({id:'plan:1',title:'Plan 1',type:'plan',mount:()=>{mounted++;}});
  tabs.activate('ssh:a');tabs.activate('overview');tabs.activate('plan:1');tabs.activate('ssh:a');
  assert.equal(mounted,2);assert.equal(disposed,0);assert.equal(disconnects,0);
  assert.equal(tabs.get('ssh:a').pane,first.pane);assert.equal(first.pane.buffer,'retained terminal output');
  tabs.destroy();assert.equal(disposed,1);assert.equal(disconnects,0); // GUI teardown leaves Agent sessions running.
});

test('SSH close waits for disconnect once; failure keeps the terminal available', async () => {
  const { tabs, errors } = fixture();
  let disconnects=0, complete;
  tabs.open({id:'ssh:a',title:'Host A',type:'ssh',onClose:()=>{disconnects++;return new Promise(resolve=>{complete=resolve;});}});
  const close=tabs.close('ssh:a');await tabs.close('ssh:a');
  assert(tabs.get('ssh:a'));assert.equal(disconnects,1);
  complete();await close;assert.equal(tabs.get('ssh:a'),undefined);assert.equal(tabs.activeId(),'overview');
  tabs.open({id:'ssh:b',title:'Host B',type:'ssh',onClose:()=>{throw Error('Disconnect failed');}});
  await tabs.close('ssh:b');assert(tabs.get('ssh:b'));assert.equal(errors.length,1);
  tabs.destroy();
});

test('closing a Plan only disposes its view; Overview cannot be closed', async () => {
  const { tabs }=fixture();let disposed=0;
  tabs.open({id:'plan:1',title:'Plan 1',type:'plan',mount:()=>({dispose:()=>disposed++})});
  await tabs.close('plan:1');assert.equal(disposed,1);assert.equal(tabs.activeId(),'overview');
  await tabs.close('overview');assert(tabs.get('overview'));
  tabs.destroy();
});


test('only terminal tabs appear; closing the last one returns to the menu workspace', async () => {
  const { tabs, bar, strip, menu } = fixture({ overview:false });
  // The application uses an invisible context pane instead of Overview/Connections tabs.
  tabs.open({ id:'workspace', title:'Connections', type:'workspace', hiddenTab:true, closable:false });
  tabs.open({ id:'plan:9', title:'Plan 9', type:'plan', hiddenTab:true });
  tabs.remove('plan:9');
  assert(bar.hidden);
  assert(!strip.innerHTML.includes('workspace'));
  for (let n=0;n<20;n++) tabs.open({id:`ssh:${n}`,title:`Server ${n}`,type:'ssh',host:`192.0.2.${n+1}`,port:22});
  assert.equal((strip.innerHTML.match(/role="tab"/g)||[]).length,20);
  assert(menu.innerHTML.includes('192.0.2.20:22 · SSH'));
  assert(!menu.innerHTML.includes('Connections'));
  for (let n=0;n<20;n++) await tabs.close(`ssh:${n}`);
  assert.equal(tabs.activeId(),'workspace');
  assert(bar.hidden);
  tabs.activate('workspace');
  assert.equal(tabs.activeId(),'workspace');
  tabs.destroy();
});


test('independent SSH sessions for the same saved connection retain separate panes and close separately', async () => {
  const {tabs}=fixture({overview:false});let closed=[];
  tabs.open({id:'workspace',title:'Connections',type:'workspace',hiddenTab:true,closable:false});
  const make=id=>({id:`ssh:${id}`,type:'ssh',title:'Same host',sessionId:id,detail:{connectionId:'one-connection'},onClose:()=>closed.push(id),mount:pane=>{pane.buffer=id;return{dispose(){}};}});
  tabs.open(make('a'));tabs.open(make('b'));tabs.open(make('c'));
  assert.equal(tabs.entries().filter(tab=>tab.type==='ssh').length,3);
  tabs.activate('ssh:a');assert.equal(tabs.get('ssh:a').pane.buffer,'a');
  await tabs.close('ssh:b');assert.deepEqual(closed,['b']);
  assert(tabs.get('ssh:a'));assert(tabs.get('ssh:c'));assert.equal(tabs.activeId(),'ssh:a');
  tabs.activate('ssh:c');assert.equal(tabs.get('ssh:c').pane.buffer,'c');tabs.destroy();
});
