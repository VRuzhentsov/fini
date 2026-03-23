-include .env
export

.PHONY: help dev build mcp android-dev android-build android-devices

help:
	@echo ""
	@echo "Linux"
	@echo "  make dev              Hot-reload dev app (Vite HMR + Rust watch)"
	@echo "  make build            Release build"
	@echo "  make mcp              Run MCP server (debug binary)"
	@echo ""
	@echo "Android"
	@echo "  make android-dev      Hot-reload on device via Wi-Fi (uses DEVICE_ADDRESS)"
	@echo "  make android-build    Build Android APK"
	@echo "  make android-devices  List connected ADB devices"
	@echo ""
	@echo "Set DEVICE_ADDRESS in .env, e.g.: DEVICE_ADDRESS=192.168.x.x:xxxxx"
	@echo ""

# ── Linux ─────────────────────────────────────────────────────────────────────

dev:
	npm run tauri dev

build:
	npm run tauri build

mcp:
	./src-tauri/target/debug/fini mcp

# ── Android ───────────────────────────────────────────────────────────────────

DEVICE_IP   = $(firstword $(subst :, ,$(DEVICE_ADDRESS)))
HOST_IP     = $(shell ip route get $(DEVICE_IP) 2>/dev/null | grep -oP 'src \K\S+' | head -1)

android-dev:
ifndef DEVICE_ADDRESS
	$(error DEVICE_ADDRESS not set — add it to .env)
endif
	adb connect $(DEVICE_ADDRESS)
	npm run tauri android dev -- --host $(HOST_IP)

android-build:
	npm run tauri android build

android-devices:
	adb devices
