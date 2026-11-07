#!/usr/bin/env sh
set -eu

# ble_gatt::backend::linux::LinuxBackend connects to the D-Bus system bus,
# and each Linux computer has one. With no bus, the app goes on a path that
# no user sees. bluetoothd cannot run here: it needs an AF_BLUETOOTH
# management socket of the kernel, which a container does not give (also with
# NET_ADMIN and NET_RAW). Thus `org.bluez` is not on this bus, and the app
# shows the usual "adapter unavailable" error. This script starts the bus.
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
  # Prove that the bus answers, not only that the socket file is there.
  # `set -e` is on, so a failed dbus-send must not stop the script before
  # the next line shows the result.
  dbus_send_status=0
  dbus-send --system --print-reply --dest=org.freedesktop.DBus \
    /org/freedesktop/DBus org.freedesktop.DBus.ListNames \
    > /tmp/dbus-listnames.log 2>&1 || dbus_send_status=$?
  echo "[e2e-runner] dbus-send exit=$dbus_send_status (see /tmp/dbus-listnames.log if this run fails)"
else
  echo "[e2e-runner] WARNING: no D-Bus system bus socket after 5 s; the LinuxBackend of ble_gatt cannot connect" >&2
fi
# The run continues in the two cases. With no bus, a Bluetooth connection
# must fail quickly and stop nothing else. If the app stops, that is a defect.

Xvfb :99 -screen 0 1280x1024x24 -nolisten tcp &
xvfb_pid=$!

cleanup() {
  kill "$xvfb_pid" >/dev/null 2>&1 || true
  kill "$dbus_pid" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

sleep 1

# One Playwright run: `cli`, `ui` and `multi-device`. The multi-device specs
# with `testDeviceChannel: ChannelKind.Bluetooth` give their processes
# FINI_BLE_MOCK_BROKER. Then `ble-gatt` uses the mock radio, not
# LinuxBackend::new(), so these processes need no D-Bus bus and no bluetoothd.
status=0
DISPLAY=:99 npm run test:e2e:ci || status=$?

# The run stops its own app processes. A fixture that failed before its
# teardown, or a test killed by its own time limit, can leave some; they
# keep their listeners, so stop them before the container ends.
pkill -f '/usr/local/bin/fini-app' 2>/dev/null || true
pkill -f '/usr/local/bin/ble-mock-broker' 2>/dev/null || true
exit "$status"
