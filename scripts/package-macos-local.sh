#!/usr/bin/env bash
# Native local DMG, matching GateTree's local packaging path. Never uploads.
set -euo pipefail
[[ $(uname -s) == Darwin ]] || { echo 'Run on macOS.' >&2; exit 1; }
repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
version="${1:?Usage: ./scripts/package-macos-local.sh X.Y.Z}"
build_dir="$HOME/Library/Caches/GateTree-Ops/release-build"
output="$repo_dir/dist/local/$version"
[[ ! -e "$output" ]] || { echo "Output already exists: $output" >&2; exit 1; }
identity="${APPLE_SIGNING_IDENTITY:-}"
if [[ -z "$identity" ]]; then
  identity="$(security find-identity -v -p codesigning | python3 -c 'import sys; n=[s.split(chr(34))[1] for s in sys.stdin if chr(34)+"Developer ID Application:" in s]; print(n[0] if len(n)==1 else "")')"
fi
[[ -n "$identity" ]] || { echo 'Set APPLE_SIGNING_IDENTITY to a Developer ID Application identity.' >&2; exit 1; }
python3 "$repo_dir/scripts/prepare-macos-release.py" "$version" "$build_dir"
cd "$build_dir/src-tauri"
host_target="$(rustc -vV | sed -n 's/^host: //p')"
[[ "$host_target" == *-apple-darwin ]] || { echo 'A native macOS Rust toolchain is required.' >&2; exit 1; }
# Local packages intentionally do not submit to Apple or publish an update feed.
unset APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID
export APPLE_SIGNING_IDENTITY="$identity"
cargo build --release --locked -p gatetree-ops-agent
mkdir -p binaries
cp target/release/gatetree-ops-agent "binaries/gatetree-ops-agent-$host_target"
config="$(python3 - "$identity" <<'PY'
import json,sys
print(json.dumps({'bundle':{'createUpdaterArtifacts':False,'macOS':{'signingIdentity':sys.argv[1]}}}))
PY
)"
cargo tauri build --bundles app --config "$config"
app="$build_dir/src-tauri/target/release/bundle/macos/GateTree-Ops.app"
codesign --verify --deep --strict "$app"
test -x "$app/Contents/MacOS/gatetree-ops-agent"
file "$app/Contents/MacOS/gatetree-ops-agent"
mkdir -p "$output"
stage="$(mktemp -d "$output/.dmg-stage.XXXXXX")"
trap 'if [[ -n ${stage:-} && -d "$stage" ]]; then python3 -c "import shutil,sys; shutil.rmtree(sys.argv[1])" "$stage"; fi' EXIT
ditto "$app" "$stage/GateTree-Ops.app"
ln -s /Applications "$stage/Applications"
dmg="$output/GateTree-Ops-$version-$host_target.dmg"
hdiutil create -volname GateTree-Ops -srcfolder "$stage" -format UDZO "$dmg"
codesign --force --sign "$identity" --timestamp "$dmg"
codesign --verify --strict "$dmg"
"$repo_dir/scripts/verify-macos-installer.sh" "$dmg"
cp "$build_dir/build-info.json" "$output/"
python3 - "$output/build-info.json" "$host_target" <<'PY'
import json,sys
from pathlib import Path
p=Path(sys.argv[1]);d=json.loads(p.read_text());d.update(target=sys.argv[2],notarized=False,distribution='local');p.write_text(json.dumps(d,indent=2)+'\n')
PY
shasum -a 256 "$dmg" > "$dmg.sha256"
python3 "$repo_dir/scripts/sign-local-macos-update.py" "$app" "$output"
echo "Local Developer ID signed DMG (not notarized; not uploaded): $dmg"
