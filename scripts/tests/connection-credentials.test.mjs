import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
const source = await readFile(new URL('../../web/modules/connections/credential-policy.js', import.meta.url), 'utf8');
const { resolveCredentialPolicy, credentialSelection, parseCredentialSelection } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const entries = [{id:'a',name:'Root account'},{id:'b',name:'Team account'}];
const folders = [{id:'root',name:'Root',parentId:null,credentialMode:'keepass',credentialReference:'a'}, {id:'team',name:'Team',parentId:'root',credentialMode:'inherit'}, {id:'child',name:'Child',parentId:'team',credentialMode:'inherit'}];
const ssh={id:'host',name:'SSH',folderId:'child',connectionType:'ssh',credentialMode:'inherit'};

test('SSH/RDP inherit the nearest assigned folder account and a connection overrides it', () => {
  for (const connectionType of ['ssh','rdp']) {
    const effective=resolveCredentialPolicy({...ssh,connectionType},folders,entries);
    assert.equal(effective.account.id,'a');assert.equal(effective.source.name,'Root');
  }
  const nested=folders.map(folder=>folder.id==='team'?{...folder,credentialMode:'keepass',credentialReference:'b'}:folder);
  assert.equal(resolveCredentialPolicy(ssh,nested,entries).account.id,'b');
  assert.equal(resolveCredentialPolicy({...ssh,credentialMode:'keepass',credentialReference:'a'},nested,entries).account.id,'a');
  assert.equal(resolveCredentialPolicy({...ssh,credentialMode:'keepass',credentialReference:'a'},nested,entries).source,null);
  assert.equal(resolveCredentialPolicy(folders[2],nested,entries,{folder:true}).account.id,'b');
});

test('manual mode stops inheritance; missing references do not fall back to another account', () => {
  assert.equal(resolveCredentialPolicy({...ssh,credentialMode:'manual'},folders,entries).mode,'manual');
  const blocked=folders.map(folder=>folder.id==='team'?{...folder,credentialMode:'manual'}:folder);
  assert.equal(resolveCredentialPolicy(ssh,blocked,entries).mode,'manual');
  const missing=resolveCredentialPolicy({...ssh,credentialMode:'keepass',credentialReference:'deleted'},folders,entries);
  assert.equal(missing.mode,'keepass');assert.equal(missing.account,null);
  assert.equal(resolveCredentialPolicy({...ssh,connectionType:'url'},folders,entries).mode,'manual');
});

test('moving folders changes inheritance immediately and malformed cycles terminate', () => {
  const moved=folders.map(folder=>folder.id==='team'?{...folder,parentId:null}:folder);
  assert.equal(resolveCredentialPolicy(ssh,moved,entries).mode,'manual');
  const cycle=folders.map(folder=>folder.id==='team'?{...folder,parentId:'child'}:folder);
  assert.equal(resolveCredentialPolicy(ssh,cycle,entries).mode,'manual');
});

test('picker preserves existing explicit account choices and distinguishes manual from inherit', () => {
  assert.equal(credentialSelection({credentialReference:'a'}),'keepass:a');
  assert.deepEqual(parseCredentialSelection('keepass:a'),{mode:'keepass',reference:'a'});
  assert.deepEqual(parseCredentialSelection('manual'),{mode:'manual',reference:null});
  assert.deepEqual(parseCredentialSelection('inherit'),{mode:'inherit',reference:null});
});
