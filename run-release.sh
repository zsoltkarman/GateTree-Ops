#!/usr/bin/env bash
# Local native Mac build, followed by binary-only GitHub release upload.
set -euo pipefail
root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="zsoltkarman/GateTree-Ops-updates"
version=""; dry_run=false; assume_yes=false; upload_only=false; publish=true
usage() {
  cat <<'HELP'
Usage: ./run-release.sh [--version X.Y.Z] [--yes] [--dry-run] [--upload-only] [--draft]

Prompts for a version, builds a local native macOS DMG and publishes a release to
zsoltkarman/GateTree-Ops-updates. Only the DMG, checksum and build metadata are
uploaded, together with the signed updater archive and latest.json. Source,
debug binaries, workspaces and the running Agent stay unchanged.

  --version X.Y.Z  Use this version without prompting.
  --yes            Use the next available patch version without prompting.
  --dry-run        Show the release plan; do not build or upload.
  --upload-only    Upload an existing dist/local/X.Y.Z package; requires --version.
  --draft          Upload for review without publishing (default: publish).

Requires macOS, Developer ID signing, native Rust, Tauri CLI and gh auth login.
This native package is not notarized. Publishing also activates its native CPU
update feed. Use scripts/release-macos.sh for notarized universal releases.
HELP
}
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) [[ $# -gt 1 ]] || { echo 'Missing version.' >&2; exit 2; }; version="$2"; shift ;;
    --yes) assume_yes=true ;;
    --dry-run) dry_run=true ;;
    --upload-only) upload_only=true ;;
    --publish) publish=true ;;
    --draft) publish=false ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done
cd "$root_dir"
if $upload_only && [[ -z "$version" ]]; then echo '--upload-only requires --version.' >&2; exit 2; fi
if [[ -z "$version" ]]; then
  next_version="$(python3 - "$root_dir" <<'PY'
import json,re,sys
from pathlib import Path
r=Path(sys.argv[1]);v=[json.loads((r/'src-tauri/tauri.conf.json').read_text())['version']]
v += [p.name for p in (r/'dist/local').glob('*') if p.is_dir() and re.fullmatch(r'(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)',p.name)]
a,b,c=max(tuple(map(int,s.split('.'))) for s in v);print(f'{a}.{b}.{c+1}')
PY
)"
  if ! $assume_yes; then
    read -r -p "Release version [$next_version]: " version || { echo 'Use --yes or --version in noninteractive mode.' >&2; exit 2; }
  fi
  version="${version:-$next_version}"
fi
[[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || { echo 'Use a stable X.Y.Z version without v.' >&2; exit 2; }
assets="$root_dir/dist/local/$version"
echo "GateTree-Ops release: v$version"
echo "Build: native macOS, Developer ID signed, not notarized"
echo "Destination: https://github.com/$repo/releases"
if $publish; then echo 'Visibility: published after upload'; else echo 'Visibility: draft for review'; fi
if $dry_run; then echo 'Dry run: no files changed, build or upload started.'; exit 0; fi
[[ $(uname -s) == Darwin ]] || { echo 'Run this local macOS release on a Mac.' >&2; exit 1; }
gh auth status >/dev/null 2>&1 || { echo 'Run gh auth login before releasing.' >&2; exit 1; }
visibility="$(gh repo view "$repo" --json visibility --jq .visibility)"
[[ "$visibility" == PUBLIC ]] || { echo 'The binary updates repository must be public.' >&2; exit 1; }
existing="$(gh api "repos/$repo/releases?per_page=100" --paginate --jq '.[] | [.tag_name, .draft] | @tsv' | awk -v tag="v$version" '$1 == tag {print $2}')"
[[ "$existing" != false ]] || { echo 'This version is already published. Use a higher version.' >&2; exit 1; }
if ! $upload_only; then
  [[ ! -e "$assets" && -z "$existing" ]] || { echo 'Package/draft exists. Use --upload-only or choose another version.' >&2; exit 1; }
  node --test scripts/tests/*.test.mjs
  python3 -m unittest discover -s scripts/tests -p 'test_*.py'
  cargo test --locked --manifest-path src-tauri/Cargo.toml -p gatetree-ops -p gatetree-ops-agent-core --lib
  "$root_dir/scripts/package-macos-local.sh" "$version"
fi
python3 - "$assets" "$version" <<'PY'
import hashlib,json,sys
from pathlib import Path
p=Path(sys.argv[1]);d=json.loads((p/'build-info.json').read_text())
assert d['version']==sys.argv[2] and d['distribution']=='local' and d['notarized'] is False, 'Unexpected package metadata'
assert d['target'] in ('aarch64-apple-darwin','x86_64-apple-darwin'), 'Unexpected package target'
dmgs=list(p.glob('*.dmg'));assert len(dmgs)==1, 'Expected exactly one DMG'
f=dmgs[0];assert f.name==f"GateTree-Ops-{sys.argv[2]}-{d['target']}.dmg" and f.stat().st_size>0
digest=hashlib.file_digest(f.open('rb'),'sha256').hexdigest()
assert Path(str(f)+'.sha256').read_text().split()[0]==digest, 'DMG checksum mismatch'
PY
shopt -s nullglob
files=("$assets"/*.dmg "$assets"/*.dmg.sha256 "$assets/build-info.json")
target="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["target"])' "$assets/build-info.json")"
case "$target" in
  aarch64-apple-darwin) platform=macos-aarch64 ;;
  x86_64-apple-darwin) platform=macos-x86_64 ;;
esac
python3 "$root_dir/scripts/create-update-manifest.py" "$assets" --version "$version" --platform "$platform" --notes-file "$root_dir/RELEASE_NOTES.md"
files+=("$assets"/*.app.tar.gz "$assets"/*.app.tar.gz.sig "$assets/latest.json")
notes="$(mktemp)"
trap 'if [[ -n ${notes:-} ]]; then rm -f "$notes"; fi' EXIT
cat "$root_dir/RELEASE_NOTES.md" > "$notes"
cat >> "$notes" <<'NOTES'

Local macOS package, Developer ID signed but not notarized. Install under
~/Applications without administrator access. The filename identifies the CPU
architecture. Automatic updates are available for that CPU architecture only.
This release contains no Windows build.
NOTES
bash "$root_dir/scripts/ensure-updates-repository.sh"
if [[ "$existing" == true ]]; then
  gh release upload "v$version" "${files[@]}" --repo "$repo" --clobber
  gh release edit "v$version" --repo "$repo" --notes-file "$notes"
else
  gh release create "v$version" "${files[@]}" --repo "$repo" --draft --title "GateTree-Ops $version · macOS" --notes-file "$notes"
fi
if $publish; then gh release edit "v$version" --repo "$repo" --draft=false --latest; fi
gh release view "v$version" --repo "$repo" --json url --jq .url
