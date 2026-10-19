#!/usr/bin/env python3
"""Set one release version across the GUI, bundled Agent, and Cargo lockfile."""
import argparse
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]


def set_version(version):
    if not re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", version):
        raise ValueError("Use a stable version such as 0.1.15 (without v).")
    for name in ("Cargo.toml", "agent/Cargo.toml", "agent-core/Cargo.toml"):
        path = ROOT / "src-tauri" / name
        path.write_text(re.sub(r'^version = "[^"]+"', f'version = "{version}"', path.read_text(), count=1, flags=re.M))
    path = ROOT / "src-tauri/tauri.conf.json"
    data = json.loads(path.read_text())
    data["version"] = version
    path.write_text(json.dumps(data, indent=2) + "\n")
    path = ROOT / "src-tauri/Cargo.lock"
    text = path.read_text()
    for name in ("gatetree-ops", "gatetree-ops-agent", "gatetree-ops-agent-core"):
        text = re.sub(r'(\[\[package\]\]\nname = "' + re.escape(name) + r'"\nversion = ")[^"]+', lambda m: m[1] + version, text)
    path.write_text(text)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version")
    args = parser.parse_args()
    set_version(args.version)
    print(f"GUI and Agent version: {args.version}")
