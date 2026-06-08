---
name: fini-setup
description: "Install or upgrade Fini for agent CLI use by selecting the correct GitHub release CLI executable for the current platform."
---

# Fini Setup

Use when the user asks to install, update, repair, or make Fini available in a terminal for agent use.

This skill installs the CLI entrypoint needed by the public `fini` skill. After installation succeeds, use `fini` for quest, space, reminder, and Focus operations.

## Outcome

Make a working `fini` command available:

- choose the correct release asset for the current OS and architecture
- download the CLI executable package, not only the GUI installer
- verify the executable runs with `fini --help`
- leave the user with the exact installed path and version evidence

## Asset Selection

Use the latest stable GitHub release unless the user asks for a specific version.

Repository:

```text
https://github.com/VRuzhentsov/fini
```

Prefer CLI assets for agent use:

- Linux x64: `fini-v<VERSION>-linux-x64-cli.tar.gz`
- Linux arm64: `fini-v<VERSION>-linux-arm64-cli.tar.gz`
- Windows x64: `fini-v<VERSION>-windows-x64-cli.zip`
- Windows arm64: `fini-v<VERSION>-windows-arm64-cli.zip`

The Linux CLI archives contain an executable named `fini`.
The Windows CLI archives contain `fini.exe`.

Use GUI/package assets only when the user explicitly asks for the desktop app installer:

- Linux: `.AppImage`, `.deb`, or `.rpm`
- Windows: `setup.exe`
- Android: `.apk` or `.aab`

## Platform Detection

Detect platform before download:

```bash
uname -s
uname -m
```

Map architecture names:

- `x86_64` or `amd64` -> `x64`
- `aarch64` or `arm64` -> `arm64`

If the platform has no matching CLI release asset, stop and say which platform was detected and which assets are available.

## Install Workflow

1. Check whether `fini` already exists:

   ```bash
   command -v fini
   fini --help
   ```

2. Resolve the release version:

   - use the user-requested `vX.Y.Z`, or
   - query the latest stable GitHub release

3. Download the matching CLI asset and its `.sig` when available.
4. Extract the archive into a temporary directory.
5. Verify the extracted executable:

   ```bash
   ./fini --help
   ```

6. Install it to a directory already on `PATH`, or to `~/.local/bin` when that is acceptable for the user environment.
7. Ensure the installed binary is executable on Unix-like systems:

   ```bash
   chmod 0755 <installed-fini-path>
   ```

8. Verify the final command:

   ```bash
   command -v fini
   fini --help
   ```

Do not run quest, space, reminder, or Focus commands until installation verification succeeds.

## Download Commands

When GitHub CLI is available, prefer:

```bash
gh release download <VERSION> --repo VRuzhentsov/fini --pattern '<ASSET_NAME>' --pattern '<ASSET_NAME>.sig' --dir <DOWNLOAD_DIR>
```

When GitHub CLI is unavailable, use the release download URL:

```text
https://github.com/VRuzhentsov/fini/releases/download/<VERSION>/<ASSET_NAME>
```

Always download into a temporary working directory first. Do not extract archives directly into the final install directory.

## Safety

- Do not overwrite an existing `fini` binary until the replacement executable has passed `--help`.
- If replacing an existing binary, keep a rollback copy unless the user asks for a clean reinstall.
- Do not install GUI packages when the user only needs CLI agent access.
- Do not assume package-manager availability; release assets are the source of truth.
- Do not write machine-specific absolute paths into project files or reusable instructions.

## Handoff

Report:

- selected release version and asset name
- installed command path
- verification command and result
- whether a previous binary was replaced or left untouched
