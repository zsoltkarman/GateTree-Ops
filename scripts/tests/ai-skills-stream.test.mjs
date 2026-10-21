import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

test('skill runs receive isolated live channels before completion', async () => {
  const calls = [];
  const window = { __TAURI__: { core: {
    Channel: class {},
    invoke: (name, args) => new Promise(resolve => calls.push({ name, args, resolve })),
  } } };
  const source = readFileSync(new URL('../../web/modules/ai-skills/ai-skills-api.js', import.meta.url), 'utf8').replaceAll('export const ', 'const ');
  const run = new Function('window', source + '; return runAiSkill;')(window);
  const first = [], second = [];
  const pendingFirst = run('first', 'task', chunk => first.push(chunk));
  const pendingSecond = run('second', '', chunk => second.push(chunk));
  assert.equal(calls[0].name, 'run_ai_skill');
  assert.equal(calls[0].args.input, 'task');
  calls[0].args.onOutput.onmessage('working');
  calls[1].args.onOutput.onmessage('other run');
  assert.deepEqual(first, ['working']);
  assert.deepEqual(second, ['other run']);
  calls[0].resolve({ output: 'working done', status: 'Completed' });
  calls[1].resolve({ output: 'other run', status: 'Completed' });
  assert.equal((await pendingFirst).output, 'working done');
  assert.equal((await pendingSecond).output, 'other run');
});
