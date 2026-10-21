import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
const root = new URL('../../', import.meta.url);
const source = (await readFile(new URL('web/modules/updates/update-controller.js', root), 'utf8')).replace(/^import .*;\n/, 'const escapeHtml = value => String(value ?? "");\n');
const { createUpdateController } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);

function fixture(invoke) {
  const elements = new Map();
  const element = name => {
    if (!elements.has(name)) elements.set(name, { textContent: '', hidden: false, disabled: false, removeAttribute() {} });
    return elements.get(name);
  };
  const handlers = {};
  const dialog = {
    setAttribute() {}, showModal() {}, focus() {}, remove() {},
    querySelector: element,
    querySelectorAll: () => [element('[data-update-install]'), element('[data-update-later]')],
    addEventListener: (name, callback) => { handlers[name] = callback; },
    close: () => handlers.close(),
  };
  globalThis.document = { activeElement: { focus() {} }, createElement: () => dialog, body: { append() {} }, querySelector: () => null };
  const controller = createUpdateController({ invoke, listen: async () => () => {} });
  return { controller, element, handlers };
}

test('manual check shows release notes but only explicit Update can install the reviewed version', async () => {
  const calls = [];
  const f = fixture(async (command, args) => {
    calls.push([command, args]);
    if (command === 'check_app_update') return { enabled: true, currentVersion: '1.0.0', version: '1.1.0', notes: '<script>release notes</script>' };
  });
  await f.controller.check();
  assert.equal(calls.length, 1);
  assert.equal(f.element('[data-update-notes]').textContent, '<script>release notes</script>');
  assert.equal(f.element('[data-update-install]').hidden, false);
  await f.element('[data-update-install]').onclick();
  assert.deepEqual(calls[1], ['install_app_update', { expectedVersion: '1.1.0' }]);
});

test('Later never installs and busy Agent errors keep the update available for retry', async () => {
  let installs = 0;
  const f = fixture(async command => {
    if (command === 'check_app_update') return { enabled: true, currentVersion: '1.0.0', version: '1.1.0' };
    installs++;
    throw new Error('Finish active deployments first');
  });
  await f.controller.check();
  await f.element('[data-update-install]').onclick();
  assert.equal(installs, 1);
  assert.match(f.element('[data-update-status]').textContent, /active deployments/);
  assert.equal(f.element('[data-update-later]').disabled, false);
  f.element('[data-update-later]').onclick();
  assert.equal(installs, 1);
});

test('debug checks cannot offer installation and offline automatic checks stay unobtrusive', async () => {
  const f = fixture(async () => ({ enabled: false, currentVersion: '1.0.0' }));
  await f.controller.check();
  assert.equal(f.element('[data-update-install]').hidden, true);
  f.controller.destroy();
  const offline = createUpdateController({ invoke: async () => { throw Error('Offline'); }, listen: async () => () => {} });
  await offline.check({ automatic: true });
});

test('release configuration isolates the update feed and uses a per-user Windows installer', async () => {
  const config = JSON.parse(await readFile(new URL('src-tauri/tauri.conf.json', root), 'utf8'));
  assert.equal(config.bundle.windows.nsis.installMode, 'currentUser');
  assert.equal(config.bundle.createUpdaterArtifacts, true);
  assert.match(config.plugins.updater.endpoints[0], /GateTree-Ops-updates\/releases\/latest\/download\/latest.json$/);
  assert(config.plugins.updater.pubkey.length > 40);
});

test('failed manual checks stop displaying Checking for updates', async () => {
  const f = fixture(async () => { throw 'The automatic-update feed is not available. A published release must include a valid latest.json file.'; });
  await f.controller.check();
  assert.equal(f.element('[data-update-version]').textContent, 'Update information unavailable');
  assert.match(f.element('[data-update-status]').textContent, /latest.json/);
});

test('install immediately shows Updating and restores the button with an explicit failure', async () => {
  let rejectInstall;
  const f = fixture(async command => {
    if (command === 'check_app_update') return { enabled: true, currentVersion: '0.1.17', version: '0.1.18' };
    return new Promise((_, reject) => { rejectInstall = reject; });
  });
  await f.controller.check();
  const pending = f.element('[data-update-install]').onclick();
  assert.equal(f.element('[data-update-install]').textContent, 'Updating…');
  assert.equal(f.element('[data-update-install]').disabled, true);
  await new Promise(resolve => setImmediate(resolve));
  rejectInstall('The installation folder is not writable');
  await pending;
  assert.equal(f.element('[data-update-install]').textContent, 'Update and restart');
  assert.equal(f.element('[data-update-install]').disabled, false);
  assert.match(f.element('[data-update-status]').textContent, /^Update failed:.*not writable/);
});
