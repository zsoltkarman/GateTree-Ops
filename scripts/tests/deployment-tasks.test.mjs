import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const base = new URL('../../web/modules/deployments/', import.meta.url);
const planner = readFileSync(new URL('planner-plan.js', base), 'utf8').replaceAll('export function ', 'function ');
const model = readFileSync(new URL('tasks-model.js', base), 'utf8').replace(/^import .*;\n/gm, '').replaceAll('export function ', 'function ');
const { taskItemProgress, taskProgress, applyTaskItem } = new Function(planner + '\n' + model + '\nreturn {taskItemProgress,taskProgress,applyTaskItem};')();
const item = { id: 'service', role: 'ping_tool', planId: 'plan' };
const job = (id, status, driver = 'container', action = 'deploy') => ({ id, status, driver, action, target: id });
const plan = jobs => ({ id: 'plan', status: 'completed', jobs });

test('unplanned services stay visible; completed deployment and validation are separate', () => {
  const task = { items: [item, { ...item, id: 'other', planId: null }] };
  let result = taskProgress(task, [plan([job('consumer-1', 'completed')])]);
  assert.equal(result.total, 2); assert.equal(result.planned, 1); assert.equal(result.deployed, 1);
  assert.equal(result.validated, 0); assert.equal(result.ready, 0);
  assert.equal(result.items[1].progress.label, 'Not planned');
  assert.equal(result.items[0].progress.validation.label, 'Not configured');
  result = taskProgress(task, [plan([job('consumer-1', 'completed'), job('check', 'completed', 'thanos', 'query')])]);
  assert.equal(result.validated, 1); assert.equal(result.ready, 1);
});

test('one failed, disabled, skipped or unfinished target never marks a service ready', () => {
  for (const status of ['failed', 'disabled', 'skipped', 'queued', 'running', 'awaiting_approval', 'stopped', 'denied']) {
    const result = taskItemProgress(item, [plan([job('one', 'completed'), job('two', status), job('validation', 'completed', 'thanos', 'query')])]);
    assert.equal(result.done, false, status); assert.equal(result.deployment.done, false, status);
    assert.equal(result.deployment.completed, 1); assert.equal(result.deployment.total, 2);
  }
  const result = taskItemProgress(item, [{ ...plan([job('one', 'completed'), job('check', 'failed', 'thanos', 'query')]), status: 'failed' }]);
  assert.equal(result.deployment.done, true); assert.equal(result.validation.done, false);
  assert.equal(result.label, 'Needs attention');
});

test('reports and preflight checks are not counted as deployments; retry follows live state', () => {
  const jobs = [job('api', 'completed', 'olam', 'validate'), job('deploy', 'completed', 'olam', 'validate_deploy'), job('check', 'completed', 'thanos', 'query'), job('report', 'failed', 'jira', 'update')];
  const result = taskItemProgress(item, [{ ...plan(jobs), status: 'failed' }]);
  assert.equal(result.deployment.total, 1); assert.equal(result.done, false);
  assert.equal(taskItemProgress(item, [plan(jobs.map(job => ({ ...job, status: 'completed' })))]).done, true);
  assert.equal(taskItemProgress(item, []).label, 'Not planned');
});

test('task prefill preserves Ansible variables and clears old targets without changing validation', () => {
  const draft = { planName: '', steps: [{ name: 'Deploy', type: 'deploy', extraVarsText: 'TOKEN: "literal: value"\nIMAGE_VERSION: old' }, { type: 'thanos_query', query: 'metric{version="${DEPLOY_IMAGE_VERSION}"}', expectedVersion: '1.97' }], targetSelection: { roles: ['other'], sites: ['prod'], excluded: ['old-host'] } };
  applyTaskItem(draft, { jiraKey: 'GPDM-25741' }, item);
  assert.equal(draft.taskItemId, item.id); assert.equal(draft.jiraNo, 'GPDM-25741');
  assert.match(draft.steps[0].extraVarsText, /TOKEN: "literal: value"/);
  assert.match(draft.steps[0].extraVarsText, /IMAGE_VERSION: old/);
  assert.deepEqual(draft.targetSelection.roles, ['ping_tool']); assert.deepEqual(draft.targetSelection.sites, []);
  assert.equal(draft.steps[1].expectedVersion, '1.97');
  assert.equal(draft.steps[1].query, 'metric{version="${DEPLOY_IMAGE_VERSION}"}');
  const invalid = { steps: [{ extraVarsText: 'invalid input' }] };
  applyTaskItem(invalid, { jiraKey: 'GPDM-25741' }, item);
  assert.equal(invalid.steps[0].extraVarsText, 'invalid input');
  assert.equal(invalid.taskItemId, item.id);
});
