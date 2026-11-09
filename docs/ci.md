# CI

Where each target of Fini is built and tested: in GitHub Actions, and on a
developer's computer. The images and their stages are in
[`container.md`](container.md).

CI calls `npm run` scripts. CI does not call `make`. A Make target is a local
wrapper of the same npm script.

## Targets

✅ = a supported target. ❌ = not a supported target.

| Target | CI | Local development | Supported | Today |
|---|---|---|---|---|
| Frontend unit tests | `linux` image, stage `build`: `npm run test:unit` | `npm run test:unit` on the host; `make pr-gate-fe-unit` in the `linux` `build` image | ✅ | As in the CI column |
| BE unit tests | `linux` image, stage `build`: `cargo test` with the features `cli-plane,ui-plane,desktop-updater` | `make backend-test` on the host; `make pr-gate-be-unit` in the `linux` `build` image | ✅ | As in the CI column |
| Linux desktop app (glibc) | `linux` image, stage `build`: `npm run tauri build` with the bundles `appimage,deb,rpm` | `make desktop-debug-dev` or `make dev` on the host | ✅ | As in the CI column; the release also builds arm64 in the arm64 `linux` image on `ubuntu-26.04-arm` |
| Linux CLI (static musl) | `linux` image, stage `build`: `cargo build --bin fini --features cli-plane --target x86_64-unknown-linux-musl`, with no Bluetooth ([`container.md`](container.md), "CLI"). x86_64 only: the release has no arm64 Linux CLI | `cargo build` on the host | ✅ | As in the CI column |
| CLI container image | `<engine> build -f containers/linux/Containerfile --target runtime`; the release publishes it to GHCR | `make runtime-image`, `make runtime-smoke` | ✅ | As in the CI column |
| Desktop e2e | `linux` image, stage `e2e` ([`e2e.md`](e2e.md)) | `make e2e` on the host, on the real screen; `make pr-gate-e2e` in the `linux` `e2e` image | ✅ | As in the CI column |
| Android app | `android` image, stage `build`: `npm run tauri android build` | `make android-dev`, `make android-debug-deploy` in the `android` image; adb connects to the phone | ✅ | As in the CI column |
| Android e2e (emulator) | `android` image, stage `e2e`: `npm run test:e2e:android` (needs /dev/kvm); the APK is built in the `android` `build` image | `npm run test:e2e:android` | ✅ | As in the CI column |
| Windows desktop app | `windows-latest` runner, no container: `npm run tauri build` with the bundle `nsis` | A Windows host | ✅ | As in the CI column; the release also builds on `windows-11-arm` |
| Windows CLI | `windows-latest` runner, no container: `cargo build --bin fini --features cli-plane --target x86_64-pc-windows-msvc`; the release also builds on `windows-11-arm` | A Windows host | ✅ | As in the CI column. It is built and shipped, but no check makes sure that it runs |
| macOS / iOS | — | — | ❌ | No workflow builds it |
| Meta Quest | — | — | ❌ | No workflow builds it. When it comes, it is a variant of the Android build ([`container.md`](container.md), "Meta Quest") |

## The container jobs in GitHub Actions

The runner of the Linux and Android container jobs is `ubuntu-26.04`
(`ubuntu-26.04-arm` for the arm64 Linux desktop app). It has Podman 5.7.0
and Buildah 1.42.1. These jobs do not use `ubuntu-24.04`: its Podman 4.9.3
has bugs in the layer cache. The images stay on `debian:bookworm-slim`. The
runner gives only the Podman version. Each Linux and Android job calls
`.github/actions/container-stage` with the container engine of the workflow
(`CONTAINER_ENGINE: podman`; see [`container.md`](container.md), "Container
engine"). It builds the stage with the GHCR layer cache (the cache rules are in [`container.md`](container.md), "How
CI and a developer get an image"), and then runs its command in the stage,
with the repo mounted at `/work`.

The jobs that build an android image, and Quality Gates, first free disk
space on the runner (`jlumbroso/free-disk-space`). The android images are
large: `build` is about 8.4 GB and `e2e` about 10.8 GB.

Only a pull request from a branch in this repository writes the cache
(`--cache-to`). For the cache, only these jobs get `packages: write`; the
release job that publishes the CLI image gets it too. Release runs
(`release-tag.yml`, `release-dry-run.yml`), `workflow_dispatch` runs and pull
requests from a fork read the cache only. The cache packages are public, so a
read needs no GHCR login.

| File | Job or step | What it does |
|---|---|---|
| `ci.yml` | `changes` | The filter `app` has `containers/**` and `.github/actions/**` |
| `ci.yml` | `fe-unit-tests` | `npm run test:unit` in the `linux` `build` image |
| `ci.yml` | `desktop-bundle-linux` | `npm run tauri build` in the `linux` `build` image |
| `ci.yml` | `desktop-bundle-windows` | `windows-latest`, `npm run tauri build`, the Windows CLI, `npm run test:e2e:device-support`; no container |
| `ci.yml` | `android-e2e` | Calls `android-e2e.yml` |
| `quality-gates.yml` | Quality Gates | The backend tests in the `linux` `build` image; the CLI image (`linux`, `runtime`) and its smoke check; the desktop e2e (`npm run gate:e2e:run`) in the `linux` `e2e` image |
| `android-e2e.yml` | Android E2E | `npm run build:android:e2e` in the `android` `build` image; `npm run test:e2e:android` in the `android` `e2e` image; no setup-java, setup-android or Rust steps |
| `release-tag.yml` | `build-linux` | The `linux` `build` image; a static musl CLI for x86_64 only, with the asset name `fini-<tag>-linux-x86_64-unknown-linux-musl-cli.tar.gz`. The arm64 matrix entry builds the arm64 desktop app only, and the release has no arm64 Linux CLI |
| `release-tag.yml` | `build-android` | `npm run tauri android build` and the signing in the `android` `build` image |
| `release-tag.yml` | `build-docker-image` | The build of the `linux` `runtime` stage, then push to GHCR (`redhat-actions/push-to-registry`, which pushes from the podman image store) |
| `release-tag.yml` | `build-windows` | `windows-latest` and `windows-11-arm`; no container |
| `release-dry-run.yml` | all | The same jobs as `release-tag.yml`, with no publish |
| `ghcr-cleanup.yml` | cleanup | Deletes the cache tags that are older than 7 days, on a push to `main` or by hand |
| `release-button.yml` | release | The only start of a release. The user starts it by hand: it commits the version on `main` (`cargo xtask release-version`) and pushes the signed tag `vX.Y.Z`, and the tag push starts `release-tag.yml`. No build of the app |
| `security.yml` | all | No build of the app |

The scripts that build the same stages on a developer's computer:

| Script | What it does |
|---|---|
| `scripts/ci-gate.mts` | `npm run gate:*`: builds a `linux` stage (`build` or `e2e`) and runs the command of the step in it. With podman, it uses the podman flags of `container-stage`, and the GHCR layer cache when `FINI_CONTAINER_CACHE` is set. In the `e2e` container (`FINI_E2E_CONTAINER_RUNNER=1`), `gate:e2e:run` builds the e2e binaries and runs `scripts/e2e-runner.sh`; CI calls it there |
| `scripts/e2e-android.mts` | `npm run test:e2e:android`: builds the `android` `e2e` stage and runs Playwright in it, with `/dev/kvm`. In the container (`FINI_E2E_CONTAINER_RUNNER=1`), it runs Playwright at once; CI calls it there |
| `containers/run.sh` | Runs a command in the `build` stage of `linux` or `android`. The Makefile uses it for the Android targets |

## Not done yet

- The cache packages are not public yet: see [`container.md`](container.md),
  "Not done yet".
