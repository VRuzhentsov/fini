#!/bin/sh
# Run a command in the build image of a platform, with the repo mounted.
#
#   containers/run.sh linux   npm ci
#   containers/run.sh android npm run tauri android build -- --debug
#
# The script builds the image first (name fini-<platform>-build). The layer
# cache makes this fast when the Containerfile does not change. The repo is
# at /work, as in CI. Named volumes keep the cargo registry and git, the
# cargo target directory, Gradle, npm and $HOME between runs.
#
# Engine: $CONTAINER_ENGINE if set, else podman, else docker. CI uses podman.
#   podman  Rootless root in the container is the user on the host, so the
#           files in the repo belong to the user. `--layers`, and `:z` (a
#           shared SELinux label) on the mounts.
#   docker  The command runs with the uid and gid of the user (--user), so
#           the files in the repo do not belong to root. Before the run, the
#           volumes get this owner. No `:z`.
# The caches and $HOME are below /cache, so a user that is not root can
# write them.
#
# Android and adb: the container uses the host network, so its adb client
# talks to the adb server of the host (localhost:5037), and the phone can
# reach the dev server of `tauri android dev`. Start the adb server on the
# host first (`adb start-server`). Use an adb on the host with the same
# version as the image (platform-tools, newest), because an adb client of a
# different version stops the server and starts its own one.
# The host ~/.android is mounted, so that all builds sign with the same debug
# keystore. Set FINI_ANDROID_USER_HOME to use a different directory.
set -eu

usage() {
  echo "usage: $0 linux|android <command> [args...]" >&2
  exit 2
}

[ $# -ge 2 ] || usage
platform=$1
shift
case "$platform" in
  linux | android) ;;
  *) usage ;;
esac

# The engine. CONTAINER_ENGINE that is set but empty, `missing` (the value
# that the Makefile gives when it finds no engine) or not installed stops the
# script.
if [ -n "${CONTAINER_ENGINE+set}" ]; then
  engine=$CONTAINER_ENGINE
else
  engine=
  for candidate in podman docker; do
    if command -v "$candidate" > /dev/null 2>&1; then
      engine=$candidate
      break
    fi
  done
fi
if [ -z "$engine" ] || [ "$engine" = missing ] || ! command -v "$engine" > /dev/null 2>&1; then
  echo "$0: no container engine ('$engine'). Install podman or docker, or set CONTAINER_ENGINE=podman|docker." >&2
  exit 1
fi
case "$(basename "$engine")" in
  docker*) docker=1 build_flags= z= ;;
  *) docker= build_flags=--layers z=:z ;;
esac

repo=$(cd "$(dirname "$0")/.." && pwd -P)
prefix=${FINI_RUN_PREFIX:-fini}
image=$prefix-$platform-build

# shellcheck disable=SC2086 # $build_flags is empty or one flag.
"$engine" build $build_flags -f "$repo/containers/$platform/Containerfile" \
  --target build -t "$image" "$repo"

# Each item is <volume>:<path>. The items have no spaces.
volumes="$prefix-cargo:/cache/cargo $prefix-npm:/cache/npm $prefix-home:/cache/home"
volumes="$volumes $prefix-$platform-target:/work/src-tauri/target"
[ "$platform" = android ] && volumes="$volumes $prefix-gradle:/cache/gradle"

volume_args=
paths=
for volume in $volumes; do
  volume_args="$volume_args -v $volume"
  paths="$paths ${volume#*:}"
done

if [ -n "$docker" ]; then
  # shellcheck disable=SC2086 # Lists without spaces in the items.
  "$engine" run --rm --user 0:0 $volume_args "$image" chown "$(id -u):$(id -g)" $paths
fi

# The command is in "$@". Add the options in front of it.
# shellcheck disable=SC2086 # $volume_args is a list without spaces in the items.
set -- \
  -v "$repo:/work$z" -w /work $volume_args \
  -e CARGO_HOME=/cache/cargo \
  -e npm_config_cache=/cache/npm \
  -e HOME=/cache/home \
  -e JAVA_TOOL_OPTIONS=-Duser.home=/cache/home \
  "$image" "$@"

[ -n "$docker" ] && set -- --user "$(id -u):$(id -g)" "$@"

if [ "$platform" = android ]; then
  android_home=${FINI_ANDROID_USER_HOME:-$HOME/.android}
  mkdir -p "$android_home"
  set -- --network=host \
    -v "$android_home:/cache/home/.android${z:+:z}" \
    -e GRADLE_USER_HOME=/cache/gradle \
    "$@"
fi

# Pass the host variables CI, FINI_*, TAURI_* (the updater signing key
# TAURI_SIGNING_*), ANDROID_KEY_* and ANDROID_KEYSTORE_* (release signing)
# into the container. awk gives the names only, so a value with more than one
# line cannot give a wrong name. The engine takes the values from the
# environment. ANDROID_HOME and NDK_HOME do not pass: the android image sets
# them to its own SDK.
for name in $(awk 'BEGIN { for (k in ENVIRON) print k }' |
  grep -E '^(CI|(FINI|TAURI|ANDROID_KEY|ANDROID_KEYSTORE)_[A-Za-z0-9_]*)$'); do
  [ "$name" = ANDROID_KEYSTORE_PATH ] || set -- -e "$name" "$@"
done

# A keystore in the repo is at /work in the container. Mount a keystore that
# is outside the repo read-only, at the same path.
if [ -n "${ANDROID_KEYSTORE_PATH:-}" ]; then
  keystore=$(cd "$(dirname "$ANDROID_KEYSTORE_PATH")" && pwd -P)/$(basename "$ANDROID_KEYSTORE_PATH")
  case "$keystore" in
    "$repo"/*) keystore=/work/${keystore#"$repo"/} ;;
    *) set -- -v "$keystore:$keystore:ro${z:+,z}" "$@" ;;
  esac
  set -- -e "ANDROID_KEYSTORE_PATH=$keystore" "$@"
fi

tty=
[ -t 0 ] && [ -t 1 ] && tty=-it
# shellcheck disable=SC2086 # $tty is empty or one flag.
exec "$engine" run --rm --init $tty "$@"
