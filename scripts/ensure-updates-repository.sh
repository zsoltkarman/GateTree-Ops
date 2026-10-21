#!/usr/bin/env bash
# A release tag cannot be published until the binary repository has a commit.
set -euo pipefail
repo="zsoltkarman/GateTree-Ops-updates"
empty="$(gh repo view "$repo" --json isEmpty --jq .isEmpty)"
case "$empty" in
  false) exit 0 ;;
  true) ;;
  *) echo 'Could not determine whether the updates repository is empty.' >&2; exit 1 ;;
esac
payload="$(mktemp)"
trap 'rm -f "$payload"' EXIT
python3 - "$payload" <<'PY'
import base64,json,sys
from pathlib import Path
readme = '# GateTree-Ops updates\n\nPublic installers and signed update packages for GateTree-Ops.\nDownload packages from Releases. Source code and private workspace data are not stored here.\n'
Path(sys.argv[1]).write_text(json.dumps({'message':'Initialize binary updates repository','content':base64.b64encode(readme.encode()).decode()}))
PY
echo 'Initializing the empty updates repository with a README…'
if ! gh api --method PUT "repos/$repo/contents/README.md" --input "$payload" --silent; then
  # Another release process may have initialized it concurrently.
  [[ "$(gh repo view "$repo" --json isEmpty --jq .isEmpty)" == false ]] || exit 1
fi
