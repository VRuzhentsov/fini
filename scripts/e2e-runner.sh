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
mkdir -p /run/dbus
dbus-uuidgen --ensure
dbus-daemon --system --nofork --nopidfile &
dbus_pid=$!

for _ in $(seq 1 50); do
  [ -S /run/dbus/system_bus_socket ] && break
  sleep 0.1
done

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
