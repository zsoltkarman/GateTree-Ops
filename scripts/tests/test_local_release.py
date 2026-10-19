import base64
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('stage_release', ROOT / 'scripts/prepare-macos-release.py')
stage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stage)


@unittest.skipIf(os.name == "nt", "The local Mac shell workflow is tested on POSIX hosts.")
class LocalReleaseTests(unittest.TestCase):
    def test_staging_keeps_source_and_debug_artifacts_unchanged(self):
        with tempfile.TemporaryDirectory() as temporary:
            temp = Path(temporary)
            source, target = temp / 'repo', temp / 'build'
            source.mkdir()
            subprocess.run(['git', 'init', '-q', str(source)], check=True)
            subprocess.run(['git', '-C', str(source), 'config', 'user.name', 'Release test'], check=True)
            subprocess.run(['git', '-C', str(source), 'config', 'user.email', 'release@test.invalid'], check=True)
            for name in ['Cargo.toml', 'agent/Cargo.toml', 'agent-core/Cargo.toml']:
                path = source / 'src-tauri' / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('[package]\nname="test"\nversion = "0.1.15"\n')
            (source / 'src-tauri/Cargo.lock').write_text('version = 4\n')
            (source / 'src-tauri/tauri.conf.json').write_text('{"version":"0.1.15"}')
            (source / 'scripts').mkdir()
            shutil.copy2(ROOT / 'scripts/set-version.py', source / 'scripts/set-version.py')
            (source / '.gitignore').write_text('src-tauri/target/\n')
            subprocess.run(['git', '-C', str(source), 'add', '.'], check=True)
            subprocess.run(['git', '-C', str(source), 'commit', '-qm', 'fixture'], check=True)
            (source / 'web').mkdir()
            (source / 'web/app.js').write_text('current uncommitted changes')
            (source / 'web/.env.private').write_text('must not be copied')
            (source / 'web/private.kdbx').write_text('must not be copied')
            debug = source / 'src-tauri/target/debug/app'
            debug.parent.mkdir(parents=True)
            debug.write_text('live debug binary')
            with patch.object(stage, 'ROOT', source):
                stage.prepare(target, '0.1.16')
                self.assertEqual(json.loads((target / 'src-tauri/tauri.conf.json').read_text())['version'], '0.1.16')
                self.assertEqual(json.loads((source / 'src-tauri/tauri.conf.json').read_text())['version'], '0.1.15')
                self.assertEqual((target / 'web/app.js').read_text(), 'current uncommitted changes')
                self.assertFalse((target / 'web/.env.private').exists())
                self.assertFalse((target / 'web/private.kdbx').exists())
                self.assertFalse((target / 'src-tauri/target').exists())
                self.assertEqual(debug.read_text(), 'live debug binary')
                (source / 'web/app.js').unlink()
                stage.prepare(target, '0.1.17')
                self.assertFalse((target / 'web/app.js').exists())
                with self.assertRaises(ValueError):
                    stage.prepare(source, '0.1.18')

    def fixture(self, temp, state=''):
        repo, bin_dir = temp / 'repo', temp / 'bin'
        scripts = repo / 'scripts'
        scripts.mkdir(parents=True)
        bin_dir.mkdir()
        for name in ['release-macos.sh', 'create-update-manifest.py']:
            shutil.copy2(ROOT / 'scripts' / name, scripts / name)
        (repo / 'RELEASE_NOTES.md').write_text('Reviewed release notes')
        assets = repo / 'dist/releases/0.1.16'
        assets.mkdir(parents=True)
        (assets / 'GateTree-Ops.dmg').write_bytes(b'dmg fixture')
        archive = assets / 'GateTree-Ops.app.tar.gz'
        archive.write_bytes(b'archive fixture')
        signature = base64.b64encode(b'untrusted comment: fixture\nsignature\ntrusted comment: fixture\nglobal signature\n').decode()
        Path(str(archive) + '.sig').write_text(signature)
        (assets / 'build-info.json').write_text('{"version":"0.1.16"}')
        (assets / 'private.key').write_text('must never be uploaded')
        (bin_dir / 'uname').write_text('#!/bin/sh\nprintf Darwin\n')
        (bin_dir / 'gh').write_text('''#!/usr/bin/env python3
import json,os,sys
args=sys.argv[1:]
with open(os.environ['GH_CALLS'],'a') as out: out.write(json.dumps(args)+'\\n')
if args[:2]==['auth','status']: pass
elif args[:2]==['repo','view']: print('PUBLIC')
elif args[:1]==['api']:
    if os.environ['RELEASE_STATE']: print('v0.1.16\\t'+os.environ['RELEASE_STATE'])
elif args[:2]==['release','view']: print('https://github.com/owner/updates/releases/tag/v0.1.16')
''')
        for path in bin_dir.iterdir():
            path.chmod(0o755)
        log = temp / 'gh.log'
        env = {**os.environ, 'PATH': str(bin_dir) + os.pathsep + os.environ['PATH'],
               'GATETREE_OPS_RELEASE_CONFIG': str(temp / 'no-config'), 'GH_CALLS': str(log), 'RELEASE_STATE': state}
        return scripts / 'release-macos.sh', assets, log, env

    def test_upload_creates_a_draft_with_only_installers_then_explicitly_publishes(self):
        with tempfile.TemporaryDirectory() as temporary:
            script, assets, log, env = self.fixture(Path(temporary))
            subprocess.run(['bash', str(script), '0.1.16', '--upload-only', '--publish'], env=env, check=True, capture_output=True)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            create = next(args for args in calls if args[:2] == ['release', 'create'])
            self.assertIn('--draft', create)
            self.assertFalse(any('private.key' in arg for arg in create))
            self.assertTrue(any(args[:2] == ['release', 'edit'] and '--draft=false' in args for args in calls))
            data = json.loads((assets / 'latest.json').read_text())
            self.assertEqual(set(data['platforms']), {'darwin-aarch64', 'darwin-x86_64'})

    def test_published_release_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as temporary:
            script, _, log, env = self.fixture(Path(temporary), state='false')
            result = subprocess.run(['bash', str(script), '0.1.16', '--upload-only', '--publish'], env=env, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            self.assertFalse(any(args[:2] in [['release','upload'], ['release','create'], ['release','edit']] for args in calls))

    def test_failed_artifact_validation_never_creates_a_release(self):
        with tempfile.TemporaryDirectory() as temporary:
            script, assets, log, env = self.fixture(Path(temporary))
            (assets / 'GateTree-Ops.app.tar.gz.sig').unlink()
            result = subprocess.run(['bash', str(script), '0.1.16', '--upload-only'], env=env, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            self.assertFalse(any(args[:1] == ['release'] for args in calls))


if __name__ == '__main__':
    unittest.main()
