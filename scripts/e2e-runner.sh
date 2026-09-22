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

# Which lanes to run, space separated. All of them by default, which is what
# CI and the PR gate want.
#
# Selectable because the lanes share one container and run one after another,
# so a lane that fails here has two possible explanations: it is broken, or
# the lane before it left something behind. Telling those apart means running
# it on its own, and that used to require editing this file.
lanes="${FINI_E2E_LANES:-main loopback ble}"
has_lane() { case " $lanes " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

# Each lane spawns its own app processes and stops them itself. When one
# does not -- a fixture that failed before its teardown, a test killed by
# its own timeout -- the survivors keep their listeners and keep dialling,
# and the next lane inherits a peer it never asked for.
#
# Measured, not assumed: on one image and one commit, the loopback lane
# passed alone in twelve seconds and failed in a minute when it followed
# the main lane. Lanes share a container, so the only thing between them
# is this.
reap_actors() {
  pkill -f '/usr/local/bin/fini-app' 2>/dev/null || true
  pkill -f '/usr/local/bin/ble-mock-broker' 2>/dev/null || true
  # Long enough for the kernel to release the listeners before the next
  # lane binds its own.
  sleep 3
}

status=0
if has_lane main; then
  DISPLAY=:99 npm run test:e2e:ci || status=$?
  reap_actors
fi
if has_lane loopback; then
  DISPLAY=:99 npm run test:e2e:ci:loopback || status=$?
  reap_actors
fi
# actors-ble's actors set FINI_BLE_MOCK_BROKER, which routes ble.rs's
# backend() to the cross-process mock radio instead of LinuxBackend::new()
# -- so unlike the other two lanes, this one needs neither the D-Bus bus
# started above nor bluetoothd (which, per this script's own comment,
# can't run here anyway).
if has_lane ble; then
  DISPLAY=:99 npm run test:e2e:ci:ble || status=$?
fi
exit "$status"
