---
name: fini-setup
description: "Install or upgrade Fini CLI from the latest GitHub release assets."
---

# Fini Setup

Use when `fini` is missing, broken, outdated, or needed for agent terminal use.

## Release Sources

Use these before doing a general web search:

- Latest release page: `https://github.com/VRuzhentsov/fini/releases/latest`
- Latest release API: `https://api.github.com/repos/VRuzhentsov/fini/releases/latest`

For a requested version, use the matching release page instead of latest.

## Selection

Choose a standalone CLI asset from the release assets for the current system.
The asset should match OS and CPU architecture and should be a CLI archive, not a GUI installer.

Common release families:

- Linux CLI archives contain `fini`
- Windows CLI archives contain `fini.exe`

Let the current system decide the exact detection method: `uname`, PowerShell, environment variables, package metadata, or another reliable local signal.

## Workflow

1. Check whether `fini` already works: `command -v fini` and `fini --help`.
2. Resolve the target OS and architecture from the current environment.
3. Open the latest release/API above, or the user-requested release.
4. Select the matching standalone CLI asset and download URL.
5. Download into `/var/tmp` or the platform's normal temporary directory.
6. Do not treat `.sig` as verified unless the release also provides public verification material or instructions.
7. Extract the archive and locate the actual executable path.
8. Smoke-test the extracted executable with `--help`.
9. Install to a location on `PATH`, or ask if multiple locations are plausible.
10. Verify the final `fini --help` command.

## Safety

- Stop if no matching CLI asset exists; say which platform was detected.
- Do not overwrite an existing binary until the replacement passes `--help`.
- Do not run quest, space, reminder, or Focus commands until setup succeeds.
- Report the release, asset, install path, and verification result.
