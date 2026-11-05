import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const source = await readFile(new URL('../../web/modules/image-hub/url-template.js', import.meta.url), 'utf8');
const { imageHubUrl, imageHubRepositoryUrl } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);

test('unconfigured endpoints stay unavailable and do not invent an environment', () => {
  assert.equal(imageHubUrl('', {}, { name: 'service' }), null);
  assert.equal(imageHubUrl('https://source.example.test/{ocid}', {}, { name: 'service' }), null);
  assert.equal(imageHubUrl('https://source.example.test/{unknown}', {}, {}), null);
});

test('configured links encode each repository and context as a single URL value', () => {
  const url = imageHubUrl('https://source.example.test/{ocid}?context={context}', { context: 'a,b & c' }, { ocid: 'id/with space' });
  const parsed = new URL(url);
  assert.equal(parsed.pathname, '/id%2Fwith%20space');
  assert.equal(parsed.searchParams.get('context'), 'a,b & c');
  assert.equal(imageHubUrl('https://images.example.test/{artifactRepository}/{repo}/{version}', { artifactRepository: 'release' }, { name: 'sample-service' }, '2026.10.7'), 'https://images.example.test/release/sample-service/2026.10.7');
});

test('unsafe link schemes and embedded credentials cannot become clickable links', () => {
  assert.equal(imageHubUrl('javascript:alert(1)'), null);
  assert.equal(imageHubUrl('https://token@source.example.test/repo'), null);
  assert.equal(imageHubUrl('https://source.example.test/repo#secret'), null);
});

test('repository links work from a saved browser URL without a template or OCID', () => {
  assert.equal(imageHubRepositoryUrl({}, {url: 'https://source.example.test/team/service'}), 'https://source.example.test/team/service');
  assert.equal(imageHubRepositoryUrl({repositoryUrlTemplate: 'https://console.example.test/{ocid}'}, {ocid: 'repo-id', url: 'https://source.example.test/service'}), 'https://console.example.test/repo-id');
  assert.equal(imageHubRepositoryUrl({repositoryUrlTemplate: 'https://console.example.test/{ocid}'}, {url: 'https://source.example.test/service'}), 'https://source.example.test/service');
  for (const url of ['git@source.example.test:team/service.git', 'ssh://source.example.test/service', 'javascript:alert(1)', 'https://user:password@source.example.test/service', 'https://source.example.test/{ocid}', 'https://source.example.test/service#secret']) {
    assert.equal(imageHubRepositoryUrl({}, {url}), null);
  }
});

test('automatically resolved project browser URLs take precedence over clone URLs', () => {
  const repo = {url: 'ssh://git.example.test/service', browserUrl: 'https://console.example.test/repositories/id?context=account'};
  assert.equal(imageHubRepositoryUrl({}, repo), repo.browserUrl);
  assert.equal(imageHubRepositoryUrl({}, {...repo, browserUrl: 'javascript:alert(1)'}), null);
});
