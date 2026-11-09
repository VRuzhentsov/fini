# Containers

> This file gives the decided design. It is not implemented yet: today the root `Dockerfile` holds all stages (see "Not done yet").

How Fini builds and tests its code in container images, on CI and on a
developer's computer. Which CI job uses which image is in
[`ci.md`](ci.md).

## Rules

- Use one container image for each platform where possible.
- Use one base version for all images: `debian:bookworm-slim`. The Linux, the
  Android and the CLI images all start from it. Bookworm gives glibc 2.36, the
  oldest glibc that the Linux desktop app needs.
- Use podman as the container engine. Each image is in a file with the name
  `Containerfile`.
- Build Windows and macOS/iOS on native runners, not in a container. Tauri
  builds them only on their own operating system: Windows on
  `windows-latest`, macOS on `macos-latest`.
- Keep Gradle and Kotlin versions in the repo, not in an image.

## Images and stages

The root `Dockerfile` is divided into these files, and then deleted. The file
names are proposals until the change is implemented.

```
containers/
├── linux/Containerfile       FROM debian:bookworm-slim
│   └── build                 Rust 1.91 (rustup), Node 24, webkit2gtk-4.1 and GTK -dev packages,
│       │                     build-essential, pkg-config, musl-tools, Rust target x86_64-unknown-linux-musl
│       └── e2e               build + Playwright browsers, Xvfb, GUI runtime libraries
├── android/Containerfile     FROM debian:bookworm-slim
│   └── build                 JDK 21 (temurin-21-jdk from the Adoptium apt repository), cmdline-tools,
│       │                     platform-tools, NDK r29, build-tools 36.x, platform 36,
│       │                     Rust 1.91 with the Android targets, Node 24
│       └── e2e               build + Android Emulator, system image API 34 (needs /dev/kvm)
├── cli/Containerfile         FROM debian:bookworm-slim, with the `fini` CLI binary
└── run.sh                    local wrapper (see "Development")
```

Bookworm's own apt repository has JDK 17 only. For this reason the android
image adds the Adoptium apt repository (`packages.adoptium.net`) and installs
`temurin-21-jdk`.

| Image, stage | Used for |
|---|---|
| `linux`, `build` | CI: the Linux desktop app (glibc), the Linux CLI (static musl), the FE and BE unit tests |
| `linux`, `e2e` | CI and local: the desktop e2e tests ([`e2e.md`](e2e.md)) |
| `android`, `build` | CI: the Android builds. Local: `make android-dev` and the other Android build targets |
| `android`, `e2e` | CI and local: the e2e tests on virtual phones ([`e2e-android.md`](e2e-android.md)) |
| `cli` | The release runtime image of the `fini` CLI, published to GHCR |

Android development needs no stage of its own. It uses the android `build`
stage.

## Versions

Pin the minor version. The build gets the newest patch at build time.

| Item | Pin | Newest patch comes from |
|---|---|---|
| Base distribution | `debian:bookworm-slim` | the tag |
| Rust | 1.91 | rustup (newest 1.91.x) |
| Node | 24 | the Node 24 package |
| JDK | 21 | apt, package `temurin-21-jdk` |
| Android NDK | r29 | sdkmanager (newest 29.x) |
| Android build-tools | 36 | sdkmanager (newest 36.x) |
| Gradle | in the repo | `src-tauri/gen/android/gradle/wrapper/gradle-wrapper.properties` |
| Kotlin | in the repo | `src-tauri/gen/android/build.gradle.kts`; it moves to 2.2.x together with JDK 21 |

## How CI and a developer get an image

No file holds an image reference. CI and a developer always build the image
from its `Containerfile` with `podman build`. A layer cache in GHCR makes the
build fast:

```
podman build --layers \
  --cache-from ghcr.io/<owner>/<cache repository> \
  --cache-to   ghcr.io/<owner>/<cache repository> \
  -f containers/<platform>/Containerfile --target <stage> .
```

Fini does not pull a prebuilt image by name. It has no digest file, no hash
tag, no version tag file, no bot pull request, and no scheduled rebuild.

The results:

- When a `Containerfile` does not change, the build uses the cache. The
  build is done in approximately 1 minute.
- When a line near the top of a `Containerfile` changes, the next build
  builds the image again one time. For the android image, this build is done
  in approximately 10 to 15 minutes.
- A new patch version comes in only when the cache does not have the layer,
  or when a person builds with `--no-cache`.

## Development

- **Desktop**: development runs on the host, as Tauri expects
  (`make desktop-debug-dev`). The host toolchain can be different from the
  toolchain in CI.
- **Android**: development runs in the android `build` image. The repo is
  mounted, and adb connects to the phone. The image gives one JDK, so a
  different host JDK (for example the JBR 25 of a host Android Studio) does
  not stop the Gradle build.
- **`containers/run.sh`** (proposal): the local wrapper. It does
  `podman run` with the repo mounted, and it keeps named volumes for the
  cargo registry, the cargo target directory, Gradle and npm.

## CLI

- The Linux CLI is a static musl binary (`x86_64-unknown-linux-musl`).
- **CLI without Bluetooth.** `ble-gatt` and `ble-gatt-iroh` belong to the
  `ui-plane` feature only. The CLI build (`cli-plane`) does not include them,
  and it does not create `BluetoothChannelService`. Thus the musl CLI links
  no `libdbus`.
- `cpal` is an optional dependency. Voice is not in the build (`mod voice` is
  a comment in `lib.rs`), and `alsa-sys` stops a musl build.
- The release asset keeps the name `fini-<tag>-linux-<triple>-cli.tar.gz`,
  with the musl triple: `fini-<tag>-linux-x86_64-unknown-linux-musl-cli.tar.gz`.
  A `SHA256SUMS` file gives the checksum of each asset.
- The CLI update accepts the musl triple too
  (`cli_asset_target_substrings` in `src-tauri/src/services/cli_update.rs`).
  A person installs a CLI from before this change again by hand, one time.
- The Windows CLI stays as it is today: `ci.yml` builds it, and the release
  ships it. This change adds nothing for it.
- A macOS CLI is not in this change.

## Meta Quest

The Meta Quest app is a variant of the Android build, with the Quest manifest.
It uses the android image. It does not have an image of its own.

## Research basis

- [Mullvad VPN, `building/`](https://github.com/mullvad/mullvadvpn-app/tree/main/building):
  the model. It has a Linux image and an Android image, uses podman, and
  builds Windows on a runner.
- [Signal Android, `docker.yml`](https://github.com/signalapp/Signal-Android/blob/main/.github/workflows/docker.yml)
  and [Bitcoin Core, `02_run_container.py`](https://github.com/bitcoin/bitcoin/blob/master/ci/test/02_run_container.py):
  they build the image in each run, with a cache.
- [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/): desktop
  development on the host.
- [Tauri, Debian distribution](https://v2.tauri.app/distribute/debian/):
  build on the oldest base that the app supports.
- [Codex CLI, `rust-release.yml`](https://github.com/openai/codex/blob/main/.github/workflows/rust-release.yml):
  Linux CLI releases as static musl binaries, with the target triple in the
  asset name.

## Not done yet

- The `containers/` files and `containers/run.sh` do not exist. The root
  `Dockerfile` has all stages: `node-deps`, `playwright-browsers`,
  `fe-unit-test`, `rust-builder-base` (`rust:1.91-bookworm`),
  `be-test-compile`, `be-unit-test`, `app-build-cli-prod`,
  `app-build-cli-dev`, `app-build-ui-dev`, `runtime-base` and `runtime`
  (`ubuntu:24.04`), `dev-runner`, `android-phones-runner`
  (`debian:trixie-slim`) and `android-build` (`eclipse-temurin:17-jdk`).
- The Makefile and the scripts find the engine as docker first, then podman
  (`CONTAINER_ENGINE` in the `Makefile`, `engine()` in
  `scripts/ci-gate.mts`). `scripts/e2e-android.mts` uses podman.
- `scripts/ci-gate.mts` and `scripts/e2e-android.mts` pull images from GHCR
  with a hash tag (`cachedBuild`, and the hash of the `Dockerfile`).
- The `android-build` stage has only JDK 17 and Node. `ANDROID_BUILD_RUN` in
  the `Makefile` mounts the Android SDK, the NDK, `~/.cargo` and `~/.rustup`
  of the host. `make android-dev` runs on the host, not in a container. The
  `Makefile` comment above `ANDROID_BUILD_RUN` refers to an
  `android-build.Containerfile` that does not exist.
- Kotlin is 1.9.25 and Gradle is 8.14.3.
- The emulator is a fixed build (14788078), not the newest patch. The reason
  is in [`e2e-android.md`](e2e-android.md), "The image".
- The Linux CLI is a glibc build (`x86_64-unknown-linux-gnu`). The CLI update
  (`cli_asset_target_substrings`) does not accept the musl triple.
- In `src-tauri/Cargo.toml`, `ble-gatt` and `ble-gatt-iroh` are Linux
  dependencies of all builds, also of the CLI. `src-tauri/src/services/cli.rs`
  creates `BluetoothChannelService`. `cpal` is not optional.
