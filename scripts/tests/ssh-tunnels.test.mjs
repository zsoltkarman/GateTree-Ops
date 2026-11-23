import test from 'node:test';
import assert from 'node:assert/strict';
import { readTunnelRows, bindTunnelEditor } from '../../web/modules/connections/tunnel-editor.js';

const tunnel = { sourcePort: '13306', destinationHost: ' 127.0.0.1 ', destinationPort: '3306' };
const makeForm = rules => ({ querySelectorAll: () => rules.map(rule => ({ querySelector: selector => ({ value: rule[selector.match(/"(.*?)"/)[1]] }) })) });

test('SQL mapping trims destination and preserves custom ports; deleting all rules is supported', () => {
  assert.deepEqual(readTunnelRows(makeForm([tunnel])), [{ sourcePort: 13306, destinationHost: '127.0.0.1', destinationPort: 3306 }]);
  assert.deepEqual(readTunnelRows(makeForm([])), []);
});

test('invalid and duplicate ports cannot be saved', () => {
  for (const sourcePort of ['0', '65536', '1.5', '']) assert.throws(() => readTunnelRows(makeForm([{ ...tunnel, sourcePort }])));
  assert.throws(() => readTunnelRows(makeForm([tunnel, tunnel])), /different local/);
  assert.throws(() => readTunnelRows(makeForm([{ ...tunnel, destinationHost: ' ' }])));
});

test('save errors retain the form and permit retry; success updates the tunnel count', async () => {
  const form = makeForm([tunnel]), listeners = {}, feedback = {}, button = {}, summary = {};
  form.dataset = { connectionId: 'sql-host' };
  form.addEventListener = (type, listener) => { listeners[type] = listener; };
  form.querySelector = selector => selector.includes('feedback') ? feedback : button;
  const root = { querySelector: selector => selector.includes('summary') ? summary : form };
  let fail = true;
  bindTunnelEditor(root, async (id, rules) => {
    assert.equal(id, 'sql-host'); assert.equal(rules[0].sourcePort, 13306);
    if (fail) throw new Error('Save failed');
  });
  await listeners.submit({ preventDefault() {} });
  assert.equal(feedback.textContent, 'Save failed'); assert.equal(button.disabled, false);
  fail = false;
  await listeners.submit({ preventDefault() {} });
  assert.match(feedback.textContent, /Saved/); assert.match(summary.textContent, /1 local/);
});
