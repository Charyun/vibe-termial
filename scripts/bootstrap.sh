#!/usr/bin/env bash
set -euo pipefail
project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_root"
target_triple="$(rustc -vV | awk '/^host:/ { print $2 }')"
cargo fetch --locked --target "$target_triple"
printf 'GPUI Kit 0.9.1 dependencies ready. Run cargo run --locked.\n'
