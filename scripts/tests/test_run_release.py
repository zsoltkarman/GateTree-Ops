import hashlib
import base64
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


@unittest.skipIf(os.name == 'nt', 'Mac shell launcher tests run on POSIX')
class ReleaseLauncherTests(unittest.TestCase):
    def fixture(self, folder, state=''):
        repo, binaries = folder / 'repo', folder / 'bin'
        (repo / 'src-tauri').mkdir(parents=True)
        binaries.mkdir()
        shutil.copy2(ROOT / 'run-release.sh', repo / 'run-release.sh')
        (repo / 'scripts').mkdir()
        shutil.copy2(ROOT / 'scripts/create-update-manifest.py', repo / 'scripts/create-update-manifest.py')
        shutil.copy2(ROOT / 'scripts/ensure-updates-repository.sh', repo / 'scripts/ensure-updates-repository.sh')
        (repo / 'RELEASE_NOTES.md').write_text('Release notes')
        (repo / 'src-tauri/tauri.conf.json').write_text('{"version":"0.1.15"}')
        assets = repo / 'dist/local/0.1.16'
        assets.mkdir(parents=True)
        name = 'GateTree-Ops-0.1.16-aarch64-apple-darwin.dmg'
        dmg = assets / name
        dmg.write_bytes(b'local dmg fixture')
        (assets / (name + '.sha256')).write_text(hashlib.sha256(dmg.read_bytes()).hexdigest() + '  ' + name)
        (assets / 'build-info.json').write_text(json.dumps({'version': '0.1.16', 'target': 'aarch64-apple-darwin', 'distribution': 'local', 'notarized': False}))
        (assets / 'secret.key').write_text('must never upload')
        (assets / 'GateTree-Ops.app.tar.gz').write_bytes(b'updater archive')
        (assets / 'GateTree-Ops.app.tar.gz.sig').write_text(base64.b64encode(b'untrusted comment: fixture\nsignature\ntrusted comment: fixture\nglobal signature\n').decode())
        (binaries / 'uname').write_text('#!/bin/sh\nprintf Darwin')
        (binaries / 'gh').write_text('''#!/usr/bin/env python3
import json,os,sys
args=sys.argv[1:]
with open(os.environ['GH_CALLS'],'a') as f:f.write(json.dumps(args)+'\\n')
if args[:2]==['auth','status']:sys.exit(int(os.environ.get('AUTH_FAIL','0')))
if args[:2]==['repo','view']:print(os.environ.get('REPO_EMPTY','false') if 'isEmpty' in args else 'PUBLIC')
if args[:1]==['api'] and '--method' in args:
    with open(os.environ['INIT_PAYLOAD'],'w') as f:f.write(open(args[args.index('--input')+1]).read())
if args[:1]==['api'] and os.environ['RELEASE_STATE']:print('v0.1.16\\t'+os.environ['RELEASE_STATE'])
if args[:2]==['release','view']:print('https://github.com/zsoltkarman/GateTree-Ops-updates/releases/tag/v0.1.16')
''')
        for p in binaries.iterdir():p.chmod(0o755)
        log = folder / 'gh.log'
        env = {**os.environ, 'PATH': str(binaries) + os.pathsep + os.environ['PATH'], 'GH_CALLS': str(log), 'RELEASE_STATE': state, 'INIT_PAYLOAD': str(folder / 'init.json')}
        return repo, assets, log, env

    def run_script(self, repo, env, *args):
        return subprocess.run(['bash', str(repo / 'run-release.sh'), *args], env=env, capture_output=True, text=True)

    def test_dry_run_chooses_unused_version_without_network_or_file_changes(self):
        with tempfile.TemporaryDirectory() as folder:
            repo, assets, log, env = self.fixture(Path(folder))
            result = self.run_script(repo, env, '--yes', '--dry-run')
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('v0.1.17', result.stdout)
            self.assertFalse(log.exists())
            self.assertFalse((assets.parent / '0.1.17').exists())
            self.assertEqual(json.loads((repo / 'src-tauri/tauri.conf.json').read_text())['version'], '0.1.15')

    def test_upload_allowlist_and_publication_include_only_native_cpu_feed(self):
        with tempfile.TemporaryDirectory() as folder:
            repo, assets, log, env = self.fixture(Path(folder))
            result = self.run_script(repo, env, '--version', '0.1.16', '--upload-only', '--publish')
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = [json.loads(s) for s in log.read_text().splitlines()]
            create = next(c for c in calls if c[:2] == ['release', 'create'])
            self.assertIn('--draft', create)
            self.assertFalse(any('secret.key' in arg for arg in create))
            publish = next(c for c in calls if c[:2] == ['release', 'edit'])
            self.assertIn('--draft=false', publish)
            self.assertIn('--latest', publish)
            self.assertEqual(set(json.loads((assets / 'latest.json').read_text())['platforms']), {'darwin-aarch64'})

    def test_corrupt_package_and_published_version_never_write_github(self):
        for state in ['', 'false']:
            with self.subTest(state=state), tempfile.TemporaryDirectory() as folder:
                repo, assets, log, env = self.fixture(Path(folder), state)
                if not state:next(assets.glob('*.dmg')).write_bytes(b'corrupt')
                result = self.run_script(repo, env, '--version', '0.1.16', '--upload-only')
                self.assertNotEqual(result.returncode, 0)
                calls = [json.loads(s) for s in log.read_text().splitlines()]
                self.assertFalse(any(c[:1] == ['release'] for c in calls))

    def test_missing_auth_stops_before_build_or_upload(self):
        with tempfile.TemporaryDirectory() as folder:
            repo, _, log, env = self.fixture(Path(folder))
            env['AUTH_FAIL'] = '1'
            result = self.run_script(repo, env, '--version', '0.1.17')
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('gh auth login', result.stderr)
            self.assertEqual([json.loads(s) for s in log.read_text().splitlines()], [['auth','status']])

    def test_no_switches_accepts_next_version_builds_uploads_and_publishes(self):
        with tempfile.TemporaryDirectory() as folder:
            repo, _, log, env = self.fixture(Path(folder))
            (repo / 'scripts/tests').mkdir()
            (repo / 'scripts/tests/test_fixture.py').write_text('import unittest\nclass Fixture(unittest.TestCase):\n    def test_fixture(self): pass\n')
            for name in ('node', 'cargo'):
                p = Path(folder) / 'bin' / name
                p.write_text('#!/bin/sh\nexit 0\n');p.chmod(0o755)
            builder = repo / 'scripts/package-macos-local.sh'
            builder.write_text('''#!/usr/bin/env python3
import hashlib,json,shutil,sys
from pathlib import Path
r=Path(__file__).resolve().parents[1];v=sys.argv[1];p=r/'dist/local'/v
shutil.copytree(r/'dist/local/0.1.16',p)
d=json.loads((p/'build-info.json').read_text());d['version']=v;(p/'build-info.json').write_text(json.dumps(d))
f=next(p.glob('*.dmg'));f.rename(p/f.name.replace('0.1.16',v));f=next(p.glob('*.dmg'))
next(p.glob('*.sha256')).unlink();Path(str(f)+'.sha256').write_text(hashlib.sha256(f.read_bytes()).hexdigest()+'  '+f.name)
''')
            builder.chmod(0o755)
            result = subprocess.run(['bash', str(repo / 'run-release.sh')], input='\n', env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('GateTree-Ops release: v0.1.17', result.stdout)
            self.assertTrue((repo/'dist/local/0.1.17').is_dir())
            calls = [json.loads(s) for s in log.read_text().splitlines()]
            self.assertTrue(any(c[:3]==['release','create','v0.1.17'] for c in calls))
            self.assertTrue(any(c[:3]==['release','edit','v0.1.17'] and '--draft=false' in c and '--latest' in c for c in calls))

    def test_empty_binary_repository_is_initialized_before_release_upload(self):
        with tempfile.TemporaryDirectory() as folder:
            repo, _, log, env = self.fixture(Path(folder), state='true')
            env['REPO_EMPTY'] = 'true'
            result = self.run_script(repo, env, '--version', '0.1.16', '--upload-only', '--publish')
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = [json.loads(s) for s in log.read_text().splitlines()]
            initialized = next(i for i,c in enumerate(calls) if c[:3] == ['api','--method','PUT'])
            uploaded = next(i for i,c in enumerate(calls) if c[:2] == ['release','upload'])
            self.assertLess(initialized, uploaded)
            self.assertIn('repos/zsoltkarman/GateTree-Ops-updates/contents/README.md', calls[initialized])
            payload = json.loads(Path(env['INIT_PAYLOAD']).read_text())
            self.assertEqual(set(payload), {'message','content'})
            self.assertIn('Public installers', base64.b64decode(payload['content']).decode())
