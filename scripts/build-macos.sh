#!/usr/bin/env bash
set -euo pipefail

cargo_bin_dir="${CARGO_HOME:-$HOME/.cargo}/bin"
export PATH="$cargo_bin_dir:$PATH"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "The DMG must be built on macOS."
  exit 1
fi

if ! cargo tauri --version >/dev/null 2>&1; then
  echo 'Install the Tauri CLI first: cargo install tauri-cli --version "^2" --locked'
  exit 1
fi

repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_dir/src-tauri"

if command -v rustup >/dev/null 2>&1; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  cargo build --release -p pharos-agent --target aarch64-apple-darwin
  cargo build --release -p pharos-agent --target x86_64-apple-darwin
  mkdir -p binaries
  lipo -create \
    target/aarch64-apple-darwin/release/pharos-agent \
    target/x86_64-apple-darwin/release/pharos-agent \
    -output binaries/pharos-agent-universal-apple-darwin
  cargo tauri build --bundles dmg --target universal-apple-darwin
  "$repo_dir/scripts/verify-macos-installer.sh" \
    "$repo_dir"/src-tauri/target/universal-apple-darwin/release/bundle/dmg/*.dmg
  echo "Universal DMG created under src-tauri/target/universal-apple-darwin/release/bundle/dmg/"
else
  echo "rustup is not installed; building for this Mac's native architecture."
  cargo build --release -p pharos-agent
  host_target="$(rustc -vV | sed -n 's/^host: //p')"
  mkdir -p binaries
  cp target/release/pharos-agent "binaries/pharos-agent-$host_target"
  cargo tauri build --bundles dmg
  "$repo_dir/scripts/verify-macos-installer.sh" \
    "$repo_dir"/src-tauri/target/release/bundle/dmg/*.dmg
  echo "DMG created under src-tauri/target/release/bundle/dmg/"
fi
