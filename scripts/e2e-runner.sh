#!/usr/bin/env sh
set -eu

Xvfb :99 -screen 0 1280x1024x24 -nolisten tcp &
xvfb_pid=$!

cleanup() {
  kill "$xvfb_pid" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

sleep 1
DISPLAY=:99 npm run test:e2e:ci
