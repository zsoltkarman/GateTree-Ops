import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const base = new URL('../../web/modules/deployments/', import.meta.url);
const source = readFileSync(new URL('tasks-view.js', base), 'utf8').replace(/^import .*;\n/gm, '').replace('export async function ', 'async function ');
const planner = readFileSync(new URL('planner-plan.js', base), 'utf8').replaceAll('export function ', 'function ');
const model = readFileSync(new URL('tasks-model.js', base), 'utf8').replace(/^import .*;\n/gm, '').replaceAll('export function ', 'function ');
const { taskProgress, applyTaskItem } = new Function(planner + '\n' + model + '\nreturn {taskProgress,applyTaskItem};')();
const escapeText = value => String(value).replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
const flush = () => new Promise(resolve => setImmediate(resolve));

class Element {
  dataset = {}; isConnected = true; value = ''; disabled = false;
  replaceChildren(child) { if (this.child) this.child.isConnected = false; this.child = child; }
  set innerHTML(html) {
    this.html = html;
    this.nodes = [...html.matchAll(/<(button|select|form|input|p)\b([^>]*)>/g)].map(([, tag, attrs]) => {
      const node = new Element(); node.tagName = tag.toUpperCase(); node.attrs = {};
      for (const [, key, value = ''] of attrs.matchAll(/([\w-]+)(?:="([^"]*)")?/g)) node.attrs[key] = value;
      node.disabled = 'disabled' in node.attrs; node.value = node.attrs.value ?? '';
      if (tag === 'select') {
        const options = html.slice(html.indexOf(`<select${attrs}>`) + `<select${attrs}>`.length).split('</select>')[0];
        node.value = [...options.matchAll(/<option\b([^>]*)>/g)].find(([, attributes]) => /\bselected\b/.test(attributes))?.[1].match(/value="([^"]*)"/)?.[1] ?? '';
      }
      for (const [key, value] of Object.entries(node.attrs)) if (key.startsWith('data-')) node.dataset[key.slice(5).replace(/-([a-z])/g, (_, char) => char.toUpperCase())] = value;
      return node;
    });
  }
  querySelectorAll(selector) {
    const match = selector.match(/^\[([^=\]]+)(?:="([^"]*)")?\]$/);
    return match ? (this.nodes ?? []).filter(node => match[1] in node.attrs && (match[2] === undefined || node.attrs[match[1]] === match[2])) : [];
  }
  querySelector(selector) { return this.querySelectorAll(selector)[0] ?? null; }
  reportValidity() { return true; }
  closest() { return null; }
  focus() {}
}

async function fixture({ tasks = [], plans = [], failSave = false, linkError = null, roles = ['ping_tool', 'acg'], rolesError = null } = {}) {
  let saved = structuredClone(tasks); const writes = [], links = [], deletions = [], opened = [], created = [], timers = [];
  const root = new Element();
  const mount = new Function('escapeText', 'getDeploymentQueue', 'getDeploymentTasks', 'saveDeploymentTask', 'deleteDeploymentTask', 'linkDeploymentTaskPlan', 'taskProgress', 'applyTaskItem', 'emptyPlannerDraft', 'document', 'setTimeout', 'CSS', 'getLocalDeploymentRoles', source + '\nreturn mountDeploymentTasksView;')(
    escapeText, async () => ({ plans: structuredClone(plans) }), async () => structuredClone(saved),
    async task => {
      if (failSave) throw Error('Task changed; refresh first');
      writes.push(structuredClone(task));
      const next = { ...structuredClone(task), id: task.id || 'new-task', revision: task.revision + 1, items: task.items.map((item, index) => ({ ...item, id: item.id || `new-service-${index}` })) };
      saved = [...saved.filter(task => task.id !== next.id), next];
      return next.id;
    },
    async id => { deletions.push(id); saved = saved.filter(task => task.id !== id); },
    async (itemId, planId) => { if (linkError) throw Error(linkError); links.push({ itemId, planId }); saved.forEach(task => task.items.forEach(item => { if (item.id === itemId) item.planId = planId; })); },
    taskProgress, applyTaskItem, () => ({ planName: '', steps: [{ type: 'deploy', extraVarsText: '' }] }),
    { createElement: () => new Element(), activeElement: null, hidden: false }, callback => timers.push(callback), { escape: value => value },
    async () => { if (rolesError) throw Error(rolesError); return roles; },
  );
  await mount(root, { taskId: tasks[0]?.id }, { onOpenPlan: id => opened.push(id), onCreatePlan: draft => created.push(draft) });
  const host = root.child;
  const click = async selector => { const node = host.querySelector(selector); assert.ok(node, selector); assert.equal(node.disabled, false, selector); node.onclick(); await flush(); };
  const input = (name, value, index) => host.querySelector('[data-task-form]')[name === 'role' ? 'onchange' : 'oninput']({ target: { name, value, closest: () => index === undefined ? null : { dataset: { serviceIndex: String(index) } } } });
  const submit = async () => { host.querySelector('[data-task-form]').onsubmit({ preventDefault() {} }); await flush(); };
  return { root, host, writes, links, deletions, opened, created, timers, click, input, submit };
}

const task = () => ({ id: 'task', jiraKey: 'GPDM-25741', name: 'Security rollout', revision: 1, items: [{ id: 'service', role: 'ping_tool', planId: null }] });

test('Tasks UI saves a Jira task and planned services without deploying anything', async () => {
  const view = await fixture();
  await view.click('[data-task-new]');
  view.input('jiraKey', 'GPDM-25741'); view.input('name', 'Security rollout');
  view.input('role', 'ping_tool', 0);
  await view.submit();
  assert.equal(view.writes[0].items[0].role, 'ping_tool');
  assert.match(view.host.html, /Security rollout/); assert.match(view.host.html, /Not planned/);
  assert.deepEqual(view.created, []); assert.deepEqual(view.links, []);
  await view.click('[data-task-create="new-service-0"]');
  assert.equal(view.created[0].taskItemId, 'new-service-0'); assert.equal(view.created[0].jiraNo, 'GPDM-25741');
  assert.equal(view.created[0].steps[0].extraVarsText, '');
  assert.equal(view.created[0].planName, 'GPDM-25741 · ping_tool');
});

test('Tasks UI links, opens and unlinks existing Plans without losing Queue history', async () => {
  const view = await fixture({ tasks: [task()], plans: [{ id: 'plan', planNumber: 42, name: 'Ping', status: 'queued', jobs: [] }] });
  view.host.querySelector('[data-task-link-choice="service"]').value = 'plan';
  view.host.querySelector('[data-task-link-choice="service"]').onchange();
  await view.click('[data-task-link="service"]');
  assert.deepEqual(view.links, [{ itemId: 'service', planId: 'plan' }]);
  await view.click('[data-task-plan="plan"]'); assert.deepEqual(view.opened, ['plan']);
  await view.click('[data-task-unlink="service"]');
  assert.deepEqual(view.links[1], { itemId: 'service', planId: null });
  assert.match(view.host.html, /Create Plan/);
  await view.click('[data-task-delete]'); assert.deepEqual(view.deletions, []);
  await view.click('[data-task-delete]'); assert.deepEqual(view.deletions, ['task']);
});

test('pending Queue selection survives live refresh and Save link persists the selected Plan', async () => {
  const plans = [{ id: 'plan', planNumber: 42, name: 'Ping', status: 'queued', jobs: [] }];
  const view = await fixture({ tasks: [task()], plans });
  assert.equal(view.host.querySelector('[data-task-link="service"]').disabled, true);
  const select = view.host.querySelector('[data-task-link-choice="service"]');
  select.value = 'plan'; select.onchange();
  plans[0].status = 'running';
  await view.timers[0]();
  assert.equal(view.host.querySelector('[data-task-link-choice="service"]').value, 'plan');
  assert.equal(view.host.querySelector('[data-task-link="service"]').disabled, false);
  await view.click('[data-task-link="service"]');
  assert.deepEqual(view.links, [{ itemId: 'service', planId: 'plan' }]);
  assert.ok(view.host.querySelector('[data-task-plan="plan"]'));
  assert.match(view.host.html, /Plan link saved/);
});

test('rejected Plan links keep the selection and display the backend reason beside Save link', async () => {
  const view = await fixture({ tasks: [task()], plans: [{ id: 'plan', planNumber: 42, name: 'Ping', status: 'queued', jobs: [] }], linkError: 'Plan Jira must match the Task Jira.' });
  const select = view.host.querySelector('[data-task-link-choice="service"]');
  select.value = 'plan'; select.onchange();
  await view.click('[data-task-link="service"]');
  assert.deepEqual(view.links, []);
  assert.equal(view.host.querySelector('[data-task-link-choice="service"]').value, 'plan');
  assert.match(view.host.html, /data-task-link-error="service" role="alert">Error: Plan Jira must match the Task Jira\./);
  assert.equal(view.host.querySelector('[data-task-link="service"]').disabled, false);
});

test('failed saves retain the draft, escape quoted service names and stop polling detached views', async () => {
  const initial = task(); initial.items[0].role = 'role" autofocus="true';
  const view = await fixture({ tasks: [initial], failSave: true });
  await view.click('[data-task-edit]');
  const role = view.host.querySelectorAll('[name="role"]')[0];
  assert.equal(role.attrs.autofocus, undefined); assert.match(view.host.html, /role&quot; autofocus=&quot;true/);
  view.input('name', 'Keep this edit'); await view.submit();
  assert.match(view.host.html, /Keep this edit/); assert.match(view.host.html, /Task changed; refresh first/);
  view.root.replaceChildren(new Element());
  const before = view.host.html;
  await view.timers[0]();
  assert.equal(view.host.html, before); assert.equal(view.timers.length, 1);
});

test('Tasks expand and collapse as table rows while keeping summary counts visible', async () => {
  const view = await fixture({ tasks: [task()] });
  assert.match(view.host.html, /class="task-table task-overview"/);
  assert.match(view.host.html, /<th>Planned<\/th>/);
  assert.match(view.host.html, /aria-expanded="true"/);
  await view.click('[data-task-select="task"]');
  assert.match(view.host.html, /aria-expanded="false"/);
  assert.equal(view.host.querySelector('[data-task-create="service"]'), null);
  await view.click('[data-task-select="task"]');
  assert.ok(view.host.querySelector('[data-task-create="service"]'));
});

test('Task creation needs only Jira and local roles, without a version field', async () => {
  const view = await fixture({ roles: ['acg', 'ping_tool'] });
  await view.click('[data-task-new]');
  assert.equal(view.host.querySelector('[name="role"]').tagName, 'SELECT');
  assert.equal(view.host.querySelector('[name="targetVersion"]'), null);
  view.input('role', 'ping_tool', 0);
  view.input('jiraKey', 'GPDM-25741');
  await view.submit();
  assert.equal(view.writes[0].jiraKey, 'GPDM-25741');
  assert.equal(view.writes[0].items[0].role, 'ping_tool');
  assert.equal(Object.hasOwn(view.writes[0].items[0], 'targetVersion'), false);
});

test('missing local Ansible folder explains unavailable roles and preserves saved roles', async () => {
  const view = await fixture({ tasks: [task()], rolesError: 'Choose a local Ansible folder first' });
  await view.click('[data-task-edit]');
  assert.match(view.host.html, /Choose a local Ansible folder first/);
  assert.match(view.host.html, /ping_tool \(not in local project\)/);
  assert.equal(view.host.querySelector('[name="targetVersion"]'), null);
});
