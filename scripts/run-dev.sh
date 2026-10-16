#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_dir/src-tauri"

# The GUI starts this sibling binary and intentionally leaves it alive when the
# Tauri process exits, so build both executables before each development run.
cargo build -p pharos-agent
host_target="$(rustc -vV | sed -n 's/^host: //p')"
mkdir -p binaries
cp "target/debug/pharos-agent" "binaries/pharos-agent-$host_target"
exec cargo run --bin pharos
