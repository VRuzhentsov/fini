#!/usr/bin/env bash
# Verify that a generated AppImage leaves the host Wayland ABI on the run host.
# Usage: scripts/verify-appimage-host-runtime.sh path/to/Fini.AppImage
set -euo pipefail

appimage=${1:?"Usage: $0 path/to/Fini.AppImage"}
if [[ ! -f "$appimage" ]]; then
  echo "AppImage not found: $appimage" >&2
  exit 2
fi

appimage=$(readlink -f "$appimage")
workspace=$(mktemp -d)
trap 'rm -rf "$workspace"' EXIT

(
  cd "$workspace"
  "$appimage" --appimage-extract >/dev/null
)

appdir="$workspace/squashfs-root"
web_process=$(find "$appdir/usr/lib" -type f -name WebKitWebProcess -print -quit)
if [[ -z "$web_process" || ! -x "$web_process" ]]; then
  echo "Expected WebKitWebProcess was not bundled under $appdir/usr/lib" >&2
  exit 1
fi

# These libraries must resolve from the operating system that runs Fini. Bundling
# them mixes the build host's Wayland ABI with the run host's Mesa/EGL stack.
for library in \
  libwayland-client.so.0 \
  libwayland-cursor.so.0 \
  libwayland-egl.so.1 \
  libwayland-server.so.0
do
  if [[ -e "$appdir/usr/lib/$library" ]]; then
    echo "AppImage must not bundle host Wayland ABI library: $library" >&2
    exit 1
  fi
done

echo "AppImage host-runtime verification passed: Wayland ABI libraries are host-provided."
