import base64
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('manifest', ROOT / 'scripts/create-update-manifest.py')
manifest = importlib.util.module_from_spec(spec)
spec.loader.exec_module(manifest)


class ReleaseManifestTests(unittest.TestCase):
    def assets(self, directory):
        signature = base64.b64encode(b'untrusted comment: fixture\nsignature\ntrusted comment: fixture\nglobal signature\n').decode()
        for name in ['GateTree-Ops.app.tar.gz', 'GateTree-Ops_0.1.15_x64-setup.exe']:
            path = directory / name
            path.write_bytes(b'package fixture')
            Path(str(path) + '.sig').write_text(signature)

    def test_manifest_uses_signed_universal_archive_for_both_macs_and_nsis_for_windows(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            self.assets(directory)
            data = manifest.create_manifest(directory, '0.1.15', 'owner/updates', 'Release notes')
            platforms = data['platforms']
            self.assertEqual(set(platforms), {'darwin-aarch64', 'darwin-x86_64', 'windows-x86_64'})
            self.assertEqual(platforms['darwin-aarch64'], platforms['darwin-x86_64'])
            self.assertTrue(platforms['windows-x86_64']['url'].endswith('-setup.exe'))
            self.assertTrue(all('/releases/download/v0.1.15/' in p['url'] and p['signature'] for p in platforms.values()))

    def test_manifest_rejects_missing_ambiguous_unsigned_and_empty_packages(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            with self.assertRaises(ValueError):
                manifest.create_manifest(directory, '0.1.15', 'owner/updates')
            self.assets(directory)
            (directory / 'second.app.tar.gz').write_bytes(b'other')
            with self.assertRaises(ValueError):
                manifest.create_manifest(directory, '0.1.15', 'owner/updates')
            (directory / 'second.app.tar.gz').unlink()
            (directory / 'GateTree-Ops.app.tar.gz.sig').write_text('not a signature')
            with self.assertRaises(ValueError):
                manifest.create_manifest(directory, '0.1.15', 'owner/updates')
            self.assets(directory)
            (directory / 'GateTree-Ops.app.tar.gz').write_bytes(b'')
            with self.assertRaises(ValueError):
                manifest.create_manifest(directory, '0.1.15', 'owner/updates')

    def test_mac_only_release_does_not_require_or_publish_a_windows_package(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            self.assets(directory)
            (directory / 'GateTree-Ops_0.1.15_x64-setup.exe').unlink()
            (directory / 'GateTree-Ops_0.1.15_x64-setup.exe.sig').unlink()
            data = manifest.create_manifest(directory, '0.1.16', 'owner/updates', platform='macos')
            self.assertEqual(set(data['platforms']), {'darwin-aarch64', 'darwin-x86_64'})
            with self.assertRaises(ValueError):
                manifest.create_manifest(directory, '0.1.16', 'owner/updates')

    def test_release_version_and_repository_cannot_inject_urls_or_commands(self):
        with tempfile.TemporaryDirectory() as temp:
            for version, repo in [('v1.2.3', 'owner/repo'), ('1.2.3;bad', 'owner/repo'), ('1.2.3', '../bad/path')]:
                with self.assertRaises(ValueError):
                    manifest.create_manifest(Path(temp), version, repo)


if __name__ == '__main__':
    unittest.main()
