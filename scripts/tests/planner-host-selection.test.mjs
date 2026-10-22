import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// Parse attributes as HTML does, including terminating values at an unescaped quote.
class Element {
  constructor(tag = '') {
    this.attributes = Object.fromEntries([...tag.matchAll(/([\w-]+)(?:="([^"]*)")?/g)].map(m => [m[1], m[2] ?? '']));
    this.dataset = Object.fromEntries(Object.entries(this.attributes).filter(([k]) => k.startsWith('data-')).map(([k,v]) => [k.slice(5).replace(/-([a-z])/g, (_,c) => c.toUpperCase()), v]));
    this.value = this.attributes.value ?? '';
    this.checked = 'checked' in this.attributes;
    this.disabled = 'disabled' in this.attributes;
    this.listeners = {}; this.scrollTop = 0;
  }
  set innerHTML(html) { this.html = html; this.nodes = [...html.matchAll(/<(?:input|select|button|details|section|div)\b[^>]*>/g)].map(m => new Element(m[0])); }
  querySelectorAll(selector) {
    if (selector.startsWith('.')) return this.nodes.filter(n => (n.attributes.class ?? '').split(' ').includes(selector.slice(1)));
    const attrs = [...selector.matchAll(/\[([^=\]]+)(?:="([^"]*)")?\]/g)];
    return this.nodes.filter(n => attrs.every(([,key,value]) => key in n.attributes && (value === undefined || n.attributes[key] === value)));
  }
  querySelector(selector) { return selector === 'summary' ? new Element() : this.querySelectorAll(selector)[0]; }
  addEventListener(type, fn) { this.listeners[type] = fn; }
  fire(type) { this.listeners[type]?.({ target: this }); }
  focus() {}
}
const source = readFileSync(new URL('../../web/modules/deployments/planner-targets.js', import.meta.url), 'utf8').replace(/^import .*;\n/gm, '');
const snapshot = { environment:'Test', readAt:1, revision:'v1', warnings:[], sites:[{name:'kc1', group:'env_kc1', hosts:[1,2,3].map(n => ({name:`redis-${n}`, roles:['cache']}))}] };
async function mount() {
  const document = {addEventListener(){}, removeEventListener(){}};
  // Same text escaping behavior as shared/ui.js: quotes are literal in text content.
  const escapeHtml = value => String(value).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;');
  const mountTargets = new Function('document','escapeHtml', source.replaceAll('export function ', 'function ') + '\nreturn mountPlannerTargets;')(document,escapeHtml);
  const root = new Element(); root.isConnected = true;
  const draft = {processId:'p', targetSelection:{processId:'p', roles:['cache'], sites:['kc1'], excluded:[]}};
  const controller = mountTargets(root, {draft,loadSettings:async()=>({path:'/inventory'}),loadInventory:async()=>structuredClone(snapshot)});
  await new Promise(resolve => setImmediate(resolve));
  return {draft,controller,content:root.querySelector('[data-target-content]')};
}
test('individual host checkboxes and bulk controls change saved deployment targets', async () => {
  const {draft,content,controller} = await mount();
  for (let index of [0,2]) { const input=content.querySelectorAll('[data-target-host]')[index]; input.checked=false; input.fire('change'); }
  assert.deepEqual(draft.olamTargets.hosts,['redis-2']);
  assert.deepEqual(content.querySelectorAll('[data-target-host]').map(n=>n.checked),[false,true,false]);
  content.querySelector('[data-deselect-all-hosts]').fire('click');
  assert.deepEqual(draft.olamTargets.hosts,[]);
  let input=content.querySelectorAll('[data-target-host]')[2]; input.checked=true; input.fire('change');
  assert.deepEqual(draft.olamTargets.hosts,['redis-3']);
  content.querySelector('[data-select-all-hosts]').fire('click');
  assert.deepEqual(draft.olamTargets.hosts,['redis-1','redis-2','redis-3']);
  controller.dispose();
});
test('moving hosts preserves selection and order through refresh and bulk selection', async () => {
  const {draft,content,controller} = await mount();
  content.querySelector('[data-move-host="2"][data-direction="-1"]').fire('click');
  content.querySelector('[data-move-host="1"][data-direction="-1"]').fire('click');
  assert.deepEqual(draft.olamTargets.hosts,['redis-3','redis-1','redis-2']);
  let input=content.querySelectorAll('[data-target-host]')[1]; input.checked=false; input.fire('change');
  assert.deepEqual(draft.olamTargets.hosts,['redis-3','redis-2']);
  content.querySelector('[data-target-refresh]').fire('click');
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(draft.olamTargets.hosts,['redis-3','redis-2']);
  content.querySelector('[data-select-all-hosts]').fire('click');
  assert.deepEqual(draft.olamTargets.hosts,['redis-3','redis-1','redis-2']);
  const mode=content.querySelector('[data-job-mode]'); mode.value='together'; mode.fire('change');
  assert(content.querySelectorAll('[data-move-host]').every(n=>n.disabled));
  controller.dispose();
});
