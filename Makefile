-include .env
export

CONTAINER ?= podman
FINI_BE_COMPILE_IMAGE ?= fini-be-compile-ci
FINI_BE_UNIT_IMAGE ?= fini-be-unit-test
FINI_BE_CACHE_IMAGE_PREFIX ?=
FINI_BE_CACHE_PUSH ?= 0
FINI_E2E_CI_RUN_ID ?= pr-gate
FINI_E2E_CI_RUN_DIR ?= /var/tmp/fini-e2e-$(FINI_E2E_CI_RUN_ID)
FINI_E2E_CI_SOCKET_DIR ?= $(FINI_E2E_CI_RUN_DIR)/sockets
FINI_E2E_CI_RESULTS_DIR ?= $(FINI_E2E_CI_RUN_DIR)/test-results
FINI_E2E_CI_NETWORK ?= fini-e2e-$(FINI_E2E_CI_RUN_ID)
FINI_E2E_CI_ACTORS ?= actor-a,actor-b
FINI_E2E_ACTOR_IMAGE ?= fini-e2e-actor-ci
FINI_E2E_RUNNER_IMAGE ?= fini-e2e-runner-ci
FINI_E2E_CACHE_IMAGE_PREFIX ?=
FINI_E2E_CACHE_PUSH ?= 0

.PHONY: help dev build mcp pr-gate-fe-unit pr-gate-be-cache-key pr-gate-be-compile pr-gate-be-unit pr-gate-e2e pr-gate-e2e-cache-key pr-gate-e2e-build-actor pr-gate-e2e-build-runner pr-gate-e2e-network pr-gate-e2e-start-actors pr-gate-e2e-wait-actors pr-gate-e2e-run pr-gate-e2e-logs pr-gate-e2e-cleanup e2e e2e-ci e2e-image e2e-build e2e-headed e2e-actors-image e2e-actors runtime-image runtime-smoke release android-connect android-dev android-build android-sign-debug android-sign-release-local android-install-debug android-install-release-local android-launch android-devices android-debug-deploy android-release-deploy-local flatpak-install-local

help:
	@echo ""
	@echo "Linux"
	@echo "  make dev              Hot-reload dev app (Vite HMR + Rust watch)"
	@echo "  make build            Release build"
	@echo "  make mcp              Run MCP server (debug binary)"
	@echo "  make pr-gate-fe-unit  Run frontend unit tests in Dockerfile stage"
	@echo "  make pr-gate-be-compile  Compile backend tests in Dockerfile stage"
	@echo "  make pr-gate-be-unit  Run backend unit tests in Dockerfile stage"
	@echo "  make pr-gate-e2e      Run npm run test:e2e:ci with Dockerfile stages"
	@echo "  make pr-gate-e2e-cache-key  Print the E2E image input cache key"
	@echo "  make pr-gate-e2e-*    Run one named CI E2E phase for easier failure diagnosis"
	@echo "  make e2e              Run visible local two-app E2E"
	@echo "  make e2e-ci           Run containerized headless E2E in CI mode"
	@echo "  make e2e-image        Build/update the container e2e image"
	@echo "  make e2e-build        Build container image and run e2e-ci inside it"
	@echo "  make e2e-headed       Run visible local two-app E2E"
	@echo "  make e2e-actors-image Force rebuild actor and runner images"
	@echo "  make e2e-actors       Run containerized full E2E suite, reusing fresh images"
	@echo "  FINI_E2E_REBUILD=1 npm run test:e2e  Force E2E image rebuild before running"
	@echo "  make runtime-image    Build/update the runtime container image"
	@echo "  make runtime-smoke    Run a runtime container smoke check"
	@echo "  make flatpak-install-local  Build release binary and reinstall local Flatpak"
	@echo "  make release VERSION=x.y.z  Bump versions, verify build, push main, and push signed tag vX.Y.Z"
	@echo ""
	@echo "Android"
	@echo "  make android-connect  Auto-discover and connect to device via adb mdns"
	@echo "  make android-dev      Hot-reload on device via Wi-Fi (auto-connects)"
	@echo "  make android-build    Build Android APK"
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

dev:
	npm run tauri dev -- app

build:
	npm run tauri build

flatpak-install-local:
	-$(MAKE) build
	flatpak run org.flatpak.Builder --force-clean --user --install flatpak-build com.fini.app.yml

mcp:
	./src-tauri/target/debug/fini mcp

pr-gate-fe-unit:
	$(CONTAINER) build --target fe-unit-test -t fini-fe-unit-test .

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
	cache_inputs | xargs -0 sha256sum | sha256sum | cut -d ' ' -f 1

pr-gate-be-compile:
	@set -eu; \
	cache_key="$$(make --no-print-directory pr-gate-be-cache-key)"; \
	cache_prefix="$(FINI_BE_CACHE_IMAGE_PREFIX)"; \
	cache_image=""; \
	if [ -n "$$cache_prefix" ]; then cache_image="$$cache_prefix-be-compile-cache:$$cache_key"; fi; \
	if [ -n "$$cache_image" ] && $(CONTAINER) pull "$$cache_image"; then \
	  printf 'Using cached backend compile image: %s\n' "$$cache_image"; \
	  $(CONTAINER) tag "$$cache_image" "$(FINI_BE_COMPILE_IMAGE)"; \
	  exit 0; \
	fi; \
	printf 'Building backend compile image for cache key: %s\n' "$$cache_key"; \
	if [ -n "$$cache_image" ]; then \
	  $(CONTAINER) build --target be-test-compile --build-arg BUILDKIT_INLINE_CACHE=1 --label "fini.be.cache-key=$$cache_key" -t "$(FINI_BE_COMPILE_IMAGE)" -t "$$cache_image" .; \
	else \
	  $(CONTAINER) build --target be-test-compile --build-arg BUILDKIT_INLINE_CACHE=1 --label "fini.be.cache-key=$$cache_key" -t "$(FINI_BE_COMPILE_IMAGE)" .; \
	fi; \
	if [ -n "$$cache_image" ] && [ "$(FINI_BE_CACHE_PUSH)" = "1" ]; then \
	  $(CONTAINER) push "$$cache_image"; \
	fi

pr-gate-be-unit:
	$(MAKE) pr-gate-be-compile
	$(CONTAINER) build --target be-unit-test --cache-from "$(FINI_BE_COMPILE_IMAGE)" -t "$(FINI_BE_UNIT_IMAGE)" .

pr-gate-e2e:
	@set -eu; \
	cleanup() { $(MAKE) pr-gate-e2e-cleanup >/dev/null 2>&1 || true; }; \
	trap cleanup EXIT INT TERM; \
	$(MAKE) pr-gate-e2e-build-actor; \
	$(MAKE) pr-gate-e2e-build-runner; \
	$(MAKE) pr-gate-e2e-network; \
	$(MAKE) pr-gate-e2e-start-actors; \
	$(MAKE) pr-gate-e2e-wait-actors; \
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
	      vite.config.ts \
	      src \
	      src-tauri/Cargo.toml \
	      src-tauri/Cargo.lock \
	      src-tauri/build.rs \
	      src-tauri/src \
	      src-tauri/migrations \
	      src-tauri/patches \
	      src-tauri/capabilities \
	      src-tauri/icons \
	      src-tauri/tauri.conf.json \
	      specs/e2e; \
	    git ls-files --others --exclude-standard -z -- \
	      Dockerfile \
	      package.json \
	      package-lock.json \
	      tsconfig\*.json \
	      index.html \
	      vite.config.ts \
	      src \
	      src-tauri/Cargo.toml \
	      src-tauri/Cargo.lock \
	      src-tauri/build.rs \
	      src-tauri/src \
	      src-tauri/migrations \
	      src-tauri/patches \
	      src-tauri/capabilities \
	      src-tauri/icons \
	      src-tauri/tauri.conf.json \
	      specs/e2e; \
	  } | sort -zu; \
	}; \
	cache_inputs | xargs -0 sha256sum | sha256sum | cut -d ' ' -f 1

pr-gate-e2e-build-actor:
	@set -eu; \
	cache_key="$$(make --no-print-directory pr-gate-e2e-cache-key)"; \
	cache_prefix="$(FINI_E2E_CACHE_IMAGE_PREFIX)"; \
	cache_image=""; \
	if [ -n "$$cache_prefix" ]; then cache_image="$$cache_prefix-e2e-actor-cache:$$cache_key"; fi; \
	if [ -n "$$cache_image" ] && $(CONTAINER) pull "$$cache_image"; then \
	  printf 'Using cached actor image: %s\n' "$$cache_image"; \
	  $(CONTAINER) tag "$$cache_image" "$(FINI_E2E_ACTOR_IMAGE)"; \
	  exit 0; \
	fi; \
	printf 'Building actor image for E2E cache key: %s\n' "$$cache_key"; \
	if [ -n "$$cache_image" ]; then \
	  $(CONTAINER) build --target e2e-actor --label "fini.e2e.cache-key=$$cache_key" -t "$(FINI_E2E_ACTOR_IMAGE)" -t "$$cache_image" .; \
	else \
	  $(CONTAINER) build --target e2e-actor --label "fini.e2e.cache-key=$$cache_key" -t "$(FINI_E2E_ACTOR_IMAGE)" .; \
	fi; \
	if [ -n "$$cache_image" ] && [ "$(FINI_E2E_CACHE_PUSH)" = "1" ]; then \
	  $(CONTAINER) push "$$cache_image"; \
	fi

pr-gate-e2e-build-runner:
	@set -eu; \
	cache_key="$$(make --no-print-directory pr-gate-e2e-cache-key)"; \
	cache_prefix="$(FINI_E2E_CACHE_IMAGE_PREFIX)"; \
	cache_image=""; \
	if [ -n "$$cache_prefix" ]; then cache_image="$$cache_prefix-e2e-runner-cache:$$cache_key"; fi; \
	if [ -n "$$cache_image" ] && $(CONTAINER) pull "$$cache_image"; then \
	  printf 'Using cached runner image: %s\n' "$$cache_image"; \
	  $(CONTAINER) tag "$$cache_image" "$(FINI_E2E_RUNNER_IMAGE)"; \
	  exit 0; \
	fi; \
	printf 'Building runner image for E2E cache key: %s\n' "$$cache_key"; \
	if [ -n "$$cache_image" ]; then \
	  $(CONTAINER) build --target e2e-runner --label "fini.e2e.cache-key=$$cache_key" -t "$(FINI_E2E_RUNNER_IMAGE)" -t "$$cache_image" .; \
	else \
	  $(CONTAINER) build --target e2e-runner --label "fini.e2e.cache-key=$$cache_key" -t "$(FINI_E2E_RUNNER_IMAGE)" .; \
	fi; \
	if [ -n "$$cache_image" ] && [ "$(FINI_E2E_CACHE_PUSH)" = "1" ]; then \
	  $(CONTAINER) push "$$cache_image"; \
	fi

pr-gate-e2e-network:
	@set -eu; \
	mkdir -p "$(FINI_E2E_CI_SOCKET_DIR)" "$(FINI_E2E_CI_RESULTS_DIR)"; \
	$(CONTAINER) network rm "$(FINI_E2E_CI_NETWORK)" >/dev/null 2>&1 || true; \
	$(CONTAINER) network create "$(FINI_E2E_CI_NETWORK)" >/dev/null

pr-gate-e2e-start-actors:
	@set -eu; \
	actor_list="$(FINI_E2E_CI_ACTORS)"; \
	IFS=','; set -- $$actor_list; \
	if [ "$$#" -lt 2 ]; then \
	  printf 'Need at least two actors, got: %s\n' "$(FINI_E2E_CI_ACTORS)" >&2; \
	  exit 1; \
	fi; \
	for actor in "$$@"; do \
	  actor_data_dir="$(FINI_E2E_CI_RUN_DIR)/$$actor-data"; \
	  mkdir -p "$$actor_data_dir"; \
	  $(CONTAINER) rm -f "fini-$(FINI_E2E_CI_RUN_ID)-$$actor" >/dev/null 2>&1 || true; \
	  $(CONTAINER) run -d --rm \
	    --name "fini-$(FINI_E2E_CI_RUN_ID)-$$actor" \
	    --hostname "$$actor" \
	    --network "$(FINI_E2E_CI_NETWORK)" \
	    -e FINI_ACTOR_SLUG="$$actor" \
	    -e FINI_E2E_SOCKET_DIR=/var/run/fini-e2e \
	    -e FINI_APP_DATA_DIR=/data \
	    -v "$(FINI_E2E_CI_SOCKET_DIR):/var/run/fini-e2e:z" \
	    -v "$$actor_data_dir:/data:Z" \
	    "$(FINI_E2E_ACTOR_IMAGE)" >/dev/null; \
	done

pr-gate-e2e-wait-actors:
	@set -eu; \
	actor_list="$(FINI_E2E_CI_ACTORS)"; \
	IFS=','; set -- $$actor_list; \
	for actor in "$$@"; do \
	  socket_path="$(FINI_E2E_CI_SOCKET_DIR)/$$actor.sock"; \
	  deadline=$$(($$(date +%s) + 60)); \
	  while [ ! -S "$$socket_path" ]; do \
	    if [ "$$(date +%s)" -ge "$$deadline" ]; then \
	      printf 'Actor socket did not appear: %s\n' "$$socket_path" >&2; \
	      $(CONTAINER) logs "fini-$(FINI_E2E_CI_RUN_ID)-$$actor" 2>/dev/null || true; \
	      exit 1; \
	    fi; \
	    sleep 1; \
	  done; \
	done

pr-gate-e2e-run:
	$(CONTAINER) rm -f "fini-$(FINI_E2E_CI_RUN_ID)-runner" >/dev/null 2>&1 || true
	$(CONTAINER) run --rm \
	  --name "fini-$(FINI_E2E_CI_RUN_ID)-runner" \
	  --network "$(FINI_E2E_CI_NETWORK)" \
	  -e FINI_E2E_ACTORS="$(FINI_E2E_CI_ACTORS)" \
	  -e FINI_E2E_SOCKET_DIR=/var/run/fini-e2e \
	  -v "$(FINI_E2E_CI_SOCKET_DIR):/var/run/fini-e2e:z" \
	  -v "$(FINI_E2E_CI_RESULTS_DIR):/app/test-results:Z" \
	  "$(FINI_E2E_RUNNER_IMAGE)"

pr-gate-e2e-logs:
	@set -eu; \
	actor_list="$(FINI_E2E_CI_ACTORS)"; \
	IFS=','; set -- $$actor_list; \
	for actor in "$$@"; do \
	  printf '\n===== logs: %s =====\n' "$$actor"; \
	  $(CONTAINER) logs "fini-$(FINI_E2E_CI_RUN_ID)-$$actor" 2>/dev/null || true; \
	done

pr-gate-e2e-cleanup:
	@set -eu; \
	$(CONTAINER) rm -f "fini-$(FINI_E2E_CI_RUN_ID)-runner" >/dev/null 2>&1 || true; \
	actor_list="$(FINI_E2E_CI_ACTORS)"; \
	IFS=','; set -- $$actor_list; \
	for actor in "$$@"; do \
	  $(CONTAINER) rm -f "fini-$(FINI_E2E_CI_RUN_ID)-$$actor" >/dev/null 2>&1 || true; \
	done; \
	$(CONTAINER) network rm "$(FINI_E2E_CI_NETWORK)" >/dev/null 2>&1 || true

# Run the real-app e2e lane locally.
e2e:
	$(MAKE) e2e-headed

# Run the same lane under CI settings.
e2e-ci:
	CI=1 $(MAKE) e2e-actors

# Build/update the container image used for CI-style local e2e runs.
e2e-image:
	$(CONTAINER) build --target test -t fini-e2e .

# Run the headless e2e tier inside the cached Podman image.
e2e-build:
	$(CONTAINER) image inspect fini-e2e >/dev/null 2>&1 || $(CONTAINER) build --target test -t fini-e2e .
	$(CONTAINER) run --rm fini-e2e

# Run the visible local two-app E2E suite against the host desktop display.
e2e-headed:
	@set -eu; \
	actor_list="$${FINI_E2E_ACTORS:-actor-a,actor-b}"; \
	keep="$${FINI_E2E_KEEP:-0}"; \
	run_root="$${FINI_E2E_ROOT:-/var/tmp/fini-e2e-headed}"; \
	base_discovery_port="$${FINI_E2E_BASE_DISCOVERY_PORT:-$$((46000 + ($$RANDOM % 1000) * 10))}"; \
	run_id="$$(date +%Y%m%d-%H%M%S)-$$$$"; \
	run_dir="$$run_root/$$run_id"; \
	socket_dir="$$run_dir/sockets"; \
	test_results_dir="$$run_dir/test-results"; \
	bin_path="./src-tauri/target/debug/fini"; \
	printf 'FINI_E2E_RUN_DIR=%s\n' "$$run_dir"; \
	mkdir -p "$$socket_dir" "$$test_results_dir"; \
	IFS=','; set -- $$actor_list; \
	if [ "$$#" -lt 2 ]; then \
	  printf 'Need at least two actors, got: %s\n' "$$actor_list" >&2; \
	  exit 1; \
	fi; \
	peer_ports=""; \
	idx=0; \
	for actor in "$$@"; do \
	  port=$$((base_discovery_port + idx * 2)); \
	  if [ -z "$$peer_ports" ]; then peer_ports="$$port"; else peer_ports="$$peer_ports,$$port"; fi; \
	  idx=$$((idx + 1)); \
	done; \
	cleanup() { \
	  status="$$?"; \
	  if [ "$$keep" = "1" ]; then \
	    printf 'Keeping local app processes for debugging: %s\n' "$$run_id"; \
	    printf 'Stop them manually if needed. Run dir: %s\n' "$$run_dir"; \
	    exit "$$status"; \
	  fi; \
	  for pid_file in "$$run_dir"/*.pid; do \
	    [ -f "$$pid_file" ] || continue; \
	    pid="$$(cat "$$pid_file")"; \
	    kill "$$pid" >/dev/null 2>&1 || true; \
	  done; \
	  wait >/dev/null 2>&1 || true; \
	  exit "$$status"; \
	}; \
	trap cleanup EXIT INT TERM; \
	node ./node_modules/@tauri-apps/cli/tauri.js build --debug --features e2e-testing --no-bundle; \
	idx=0; \
	for actor in "$$@"; do \
	  actor_data_dir="$$run_dir/$$actor-data"; \
	  actor_socket="$$socket_dir/$$actor.sock"; \
	  actor_log="$$run_dir/$$actor.log"; \
	  discovery_port=$$((base_discovery_port + idx * 2)); \
	  ws_port=$$((base_discovery_port + idx * 2 + 1)); \
	  mkdir -p "$$actor_data_dir"; \
	  rm -f "$$actor_socket"; \
	  printf 'Launching visible app window: %s (discovery=%s ws=%s)\n' "$$actor" "$$discovery_port" "$$ws_port"; \
	  HOSTNAME="$$actor" FINI_APP_DATA_DIR="$$actor_data_dir" TAURI_PLAYWRIGHT_SOCKET="$$actor_socket" FINI_DISCOVERY_PORT="$$discovery_port" FINI_DISCOVERY_PEER_PORTS="$$peer_ports" FINI_SPACE_SYNC_WS_PORT="$$ws_port" TZ=UTC "$$bin_path" app >"$$actor_log" 2>&1 & \
	  printf '%s' "$$!" > "$$run_dir/$$actor.pid"; \
	  idx=$$((idx + 1)); \
	done; \
	for actor in "$$@"; do \
	  socket_path="$$socket_dir/$$actor.sock"; \
	  deadline=$$(($$(date +%s) + 60)); \
	  while [ ! -S "$$socket_path" ]; do \
	    if [ "$$(date +%s)" -ge "$$deadline" ]; then \
	      printf 'Actor socket did not appear: %s\n' "$$socket_path" >&2; \
	      log_path="$$run_dir/$$actor.log"; \
	      [ -f "$$log_path" ] && printf 'Actor log: %s\n' "$$log_path" >&2; \
	      exit 1; \
	    fi; \
	    sleep 1; \
	  done; \
	done; \
	FINI_E2E_ACTORS="$$actor_list" FINI_E2E_SOCKET_DIR="$$socket_dir" FINI_E2E_HEADFUL=1 TZ=UTC npx playwright test --config specs/e2e/playwright.config.ts --project actors --output "$$test_results_dir"

# Build/update the actor and runner images for multi-actor desktop e2e.
e2e-actors-image:
	@set -eu; \
	cache_inputs() { \
	  { \
	    git ls-files -z -- \
	      Dockerfile \
	      package.json \
	      package-lock.json \
	      tsconfig\*.json \
	      index.html \
	      vite.config.ts \
	      src \
	      src-tauri/Cargo.toml \
	      src-tauri/Cargo.lock \
	      src-tauri/build.rs \
	      src-tauri/src \
	      src-tauri/migrations \
	      src-tauri/patches \
	      src-tauri/capabilities \
	      src-tauri/icons \
	      src-tauri/tauri.conf.json \
	      specs/e2e; \
	    git ls-files --others --exclude-standard -z -- \
	      Dockerfile \
	      package.json \
	      package-lock.json \
	      tsconfig\*.json \
	      index.html \
	      vite.config.ts \
	      src \
	      src-tauri/Cargo.toml \
	      src-tauri/Cargo.lock \
	      src-tauri/build.rs \
	      src-tauri/src \
	      src-tauri/migrations \
	      src-tauri/patches \
	      src-tauri/capabilities \
	      src-tauri/icons \
	      src-tauri/tauri.conf.json \
	      specs/e2e; \
	  } | sort -zu; \
	}; \
	cache_key="$$(cache_inputs | xargs -0 sha256sum | sha256sum | cut -d ' ' -f 1)"; \
	$(CONTAINER) build --target e2e-actor --label "fini.e2e.cache-key=$$cache_key" -t fini-e2e-actor .; \
	$(CONTAINER) build --target e2e-runner --label "fini.e2e.cache-key=$$cache_key" -t fini-e2e-runner .

# Run the multi-actor desktop e2e smoke lane.
e2e-actors:
	@set -eu; \
	actor_image="$${FINI_E2E_ACTOR_IMAGE:-fini-e2e-actor}"; \
	runner_image="$${FINI_E2E_RUNNER_IMAGE:-fini-e2e-runner}"; \
	force_rebuild="$${FINI_E2E_REBUILD:-0}"; \
	actor_list="$${FINI_E2E_ACTORS:-actor-a,actor-b}"; \
	keep="$${FINI_E2E_KEEP:-0}"; \
	run_root="$${FINI_E2E_ROOT:-/var/tmp/fini-e2e-actors}"; \
	run_id="$$(date +%Y%m%d-%H%M%S)-$$$$"; \
	run_dir="$$run_root/$$run_id"; \
	socket_dir="$$run_dir/sockets"; \
	test_results_dir="$$run_dir/test-results"; \
	network_name="fini-e2e-$$run_id"; \
	cache_inputs() { \
	  { \
	    git ls-files -z -- \
	      Dockerfile \
	      package.json \
	      package-lock.json \
	      tsconfig\*.json \
	      index.html \
	      vite.config.ts \
	      src \
	      src-tauri/Cargo.toml \
	      src-tauri/Cargo.lock \
	      src-tauri/build.rs \
	      src-tauri/src \
	      src-tauri/migrations \
	      src-tauri/patches \
	      src-tauri/capabilities \
	      src-tauri/icons \
	      src-tauri/tauri.conf.json \
	      specs/e2e; \
	    git ls-files --others --exclude-standard -z -- \
	      Dockerfile \
	      package.json \
	      package-lock.json \
	      tsconfig\*.json \
	      index.html \
	      vite.config.ts \
	      src \
	      src-tauri/Cargo.toml \
	      src-tauri/Cargo.lock \
	      src-tauri/build.rs \
	      src-tauri/src \
	      src-tauri/migrations \
	      src-tauri/patches \
	      src-tauri/capabilities \
	      src-tauri/icons \
	      src-tauri/tauri.conf.json \
	      specs/e2e; \
	  } | sort -zu; \
	}; \
	image_cache_key() { \
	  $(CONTAINER) image inspect "$$1" --format '{{ index .Config.Labels "fini.e2e.cache-key" }}' 2>/dev/null || true; \
	}; \
	cache_key="$$(cache_inputs | xargs -0 sha256sum | sha256sum | cut -d ' ' -f 1)"; \
	actor_current_key="$$(image_cache_key "$$actor_image")"; \
	runner_current_key="$$(image_cache_key "$$runner_image")"; \
	printf 'FINI_E2E_RUN_DIR=%s\n' "$$run_dir"; \
	mkdir -p "$$socket_dir" "$$test_results_dir"; \
	IFS=','; set -- $$actor_list; \
	if [ "$$#" -lt 2 ]; then \
	  printf 'Need at least two actors, got: %s\n' "$$actor_list" >&2; \
	  exit 1; \
	fi; \
	cleanup() { \
	  status="$$?"; \
	  if [ "$$status" -ne 0 ]; then \
	    for actor in "$$@"; do $(CONTAINER) logs "fini-$$run_id-$$actor" 2>/dev/null || true; done; \
	  fi; \
	  if [ "$$keep" = "1" ]; then \
	    printf 'Keeping containers and network for debugging: %s\n' "$$run_id"; \
	    exit "$$status"; \
	  fi; \
	  $(CONTAINER) rm -f "fini-$$run_id-runner" >/dev/null 2>&1 || true; \
	  for actor in "$$@"; do $(CONTAINER) rm -f "fini-$$run_id-$$actor" >/dev/null 2>&1 || true; done; \
	  $(CONTAINER) network rm "$$network_name" >/dev/null 2>&1 || true; \
	  exit "$$status"; \
	}; \
	trap 'cleanup "$$@"' EXIT INT TERM; \
	if [ "$$force_rebuild" = "1" ] || [ "$$actor_current_key" != "$$cache_key" ]; then \
	  printf 'Building actor image: %s\n' "$$actor_image"; \
	  $(CONTAINER) build --target e2e-actor --label "fini.e2e.cache-key=$$cache_key" -t "$$actor_image" .; \
	else \
	  printf 'Using cached actor image: %s\n' "$$actor_image"; \
	fi; \
	if [ "$$force_rebuild" = "1" ] || [ "$$runner_current_key" != "$$cache_key" ]; then \
	  printf 'Building runner image: %s\n' "$$runner_image"; \
	  $(CONTAINER) build --target e2e-runner --label "fini.e2e.cache-key=$$cache_key" -t "$$runner_image" .; \
	else \
	  printf 'Using cached runner image: %s\n' "$$runner_image"; \
	fi; \
	$(CONTAINER) network create "$$network_name" >/dev/null; \
	for actor in "$$@"; do \
	  actor_data_dir="$$run_dir/$$actor-data"; \
	  mkdir -p "$$actor_data_dir"; \
	  $(CONTAINER) run -d --rm \
	    --name "fini-$$run_id-$$actor" \
	    --hostname "$$actor" \
	    --network "$$network_name" \
	    -e FINI_ACTOR_SLUG="$$actor" \
	    -e FINI_E2E_SOCKET_DIR=/var/run/fini-e2e \
	    -e FINI_APP_DATA_DIR=/data \
	    -v "$$socket_dir:/var/run/fini-e2e:z" \
	    -v "$$actor_data_dir:/data:Z" \
	    "$$actor_image" >/dev/null; \
	done; \
	for actor in "$$@"; do \
	  socket_path="$$socket_dir/$$actor.sock"; \
	  deadline=$$(($$(date +%s) + 60)); \
	  while [ ! -S "$$socket_path" ]; do \
	    if [ "$$(date +%s)" -ge "$$deadline" ]; then \
	      printf 'Actor socket did not appear: %s\n' "$$socket_path" >&2; \
	      exit 1; \
	    fi; \
	    sleep 1; \
	  done; \
	done; \
	$(CONTAINER) run --rm \
	  --name "fini-$$run_id-runner" \
	  --network "$$network_name" \
	  -e FINI_E2E_ACTORS="$$actor_list" \
	  -e FINI_E2E_SOCKET_DIR=/var/run/fini-e2e \
	  -v "$$socket_dir:/var/run/fini-e2e:z" \
	  -v "$$test_results_dir:/app/test-results:Z" \
	  "$$runner_image"

# Build/update the published headless runtime image locally.
runtime-image:
	$(CONTAINER) build --target runtime -t fini-runtime .

# Verify the runtime container executes the CLI surface.
runtime-smoke:
	$(CONTAINER) image inspect fini-runtime >/dev/null 2>&1 || $(CONTAINER) build --target runtime -t fini-runtime .
	$(CONTAINER) run --rm fini-runtime --help

release:
	@test -n "$(VERSION)" || (echo "VERSION is required. Use: make release VERSION=x.y.z" && exit 1)
	@printf '%s\n' "$(VERSION)" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$$' || (echo "VERSION must match x.y.z" && exit 1)
	@branch="$$(git branch --show-current)"; \
	if [ "$$branch" != "main" ]; then \
	  echo "Release must run from main"; \
	  echo "current branch=$$branch"; \
	  exit 1; \
	fi; \
	git diff --quiet || (echo "Working tree has unstaged changes" && exit 1); \
	git diff --cached --quiet || (echo "Working tree has staged changes" && exit 1); \
	test -z "$$(git ls-files --others --exclude-standard)" || (echo "Working tree has untracked files" && exit 1); \
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
	cargo run --manifest-path xtask/Cargo.toml -- release-version "$(VERSION)"
	$(MAKE) build
	git add package.json package-lock.json src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/tauri.conf.json
	git commit -m "chore: release v$(VERSION)"
	git push origin main
	git -c user.email="v.ruzhentsov@gmail.com" -c user.signingkey="199DFE796EA43C00" tag -s -a "v$(VERSION)" -m "v$(VERSION)"
	git tag -v "v$(VERSION)"
	git push origin "v$(VERSION)"
	@echo "Released v$(VERSION)"

# ── Android ───────────────────────────────────────────────────────────────────

DEVICE_ADDRESS = $(shell adb mdns services 2>/dev/null | grep '_adb-tls-connect' | head -1 | awk '{print $$NF}')
DEVICE_IP      = $(firstword $(subst :, ,$(DEVICE_ADDRESS)))
HOST_IP        = $(shell ip route get $(DEVICE_IP) 2>/dev/null | grep -oP 'src \K\S+' | head -1)
LATEST_TAG = $(shell git describe --tags --abbrev=0 2>/dev/null || printf 'v0.0.0')
GIT_SHA = $(shell git rev-parse --short HEAD 2>/dev/null || printf 'unknown')
ANDROID_DEBUG_VERSION_NAME = $(patsubst v%,%,$(LATEST_TAG))+dev.$(GIT_SHA)
ANDROID_DEBUG_VERSION_CODE = $(shell date +%s)
ANDROID_UNSIGNED_APK = src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk
ANDROID_SIGNED_APK = bin/fini.apk
ANDROID_RELEASE_SIGNED_APK = bin/fini-release.apk
APKSIGNER = $(lastword $(sort $(wildcard $(ANDROID_HOME)/build-tools/*/apksigner)))

android-connect:
	@test -n "$(DEVICE_ADDRESS)" || (echo "No device found via adb mdns. Enable wireless debugging on the phone." && exit 1)
	adb connect $(DEVICE_ADDRESS)

android-dev: android-connect
	npm run tauri android dev -- --host $(HOST_IP)

android-build:
	npm run tauri android build

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
	FINI_ANDROID_VERSION_NAME="$(ANDROID_DEBUG_VERSION_NAME)" FINI_ANDROID_VERSION_CODE="$(ANDROID_DEBUG_VERSION_CODE)" npm run tauri android build
	$(MAKE) android-sign-debug
	$(MAKE) android-install-debug
	$(MAKE) android-launch

android-release-deploy-local:
	@printf 'Android local release version: %s (%s)\n' "$(ANDROID_DEBUG_VERSION_NAME)" "$(ANDROID_DEBUG_VERSION_CODE)"
	FINI_ANDROID_VERSION_NAME="$(ANDROID_DEBUG_VERSION_NAME)" FINI_ANDROID_VERSION_CODE="$(ANDROID_DEBUG_VERSION_CODE)" npm run tauri android build
	$(MAKE) android-sign-release-local
	$(MAKE) android-install-release-local
	$(MAKE) android-launch

android-devices:
	adb devices
