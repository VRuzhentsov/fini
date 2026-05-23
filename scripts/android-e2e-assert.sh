#!/usr/bin/env bash
set -euo pipefail

adb install "$ANDROID_E2E_APK"
adb shell pm grant com.fini.app android.permission.POST_NOTIFICATIONS
adb shell am start -n com.fini.app/.MainActivity

echo "Waiting for app process..."
for i in $(seq 1 30); do
  pid=$(adb shell pidof com.fini.app 2>/dev/null || true)
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
    adb shell dumpsys notification | grep -A3 "com.fini.app" >&2 || true
    exit 1
  fi
  sleep 1
done

echo "Android smoke + notification channel assertions passed"
