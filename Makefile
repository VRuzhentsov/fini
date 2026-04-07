-include .env
export

.PHONY: help dev build mcp android-connect android-dev android-build android-sign-debug android-install-debug android-launch android-devices

help:
	@echo ""
	@echo "Linux"
	@echo "  make dev              Hot-reload dev app (Vite HMR + Rust watch)"
	@echo "  make build            Release build"
	@echo "  make mcp              Run MCP server (debug binary)"
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
	npm run tauri dev

build:
	npm run tauri build

mcp:
	./src-tauri/target/debug/fini mcp

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
