-include .env
export

CONTAINER ?= auto
CONTAINER_ENGINE = $(shell if [ "$(CONTAINER)" = "auto" ]; then if command -v docker >/dev/null 2>&1; then printf docker; elif command -v podman >/dev/null 2>&1; then printf podman; else printf missing; fi; else printf '%s' "$(CONTAINER)"; fi)
FINI_BE_COMPILE_IMAGE ?= fini-be-compile-ci
FINI_BE_UNIT_IMAGE ?= fini-be-unit-test
FINI_BE_CACHE_IMAGE_PREFIX ?=
FINI_BE_CACHE_PUSH ?= 0
FINI_E2E_CI_RUN_ID ?= pr-gate
FINI_SCRATCH_DIR ?= $(CURDIR)/tmp
FINI_E2E_CI_RUN_DIR ?= $(FINI_SCRATCH_DIR)/fini-e2e-$(FINI_E2E_CI_RUN_ID)
FINI_E2E_CI_RESULTS_DIR ?= $(FINI_E2E_CI_RUN_DIR)/test-results
FINI_E2E_CI_ACTORS ?= actor-a,actor-b
FINI_E2E_CI_ACTOR_WAIT_SECS ?= 180
FINI_DEV_RUNNER_IMAGE ?= fini-dev-runner-ci
FINI_E2E_CACHE_IMAGE_PREFIX ?=
FINI_E2E_CACHE_PUSH ?= 0
RELEASE_BUNDLES ?= deb,rpm
# linuxdeploy vendors an older `strip` that cannot parse `.relr.dyn` sections
# emitted by newer host toolchains (e.g. Fedora 44+), which aborts local Linux
# bundling before it produces an AppImage. Skip stripping by default for local
# builds; override with NO_STRIP=false if your toolchain doesn't need this.
# CI release builds run `npm run tauri build` directly, not this target, so
# this default has no effect on published release artifacts.
NO_STRIP ?= true

.PHONY: help require-container dev build play-store-screenshots pr-gate-fe-unit pr-gate-be-cache-key pr-gate-be-compile pr-gate-be-unit pr-gate-e2e pr-gate-e2e-cache-key pr-gate-e2e-build-dev-runner pr-gate-e2e-run pr-gate-e2e-artifacts pr-gate-e2e-cleanup e2e e2e-ci e2e-image e2e-build e2e-headed e2e-phone e2e-devices desktop-debug desktop-debug-build runtime-image runtime-smoke pre-release-check release android-connect android-dev android-build android-build-emulator-e2e android-sign-debug android-sign-release-local android-launch android-launch-debug android-devices android-e2e-assert android-release-deploy-debugsigned android-debug-deploy android-release-deploy-local android-build-image android-require-build-image flatpak-install-local

help:
	@echo ""
	@echo "Linux"
	@echo "  make dev              Hot-reload dev app (Vite HMR + Rust watch)"
	@echo "  make build            Release build"
	@echo "  make pr-gate-fe-unit  Run frontend unit tests in Dockerfile stage"
	@echo "  make pr-gate-be-compile  Compile backend tests in Dockerfile stage"
	@echo "  make pr-gate-be-unit  Run backend unit tests in Dockerfile stage"
	@echo "  make pr-gate-e2e      Run npm run test:e2e:ci with Dockerfile stages"
	@echo "  make pr-gate-e2e-cache-key  Print the E2E image input cache key"
	@echo "  make pr-gate-e2e-*    Run one named CI E2E phase for easier failure diagnosis"
	@echo "  make e2e              Run visible local E2E (UI + multi-actor) alongside make dev"
	@echo "  make e2e-ci           Run containerized headless E2E in CI mode"
	@echo "  make e2e-image        Build/update the dev-runner E2E image"
	@echo "  make e2e-build        Build dev-runner image and run E2E inside it"
	@echo "  make e2e-headed       Run visible local E2E (UI + multi-actor) alongside make dev"
	@echo "  make pr-gate-e2e      Run containerized full E2E suite in dev-runner"
	@echo "  FINI_E2E_REBUILD=1 npm run test:e2e  Force E2E image rebuild before running"
	@echo "  make runtime-image    Build/update the runtime container image"
	@echo "  make runtime-smoke    Run a runtime container smoke check"
	@echo "  make play-store-screenshots  Validate Play Store screenshots and write manifest"
	@echo "  make flatpak-install-local  Build release binary and reinstall local Flatpak"
	@echo "  make pre-release-check  Run pre-release checks locally with trace log"
	@echo "  make release VERSION=x.y.z  Run pre-release check, bump versions, push main, and push signed tag vX.Y.Z"
	@echo ""
	@echo "Android"
	@echo "  make android-connect  Auto-discover and connect to device via adb mdns"
	@echo "  make android-dev      Hot-reload on device via Wi-Fi (auto-connects)"
	@echo "  make android-build    Build Android APK"
	@echo "  make android-build-emulator-e2e  Build x86_64 debug APK for emulator E2E gate"
	@echo "  make android-sign-debug   Sign APK to bin/fini.apk using debug keystore"
	@echo "  make android-sign-release-local  Sign APK with local release keystore env vars"
	@echo "  make android-install-debug  Install bin/fini.apk on connected device"
	@echo "  make android-install-release-local  Install bin/fini-release.apk on device"
	@echo "  make android-launch   Launch app on connected device"
	@echo "  make android-release-deploy-debugsigned  Build release, debug-keystore sign, install, and launch (shares com.fini.app id)"
	@echo "  make android-debug-deploy  Build the debug variant (separate com.fini.app.debug id) with Tauri logging enabled"
	@echo "  make android-release-deploy-local  Local release-signed deploy preserving app identity"
	@echo "  make android-devices  List connected ADB devices"
	@echo ""

# ── Linux ─────────────────────────────────────────────────────────────────────

require-container:
	@test "$(CONTAINER_ENGINE)" != "missing" || (echo "No container engine found. Install Docker or Podman, or set CONTAINER=docker|podman." && exit 1)

dev:
	@set -eu; \
	mkdir -p "$(FINI_SCRATCH_DIR)"; \
	capability_backup="$$(mktemp "$(FINI_SCRATCH_DIR)/fini-default-capability.XXXXXX")"; \
	cp src-tauri/capabilities/default.json "$$capability_backup"; \
	restore_capability() { cp "$$capability_backup" src-tauri/capabilities/default.json; rm -f "$$capability_backup"; }; \
	trap restore_capability EXIT INT TERM; \
	cp src-tauri/devtools-capabilities/default.json src-tauri/capabilities/default.json; \
	npm run tauri dev -- --features ui-plane,desktop-updater,devtools

build:
	npm run tauri build -- --features ui-plane,desktop-updater

flatpak-install-local:
	-$(MAKE) build
	flatpak run org.flatpak.Builder --force-clean --user --install flatpak-build com.fini.app.yml

play-store-screenshots:
	cargo run --manifest-path xtask/Cargo.toml -- play-store-screenshots

pr-gate-fe-unit: require-container
	$(CONTAINER_ENGINE) build --target fe-unit-test -t fini-fe-unit-test .

pr-gate-be-cache-key:
	@set -eu; \
	cache_inputs() { \
	  { \
	    git ls-files -z -- \
	      Dockerfile \
	      src-tauri/Cargo.toml \
	      src-tauri/Cargo.lock \
	      src-tauri/build.rs \
	      src-tauri/src \
	      src-tauri/migrations \
	      src-tauri/patches \
	      src-tauri/capabilities \
	      src-tauri/icons \
	      src-tauri/tauri.conf.json; \
	    git ls-files --others --exclude-standard -z -- \
	      Dockerfile \
	      src-tauri/Cargo.toml \
	      src-tauri/Cargo.lock \
	      src-tauri/build.rs \
	      src-tauri/src \
	      src-tauri/migrations \
	      src-tauri/patches \
	      src-tauri/capabilities \
	      src-tauri/icons \
	      src-tauri/tauri.conf.json; \
	  } | sort -zu; \
	}; \
	cache_inputs | xargs -0 sha256sum 2>/dev/null | sha256sum | cut -d ' ' -f 1

pr-gate-be-compile:
	@set -eu; \
	cache_key="$$(make --no-print-directory pr-gate-be-cache-key)"; \
	cache_prefix="$(FINI_BE_CACHE_IMAGE_PREFIX)"; \
	cache_image=""; \
	if [ -n "$$cache_prefix" ]; then cache_image="$$cache_prefix-be-compile-cache:$$cache_key"; fi; \
	if [ -n "$$cache_image" ] && $(CONTAINER_ENGINE) pull "$$cache_image"; then \
	  printf 'Using cached backend compile image: %s\n' "$$cache_image"; \
	  $(CONTAINER_ENGINE) tag "$$cache_image" "$(FINI_BE_COMPILE_IMAGE)"; \
	  exit 0; \
	fi; \
	printf 'Building backend compile image for cache key: %s\n' "$$cache_key"; \
	if [ -n "$$cache_image" ]; then \
	  $(CONTAINER_ENGINE) build --target be-test-compile --build-arg BUILDKIT_INLINE_CACHE=1 --label "fini.be.cache-key=$$cache_key" -t "$(FINI_BE_COMPILE_IMAGE)" -t "$$cache_image" .; \
	else \
	  $(CONTAINER_ENGINE) build --target be-test-compile --build-arg BUILDKIT_INLINE_CACHE=1 --label "fini.be.cache-key=$$cache_key" -t "$(FINI_BE_COMPILE_IMAGE)" .; \
	fi; \
	if [ -n "$$cache_image" ] && [ "$(FINI_BE_CACHE_PUSH)" = "1" ]; then \
	  $(CONTAINER_ENGINE) push "$$cache_image"; \
	fi

pr-gate-be-unit:
	$(MAKE) pr-gate-be-compile
	$(CONTAINER_ENGINE) build --target be-unit-test --cache-from "$(FINI_BE_COMPILE_IMAGE)" -t "$(FINI_BE_UNIT_IMAGE)" .

pr-gate-e2e:
	@set -eu; \
	cleanup() { $(MAKE) pr-gate-e2e-cleanup >/dev/null 2>&1 || true; }; \
	trap cleanup EXIT INT TERM; \
	$(MAKE) pr-gate-e2e-build-dev-runner; \
	$(MAKE) pr-gate-e2e-run

pr-gate-e2e-cache-key:
	@set -eu; \
	cache_inputs() { \
	  { \
	    git ls-files -z -- \
	      Dockerfile \
	      package.json \
	      package-lock.json \
	      tsconfig\*.json \
	      index.html \
	      scripts/e2e-runner.sh \
	      vite.config.ts \
	      src \
	      src-tauri/Cargo.toml \
	      src-tauri/Cargo.lock \
	      src-tauri/build.rs \
	      src-tauri/src \
	      src-tauri/migrations \
	      src-tauri/patches \
	      src-tauri/capabilities \
	      src-tauri/devtools-capabilities \
	      src-tauri/icons \
	      src-tauri/tauri.conf.json \
	      specs/e2e; \
	    git ls-files --others --exclude-standard -z -- \
	      Dockerfile \
	      package.json \
	      package-lock.json \
	      tsconfig\*.json \
	      index.html \
	      scripts/e2e-runner.sh \
	      vite.config.ts \
	      src \
	      src-tauri/Cargo.toml \
	      src-tauri/Cargo.lock \
	      src-tauri/build.rs \
	      src-tauri/src \
	      src-tauri/migrations \
	      src-tauri/patches \
	      src-tauri/capabilities \
	      src-tauri/devtools-capabilities \
	      src-tauri/icons \
	      src-tauri/tauri.conf.json \
	      specs/e2e; \
	  } | sort -zu; \
	}; \
	cache_inputs | xargs -0 sha256sum 2>/dev/null | sha256sum | cut -d ' ' -f 1

pr-gate-e2e-build-dev-runner:
	@set -eu; \
	cache_key="$$(make --no-print-directory pr-gate-e2e-cache-key)"; \
	cache_prefix="$(FINI_E2E_CACHE_IMAGE_PREFIX)"; \
	cache_image=""; \
	if [ -n "$$cache_prefix" ]; then cache_image="$$cache_prefix-dev-runner-cache:$$cache_key"; fi; \
	if [ -n "$$cache_image" ] && $(CONTAINER_ENGINE) pull "$$cache_image"; then \
	  printf 'Using cached dev-runner image: %s\n' "$$cache_image"; \
	  $(CONTAINER_ENGINE) tag "$$cache_image" "$(FINI_DEV_RUNNER_IMAGE)"; \
	  exit 0; \
	fi; \
	printf 'Building dev-runner image for E2E cache key: %s\n' "$$cache_key"; \
	if [ -n "$$cache_image" ]; then \
	  $(CONTAINER_ENGINE) build --target dev-runner --label "fini.e2e.cache-key=$$cache_key" -t "$(FINI_DEV_RUNNER_IMAGE)" -t "$$cache_image" .; \
	else \
	  $(CONTAINER_ENGINE) build --target dev-runner --label "fini.e2e.cache-key=$$cache_key" -t "$(FINI_DEV_RUNNER_IMAGE)" .; \
	fi; \
	if [ -n "$$cache_image" ] && [ "$(FINI_E2E_CACHE_PUSH)" = "1" ]; then \
	  $(CONTAINER_ENGINE) push "$$cache_image"; \
	fi

pr-gate-e2e-run:
	@set -eu; \
	mkdir -p "$(FINI_E2E_CI_RESULTS_DIR)"; \
	$(CONTAINER_ENGINE) rm -f "fini-$(FINI_E2E_CI_RUN_ID)-runner" >/dev/null 2>&1 || true; \
	$(CONTAINER_ENGINE) run --rm \
	  --name "fini-$(FINI_E2E_CI_RUN_ID)-runner" \
	  -e FINI_E2E_ACTORS="$(FINI_E2E_CI_ACTORS)" \
	  -e FINI_E2E_CI_RUN_ID="$(FINI_E2E_CI_RUN_ID)" \
	  -e FINI_E2E_RUN_ID="$(FINI_E2E_CI_RUN_ID)" \
	  -e FINI_E2E_CI_ACTOR_WAIT_SECS="$(FINI_E2E_CI_ACTOR_WAIT_SECS)" \
	  -v "$(FINI_E2E_CI_RESULTS_DIR):/app/test-results:Z" \
	  "$(FINI_DEV_RUNNER_IMAGE)"

pr-gate-e2e-artifacts:
	@set -eu; \
	run_root="$(FINI_E2E_CI_RESULTS_DIR)/fini-e2e-runs/$(FINI_E2E_CI_RUN_ID)/actors"; \
	printf '\n===== actor logs =====\n'; \
	if [ -d "$$run_root" ]; then \
	  for log_path in "$$run_root"/*.log; do \
	    [ -f "$$log_path" ] || continue; \
	    printf '\n--- %s ---\n' "$$log_path"; \
	    tail -n 200 "$$log_path" || true; \
	  done; \
	else \
	  printf 'No actor log directory found: %s\n' "$$run_root"; \
	fi

pr-gate-e2e-cleanup:
	@set -eu; \
	$(CONTAINER_ENGINE) rm -f "fini-$(FINI_E2E_CI_RUN_ID)-runner" >/dev/null 2>&1 || true

# Run the real-app e2e lane locally.
e2e:
	$(MAKE) e2e-headed

# Run the same lane under CI settings.
e2e-ci:
	CI=1 $(MAKE) pr-gate-e2e

# Build/update the container image used for CI-style local e2e runs.
e2e-image:
	$(MAKE) require-container
	$(CONTAINER_ENGINE) build --target dev-runner -t fini-dev-runner .

# Run the headless e2e tier inside the cached Podman image.
e2e-build:
	$(MAKE) require-container
	$(CONTAINER_ENGINE) image inspect fini-dev-runner >/dev/null 2>&1 || $(CONTAINER_ENGINE) build --target dev-runner -t fini-dev-runner .
	$(CONTAINER_ENGINE) run --rm fini-dev-runner

# Run the visible local E2E suite (UI single-actor + multi-actor) against the host desktop display.
# Builds the local binaries, then lets the Playwright fixtures spawn the real app processes.
e2e-headed:
	@set -eu; \
	run_root="$${FINI_E2E_ROOT:-$(FINI_SCRATCH_DIR)/fini-e2e-headed}"; \
	e2e_target_dir="$$(pwd)/src-tauri/target/debug-e2e"; \
	app_bin_path="$$e2e_target_dir/debug/fini-app"; \
	cli_bin_path="$$e2e_target_dir/debug/fini"; \
	ble_broker_bin_path="$$e2e_target_dir/debug/ble-mock-broker"; \
	mkdir -p "$(FINI_SCRATCH_DIR)"; \
	capability_backup="$$(mktemp "$(FINI_SCRATCH_DIR)/fini-default-capability.XXXXXX")"; \
	cp src-tauri/capabilities/default.json "$$capability_backup"; \
	restore_capability() { cp "$$capability_backup" src-tauri/capabilities/default.json; rm -f "$$capability_backup"; }; \
	trap restore_capability EXIT INT TERM; \
	mkdir -p "$$run_root"; \
	cp src-tauri/devtools-capabilities/default.json src-tauri/capabilities/default.json; \
	CARGO_TARGET_DIR="$$e2e_target_dir" npm run tauri -- build --debug --features ui-plane,desktop-updater,devtools --no-bundle -- --bin fini-app; \
	restore_capability; \
	trap - EXIT INT TERM; \
	CARGO_TARGET_DIR="$$e2e_target_dir" cargo build --manifest-path src-tauri/Cargo.toml --bin fini --features cli-plane; \
	CARGO_TARGET_DIR="$$e2e_target_dir" cargo build --manifest-path ble-mock-broker/Cargo.toml; \
	FINI_E2E_ROOT="$$run_root" FINI_E2E_HEADFUL=1 FINI_APP_BINARY="$$app_bin_path" FINI_CLI_BINARY="$$cli_bin_path" TZ=UTC npx playwright test --config specs/e2e/playwright.config.ts --project ui --project actors; \
	FINI_E2E_ROOT="$$run_root" FINI_E2E_HEADFUL=1 FINI_E2E_TRANSPORT=sim FINI_APP_BINARY="$$app_bin_path" FINI_CLI_BINARY="$$cli_bin_path" TZ=UTC npx playwright test --config specs/e2e/playwright.config.ts --project actors-sim; \
	FINI_E2E_ROOT="$$run_root" FINI_E2E_HEADFUL=1 FINI_E2E_TRANSPORT=ble FINI_APP_BINARY="$$app_bin_path" FINI_CLI_BINARY="$$cli_bin_path" FINI_BLE_MOCK_BROKER_BINARY="$$ble_broker_bin_path" TZ=UTC npx playwright test --config specs/e2e/playwright.config.ts --project actors-ble

# "Fini Debug": a genuinely separate desktop application that lives beside the
# production "Fini" install instead of replacing it -- the desktop counterpart
# of com.fini.app.debug on the phone.
#
# Separated the same way Tauri separates any two apps: a distinct `identifier`
# and `productName`, overridden at build time via `--config`. That is what
# makes the split real rather than cosmetic -- the identifier is what Tauri
# derives the app data directory from, so "Fini Debug" gets its own database,
# device identity and pairings automatically, with no environment tricks and
# no way to accidentally write into production data.
#
# Unlike Android (where `--config identifier` is rejected because the
# generated Gradle project's package directories are tied to it), desktop has
# no generated project to keep in sync, so the override is clean here.
#
# Built with devtools and its frontend baked in (`--no-bundle` still produces
# a runnable binary), so it starts standalone with a working webview and a
# Playwright control channel -- a plain `cargo build` binary would expect a
# Vite dev server and come up blank.
DESKTOP_DEBUG_IDENTIFIER ?= fini-debug
DESKTOP_DEBUG_NAME ?= Fini Debug
DESKTOP_DEBUG_PORT ?= 9224
DESKTOP_DEBUG_DISCOVERY_PORT ?= 45464
DESKTOP_DEBUG_WS_PORT ?= 45465
DESKTOP_DEBUG_TARGET_DIR = $(CURDIR)/src-tauri/target/debug-app
DESKTOP_DEBUG_BIN = $(DESKTOP_DEBUG_TARGET_DIR)/debug/fini-app
DESKTOP_DEBUG_CONFIG = {"productName":"$(DESKTOP_DEBUG_NAME)","identifier":"$(DESKTOP_DEBUG_IDENTIFIER)"}

desktop-debug-build:
	@set -eu; \
	mkdir -p "$(FINI_SCRATCH_DIR)"; \
	capability_backup="$$(mktemp "$(FINI_SCRATCH_DIR)/fini-default-capability.XXXXXX")"; \
	cp src-tauri/capabilities/default.json "$$capability_backup"; \
	restore_capability() { cp "$$capability_backup" src-tauri/capabilities/default.json; rm -f "$$capability_backup"; }; \
	trap restore_capability EXIT INT TERM; \
	cp src-tauri/devtools-capabilities/default.json src-tauri/capabilities/default.json; \
	CARGO_TARGET_DIR="$(DESKTOP_DEBUG_TARGET_DIR)" npm run tauri -- build --debug --no-bundle \
		--features ui-plane,devtools --config '$(DESKTOP_DEBUG_CONFIG)' -- --bin fini-app; \
	printf 'Built "%s" (identifier %s) at %s\n' "$(DESKTOP_DEBUG_NAME)" "$(DESKTOP_DEBUG_IDENTIFIER)" "$(DESKTOP_DEBUG_BIN)"

# Data dir and ports are set here rather than baked into the build config:
# they are runtime coordinates, not app identity. The separate `identifier`
# alone would already give this app its own Tauri data directory, but
# FINI_APP_DATA_DIR is passed explicitly so the location is stated rather than
# inferred -- this is the database that must never be the production one, so
# it should be obvious and greppable, not a derived side effect. Ports are
# shifted for the same practical reason: both apps have to run at once
# without fighting over :45454/:45455.
DESKTOP_DEBUG_DATA_DIR ?= $(HOME)/.local/share/fini-debug
# Broadcasts to the default discovery port as well as its own, so a device
# that only knows the default -- a phone, which cannot be told otherwise --
# still receives this app's presence. Listening stays on the shifted port so
# the production app keeps :45454 to itself.
DESKTOP_DEBUG_PEER_PORTS ?= 45454,$(DESKTOP_DEBUG_DISCOVERY_PORT)
desktop-debug:
	@set -eu; \
	if ss -lnt "sport = :$(DESKTOP_DEBUG_PORT)" 2>/dev/null | grep -q LISTEN; then \
		printf 'refusing to start: %s is already running (devtools port %s is bound).\n' "$(DESKTOP_DEBUG_NAME)" "$(DESKTOP_DEBUG_PORT)" >&2; \
		printf 'two debug apps would both dial the phone over BLE and make results ambiguous.\n' >&2; \
		printf 'close the running one first, or override DESKTOP_DEBUG_PORT.\n' >&2; \
		exit 1; \
	fi; \
	test -x "$(DESKTOP_DEBUG_BIN)" || $(MAKE) desktop-debug-build; \
	mkdir -p "$(DESKTOP_DEBUG_DATA_DIR)"; \
	printf '%s: data=%s devtools=tcp:%s discovery=%s ws=%s\n' "$(DESKTOP_DEBUG_NAME)" "$(DESKTOP_DEBUG_DATA_DIR)" "$(DESKTOP_DEBUG_PORT)" "$(DESKTOP_DEBUG_DISCOVERY_PORT)" "$(DESKTOP_DEBUG_WS_PORT)"; \
	FINI_APP_DATA_DIR="$(DESKTOP_DEBUG_DATA_DIR)" \
	FINI_DEVTOOLS_TCP_PORT="$(DESKTOP_DEBUG_PORT)" \
	FINI_DISCOVERY_PORT="$(DESKTOP_DEBUG_DISCOVERY_PORT)" \
	FINI_DISCOVERY_PEER_PORTS="$(DESKTOP_DEBUG_PEER_PORTS)" \
	FINI_SPACE_SYNC_WS_PORT="$(DESKTOP_DEBUG_WS_PORT)" \
	WEBKIT_DISABLE_DMABUF_RENDERER=1 \
	"$(DESKTOP_DEBUG_BIN)"

# Run the actor suite with a real Android device joined as an actor, against
# whatever `adb` is pointed at. The phone runs the debug build (see
# android-debug-deploy) and is *borrowed*, not managed: this never installs,
# configures or stops it -- only drives it, so its real pairing and radio
# state are what gets exercised. That is the whole point; no emulator
# reproduces real BLE behaviour.
#
#   make e2e-phone                      # default: actor-a + phone
#   make e2e-phone E2E_PHONE_SPEC="ble" # -g filter passed to playwright
#
# Everything the run needs is set here rather than left to the caller: the
# E2E binary is built the same way e2e-headed builds it (frontend baked in --
# a plain `cargo build` binary expects a Vite dev server and would come up
# blank), devtools capabilities are swapped in and restored, and the adb port
# forward for the phone's control channel is established.
# Both actors are apps this target does not own: the desktop "Fini Debug"
# (make desktop-debug) and the phone's com.fini.app.debug. Nothing is spawned
# or torn down -- their real pairing, storage and radio state is the thing
# under test, which is exactly what a spawned throwaway actor cannot give.
#
# Deliberately not headful: an external actor's window belongs to whoever
# started it, and this harness drives it through the plugin channel rather
# than the screen, so there is nothing to show. Forcing FINI_E2E_HEADFUL here
# additionally proved able to lock up the desktop session on this machine's
# GPU path, which is a bad trade for output nobody reads.
#
#   make e2e-devices                          # whole actors suite
#   make e2e-devices E2E_DEVICES_SPEC="sync"  # -g filter
#
# Requires both debug apps already running (see desktop-debug and
# android-debug-deploy).
E2E_DEVICES_SPEC ?=
e2e-devices:
	@set -eu; \
	adb get-state >/dev/null 2>&1 || (echo "No adb device. Connect the phone and run 'make android-devices'." && exit 1); \
	adb forward tcp:$(E2E_PHONE_PORT) tcp:$(E2E_PHONE_PORT) >/dev/null; \
	run_root="$${FINI_E2E_ROOT:-$(FINI_SCRATCH_DIR)/fini-e2e-devices}"; \
	mkdir -p "$$run_root"; \
	FINI_E2E_ROOT="$$run_root" \
	FINI_E2E_ACTORS="desktop,$(E2E_PHONE_ACTOR)" \
	FINI_E2E_EXTERNAL_ACTORS="desktop=$(DESKTOP_DEBUG_PORT),$(E2E_PHONE_ACTOR)=$(E2E_PHONE_PORT)" \
	TZ=UTC \
	npx playwright test --config specs/e2e/playwright.config.ts --project actors $(if $(E2E_DEVICES_SPEC),-g "$(E2E_DEVICES_SPEC)",)

E2E_PHONE_ACTOR ?= phone
E2E_PHONE_PORT ?= 9223
E2E_PHONE_SPEC ?= external actor
e2e-phone:
	@set -eu; \
	adb get-state >/dev/null 2>&1 || (echo "No adb device. Connect the phone and run 'make android-devices'." && exit 1); \
	run_root="$${FINI_E2E_ROOT:-$(FINI_SCRATCH_DIR)/fini-e2e-phone}"; \
	e2e_target_dir="$$(pwd)/src-tauri/target/debug-e2e"; \
	app_bin_path="$$e2e_target_dir/debug/fini-app"; \
	mkdir -p "$(FINI_SCRATCH_DIR)" "$$run_root"; \
	capability_backup="$$(mktemp "$(FINI_SCRATCH_DIR)/fini-default-capability.XXXXXX")"; \
	cp src-tauri/capabilities/default.json "$$capability_backup"; \
	restore_capability() { cp "$$capability_backup" src-tauri/capabilities/default.json; rm -f "$$capability_backup"; }; \
	trap restore_capability EXIT INT TERM; \
	cp src-tauri/devtools-capabilities/default.json src-tauri/capabilities/default.json; \
	CARGO_TARGET_DIR="$$e2e_target_dir" npm run tauri -- build --debug --features ui-plane,desktop-updater,devtools --no-bundle -- --bin fini-app; \
	restore_capability; \
	trap - EXIT INT TERM; \
	adb forward tcp:$(E2E_PHONE_PORT) tcp:$(E2E_PHONE_PORT) >/dev/null; \
	FINI_E2E_ROOT="$$run_root" FINI_E2E_HEADFUL=1 \
	FINI_E2E_ACTORS="actor-a,$(E2E_PHONE_ACTOR)" \
	FINI_E2E_EXTERNAL_ACTORS="$(E2E_PHONE_ACTOR)=$(E2E_PHONE_PORT)" \
	FINI_APP_BINARY="$$app_bin_path" TZ=UTC \
	npx playwright test --config specs/e2e/playwright.config.ts --project actors -g "$(E2E_PHONE_SPEC)"

# Build/update the published headless runtime image locally.
runtime-image:
	$(MAKE) require-container
	$(CONTAINER_ENGINE) build --target runtime -t fini-runtime .

# Verify the runtime container executes the CLI surface.
runtime-smoke:
	$(MAKE) require-container
	$(CONTAINER_ENGINE) image inspect fini-runtime >/dev/null 2>&1 || $(CONTAINER_ENGINE) build --target runtime -t fini-runtime .
	$(CONTAINER_ENGINE) run --rm fini-runtime --help

# Real-Bluetooth transport verification: local/manual only, never CI (no
# radio on GitHub-hosted runners). Lands with the real Bluetooth adapter —
# see docs/adr/0001-transport-neutral-peer-protocol.md and
# specs/e2e/transports.md.
e2e-bt-local:
	@echo "e2e-bt-local: real Bluetooth adapter not implemented yet (follow-up PR)." >&2; \
	echo "This target will pair over real Bluetooth with LAN disabled once it lands." >&2; \
	exit 1

pre-release-check:
	@set -eu; \
	log_dir="$${PRE_RELEASE_LOG_DIR:-$(FINI_SCRATCH_DIR)/fini-pre-release}"; \
	mkdir -p "$$log_dir"; \
	log_file="$$log_dir/pre-release-check-$$(date -u +%Y%m%dT%H%M%SZ).log"; \
	printf 'Writing pre-release log: %s\n' "$$log_file"; \
	bash -o pipefail -c 'set -eu; \
	  cleanup() { $(MAKE) pr-gate-e2e-cleanup >/dev/null 2>&1 || true; }; \
	  step() { printf "\n[%s] pre-release-check: %s\n" "$$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$$1"; }; \
	  trap cleanup EXIT INT TERM; \
	  step "require container engine"; \
	  $(MAKE) require-container; \
	  step "backend unit tests"; \
	  $(MAKE) pr-gate-be-unit; \
	  step "runtime image"; \
	  $(MAKE) runtime-image; \
	  step "runtime smoke"; \
	  $(MAKE) runtime-smoke; \
	  step "E2E dev-runner image"; \
	  $(MAKE) pr-gate-e2e-build-dev-runner; \
	  step "E2E run"; \
	  $(MAKE) pr-gate-e2e-run' 2>&1 | tee "$$log_file"

release:
	@test -n "$(VERSION)" || (echo "VERSION is required. Use: make release VERSION=x.y.z" && exit 1)
	@printf '%s\n' "$(VERSION)" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$$' || (echo "VERSION must match x.y.z" && exit 1)
	@set -eu; \
	branch="$$(git branch --show-current)"; \
	if [ "$$branch" != "main" ]; then \
	  echo "Release must run from main"; \
	  echo "current branch=$$branch"; \
	  exit 1; \
	fi; \
	git diff --quiet || { echo "Working tree has unstaged changes"; exit 1; }; \
	git diff --cached --quiet || { echo "Working tree has staged changes"; exit 1; }; \
	test -z "$$(git ls-files --others --exclude-standard)" || { echo "Working tree has untracked files"; exit 1; }; \
	git fetch origin main --tags --force; \
	main_commit="$$(git rev-parse origin/main)"; \
	current_commit="$$(git rev-parse HEAD)"; \
	if [ "$$current_commit" != "$$main_commit" ]; then \
	  echo "HEAD must match origin/main before release"; \
	  echo "HEAD=$$current_commit"; \
	  echo "origin/main=$$main_commit"; \
	  exit 1; \
	fi; \
	tag="v$(VERSION)"; \
	if git rev-parse -q --verify "refs/tags/$$tag" >/dev/null; then \
	  echo "Tag already exists: $$tag"; \
	  exit 1; \
	fi; \
	$(MAKE) pre-release-check; \
	cargo run --manifest-path xtask/Cargo.toml -- release-version "$(VERSION)"
	git add package.json package-lock.json src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/tauri.conf.json
	git commit -m "chore: release v$(VERSION)"
	git push origin main
	git -c user.email="v.ruzhentsov@gmail.com" -c user.signingkey="199DFE796EA43C00" tag -s -a "v$(VERSION)" -m "v$(VERSION)"
	git tag -v "v$(VERSION)"
	git push origin "v$(VERSION)"
	@echo "Released v$(VERSION); CI release workflow is triggered by the pushed tag."

# ── Android ───────────────────────────────────────────────────────────────────

DEVICE_ADDRESS = $(shell adb mdns services 2>/dev/null | grep '_adb-tls-connect' | head -1 | awk '{print $$NF}')
DEVICE_IP      = $(firstword $(subst :, ,$(DEVICE_ADDRESS)))
HOST_IP        = $(shell ip route get $(DEVICE_IP) 2>/dev/null | grep -oP 'src \K\S+' | head -1)
ANDROID_TARGET ?= aarch64
LATEST_TAG = $(shell git describe --tags --abbrev=0 2>/dev/null || printf 'v0.0.0')
GIT_SHA = $(shell git rev-parse --short HEAD 2>/dev/null || printf 'unknown')
ANDROID_DEBUG_VERSION_NAME = $(patsubst v%,%,$(LATEST_TAG))+dev.$(GIT_SHA)
ANDROID_DEBUG_VERSION_CODE = $(shell date +%s)
ANDROID_UNSIGNED_APK = src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk
ANDROID_SIGNED_APK = bin/fini.apk
ANDROID_RELEASE_SIGNED_APK = bin/fini-release.apk
APKSIGNER = $(lastword $(sort $(wildcard $(ANDROID_HOME)/build-tools/*/apksigner)))
ADB_CONNECT_TIMEOUT ?= 15

# Gradle half of the Android build runs in a JDK 17 container (see
# android-build.Containerfile for why). Only the build is containerised: adb
# stays on the host, where the USB device and adb server are.
#
# The repo, SDK/NDK and the cargo/gradle/android caches are bind-mounted at
# their host paths rather than copied, so cached state is shared with host
# builds and the image holds no project state. Mounting at the *same* absolute
# path matters: cargo and gradle both record absolute paths in their caches, so
# relocating the tree would invalidate them on every switch. `--userns=keep-id`
# keeps written files owned by the invoking user, and mounting ~/.android keeps
# one stable debug keystore -- without it the container generates a fresh one
# per run and every install fails on a signature mismatch.
#
# GRADLE_OPTS pins `user.home` because mounting ~/.android is not on its own
# enough: the Android Gradle Plugin locates the default debug keystore through
# the JVM's `user.home` system property, which the JVM derives from /etc/passwd
# for the running uid -- not from $HOME. Under --userns=keep-id that uid has no
# passwd entry, so `user.home` resolved somewhere else entirely and Gradle
# generated a brand-new debug keystore on every run, next to a perfectly good
# mounted one it never looked at. Symptom: INSTALL_FAILED_UPDATE_INCOMPATIBLE
# on the second and every subsequent deploy.
#
# JAVA_TOOL_OPTIONS rather than GRADLE_OPTS, and --passwd-entry alongside it:
# the build runs in a long-lived Gradle *daemon* JVM, and GRADLE_OPTS reaches
# only the client, so the daemon kept resolving user.home its own way and kept
# minting a fresh keystore inside the container. JAVA_TOOL_OPTIONS is read by
# every JVM that starts in the container, daemon included; the passwd entry
# fixes the same thing at the OS level for anything that consults getpwuid.
#
# To check this is still working, compare the signer of a freshly built APK
# against the keystore it is supposed to come from -- they must match:
#   keytool -list -v -keystore ~/.android/debug.keystore -storepass android
#   $ANDROID_HOME/build-tools/*/apksigner verify --print-certs <apk>
# Resolved rather than used as-is: on this class of host (Fedora Silverblue and
# friends) $(HOME) is /home/<user>, a symlink to the real /var/home/<user>.
# Cargo canonicalises paths it reads back, so a cache mounted only at the
# symlinked path is looked up at the real one and appears missing inside the
# container ("failed to read plugin permissions ... No such file or directory").
# Mounting at the resolved path makes both spellings agree with $(CURDIR),
# which is already resolved. On hosts without that symlink this is a no-op.
ANDROID_BUILD_HOME := $(shell readlink -f "$(HOME)")
ANDROID_BUILD_IMAGE ?= fini-android-build
ANDROID_BUILD_RUN = podman run --rm -t \
	--userns=keep-id \
	--passwd-entry "builder:x:$(shell id -u):$(shell id -g):builder:$(ANDROID_BUILD_HOME):/bin/bash" \
	-e HOME="$(ANDROID_BUILD_HOME)" \
	-e JAVA_TOOL_OPTIONS="-Duser.home=$(ANDROID_BUILD_HOME)" \
	-e PATH="$(ANDROID_BUILD_HOME)/.cargo/bin:/opt/java/openjdk/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
	-e ANDROID_HOME="$(shell readlink -f "$(ANDROID_HOME)")" \
	-e NDK_HOME="$(shell readlink -f "$(NDK_HOME)")" \
	-e FINI_ANDROID_VERSION_NAME -e FINI_ANDROID_VERSION_CODE \
	-v "$(CURDIR)":"$(CURDIR)":Z \
	-v "$(ANDROID_BUILD_HOME)/Android":"$(ANDROID_BUILD_HOME)/Android" \
	-v "$(ANDROID_BUILD_HOME)/.cargo":"$(ANDROID_BUILD_HOME)/.cargo" \
	-v "$(ANDROID_BUILD_HOME)/.rustup":"$(ANDROID_BUILD_HOME)/.rustup" \
	-v "$(ANDROID_BUILD_HOME)/.gradle":"$(ANDROID_BUILD_HOME)/.gradle" \
	-v "$(ANDROID_BUILD_HOME)/.android":"$(ANDROID_BUILD_HOME)/.android" \
	-w "$(CURDIR)" \
	"$(ANDROID_BUILD_IMAGE)"

android-build-image:
	podman build -f Dockerfile --target android-build -t "$(ANDROID_BUILD_IMAGE)" .

# Fails loudly rather than silently falling back to the host JDK, which would
# reintroduce exactly the Gradle failure this indirection exists to avoid.
android-require-build-image:
	@podman image exists "$(ANDROID_BUILD_IMAGE)" \
		|| (echo "Missing image $(ANDROID_BUILD_IMAGE). Run 'make android-build-image' first." && exit 1)

android-connect:
	@test -n "$(DEVICE_ADDRESS)" || (echo "No device found via adb mdns. Enable wireless debugging on the phone." && exit 1)
	@timeout "$(ADB_CONNECT_TIMEOUT)s" adb connect $(DEVICE_ADDRESS) || (echo "ADB connect timed out for $(DEVICE_ADDRESS). Re-authorize wireless debugging, reconnect USB, or start an emulator." && exit 1)

android-dev: android-connect
	npm run tauri android dev -- --features ui-plane --host $(HOST_IP)

android-build: android-require-build-image
	$(ANDROID_BUILD_RUN) npm run tauri android build -- --features ui-plane --target "$(ANDROID_TARGET)"

# Deliberately NOT containerised: this is the CI lane (ci.yml), where
# setup-java already provides the right JDK and podman is the wrong tool.
android-build-emulator-e2e:
	npm run tauri android build -- --features ui-plane --ci --debug --apk --target x86_64

# Runs the same smoke assertions CI runs on the emulator (app process starts,
# `fini.reminders` notification channel registers), but against whatever
# device `adb` is currently pointed at and whatever APK is passed in --
# so the debug build on a real phone can be checked with the same script CI
# uses, rather than a separate ad-hoc procedure.
#
#   make android-e2e-assert ANDROID_E2E_APK=<path> [FINI_E2E_PACKAGE=<id>]
#
# FINI_E2E_PACKAGE defaults to the release application id; a debug-buildType
# APK installs under the `.debug` suffix (tauri.conf.json's
# bundle.android.debugApplicationIdSuffix) and must set it explicitly.
android-e2e-assert:
	@test -n "$(ANDROID_E2E_APK)" || (echo "ANDROID_E2E_APK is not set" && exit 1)
	ANDROID_E2E_APK="$(ANDROID_E2E_APK)" FINI_E2E_PACKAGE="$(FINI_E2E_PACKAGE)" bash scripts/android-e2e-assert.sh

android-sign-debug:
	@test -n "$(ANDROID_HOME)" || (echo "ANDROID_HOME is not set" && exit 1)
	@test -n "$(APKSIGNER)" || (echo "apksigner not found under $$ANDROID_HOME/build-tools" && exit 1)
	@test -f "$(ANDROID_UNSIGNED_APK)" || (echo "Unsigned APK not found: $(ANDROID_UNSIGNED_APK)" && exit 1)
	mkdir -p bin
	"$(APKSIGNER)" sign --ks "$$HOME/.android/debug.keystore" --ks-key-alias androiddebugkey --ks-pass pass:android --key-pass pass:android --out "$(ANDROID_SIGNED_APK)" "$(ANDROID_UNSIGNED_APK)"
	"$(APKSIGNER)" verify "$(ANDROID_SIGNED_APK)"

android-sign-release-local:
	@test -n "$(ANDROID_HOME)" || (echo "ANDROID_HOME is not set" && exit 1)
	@test -n "$(APKSIGNER)" || (echo "apksigner not found under $$ANDROID_HOME/build-tools" && exit 1)
	@test -f "$(ANDROID_UNSIGNED_APK)" || (echo "Unsigned APK not found: $(ANDROID_UNSIGNED_APK)" && exit 1)
	@test -n "$$ANDROID_KEYSTORE_PASSWORD" || (echo "ANDROID_KEYSTORE_PASSWORD is not set" && exit 1)
	@test -n "$$ANDROID_KEY_ALIAS" || (echo "ANDROID_KEY_ALIAS is not set" && exit 1)
	@test -n "$$ANDROID_KEY_PASSWORD" || (echo "ANDROID_KEY_PASSWORD is not set" && exit 1)
	@keystore_path="$$ANDROID_KEYSTORE_PATH"; \
	if [ -z "$$keystore_path" ]; then \
	  if [ -z "$$ANDROID_KEYSTORE_BASE64" ]; then \
	    echo "Set ANDROID_KEYSTORE_PATH or ANDROID_KEYSTORE_BASE64 for local release signing"; \
	    exit 1; \
	  fi; \
	  mkdir -p "$(FINI_SCRATCH_DIR)"; \
	  keystore_path="$(FINI_SCRATCH_DIR)/fini-release.keystore"; \
	  printf '%s' "$$ANDROID_KEYSTORE_BASE64" | base64 --decode > "$$keystore_path"; \
	fi; \
	test -f "$$keystore_path" || (echo "Release keystore not found: $$keystore_path" && exit 1); \
	mkdir -p bin; \
	"$(APKSIGNER)" sign --ks "$$keystore_path" --ks-key-alias "$$ANDROID_KEY_ALIAS" --ks-pass "pass:$$ANDROID_KEYSTORE_PASSWORD" --key-pass "pass:$$ANDROID_KEY_PASSWORD" --out "$(ANDROID_RELEASE_SIGNED_APK)" "$(ANDROID_UNSIGNED_APK)"; \
	"$(APKSIGNER)" verify "$(ANDROID_RELEASE_SIGNED_APK)"

android-install-debug:
	@test -f "$(ANDROID_SIGNED_APK)" || (echo "Signed APK not found: $(ANDROID_SIGNED_APK). Run make android-sign-debug first." && exit 1)
	adb install -r "$(ANDROID_SIGNED_APK)"

android-install-release-local:
	@test -f "$(ANDROID_RELEASE_SIGNED_APK)" || (echo "Signed APK not found: $(ANDROID_RELEASE_SIGNED_APK). Run make android-sign-release-local first." && exit 1)
	adb install -r "$(ANDROID_RELEASE_SIGNED_APK)"

android-launch:
	adb shell am start -n com.fini.app/.MainActivity

# com.fini.app.debug, not com.fini.app -- android-debug-deploy (below) builds
# a `debug` buildType APK, which build.gradle.kts's applicationIdSuffix
# installs as its own package, separate from the Play Store release.
# android-launch's component name is only ever correct for a
# release-flavored install (android-release-deploy-debugsigned's and
# android-release-deploy-local's own use of it).
android-launch-debug:
	# Activity class spelled out rather than the `.MainActivity` shorthand:
	# that shorthand expands against the *application id*, which the debug
	# build suffixes to com.fini.app.debug, while the activity itself stays in
	# the unsuffixed com.fini.app namespace (applicationIdSuffix changes package
	# identity, not the Kotlin package). The shorthand therefore asks for a
	# com.fini.app.debug.MainActivity that has never existed.
	adb shell am start -n com.fini.app.debug/com.fini.app.MainActivity

# Builds the `release` buildType (no --debug flag), just locally signed with
# the debug keystore for fast iteration -- so, unlike android-debug-deploy
# below, this produces a plain com.fini.app APK with NO applicationIdSuffix.
# Installing it over a Play-Store-signed com.fini.app still hits the same
# certificate conflict described on android-debug-deploy's own comment.
# Prefer that target instead when a Play Store install already exists on
# the device.
android-release-deploy-debugsigned:
	@printf 'Android debug-signed release version: %s (%s)\n' "$(ANDROID_DEBUG_VERSION_NAME)" "$(ANDROID_DEBUG_VERSION_CODE)"
	FINI_ANDROID_VERSION_NAME="$(ANDROID_DEBUG_VERSION_NAME)" FINI_ANDROID_VERSION_CODE="$(ANDROID_DEBUG_VERSION_CODE)" npm run tauri android build -- --features ui-plane --target "$(ANDROID_TARGET)"
	$(MAKE) android-sign-debug
	$(MAKE) android-install-debug
	$(MAKE) android-launch

# The true `debug` buildType (--debug flag) -- gets build.gradle.kts's
# com.fini.app.debug applicationIdSuffix, so it always coexists safely
# alongside a Play Store com.fini.app install. Also enables Tauri's own
# Kotlin logging (BuildConfig.DEBUG=true, suppressed in the release
# profile) -- the target to reach for when diagnosing a silently-failing
# plugin command, not just for the separate-package safety.
#
# Built with `devtools` (not just ui-plane): that feature is what compiles in
# tauri-plugin-playwright, the control socket every automation surface needs
# (the actors harness's TAURI_PLAYWRIGHT_SOCKET, and the tauri-mcp
# driver_session/webview_*/ipc_* tools). Without it the debug build is
# observable only through logcat -- which is exactly the gap that makes a
# debug build worth installing in the first place. Safe here precisely
# because this variant ships under its own `.debug` application id and never
# replaces the Play Store install.
android-debug-deploy: android-require-build-image
	@set -eu; \
	printf 'Android debug (debug profile) version: %s (%s)\n' "$(ANDROID_DEBUG_VERSION_NAME)" "$(ANDROID_DEBUG_VERSION_CODE)"; \
	mkdir -p "$(FINI_SCRATCH_DIR)"; \
	capability_backup="$$(mktemp "$(FINI_SCRATCH_DIR)/fini-default-capability.XXXXXX")"; \
	cp src-tauri/capabilities/default.json "$$capability_backup"; \
	restore_capability() { cp "$$capability_backup" src-tauri/capabilities/default.json; rm -f "$$capability_backup"; }; \
	trap restore_capability EXIT INT TERM; \
	cp src-tauri/devtools-capabilities/default.json src-tauri/capabilities/default.json; \
	FINI_ANDROID_VERSION_NAME="$(ANDROID_DEBUG_VERSION_NAME)" FINI_ANDROID_VERSION_CODE="$(ANDROID_DEBUG_VERSION_CODE)" $(ANDROID_BUILD_RUN) npm run tauri android build -- --features ui-plane,devtools --debug --target "$(ANDROID_TARGET)"
	adb install -r "src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk"
	$(MAKE) android-launch-debug

android-release-deploy-local: android-require-build-image
	@printf 'Android local release version: %s (%s)\n' "$(ANDROID_DEBUG_VERSION_NAME)" "$(ANDROID_DEBUG_VERSION_CODE)"
	FINI_ANDROID_VERSION_NAME="$(ANDROID_DEBUG_VERSION_NAME)" FINI_ANDROID_VERSION_CODE="$(ANDROID_DEBUG_VERSION_CODE)" $(ANDROID_BUILD_RUN) npm run tauri android build -- --features ui-plane --target "$(ANDROID_TARGET)"
	$(MAKE) android-sign-release-local
	$(MAKE) android-install-release-local
	$(MAKE) android-launch

android-devices:
	adb devices
