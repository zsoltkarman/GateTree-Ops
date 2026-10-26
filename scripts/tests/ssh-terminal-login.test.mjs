import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../../web/modules/connections/connections-view.js', import.meta.url), 'utf8');
const flowSource = source.slice(source.indexOf('async function connectSshInTerminal('), source.indexOf('async function connectSshWithDialog('));

function fixture({ transport = 'embedded', cancel = false, saveError = false } = {}) {
  const calls = [];
  const connection = { id: 'c', name: 'Host', connectionType: 'ssh', target: 'host', port: 2222,
    tag: 'tag', username: 'user', authentication: 'password', credentialReference: 'account',
    credentialMode: 'inherit', sshTransport: transport, icon: 'icon' };
  const session = { id: 's', connectionId: 'c' };
  const state = { sshSessions: [] };
  const connect = new Function('runAction', 'updateConnectionItem', 'connectSshWithDialog', 'showSshSession',
    `${flowSource}; return connectSshInTerminal;`)(
    async (_root, _state, action) => action(),
    async (...args) => { calls.push(['save', ...args]); if (saveError) throw new Error('Save failed'); return { ...connection, sshTransport: args[8] }; },
    async item => { calls.push(['connect', item.sshTransport]); return cancel ? null : session; },
    (_state, item) => calls.push(['show', item.id]),
  );
  return { calls, connection, session, state, run: () => connect({}, state, connection) };
}

test('terminal login saves the selected mode before connecting and preserves account settings', async () => {
  const f = fixture(); await f.run();
  assert.deepEqual(f.calls, [
    ['save', 'c', 'Host', 'host', 2222, 'tag', 'user', 'password', 'account', 'openssh', 'icon', 'inherit'],
    ['connect', 'openssh'], ['show', 's'],
  ]);
  assert.deepEqual(f.state.sshSessions, [f.session]);
});

test('an existing terminal login opens without rewriting its connection', async () => {
  const f = fixture({ transport: 'openssh' }); await f.run();
  assert.deepEqual(f.calls, [['connect', 'openssh'], ['show', 's']]);
});

test('cancelled login opens no session and failed preference save cannot start a connection', async () => {
  const cancelled = fixture({ cancel: true }); await cancelled.run();
  assert.deepEqual(cancelled.state.sshSessions, []);
  assert(!cancelled.calls.some(([action]) => action === 'show'));
  const failed = fixture({ saveError: true });
  await assert.rejects(failed.run(), /Save failed/);
  assert.equal(failed.connection.sshTransport, 'embedded');
  assert(!failed.calls.some(([action]) => action === 'connect'));
});
