#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 1 || ! -f "$1" ]]; then
  echo "Usage: $0 <GateTree-Ops.dmg>"
  exit 1
fi

mount_dir="$(mktemp -d)"
cleanup() {
  hdiutil detach "$mount_dir" -quiet >/dev/null 2>&1 || true
  rmdir "$mount_dir" >/dev/null 2>&1 || true
}
trap cleanup EXIT

hdiutil attach "$1" -nobrowse -readonly -mountpoint "$mount_dir" -quiet
agent_path="$(find "$mount_dir" -type f -path '*/GateTree-Ops.app/Contents/MacOS/gatetree-ops-agent' -print -quit)"
if [[ -z "$agent_path" || ! -x "$agent_path" ]]; then
  echo "The DMG does not contain an executable GateTree-Ops Agent sidecar."
  exit 1
fi

file "$agent_path"
echo "Verified bundled Agent: $agent_path"
