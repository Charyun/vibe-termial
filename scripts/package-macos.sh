#!/usr/bin/env bash
set -euo pipefail
if [[ "$(uname -s)" != Darwin ]]; then
  echo 'Run this script on macOS.' >&2
  exit 1
fi
project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_root"
bash scripts/bootstrap.sh
target_directory="$(cargo metadata --format-version 1 --no-deps --locked | sed -n 's/.*"target_directory": *"\([^"]*\)".*/\1/p')"
if [[ -z "$target_directory" ]]; then
  echo 'Could not resolve the cargo target directory.' >&2
  exit 1
fi
cargo build --release --locked
bundle="$target_directory/bundle/Vibe Terminal.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "$target_directory/release/vibe-terminal" "$bundle/Contents/MacOS/vibe-terminal"
cp assets/macos/Info.plist "$bundle/Contents/Info.plist"
cp assets/fonts/*OFL.txt "$bundle/Contents/Resources/"
printf 'App bundle: %s\nUnsigned development build.\n' "$bundle"
