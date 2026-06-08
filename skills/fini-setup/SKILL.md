---
name: fini-setup
description: "Install or upgrade Fini CLI from the matching GitHub release executable archive."
---

# Fini Setup

Use when `fini` is missing, broken, outdated, or needed for agent terminal use.

## Goal

Make a working `fini` command available:

- choose the correct release asset for the current OS and architecture
- download the standalone CLI package, not a GUI installer
- verify the executable with `fini --help`
- report version, install path, and replacement status

## Assets

Use the requested `vX.Y.Z`, or the latest stable release from `VRuzhentsov/fini`.

CLI asset patterns:

- Linux x64: `fini-v<VERSION>-linux-x64-cli.tar.gz`
- Linux arm64: `fini-v<VERSION>-linux-arm64-cli.tar.gz`
- Windows x64: `fini-v<VERSION>-windows-x64-cli.zip`
- Windows arm64: `fini-v<VERSION>-windows-arm64-cli.zip`

Map `x86_64`/`amd64` to `x64`; map `aarch64`/`arm64` to `arm64`.
Linux archives contain `fini`; Windows archives contain `fini.exe`.
Use GUI/package assets only when the user explicitly asks for the desktop app.

## Workflow

1. Check existing binary: `command -v fini` and `fini --help`.
2. Detect platform with `uname -s` and `uname -m`.
3. Resolve the release version and matching CLI asset.
4. Download with `gh release download --repo VRuzhentsov/fini` or the release asset URL.
5. Extract under `/var/tmp`; never extract directly into the install directory.
6. Verify the extracted binary with `--help`.
7. Install to a directory already on `PATH`, or `~/.local/bin` when acceptable.
8. On Unix, set executable mode with `chmod 0755 <installed-fini-path>`.
9. Verify final state with `command -v fini` and `fini --help`.

## Safety

- Stop if no matching CLI asset exists; say which platform was detected.
- Do not overwrite an existing binary until the replacement passes `--help`.
- Do not run quest, space, reminder, or Focus commands until setup succeeds.
