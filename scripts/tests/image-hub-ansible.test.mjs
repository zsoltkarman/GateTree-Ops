import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const view = await readFile(new URL('../../web/modules/image-hub/image-hub-view.js', import.meta.url), 'utf8');
const urlSource = await readFile(new URL('../../web/modules/image-hub/url-template.js', import.meta.url), 'utf8');
const { imageHubUrl, imageHubRepositoryUrl } = await import(`data:text/javascript;base64,${Buffer.from(urlSource).toString('base64')}`);
const escape = value => String(value).replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');
const repo = { id: 'r1', name: 'sample-image' };
function setup() {
  const state = { latestImages: [], securityReports: [], expandedImages: new Set(), ansibleStatus: 'loaded', runtimeConfig: {
    artifactPageUrlTemplate: 'https://images.example.test/{repo}/{version}',
    slapsUrlTemplate: 'https://security.example.test/?artifact={repo}',
    buildLogsUrlTemplate: 'https://build.example.test/{repo}',
  }, ansibleImages: ['test', 'stage', 'production', 'other'].map((environment, i) => ({
    environment, imageName: repo.name, role: 'sample', source: 'vars/sample.yml', version: `1.0.${i}`,
  })) };
  const functions = view.slice(view.indexOf('  function imageRows('), view.indexOf('  function artifactCell('));
  const { imageRows } = new Function('state', 'e', 'imageHubUrl', 'icon', 'artifactCell', 'attr', functions + '\nreturn {imageRows};')(state, escape, imageHubUrl, kind => kind, () => '2.0.0', escape);
  return { state, imageRows };
}
test('images start collapsed; expanding shows all environments and their own AF versions and report links', () => {
  const { state, imageRows } = setup();
  let html = imageRows(repo);
  assert.equal((html.match(/<tr /g) || []).length, 1);
  assert.match(html, /aria-expanded="false"/);
  assert.doesNotMatch(html, /Ansible ·/);
  state.expandedImages.add(repo.id);
  html = imageRows(repo);
  assert.equal((html.match(/<tr /g) || []).length, 5);
  assert.match(html, /aria-expanded="true"/);
  for (const image of state.ansibleImages) {
    assert.ok(html.includes(`Ansible · ${image.environment}`));
    assert.ok(html.includes(`https://images.example.test/sample-image/${image.version}`));
  }
  assert.equal((html.match(/https:\/\/security.example.test/g) || []).length, 5);
  assert.equal((html.match(/https:\/\/build.example.test/g) || []).length, 5);
  state.expandedImages.clear();
  assert.equal((imageRows(repo).match(/<tr /g) || []).length, 1);
});
test('role matching uses actual image for artifacts and scans, and repository name for builds', () => {
  const { state, imageRows } = setup();
  const repository = { id: 'r2', name: 'sample-service' };
  state.expandedImages.add(repository.id);
  state.ansibleImages = [{ imageName: 'different-image', role: 'sample_service', environment: 'test', version: '1.2.3', source: 'vars/config.yml' }];
  const html = imageRows(repository);
  assert.ok(html.includes('https://images.example.test/different-image/1.2.3'));
  assert.ok(html.includes('https://security.example.test/?artifact=different-image'));
  assert.ok(html.includes('https://build.example.test/sample-service'));
  assert.ok(!html.includes('https://build.example.test/different-image'));
});
test('missing config and unresolved versions are visible without inventing a version', () => {
  const { state, imageRows } = setup();
  state.expandedImages.add(repo.id);
  state.ansibleStatus = 'error'; state.ansibleError = 'Set ansible_dir <here>';
  assert.match(imageRows(repo), /Set ansible_dir &lt;here&gt;/);
  state.ansibleStatus = 'loaded';
  state.ansibleImages = [{ imageName: repo.name, environment: 'test', version: null, error: 'Unsupported expression' }];
  const html = imageRows(repo);
  assert.match(html, /Unsupported expression/);
  assert.doesNotMatch(html, /images.example.test/);
});
test('the disclosure toggles only its own image and requests Ansible data on expansion', () => {
  const state = { expandedImages: new Set() };
  const button = { dataset: { hubImageToggle: 'r1' } };
  let renders = 0, requests = 0, prevented = 0, stopped = 0;
  const event = { preventDefault: () => prevented++, stopPropagation: () => stopped++ };
  const binding = view.slice(view.indexOf('    root.querySelectorAll("[data-hub-image-toggle]")'), view.indexOf('    for (const area', view.indexOf('  function bind()')));
  new Function('root', 'state', 'render', 'loadAnsibleImages', binding)({querySelectorAll: () => [button]}, state, () => renders++, () => requests++);
  button.onclick(event);
  assert.deepEqual([...state.expandedImages], ['r1']);
  state.expandedImages.add('r2');
  button.onclick(event);
  assert.deepEqual([...state.expandedImages], ['r2']);
  assert.equal(renders, 2);
  assert.equal(requests, 1);
  assert.equal(prevented, 2);
  assert.equal(stopped, 2);
});

test('a disclosure click never enters repository URL validation even when forwarded to the repository handler', async () => {
  const button = { dataset: { hubRepository: 'r1' } };
  const start = view.indexOf('    root.querySelectorAll("[data-hub-repository]").forEach');
  const binding = view.slice(start, view.indexOf('    root.querySelectorAll("[data-hub-toggle]")', start));
  const fail = () => { throw new Error('Repository URL must not be accessed during expansion'); };
  new Function('root', 'state', 'imageHubUrl', 'openImageHubReport', binding)(
    { querySelectorAll: () => [button], querySelector: fail },
    { get repositories() { return fail(); } }, fail, fail,
  );
  await button.onclick({ target: { closest: selector => selector === '[data-hub-image-toggle]' ? {} : null }, preventDefault() {} });
});

test('a repository name opens its saved HTTP URL without demanding config or OCID', async () => {
  const button = { dataset: { hubRepository: 'r1' } };
  const start = view.indexOf('    root.querySelectorAll("[data-hub-repository]").forEach');
  const binding = view.slice(start, view.indexOf('    root.querySelectorAll("[data-hub-toggle]")', start));
  const opened = [];
  new Function('root', 'state', 'imageHubRepositoryUrl', 'openImageHubReport', binding)(
    { querySelectorAll: () => [button], querySelector: () => { throw new Error('No configuration warning should be displayed'); } },
    { runtimeConfig: {}, repositories: [{id: 'r1', url: 'https://source.example.test/service'}] },
    imageHubRepositoryUrl, async url => opened.push(url),
  );
  await button.onclick({target: {closest: () => null}, preventDefault() {}});
  assert.deepEqual(opened, ['https://source.example.test/service']);
});

test('latest and expanded environment rows stay aligned with the combined inventory columns',()=>{
 const {state,imageRows}=setup();state.expandedImages.add(repo.id);
 const rows=[...imageRows({...repo,inventoryFolder:'Applications / Exporters',inventoryProject:'Monitoring'}).matchAll(/<tr[^>]*>([\s\S]*?)<\/tr>/g)];
 assert.equal(rows.length,5);
 for(const [,row] of rows)assert.equal((row.match(/<td(?:\s|>)/g)||[]).length,6);
 assert.ok(rows[0][1].includes('Applications / Exporters'));assert.ok(rows[0][1].includes('Monitoring'));
});
