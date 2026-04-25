-include .env
export

.PHONY: help dev build mcp e2e e2e-ci e2e-image e2e-build release-tag android-connect android-dev android-build android-sign-debug android-install-debug android-launch android-devices

help:
	@echo ""
	@echo "Linux"
	@echo "  make dev              Hot-reload dev app (Vite HMR + Rust watch)"
	@echo "  make build            Release build"
	@echo "  make mcp              Run MCP server (debug binary)"
	@echo "  make e2e              Run real-app e2e tests locally"
	@echo "  make e2e-ci           Run real-app e2e tests in CI mode"
	@echo "  make e2e-image        Build/update the Podman e2e image"
	@echo "  make e2e-build        Build Podman image and run e2e-ci inside it"
	@echo "  make release-tag VERSION=x.y.z  Create signed annotated release tag vX.Y.Z"
	@echo ""
	@echo "Android"
	@echo "  make android-connect  Auto-discover and connect to device via adb mdns"
	@echo "  make android-dev      Hot-reload on device via Wi-Fi (auto-connects)"
	@echo "  make android-build    Build Android APK"
	@echo "  make android-sign-debug   Sign APK to bin/fini.apk using debug keystore"
	@echo "  make android-install-debug  Install bin/fini.apk on connected device"
	@echo "  make android-launch   Launch app on connected device"
	@echo "  make android-devices  List connected ADB devices"
	@echo ""

# ── Linux ─────────────────────────────────────────────────────────────────────

dev:
	npm run tauri dev -- app

build:
	npm run tauri build

mcp:
	./src-tauri/target/debug/fini mcp

# Run the real-app e2e lane locally.
e2e:
	npm run test:e2e

# Run the same lane under CI settings.
e2e-ci:
	npm run test:e2e:ci

# Build/update the container image used for CI-style local e2e runs.
e2e-image:
	podman build --target test -t fini-e2e .

# Run the headless e2e tier inside the cached Podman image.
e2e-build:
	podman image exists fini-e2e || podman build --target test -t fini-e2e .
	podman run --rm fini-e2e

release-tag:
	@test -n "$(VERSION)" || (echo "VERSION is required. Use: make release-tag VERSION=x.y.z" && exit 1)
	@tag="v$(VERSION)"; \
	git fetch origin main --tags --force; \
	main_commit="$$(git rev-parse origin/main)"; \
	current_commit="$$(git rev-parse HEAD)"; \
	if [ "$$current_commit" != "$$main_commit" ]; then \
	  echo "HEAD must match origin/main before creating a release tag"; \
	  echo "HEAD=$$current_commit"; \
	  echo "origin/main=$$main_commit"; \
	  exit 1; \
	fi; \
	if git rev-parse -q --verify "refs/tags/$$tag" >/dev/null; then \
	  echo "Tag already exists: $$tag"; \
	  exit 1; \
	fi; \
	git -c user.email="v.ruzhentsov@gmail.com" -c user.signingkey="199DFE796EA43C00" tag -s -a "$$tag" -m "$$tag"; \
	git tag -v "$$tag"; \
	echo "Created signed annotated tag $$tag"; \
	echo "Push with: git push origin $$tag"

# ── Android ───────────────────────────────────────────────────────────────────

DEVICE_ADDRESS = $(shell adb mdns services 2>/dev/null | grep '_adb-tls-connect' | head -1 | awk '{print $$NF}')
DEVICE_IP      = $(firstword $(subst :, ,$(DEVICE_ADDRESS)))
HOST_IP        = $(shell ip route get $(DEVICE_IP) 2>/dev/null | grep -oP 'src \K\S+' | head -1)
ANDROID_UNSIGNED_APK = src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk
ANDROID_SIGNED_APK = bin/fini.apk
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

android-install-debug:
	@test -f "$(ANDROID_SIGNED_APK)" || (echo "Signed APK not found: $(ANDROID_SIGNED_APK). Run make android-sign-debug first." && exit 1)
	adb install -r "$(ANDROID_SIGNED_APK)"

android-launch:
	adb shell am start -n com.fini.app/.MainActivity

android-devices:
	adb devices
