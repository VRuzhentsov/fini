-include .env
export

CONTAINER ?= auto
CONTAINER_ENGINE = $(shell if [ "$(CONTAINER)" = "auto" ]; then if command -v docker >/dev/null 2>&1; then printf docker; elif command -v podman >/dev/null 2>&1; then printf podman; else printf missing; fi; else printf '%s' "$(CONTAINER)"; fi)
FINI_BE_COMPILE_IMAGE ?= fini-be-compile-ci
FINI_BE_UNIT_IMAGE ?= fini-be-unit-test
FINI_BE_CACHE_IMAGE_PREFIX ?=
FINI_BE_CACHE_PUSH ?= 0
FINI_E2E_CI_RUN_ID ?= pr-gate
FINI_E2E_CI_RUN_DIR ?= /var/tmp/fini-e2e-$(FINI_E2E_CI_RUN_ID)
FINI_E2E_CI_RESULTS_DIR ?= $(FINI_E2E_CI_RUN_DIR)/test-results
FINI_E2E_CI_ACTORS ?= actor-a,actor-b
FINI_E2E_CI_ACTOR_WAIT_SECS ?= 180
FINI_DEV_RUNNER_IMAGE ?= fini-dev-runner-ci
FINI_E2E_CACHE_IMAGE_PREFIX ?=
FINI_E2E_CACHE_PUSH ?= 0
RELEASE_BUNDLES ?= deb,rpm

.PHONY: help require-container dev build play-store-screenshots pr-gate-fe-unit pr-gate-be-cache-key pr-gate-be-compile pr-gate-be-unit pr-gate-e2e pr-gate-e2e-cache-key pr-gate-e2e-build-dev-runner pr-gate-e2e-run pr-gate-e2e-artifacts pr-gate-e2e-cleanup e2e e2e-ci e2e-image e2e-build e2e-headed runtime-image runtime-smoke pre-release-check release android-connect android-dev android-build android-build-emulator-e2e android-sign-debug android-sign-release-local android-launch android-devices android-debug-deploy android-debug-deploy-debug android-release-deploy-local flatpak-install-local

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
	@echo "  make android-debug-deploy  Build with git-derived version, sign, install, and launch"
	@echo "  make android-release-deploy-local  Local release-signed deploy preserving app identity"
	@echo "  make android-devices  List connected ADB devices"
	@echo ""

# ── Linux ─────────────────────────────────────────────────────────────────────

require-container:
	@test "$(CONTAINER_ENGINE)" != "missing" || (echo "No container engine found. Install Docker or Podman, or set CONTAINER=docker|podman." && exit 1)

dev:
	@set -eu; \
	capability_backup="$$(mktemp /var/tmp/fini-default-capability.XXXXXX)"; \
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
	run_root="$${FINI_E2E_ROOT:-/var/tmp/fini-e2e-headed}"; \
	e2e_target_dir="$$(pwd)/src-tauri/target/debug-e2e"; \
	app_bin_path="$$e2e_target_dir/debug/fini-app"; \
	cli_bin_path="$$e2e_target_dir/debug/fini"; \
	capability_backup="$$(mktemp /var/tmp/fini-default-capability.XXXXXX)"; \
	cp src-tauri/capabilities/default.json "$$capability_backup"; \
	restore_capability() { cp "$$capability_backup" src-tauri/capabilities/default.json; rm -f "$$capability_backup"; }; \
	trap restore_capability EXIT INT TERM; \
	mkdir -p "$$run_root"; \
	cp src-tauri/devtools-capabilities/default.json src-tauri/capabilities/default.json; \
	CARGO_TARGET_DIR="$$e2e_target_dir" npm run tauri -- build --debug --features ui-plane,desktop-updater,devtools --no-bundle -- --bin fini-app; \
	restore_capability; \
	trap - EXIT INT TERM; \
	CARGO_TARGET_DIR="$$e2e_target_dir" cargo build --manifest-path src-tauri/Cargo.toml --bin fini --features cli-plane; \
	FINI_E2E_ROOT="$$run_root" FINI_E2E_HEADFUL=1 FINI_APP_BINARY="$$app_bin_path" FINI_CLI_BINARY="$$cli_bin_path" TZ=UTC npx playwright test --config specs/e2e/playwright.config.ts --project ui --project actors

# Build/update the published headless runtime image locally.
runtime-image:
	$(MAKE) require-container
	$(CONTAINER_ENGINE) build --target runtime -t fini-runtime .

# Verify the runtime container executes the CLI surface.
runtime-smoke:
	$(MAKE) require-container
	$(CONTAINER_ENGINE) image inspect fini-runtime >/dev/null 2>&1 || $(CONTAINER_ENGINE) build --target runtime -t fini-runtime .
	$(CONTAINER_ENGINE) run --rm fini-runtime --help

pre-release-check:
	@set -eu; \
	log_dir="$${PRE_RELEASE_LOG_DIR:-/var/tmp/fini-pre-release}"; \
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

android-connect:
	@test -n "$(DEVICE_ADDRESS)" || (echo "No device found via adb mdns. Enable wireless debugging on the phone." && exit 1)
	@timeout "$(ADB_CONNECT_TIMEOUT)s" adb connect $(DEVICE_ADDRESS) || (echo "ADB connect timed out for $(DEVICE_ADDRESS). Re-authorize wireless debugging, reconnect USB, or start an emulator." && exit 1)

android-dev: android-connect
	npm run tauri android dev -- --features ui-plane --host $(HOST_IP)

android-build:
	npm run tauri android build -- --features ui-plane --target "$(ANDROID_TARGET)"

android-build-emulator-e2e:
	npm run tauri android build -- --features ui-plane --ci --debug --apk --target x86_64

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
	  keystore_path="/var/tmp/fini-release.keystore"; \
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

android-debug-deploy:
	@printf 'Android debug version: %s (%s)\n' "$(ANDROID_DEBUG_VERSION_NAME)" "$(ANDROID_DEBUG_VERSION_CODE)"
	FINI_ANDROID_VERSION_NAME="$(ANDROID_DEBUG_VERSION_NAME)" FINI_ANDROID_VERSION_CODE="$(ANDROID_DEBUG_VERSION_CODE)" npm run tauri android build -- --features ui-plane --target "$(ANDROID_TARGET)"
	$(MAKE) android-sign-debug
	$(MAKE) android-install-debug
	$(MAKE) android-launch

android-debug-deploy-debug:
	@printf 'Android debug (debug profile) version: %s (%s)\n' "$(ANDROID_DEBUG_VERSION_NAME)" "$(ANDROID_DEBUG_VERSION_CODE)"
	FINI_ANDROID_VERSION_NAME="$(ANDROID_DEBUG_VERSION_NAME)" FINI_ANDROID_VERSION_CODE="$(ANDROID_DEBUG_VERSION_CODE)" npm run tauri android build -- --features ui-plane --debug --target "$(ANDROID_TARGET)"
	adb install -r "src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk"
	$(MAKE) android-launch

android-release-deploy-local:
	@printf 'Android local release version: %s (%s)\n' "$(ANDROID_DEBUG_VERSION_NAME)" "$(ANDROID_DEBUG_VERSION_CODE)"
	FINI_ANDROID_VERSION_NAME="$(ANDROID_DEBUG_VERSION_NAME)" FINI_ANDROID_VERSION_CODE="$(ANDROID_DEBUG_VERSION_CODE)" npm run tauri android build -- --features ui-plane --target "$(ANDROID_TARGET)"
	$(MAKE) android-sign-release-local
	$(MAKE) android-install-release-local
	$(MAKE) android-launch

android-devices:
	adb devices
