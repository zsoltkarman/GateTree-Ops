#!/usr/bin/env bash
# Run once after `gh auth login`. Never prints private key material.
set -euo pipefail
repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
source_repo="zsoltkarman/GateTree-Ops"
updates_repo="zsoltkarman/GateTree-Ops-updates"
private_key="${GATETREE_OPS_UPDATER_KEY:-$HOME/.config/gatetree-ops/updater.key}"
gh auth status
if [[ ! -s "$private_key" ]]; then
  echo "Missing private updater key: $private_key"
  exit 1
fi
python3 - "$repo_dir/src-tauri/tauri.conf.json" "$private_key.pub" <<'PY'
import json,sys
from pathlib import Path
assert json.loads(Path(sys.argv[1]).read_text())['plugins']['updater']['pubkey'] == Path(sys.argv[2]).read_text().strip(), 'Updater public/private key files do not match this application configuration.'
PY
if ! gh repo view "$updates_repo" >/dev/null 2>&1; then
  gh repo create "$updates_repo" --public --add-readme --description "Signed GateTree-Ops installers and update feed"
fi
gh secret set TAURI_SIGNING_PRIVATE_KEY --repo "$source_repo" < "$private_key"
echo "Updater key configured. Add GATETREE_OPS_UPDATES_TOKEN and Apple signing/notarization secrets as described in docs/releases.md."
echo "The private updater key must be backed up securely; keep the same key for future releases."
