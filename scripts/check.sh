#!/usr/bin/env bash
set -e
source "$(dirname "${BASH_SOURCE[0]}")/env.sh"
pnpm typecheck
pnpm test
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
