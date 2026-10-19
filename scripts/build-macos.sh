#!/usr/bin/env bash
set -euo pipefail
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
[[ "$(uname -s)" == Darwin ]] || { echo "Build the macOS release on macOS."; exit 1; }
cargo tauri --version >/dev/null
repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
export TAURI_SIGNING_PRIVATE_KEY="${TAURI_SIGNING_PRIVATE_KEY:-$HOME/.config/gatetree-ops/updater.key}"
: "${APPLE_SIGNING_IDENTITY:?Set APPLE_SIGNING_IDENTITY to your Developer ID Application identity.}"
cd "$repo_dir/src-tauri"
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --locked -p gatetree-ops-agent --target aarch64-apple-darwin
cargo build --release --locked -p gatetree-ops-agent --target x86_64-apple-darwin
mkdir -p binaries
lipo -create target/aarch64-apple-darwin/release/gatetree-ops-agent target/x86_64-apple-darwin/release/gatetree-ops-agent \
  -output binaries/gatetree-ops-agent-universal-apple-darwin
cargo tauri build --bundles app,dmg --target universal-apple-darwin
bundle_dir="$repo_dir/src-tauri/target/universal-apple-darwin/release/bundle"
"$repo_dir/scripts/verify-macos-installer.sh" "$bundle_dir"/dmg/*.dmg
codesign --verify --deep --strict "$bundle_dir/macos/GateTree-Ops.app"
xcrun stapler validate "$bundle_dir/macos/GateTree-Ops.app"
test -s "$bundle_dir/macos/GateTree-Ops.app.tar.gz.sig"
echo "Signed, notarized DMG and signed updater archive: $bundle_dir"
