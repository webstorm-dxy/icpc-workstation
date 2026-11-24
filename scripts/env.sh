#!/usr/bin/env bash
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export TMPDIR="$ROOT/.tmp"; mkdir -p "$TMPDIR"
export PNPM_HOME="$ROOT/.pnpm-home"; mkdir -p "$PNPM_HOME"
export CARGO_HOME="$ROOT/.cargo-home"; mkdir -p "$CARGO_HOME"
export XDG_CACHE_HOME="$ROOT/.cache"; mkdir -p "$XDG_CACHE_HOME"
export PATH="$PNPM_HOME:$PATH"
export ICPC_DATA_DIR="${ICPC_DATA_DIR:-$ROOT/.appdata}"
cd "$ROOT"
