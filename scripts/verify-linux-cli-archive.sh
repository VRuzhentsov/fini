#!/usr/bin/env bash
# Verify that the exact staged Linux CLI archive has the expected arch and version.
set -euo pipefail

archive="${1:?usage: $0 <archive.tar.gz> <expected-version> [x64|arm64]}"
expected_version="${2:?usage: $0 <archive.tar.gz> <expected-version> [x64|arm64]}"
expected_arch="${3:-}"
workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

echo "Verifying staged Linux CLI archive: $archive"
tar -xzf "$archive" -C "$workdir"
cli="$workdir/fini"
if [[ ! -f "$cli" ]]; then
  echo "Archive does not contain fini at top level" >&2
  exit 1
fi
chmod +x "$cli"

file_output="$(file "$cli")"
printf '%s\n' "$file_output"
case "$expected_arch" in
  x64)
    if [[ "$file_output" != *"x86-64"* ]]; then
      echo "Expected x64 Linux CLI archive, got: $file_output" >&2
      exit 1
    fi
    ;;
  arm64)
    if [[ "$file_output" != *"ARM aarch64"* && "$file_output" != *"AArch64"* ]]; then
      echo "Expected arm64 Linux CLI archive, got: $file_output" >&2
      exit 1
    fi
    ;;
  "") ;;
  *)
    echo "Unsupported expected architecture: $expected_arch" >&2
    exit 1
    ;;
esac
readelf -l "$cli" 2>/dev/null | grep 'Requesting program interpreter' || true

set +e
stdout="$($cli --version 2>"$workdir/stderr.txt")"
status=$?
set -e
stderr="$(<"$workdir/stderr.txt")"
printf 'fini --version exit=%s\nstdout=%s\nstderr=%s\n' "$status" "$stdout" "$stderr"

if [[ "$status" -ne 0 ]]; then
  exit "$status"
fi
if ! printf '%s\n' "$stdout" | grep -Eq "^fini ${expected_version}(-rc\.[0-9]+)?$"; then
  echo "Unexpected fini --version output for $archive" >&2
  exit 1
fi
