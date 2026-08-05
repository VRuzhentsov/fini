#!/usr/bin/env sh
set -eu

# A live D-Bus system bus is what ble_gatt::backend::linux::LinuxBackend
# actually connects to on every real Linux deployment target. Without one
# running at all, that connection attempt takes a path no real target
# ever hits (the bus itself unreachable, not merely BlueZ absent from
# it) -- not representative of anything users see. bluetoothd itself
# can't run here (it needs a kernel-level AF_BLUETOOTH management socket
# containers don't expose, confirmed: it fails identically with
# NET_ADMIN/NET_RAW granted), so `org.bluez` genuinely won't be on this
# bus -- that absence is real and expected in this environment, and the
# app already handles it as an ordinary "adapter unavailable" error. The
# bus itself existing is the part worth fixing.
echo "[e2e-runner] starting D-Bus system bus"
mkdir -p /run/dbus
dbus-uuidgen --ensure || echo "[e2e-runner] WARNING: dbus-uuidgen --ensure failed (exit=$?)" >&2
dbus-daemon --system --nofork --nopidfile &
dbus_pid=$!

dbus_ready=0
for _ in $(seq 1 50); do
  if [ -S /run/dbus/system_bus_socket ]; then
    dbus_ready=1
    break
  fi
  sleep 0.1
done

if [ "$dbus_ready" = "1" ]; then
  echo "[e2e-runner] D-Bus system bus socket present -- dbus-daemon pid=$dbus_pid"
  # Prove the bus actually answers, not just that the socket file exists.
  # `set -e` is active, so this is guarded: a failing dbus-send must not
  # kill the script before the diagnostic below can report it.
  dbus_send_status=0
  dbus-send --system --print-reply --dest=org.freedesktop.DBus \
    /org/freedesktop/DBus org.freedesktop.DBus.ListNames \
    > /tmp/dbus-listnames.log 2>&1 || dbus_send_status=$?
  echo "[e2e-runner] dbus-send exit=$dbus_send_status (see /tmp/dbus-listnames.log if this run fails)"
else
  echo "[e2e-runner] WARNING: D-Bus system bus socket never appeared after 5s -- ble_gatt's LinuxBackend will fail its connection attempt, same as before this fix" >&2
fi
# Not fatal either way: the whole point of this fix is that a *missing*
# bus shouldn't be able to break anything downstream, only make BLE
# connection attempts fail cleanly and fast. If it's still fatal to the
# app after this, that itself is the finding.

Xvfb :99 -screen 0 1280x1024x24 -nolisten tcp &
xvfb_pid=$!

cleanup() {
  kill "$xvfb_pid" >/dev/null 2>&1 || true
  kill "$dbus_pid" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

sleep 1
status=0
DISPLAY=:99 npm run test:e2e:ci || status=$?
DISPLAY=:99 npm run test:e2e:ci:sim || status=$?
exit "$status"
