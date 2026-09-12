#!/usr/bin/env bash
set -euo pipefail

# Which installed package to assert against. Defaults to the release
# application id, which is what the emulator CI lane builds. A debug-buildType
# APK installs under a different id (see tauri.conf.json's
# bundle.android.debugApplicationIdSuffix), so running this against one needs
# FINI_E2E_PACKAGE set to match -- otherwise every assertion below would look
# at a package that isn't the one just installed.
PACKAGE="${FINI_E2E_PACKAGE:-com.fini.app}"

# `-r` so re-running against an already-installed build reinstalls instead of
# failing with INSTALL_FAILED_ALREADY_EXISTS.
adb install -r "$ANDROID_E2E_APK"
adb shell pm grant "$PACKAGE" android.permission.POST_NOTIFICATIONS
# The activity class stays com.fini.app.MainActivity regardless of the
# application id: applicationIdSuffix changes the package identity, not the
# Kotlin namespace the activity is declared in.
adb shell am start -n "$PACKAGE/com.fini.app.MainActivity"

echo "Waiting for app process..."
for i in $(seq 1 30); do
  pid=$(adb shell pidof "$PACKAGE" 2>/dev/null || true)
  if [ -n "$pid" ]; then echo "App process alive: $pid"; break; fi
  if [ "$i" -eq 30 ]; then
    echo "App process did not start" >&2
    adb logcat -d -s AndroidRuntime:E >&2
    exit 1
  fi
  sleep 1
done

echo "Waiting for notification channel fini.reminders..."
for i in $(seq 1 20); do
  if adb shell dumpsys notification 2>/dev/null | grep -q "fini.reminders"; then
    echo "Notification channel fini.reminders confirmed"
    break
  fi
  if [ "$i" -eq 20 ]; then
    echo "Notification channel fini.reminders not found" >&2
    adb shell dumpsys notification | grep -A3 "$PACKAGE" >&2 || true
    exit 1
  fi
  sleep 1
done

echo "Android smoke + notification channel assertions passed"
