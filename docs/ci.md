# CI

> This file gives the decided design. Most of it is not implemented yet: see the "Today" column and "Not done yet".

Where each target of Fini is built and tested: in GitHub Actions, and on a
developer's computer. The images and their stages are in
[`container.md`](container.md).

CI calls `npm run` scripts. CI does not call `make`. A Make target is a local
wrapper of the same npm script.

## Targets

✅ = a supported target. ❌ = not a supported target.

| Target | CI | Local development | Supported | Today |
|---|---|---|---|---|
| Frontend unit tests | `linux` image, stage `build`: `npm run test:unit` | `npm run test:unit` on the host | ✅ | `npm run gate:fe-unit` builds the `Dockerfile` stage `fe-unit-test` with docker |
| Backend unit tests | `linux` image, stage `build`: `cargo test` with the features `cli-plane,ui-plane,desktop-updater` | `make backend-test` on the host | ✅ | `npm run gate:be-compile` and `npm run gate:be-unit` build the stages `be-test-compile` and `be-unit-test` with docker |
| Linux desktop app (glibc) | `linux` image, stage `build`: `npm run tauri build` with the bundles `appimage,deb,rpm` | `make desktop-debug-dev` or `make dev` on the host | ✅ | Built on the runner host with apt packages and `dtolnay/rust-toolchain@stable`; the release also builds arm64 on `ubuntu-24.04-arm` |
| Linux CLI (static musl) | `linux` image, stage `build`: `cargo build --bin fini --features cli-plane --target x86_64-unknown-linux-musl` | `cargo build` on the host | ✅ | A glibc build on the runner host (`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`) |
| CLI container image | `podman build -f containers/cli/Containerfile`; the release publishes it to GHCR | `make runtime-image` | ✅ | `Dockerfile` stage `runtime` (`ubuntu:24.04`), built with docker buildx |
| Desktop e2e | `linux` image, stage `e2e` ([`e2e.md`](e2e.md)) | `make e2e` on the host, on the real screen | ✅ | `Dockerfile` stage `dev-runner`, with a GHCR image for each hash of its inputs |
| Android app | `android` image, stage `build`: `npm run tauri android build` | `make android-dev`, `make android-debug-deploy` in the `android` image; adb connects to the phone | ✅ | On the runner host: `actions/setup-java` (JDK 17), `android-actions/setup-android`, `dtolnay/rust-toolchain@stable` |
| Android e2e (emulator) | `android` image, stage `e2e`: `npm run test:e2e:android` (needs /dev/kvm) | `npm run test:e2e:android` | ✅ | `Dockerfile` stage `android-phones-runner`; the APK is built on the runner host |
| Windows desktop app | `windows-latest` runner, no container: `npm run tauri build` with the bundle `nsis` | A Windows host | ✅ | As in the CI column; the release also builds on `windows-11-arm` |
| Windows CLI | — | — | ❌ | `ci.yml` and `release-tag.yml` still build it, and the release ships it |
| macOS / iOS | — | — | ❌ | No workflow builds it |
| Meta Quest | — | — | ❌ | No workflow builds it. When it comes, it is a variant of the Android build ([`container.md`](container.md), "Meta Quest") |

## What changes in GitHub Actions

The runner is `ubuntu-latest` (Ubuntu 24.04). It has podman 4.9.3. Each
Linux and Android job does `podman build` of its image with the GHCR layer
cache, and then runs its `npm run` script in the image.

| File | Today | After |
|---|---|---|
| `ci.yml`, job `changes` | The filter `app` has `Dockerfile` | The filter has `containers/**` |
| `ci.yml`, job `fe-unit-tests` | `npm run gate:fe-unit` with `CONTAINER=docker` builds the stage `fe-unit-test` | `npm run test:unit` in the `linux` `build` image, with podman |
| `ci.yml`, job `desktop-bundle-linux` | apt packages, `actions/setup-node`, `dtolnay/rust-toolchain@stable` and `Swatinem/rust-cache` on the host | `npm run tauri build` in the `linux` `build` image |
| `ci.yml`, job `desktop-bundle-windows` | `windows-latest`, `npm run tauri build`, the Windows CLI, `npm run test:e2e:device-support` | No change |
| `ci.yml`, job `android-e2e` | Calls `android-e2e.yml` | No change |
| `quality-gates.yml` | docker buildx; `npm run gate:be-compile` and `npm run gate:be-unit` with GHCR hash-tag images; the stage `runtime` with the GitHub Actions cache; `npm run gate:e2e:build-image` and `npm run gate:e2e:run` with the stage `dev-runner` | podman; the backend tests in the `linux` `build` image; the CLI image from `containers/cli/Containerfile`; the desktop e2e in the `linux` `e2e` image |
| `android-e2e.yml` | `actions/setup-java` (JDK 17), `android-actions/setup-android`, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache` and a Gradle cache on the host; `npm run build:android:e2e` on the host; `npm run test:e2e:android` pulls a GHCR hash-tag image | `npm run build:android:e2e` in the `android` `build` image; `npm run test:e2e:android` in the `android` `e2e` image; no setup-java, setup-android or Rust steps |
| `release-tag.yml`, job `build-linux` | apt packages and `dtolnay/rust-toolchain@stable` on the host; a glibc CLI | The `linux` `build` image; a static musl CLI with the asset name `fini-<target-triple>.tar.gz` and `SHA256SUMS` |
| `release-tag.yml`, job `build-android` | `actions/setup-java` (JDK 17), `android-actions/setup-android`, `dtolnay/rust-toolchain@stable` on the host | `npm run tauri android build` in the `android` `build` image |
| `release-tag.yml`, job `build-docker-image` | `docker/build-push-action` with `./Dockerfile`, target `runtime` | `podman build` of `containers/cli/Containerfile`, then push to GHCR |
| `release-tag.yml`, job `build-windows` | `windows-latest` and `windows-11-arm` | No change |
| `release-dry-run.yml` | The same jobs as `release-tag.yml`, with no publish | The same changes as `release-tag.yml` |
| `release-button.yml`, `security.yml` | No build of the app | No change |

The scripts that the workflows call change too:

| Script | Today | After |
|---|---|---|
| `scripts/e2e-android.mts` | Pulls `<prefix>-android-phones-runner-cache:<hash of the Dockerfile>` from GHCR, or builds the stage `android-phones-runner` and pushes it | Builds the `android` `e2e` stage with the GHCR layer cache; no hash tag |
| `scripts/ci-gate.mts` | `cachedBuild` pulls or pushes GHCR images with a hash of the inputs; `engine()` uses docker first | Builds from the `Containerfile` with the GHCR layer cache; podman |

## Not done yet

- All rows of "What changes in GitHub Actions" and of the script table.
- The `containers/` files: see [`container.md`](container.md), "Not done
  yet".
