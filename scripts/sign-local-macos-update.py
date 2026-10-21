#!/usr/bin/env python3
"""Package the verified local app and sign it with the dedicated updater key."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tarfile


def package(app, output, root):
    info = json.loads((output / 'build-info.json').read_text())
    target = info['target']
    expected = {'aarch64-apple-darwin': 'arm64', 'x86_64-apple-darwin': 'x86_64'}[target]
    key = Path(os.environ.get('TAURI_SIGNING_PRIVATE_KEY_PATH', str(Path.home() / '.config/gatetree-ops/updater.key'))).expanduser()
    public = Path(str(key) + '.pub')
    embedded = json.loads((root / 'src-tauri/tauri.conf.json').read_text())['plugins']['updater']['pubkey']
    if not key.is_file() or not public.is_file() or public.read_text().strip() != embedded:
        raise ValueError('A local updater signing key matching the application public key is required.')
    import plistlib
    plist = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
    if plist['CFBundleShortVersionString'] != info['version']:
        raise ValueError('Application and package versions differ.')
    for name in ('gatetree-ops', 'gatetree-ops-agent'):
        executable = app / 'Contents/MacOS' / name
        subprocess.run(['codesign', '--verify', '--strict', str(executable)], check=True)
        if subprocess.check_output(['lipo', '-archs', str(executable)], text=True).strip() != expected:
            raise ValueError('The updater package does not match the declared CPU architecture.')
    archive = output / f'GateTree-Ops-{info["version"]}-{target}.app.tar.gz'
    if archive.exists() or Path(str(archive) + '.sig').exists():
        raise ValueError('An updater archive already exists; do not overwrite it.')
    with tarfile.open(archive, 'w:gz') as tar:
        tar.add(app, arcname='GateTree-Ops.app')
    signed = subprocess.run(['cargo', 'tauri', 'signer', 'sign', '--private-key-path', str(key), '--app-version', info['version'], str(archive)],
                            env={**os.environ, 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD': os.environ.get('TAURI_SIGNING_PRIVATE_KEY_PASSWORD', '')},
                            capture_output=True, text=True)
    if signed.returncode or not Path(str(archive) + '.sig').is_file():
        archive.unlink(missing_ok=True)
        raise ValueError('Updater signing failed. Check the local key and TAURI_SIGNING_PRIVATE_KEY_PASSWORD.')
    subprocess.run(['python3', str(root / 'scripts/create-update-manifest.py'), str(output), '--version', info['version'],
                    '--platform', 'macos-' + ('aarch64' if expected == 'arm64' else 'x86_64'), '--notes-file', str(root / 'RELEASE_NOTES.md')], check=True)
    print('Created signed native Mac updater archive and latest.json.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('app', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    package(args.app, args.output, Path(__file__).resolve().parents[1])
