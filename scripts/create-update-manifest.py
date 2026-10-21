#!/usr/bin/env python3
"""Create latest.json from signed universal Mac / Windows NSIS packages."""
import argparse
import base64
from datetime import datetime, timezone
import json
from pathlib import Path
import re
from urllib.parse import quote


def create_manifest(directory, version, repository, notes="", platform="all"):
    if not re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", version):
        raise ValueError("A stable x.y.z version is required.")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("Use owner/repository for the public updates repository.")
    if platform not in {"all", "macos", "macos-aarch64", "macos-x86_64", "windows"}:
        raise ValueError("Choose all, macos, macos-aarch64, macos-x86_64, or windows.")
    result = {"version": version, "notes": notes,
              "pub_date": datetime.now(timezone.utc).isoformat(), "platforms": {}}
    for pattern, platforms in (("*.app.tar.gz", ["darwin-aarch64", "darwin-x86_64"]),
                               ("*-setup.exe", ["windows-x86_64"])):
        if platform.startswith("macos") and pattern.endswith("exe") or platform == "windows" and pattern.endswith("tar.gz"):
            continue
        if platform in {"macos-aarch64", "macos-x86_64"}:
            platforms = ["darwin-" + platform.removeprefix("macos-")]
        candidates = sorted(directory.rglob(pattern))
        if len(candidates) != 1:
            raise ValueError(f"Expected exactly one {pattern}; found {len(candidates)}.")
        artifact = candidates[0]
        if artifact.stat().st_size == 0:
            raise ValueError(f"Empty updater package: {artifact.name}")
        signature = Path(str(artifact) + ".sig").read_text().strip()
        decoded = base64.b64decode(signature, validate=True)
        if not decoded.startswith(b"untrusted comment:") or len(decoded.splitlines()) < 4:
            raise ValueError(f"Invalid updater signature: {artifact.name}")
        entry = {"url": f"https://github.com/{repository}/releases/download/v{version}/{quote(artifact.name)}",
                 "signature": signature}
        for platform_key in platforms:
            result["platforms"][platform_key] = entry.copy()
    # Reject duplicate asset names before flattening/uploading artifacts.
    names = [p.name for p in directory.rglob("*") if p.is_file()]
    if len(names) != len(set(names)):
        raise ValueError("Release assets contain duplicate filenames.")
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--repository", default="zsoltkarman/GateTree-Ops-updates")
    parser.add_argument("--notes-file", type=Path)
    parser.add_argument("--platform", choices=["all", "macos", "macos-aarch64", "macos-x86_64", "windows"], default="all")
    args = parser.parse_args()
    notes = args.notes_file.read_text() if args.notes_file else ""
    manifest = create_manifest(args.directory, args.version, args.repository, notes, args.platform)
    output = args.directory / "latest.json"
    output.write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"Created {output} for: {', '.join(manifest['platforms'])}.")
