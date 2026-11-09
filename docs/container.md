# Containers

How Fini builds and tests its code in container images, on CI and on a
developer's computer. Which CI job uses which image is in
[`ci.md`](ci.md).

## Rules

- Use one container image for each platform where possible.
- Use one base version for all images: `debian:bookworm-slim`. The Linux and
  the Android images, and the CLI runtime stage, all start from it. Bookworm
  gives glibc 2.36, the oldest glibc that the Linux desktop app needs.
- Each image is in a file with the name `Containerfile`. All images build and
  run with each container engine (see "Container engine").
- Build Windows and macOS/iOS on native runners, not in a container. Tauri
  builds them only on their own operating system: Windows on
  `windows-latest`, macOS on `macos-latest`.
- Keep Gradle and Kotlin versions in the repo, not in an image.

## Container engine

Fini is container-engine agnostic. Every image builds and runs with podman or
docker. CI uses podman (selected with `CONTAINER_ENGINE`). Local development
auto-detects the engine, podman first, and `CONTAINER_ENGINE` overrides it.

- CI: each workflow that builds an image sets `CONTAINER_ENGINE: podman` one
  time, in its top-level `env`, and gives it to
  `.github/actions/container-stage` as the input `engine`. The action calls
  that engine for each command.
- Local: `containers/run.sh`, `scripts/ci-gate.mts`, `scripts/e2e-android.mts`
  and the `Makefile` find the engine in the same way. A `CONTAINER_ENGINE`
  that is empty, `missing` or not installed stops them with one message.
- Only podman uses the GHCR layer cache and its flags (`--layers`,
  `--cache-from`, `--cache-to`, `--cache-ttl`, `--retry`) and the `:z` mount
  option (a shared SELinux label). Docker builds with its local cache only.
- With docker, the local commands run with the uid and gid of the user
  (`--user`), so that the files in the repo do not belong to root. With
  rootless podman, root in the container is the user on the host.

## Images and stages

The images are in these files:

```
containers/
├── linux/Containerfile       FROM debian:bookworm-slim
│   ├── build                 Rust 1.91 (rustup), Node 24, webkit2gtk-4.1 and GTK -dev packages,
│   │   │                     build-essential, pkg-config, musl-tools, Rust target x86_64-unknown-linux-musl
│   │   ├── e2e               build + Playwright browsers, Xvfb, GUI runtime libraries
│   │   └── cli-build         build + the static musl CLI (x86_64 only)
│   └── runtime               FROM debian:bookworm-slim again, with only the `fini` CLI binary of cli-build
├── android/Containerfile     FROM debian:bookworm-slim
│   └── build                 JDK 21 (temurin-21-jdk from the Adoptium apt repository), cmdline-tools,
│       │                     platform-tools, NDK r29, build-tools 36.x and 35.0.0 (AGP), platform 36,
│       │                     Rust 1.91 with the Android targets, Node 24
│       └── e2e               build + Android Emulator, system image API 34, AVD fini-e2e (needs /dev/kvm)
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
| `linux`, `cli-build` | Only the build of the CLI for the `runtime` stage |
| `linux`, `runtime` | The release runtime image of the `fini` CLI, published to GHCR. Local: `make runtime-image`, `make runtime-smoke` |

Android development needs no stage of its own. It uses the android `build`
stage.

## Layer order

A change to one tool must not build the other tools again. For this reason:

- Each large tool installs in a stage of its own. The stages that callers use
  (`build`, `e2e`, `cli-build`, `runtime`) install their apt packages, and
  then copy each tool with `COPY --from=<tool stage>`.
- The COPY lines are in this order: the tool that changes least comes first.
  A changed tool builds its own stage and the steps after its COPY again. The
  stages of the other tools stay in the cache.
- Declare each `ARG` just before the RUN that uses it, in the stage that uses
  it. A changed `ARG` makes all the steps after its declaration build again.
- One RUN for each large download or install. Each RUN removes its temporary
  files (`/var/lib/apt/lists/*`, the `.temp` folder of sdkmanager).

```
containers/linux/Containerfile
├── fetch                     curl, xz-utils, jq
│   ├── rust                  rustup, Rust 1.91, the musl target
│   └── node                  Node 24
│       └── playwright        Chromium of the Playwright version in package-lock.json
├── build                     apt -dev packages, then COPY rust, node
│   ├── e2e                   apt GUI libraries, then COPY playwright
│   └── cli-build             the static musl CLI
└── runtime                   COPY the CLI of cli-build

containers/android/Containerfile
└── jdk                       JDK 21 (sdkmanager needs it), curl, unzip
    ├── android-cmdline-tools
    │   ├── android-sdk       one RUN each: platform 36, build-tools 35.0.0, build-tools 36.x, platform-tools
    │   ├── android-ndk       NDK r29
    │   ├── android-emulator  the emulator build 14788078
    │   └── android-system-image  API 34
    ├── rust                  rustup, Rust 1.91, the Android targets
    ├── node                  Node 24
    └── build                 apt packages, then COPY android-ndk, android-sdk, rust, node
        └── e2e               apt emulator libraries, then COPY android-emulator,
                              android-system-image, and the AVD fini-e2e
```

Thus a new Rust version builds the `rust` stage and the COPY steps after it
again. It does not run sdkmanager, and it does not download the NDK, the
emulator or Node again. A change of the JDK builds all of the android image
again, because sdkmanager runs on it.

## Versions

Pin the minor version. The build gets the newest patch at build time. The
exceptions have an exact pin: the Android Emulator (see below), the
cmdline-tools and the build-tools of AGP.

| Item | Pin | Newest patch comes from |
|---|---|---|
| Base distribution | `debian:bookworm-slim` | the tag |
| Rust | 1.91 | rustup (newest 1.91.x) |
| Node | 24 | nodejs.org, `latest-v24.x` (newest 24.x), checked against its `SHASUMS256.txt` |
| JDK | 21 | apt, package `temurin-21-jdk` |
| Android NDK | r29 | sdkmanager (newest 29.x) |
| Android build-tools | 36 | sdkmanager (newest 36.x) |
| Android build-tools of AGP | 35.0.0, exact | no newest patch: the pin of AGP 8.11. It is the default build-tools of AGP for each module that does not set `buildToolsVersion`, also for the Tauri plugin modules. `AGP_BUILD_TOOLS` in `containers/android/Containerfile` changes with the AGP version |
| Android cmdline-tools | the exact zip 11076708 (12.0) | no newest patch: no download URL gives the newest patch, only one exact zip (`ANDROID_CMDLINE_TOOLS`) |
| Gradle | 8.14.6, in the repo | `src-tauri/gen/android/gradle/wrapper/gradle-wrapper.properties` |
| Android Gradle Plugin (AGP) | 8.11.1, in the repo | `src-tauri/gen/android/build.gradle.kts` and `src-tauri/gen/android/buildSrc/build.gradle.kts` |
| Kotlin | 1.9.25, in the repo | `src-tauri/gen/android/build.gradle.kts`; Kotlin stays 1.9.25 until the Tauri 2.12 migration |
| Android Emulator | the exact build 14788078 (36.4.9) | no newest patch: the exception below |
| Android system image (e2e) | API 34, `default` (AOSP), x86_64 | sdkmanager (newest revision). Not `google_apis`: an ANR of Google Play services stops its FontsProvider, and Android then stops each WebView app that uses it, also Fini |

The Android Emulator stays at the exact build 14788078. The stable emulator 37.2.13 stops with SIGSEGV at start
on Bazzite, and it does not know `-gpu angle_indirect` (the comment above
`ANDROID_EMULATOR_BUILD` in `containers/android/Containerfile`, stage `e2e`).
Examine this pin again when a stable emulator newer than 37.2.13 is released.

## How CI and a developer get an image

No file holds an image reference. CI and a developer always build the image
from its `Containerfile`, with `.github/actions/container-stage` in CI. With
podman, the engine of CI, a layer cache in GHCR makes the build fast. The cache repositories are
`ghcr.io/<owner>/fini-cache/linux` and `ghcr.io/<owner>/fini-cache/android`:

```
podman build --layers \
  --cache-from ghcr.io/<owner>/<cache repository> \
  --cache-to   ghcr.io/<owner>/<cache repository> \
  --cache-ttl=168h \
  --retry 5 --retry-delay 5s \
  -f containers/<platform>/Containerfile --target <stage> .
```

With podman, a developer's build uses the same flags. With docker, the build
is `docker build -f containers/<platform>/Containerfile --target <stage> .`
with the local cache of docker only. The flags `--layers`, `--cache-ttl`,
`--retry`, `--cache-from`/`--cache-to` with a GHCR repository, and the `:z`
mount option (a shared SELinux label) are for podman only.

Fini does not pull a prebuilt image by name. It has no digest file, no hash
tag, no version tag file, no bot pull request, and no scheduled rebuild.

The cache rules:

- All runs use `--cache-from`.
- The cache packages in GHCR are public. Thus all runs read the cache with
  no login. Set each cache package public by hand one time, after the first
  push makes it (the package, "Package settings", "Change visibility").
- Only a pull request from a branch in this repository uses `--cache-to`.
  Release runs, `workflow_dispatch` runs and pull requests from a fork read
  the cache only. A pull request from a fork gets a read-only token, so it
  cannot write to GHCR.
- All runs use `--retry 5 --retry-delay 5s` for the pulls and pushes to GHCR.
- All runs use `--cache-ttl=168h`. The build does not use a cache layer that
  is older than 7 days. It builds that layer again and gets the newest
  patches. For this reason, Fini needs no scheduled rebuild.
- A GHCR cleanup job (`.github/workflows/ghcr-cleanup.yml`, on a push to
  `main` or by hand) deletes the cache tags that are older than 7 days.

The results:

- When a `Containerfile` does not change, the build uses the cache. The
  build is done in approximately 1 minute.
- When a line near the top of a `Containerfile` changes, the next build
  builds the image again one time. For the android image, this build is done
  in approximately 10 to 15 minutes.
- A new patch version comes in when the cache layer is older than 7 days,
  when the cache does not have the layer, or when a person builds with
  `--no-cache`.

## Development

- **Desktop**: development runs on the host, as Tauri expects
  (`make desktop-debug-dev`). The host toolchain can be different from the
  toolchain in CI.
- **Android**: development runs in the android `build` image
  (`make android-dev`, `make android-build` and the other Android targets,
  through `containers/run.sh`). The repo is mounted, and the adb client in
  the container uses the adb server of the host (host network). The image gives one JDK, so a
  different host JDK (for example the JBR 25 of a host Android Studio) does
  not stop the Gradle build.
- **`containers/run.sh linux|android <command>`**: the local wrapper. It
  builds the `build` stage of the platform (local name
  `fini-<platform>-build`), and then runs the command with the container
  engine ("Container engine"), with the repo mounted at `/work`. It keeps named
  volumes for the cargo registry, the cargo target directory, Gradle, npm and
  `$HOME` (below `/cache`). It passes the host variables `CI`, `FINI_*`,
  `TAURI_*`, `ANDROID_KEY_*` and `ANDROID_KEYSTORE_*`. `scripts/ci-gate.mts`
  and `scripts/e2e-android.mts` start their containers in the same way.

## CLI

- The Linux CLI is a static musl binary (`x86_64-unknown-linux-musl`).
- The release has an x86_64 Linux CLI only. For now, it has no arm64 Linux
  CLI (`aarch64-unknown-linux-gnu`). The arm64 Linux desktop app does not
  change.
- **CLI without Bluetooth.** `ble-gatt` and `ble-gatt-iroh` belong to the
  `ui-plane` feature only. The CLI build (`cli-plane`) does not include them,
  and it does not create `BluetoothChannelService`. Thus the musl CLI links
  no `libdbus`.
- `cpal` is an optional dependency. Voice is not in the build (`mod voice` is
  a comment in `lib.rs`), and `alsa-sys` stops a musl build.
- The release asset keeps the name `fini-<tag>-linux-<triple>-cli.tar.gz`,
  with the musl triple: `fini-<tag>-linux-x86_64-unknown-linux-musl-cli.tar.gz`.
  The release has no `SHA256SUMS` file. The CLI update checks the zipsign
  signature of the asset, and that is sufficient.
- The CLI update looks for the musl triple
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

- The cache packages `fini-cache/linux` and `fini-cache/android` are not
  public yet. The first pull request run that writes the cache makes them.
  Then set each one public by hand, one time (see "How CI and a developer get
  an image").
- The CLI build (`cli-plane`) has 50 dead-code warnings (58 before the CLI
  lost Bluetooth). 17 of them come from the path that accepts and serves
  links (`LinkAcceptor`, `accept`, `serve`): the Network channel shares this
  path, and the CLI does not use it. A follow-up gates it with
  `#[cfg(any(feature = "ui-plane", test))]`. The other 33 were there before.
- `cli_asset_target_substrings` in `src-tauri/src/services/cli_update.rs`
  still maps `cli-linux-aarch64` to `aarch64-unknown-linux-gnu`. The release
  has no such asset, so an arm64 Linux CLI finds no update.
