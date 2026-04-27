#!/usr/bin/env bash

set -euo pipefail

actor_slug="${FINI_ACTOR_SLUG:-actor-a}"
socket_dir="${FINI_E2E_SOCKET_DIR:-/var/run/fini-e2e}"
app_data_dir="${FINI_APP_DATA_DIR:-/data}"
socket_path="${TAURI_PLAYWRIGHT_SOCKET:-${socket_dir}/${actor_slug}.sock}"
display="${DISPLAY:-:99}"

mkdir -p "$socket_dir" "$app_data_dir"
rm -f "$socket_path"

export FINI_APP_DATA_DIR="$app_data_dir"
export TAURI_PLAYWRIGHT_SOCKET="$socket_path"
export XDG_DATA_HOME="${XDG_DATA_HOME:-/data}"
export TZ="${TZ:-UTC}"
export DISPLAY="$display"

Xvfb "$display" -screen 0 1280x720x24 -nolisten tcp &
xvfb_pid="$!"

cleanup() {
  if kill -0 "$xvfb_pid" 2>/dev/null; then
    kill "$xvfb_pid" 2>/dev/null || true
    wait "$xvfb_pid" 2>/dev/null || true
  fi
}

trap cleanup EXIT INT TERM

for _ in $(seq 1 50); do
  if [ -S "/tmp/.X11-unix/X${display#:}" ]; then
    break
  fi
  sleep 0.1
done

exec /usr/local/bin/fini app
