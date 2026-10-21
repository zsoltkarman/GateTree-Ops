#!/usr/bin/env python3
"""Stage the current source (including uncommitted work) outside the debug tree."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def prepare(destination, version):
    if not re.fullmatch(r'(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)', version):
        raise ValueError('Use a stable x.y.z version without v.')
    destination = destination.expanduser().resolve()
    if destination == ROOT or destination in ROOT.parents or ROOT in destination.parents:
        raise ValueError('The release build directory must be outside the source checkout.')
    marker = destination / '.gatetree-ops-release-build.json'
    if destination.exists() and any(destination.iterdir()) and not marker.is_file():
        raise ValueError('Refusing to reuse an unrecognized nonempty build directory.')
    files = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'], cwd=ROOT).decode().split('\0')
    selected = []
    for name in sorted(set(files)):
        path = Path(name)
        if not name or (path.parts[0] not in {'src-tauri', 'web', 'scripts'} and name not in {'run-dev.sh', 'run-release.sh'}):
            continue
        if any(part.startswith('.env') or part in {'target', 'node_modules', '__pycache__', 'binaries'} for part in path.parts):
            continue
        if path.suffix.lower() in {'.kdbx', '.sqlite', '.salt', '.key', '.p8', '.p12'}:
            continue
        source = ROOT / path
        if source.is_symlink():
            raise ValueError(f'Release source must not contain symlinks: {name}')
        if source.is_file():
            selected.append(name)
    previous = json.loads(marker.read_text()).get('files', []) if marker.exists() else []
    destination.mkdir(parents=True, exist_ok=True)
    for name in set(previous) - set(selected):
        old = destination / name
        if old.resolve().is_relative_to(destination) and old.is_file():
            old.unlink()
    digest = hashlib.sha256()
    for name in selected:
        source, target = ROOT / name, destination / name
        if not target.resolve().is_relative_to(destination):
            raise ValueError(f"Build path escapes release directory: {name}")
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
        digest.update(name.encode() + b'\0' + source.read_bytes() + b'\0')
    for name in ['RELEASE_NOTES.md']:
        if (ROOT / name).is_file():
            shutil.copy2(ROOT / name, destination / name)
    subprocess.run([sys.executable, str(destination / 'scripts/set-version.py'), version], check=True)
    info = {
        'version': version,
        'sourceCommit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT).decode().strip(),
        'sourceDirty': bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT)),
        'sourceDigest': digest.hexdigest(),
        'builtAt': datetime.now(timezone.utc).isoformat(),
    }
    (destination / 'build-info.json').write_text(json.dumps(info, indent=2) + '\n')
    marker.write_text(json.dumps({'files': selected}, indent=2) + '\n')
    return destination


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('version')
    parser.add_argument('destination', type=Path)
    args = parser.parse_args()
    print(f'Release source prepared: {prepare(args.destination, args.version)}')
