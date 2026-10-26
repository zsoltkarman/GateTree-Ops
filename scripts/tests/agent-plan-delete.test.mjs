import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../../web/modules/agent/agent-view.js', import.meta.url), 'utf8')
  .replace(/^import[\s\S]*?;\n/gm, '').replace(/^export /gm, '');

function fixture({ status = 'stopped', confirm = true, failure = false } = {}) {
  const calls = [];
  const state = { busy: false, error: '', jobSets: [{ id: 'plan', name: 'Kafka upgrade', planNumber: 10, status }],
    onQueueChange: plans => calls.push(['queue', plans]) };
  const view = new Function('window', 'deleteDeploymentPlan', 'escapeHtml', `${source}\nrender = () => {}; return { deletePlan, jobSetRows };`)(
    { confirm: () => confirm }, async id => {
      calls.push(['delete', id]);
      if (failure) throw new Error('Stop the Plan before deleting it.');
      return { plans: [] };
    }, String);
  return { calls, state, ...view };
}

test('stopped Plan offers deletion beside Focus; active Plans only offer Focus', () => {
  for (const status of ['stopped', 'failed', 'queued', 'running', 'awaiting_approval']) {
    const f = fixture({ status });
    const markup = f.jobSetRows(f.state);
    assert.match(markup, /data-action="focus-plan"/);
    assert.equal(markup.includes('data-action="delete-plan"'), !['running', 'awaiting_approval'].includes(status));
  }
});

test('confirmed deletion removes Agent row and updates the Queue', async () => {
  const f = fixture();
  await f.deletePlan({}, f.state, 'plan');
  assert.deepEqual(f.calls, [['delete', 'plan'], ['queue', []]]);
  assert.deepEqual(f.state.jobSets, []);
  assert.equal(f.state.error, '');
});

test('cancel, active status and repeated clicks do not delete a Plan', async () => {
  for (const options of [{ confirm: false }, { status: 'running' }, { status: 'awaiting_approval' }, { busy: true }]) {
    const f = fixture(options);
    if (options.busy) f.state.busy = true;
    await f.deletePlan({}, f.state, 'plan');
    assert.deepEqual(f.calls, []);
    assert.equal(f.state.jobSets.length, 1);
  }
});

test('a rejected deletion preserves the Plan and shows the error', async () => {
  const f = fixture({ failure: true });
  await f.deletePlan({}, f.state, 'plan');
  assert.equal(f.state.jobSets.length, 1);
  assert.match(f.state.error, /Stop the Plan/);
  assert.deepEqual(f.calls, [['delete', 'plan']]);
  assert.equal(f.state.busy, false);
});
