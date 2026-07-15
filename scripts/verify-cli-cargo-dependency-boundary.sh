#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf 'Usage: %s [--tree-file PATH]\n' "$0" >&2
}

tree_file=""
if [[ "${1:-}" == "--tree-file" ]]; then
  tree_file="${2:-}"
  [[ -n "$tree_file" ]] || { usage; exit 2; }
  shift 2
fi
[[ "$#" -eq 0 ]] || { usage; exit 2; }

if [[ -n "$tree_file" ]]; then
  tree=$(<"$tree_file")
else
  tree=$(cargo tree --manifest-path src-tauri/Cargo.toml --no-default-features --features cli-plane -e normal)
fi

forbidden='(^|[[:space:]│├└])((tauri|tauri-plugin-[^[:space:]]*|gtk|webkit[^[:space:]]*|javascriptcore[^[:space:]]*|wry|tao|muda)[[:space:]]+v)'
matches=$(printf '%s\n' "$tree" | grep -Ei "$forbidden" || true)
if [[ -n "$matches" ]]; then
  printf '%s\n' 'Standalone CLI dependency graph contains desktop runtime dependencies:' >&2
  printf '%s\n' "$matches" >&2
  exit 1
fi
