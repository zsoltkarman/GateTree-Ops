import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
const source = await readFile(new URL('../../web/shared/connection-icons.js', import.meta.url), 'utf8');
const { URL_ICONS, connectionIcon, connectionIconPicker } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);

test('GateTree URL choices have stable IDs and an accessible selection', () => {
  assert.deepEqual(URL_ICONS.map(icon => icon.id), Array.from({ length: 12 }, (_, i) => `icon${i+1}`));
  const picker = connectionIconPicker('icon9');
  assert.equal((picker.match(/type="radio"/g) || []).length, 12);
  assert.equal((picker.match(/ checked/g) || []).length, 1);
  assert.match(picker, /value="icon9" aria-label="Chart" checked/);
});

test('shared URL icons use the selected palette while SSH and RDP stay distinct and unsafe IDs fall back', () => {
  assert(connectionIcon('url', 'icon1').includes('#0052c7'));
  assert(connectionIcon('url', 'icon9').includes('#b049ca'));
  assert.notEqual(connectionIcon('ssh'), connectionIcon('rdp'));
  assert.equal(connectionIcon('url', '<script>'), connectionIcon('url', 'icon12'));
  assert(!connectionIcon('url', '<script>').includes('<script>'));
});
