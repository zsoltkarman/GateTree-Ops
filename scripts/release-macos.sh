#!/usr/bin/env bash
# Local Mac build and upload. Never commits/pushes source or restarts the debug Agent.
set -euo pipefail
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
updates_repo="zsoltkarman/GateTree-Ops-updates"
usage() {
  cat <<'HELP'
Usage: ./scripts/release-macos.sh X.Y.Z [options]
  --check          Check prerequisites only; do not build or upload.
  --build-only     Create local signed installers, without GitHub access.
  --upload-only    Upload existing dist/releases/X.Y.Z assets; resume drafts safely.
  --publish        Publish after upload so installed Mac apps can update.
  --notes-file P   Release notes (default: RELEASE_NOTES.md).
Default: build locally, then upload a draft to GateTree-Ops-updates.
The source checkout/debug binary stay unchanged. Windows requires a separate build.
HELP
}
[[ ${1:-} != -h && ${1:-} != --help ]] || { usage; exit 0; }
[[ $# -gt 0 ]] || { usage; exit 2; }
version="$1"; shift
check=false; build_only=false; upload_only=false; publish=false
notes="$repo_dir/RELEASE_NOTES.md"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --check) check=true ;;
    --build-only) build_only=true ;;
    --upload-only) upload_only=true ;;
    --publish) publish=true ;;
    --notes-file) [[ $# -gt 1 ]] || { echo 'Missing notes file.' >&2; exit 2; }; notes="$2"; shift ;;
    *) echo "Unknown option: $1" >&2; usage; exit 2 ;;
  esac
  shift
done
[[ $version =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || { echo 'Use a stable x.y.z version without v.' >&2; exit 2; }
if $build_only && { $publish || $upload_only; }; then echo '--build-only cannot upload/publish.' >&2; exit 2; fi
notes="$(python3 -c 'from pathlib import Path; import sys; print(Path(sys.argv[1]).expanduser().resolve())' "$notes")"
config="${GATETREE_OPS_RELEASE_CONFIG:-$HOME/.config/gatetree-ops/release.env}"
if [[ -f "$config" ]]; then
  python3 - "$config" <<'PY'
import os,stat,sys
s=os.stat(sys.argv[1])
if s.st_uid != os.getuid() or stat.S_IMODE(s.st_mode) & 0o077:
    sys.exit('Protect the local release config with chmod 600 before using it.')
PY
  set -a
  source "$config"
  set +a
fi
export TAURI_SIGNING_PRIVATE_KEY="${TAURI_SIGNING_PRIVATE_KEY:-$HOME/.config/gatetree-ops/updater.key}"
errors=0
fail() { echo "Missing: $*" >&2; errors=$((errors + 1)); }
[[ $(uname -s) == Darwin ]] || fail 'Run this command on macOS.'
[[ -f "$notes" ]] || fail "Release notes file: $notes"
for tool in python3 git; do command -v "$tool" >/dev/null || fail "$tool"; done
if ! $upload_only; then
  for tool in cargo rustc rustup node codesign xcrun lipo; do command -v "$tool" >/dev/null || fail "$tool (universal Mac builds require rustup and both Apple targets)"; done
  cargo tauri --version >/dev/null 2>&1 || fail 'Tauri CLI v2 (cargo install tauri-cli --version ^2 --locked)'
  if [[ -z ${APPLE_SIGNING_IDENTITY:-} ]]; then
    APPLE_SIGNING_IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null | python3 -c 'import sys; names=[line.split(chr(34))[1] for line in sys.stdin if chr(34)+"Developer ID Application:" in line]; print(names[0] if len(names)==1 else "")')"
    export APPLE_SIGNING_IDENTITY
  fi
  [[ -n ${APPLE_SIGNING_IDENTITY:-} ]] || fail "Set APPLE_SIGNING_IDENTITY in $config (Developer ID Application)."
  if ! { [[ -n ${APPLE_API_KEY:-} && -n ${APPLE_API_ISSUER:-} && -f ${APPLE_API_KEY_PATH:-/nonexistent} ]] || [[ -n ${APPLE_ID:-} && -n ${APPLE_PASSWORD:-} && -n ${APPLE_TEAM_ID:-} ]]; }; then
    fail "Apple notarization credentials in $config (see docs/releases.md)."
  fi
  if [[ $TAURI_SIGNING_PRIVATE_KEY == /* && ! -s "$TAURI_SIGNING_PRIVATE_KEY" ]]; then fail 'The dedicated updater signing key is missing.'; fi
  if [[ $TAURI_SIGNING_PRIVATE_KEY == /* && -f "$TAURI_SIGNING_PRIVATE_KEY.pub" ]]; then
    if ! python3 - "$repo_dir/src-tauri/tauri.conf.json" "$TAURI_SIGNING_PRIVATE_KEY.pub" <<'PYKEY'
import json,sys
from pathlib import Path
sys.exit(0 if json.loads(Path(sys.argv[1]).read_text())['plugins']['updater']['pubkey'] == Path(sys.argv[2]).read_text().strip() else 1)
PYKEY
    then fail 'Updater key does not match the application public key.'; fi
  fi
fi
if ! $build_only; then
  if ! command -v gh >/dev/null; then fail 'GitHub CLI (gh)';
  elif ! gh auth status >/dev/null 2>&1; then fail 'GitHub sign-in: run gh auth login'; fi
fi
[[ $errors -eq 0 ]] || { echo "Preflight failed ($errors prerequisite(s)); no build or upload was started." >&2; exit 1; }
$check && { echo 'Local release prerequisites are available.'; exit 0; }
assets="$repo_dir/dist/releases/$version"
existing=""
if ! $build_only; then
  visibility="$(gh repo view "$updates_repo" --json visibility --jq .visibility)"
  [[ "$visibility" == PUBLIC ]] || { echo 'The binary update repository must be public.' >&2; exit 1; }
  existing="$(gh api "repos/$updates_repo/releases?per_page=100" --paginate --jq '.[] | [.tag_name, .draft] | @tsv' | awk -v tag="v$version" '$1 == tag {print $2}')"
  [[ "$existing" != false ]] || { echo 'This release is published; choose a higher version.' >&2; exit 1; }
  if [[ "$existing" == true ]] && ! $upload_only; then echo 'A draft exists; use --upload-only to resume with the prepared local assets.' >&2; exit 1; fi
fi
if ! $upload_only; then
  [[ ! -e "$assets" ]] || { echo "Local release assets already exist: $assets. Use --upload-only or a new version." >&2; exit 1; }
  cd "$repo_dir"
  node --test scripts/tests/*.test.mjs
  python3 -m unittest discover -s scripts/tests -p 'test_*.py'
  cargo test --locked --manifest-path src-tauri/Cargo.toml -p gatetree-ops -p gatetree-ops-agent-core --lib
  build_dir="$HOME/Library/Caches/GateTree-Ops/release-build"
  python3 scripts/prepare-macos-release.py "$version" "$build_dir"
  "$build_dir/scripts/build-macos.sh"
  mkdir -p "$assets"
  bundle="$build_dir/src-tauri/target/universal-apple-darwin/release/bundle"
  # Explicit allowlist: no source, workspace, KeePass files, keys or logs are uploaded.
  cp "$bundle"/dmg/*.dmg "$assets/"
  cp "$bundle"/macos/*.app.tar.gz "$bundle"/macos/*.app.tar.gz.sig "$assets/"
  cp "$build_dir/build-info.json" "$assets/"
fi
python3 - "$assets/build-info.json" "$version" <<'PY'
import json,sys
from pathlib import Path
path=Path(sys.argv[1])
assert json.loads(path.read_text())['version'] == sys.argv[2], 'Local artifact version mismatch'
dmgs=list(path.parent.glob('*.dmg'))
assert len(dmgs)==1 and dmgs[0].stat().st_size, 'Expected one nonempty DMG'
PY
python3 "$repo_dir/scripts/create-update-manifest.py" "$assets" --version "$version" --platform macos --notes-file "$notes"
$build_only && { echo "Signed local Mac release: $assets"; exit 0; }
shopt -s nullglob
files=("$assets"/*.dmg "$assets"/*.app.tar.gz "$assets"/*.app.tar.gz.sig "$assets/latest.json" "$assets/build-info.json")
if [[ "$existing" == true ]]; then
  gh release upload "v$version" "${files[@]}" --repo "$updates_repo" --clobber
  gh release edit "v$version" --repo "$updates_repo" --notes-file "$notes"
else
  gh release create "v$version" "${files[@]}" --repo "$updates_repo" --draft --title "GateTree-Ops $version · macOS" --notes-file "$notes"
fi
if $publish; then gh release edit "v$version" --repo "$updates_repo" --draft=false --latest; fi
gh release view "v$version" --repo "$updates_repo" --json url --jq .url
if ! $publish; then echo 'Draft uploaded. Test the DMG, then publish the release to enable Mac updates.'; fi
