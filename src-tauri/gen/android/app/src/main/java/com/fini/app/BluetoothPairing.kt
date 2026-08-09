package com.fini.app

import android.Manifest
import android.app.Activity
import android.bluetooth.BluetoothManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat

/**
 * Bluetooth OS-pairing/permission preconditions for `transport::ble`'s
 * dial-role check.
 *
 * Deliberately separate from `dev.blegatt.BleGattBridge`: these are Fini
 * pairing preconditions (mirrors the Linux transport's own direct
 * `bluetoothctl` call, which likewise bypasses `ble_gatt::backend::linux`),
 * not part of ble-gatt's reusable GATT transport surface. Called from Rust
 * via raw JNI (`services::android_context`), so this is a plain `object`
 * with `@JvmStatic`, not a Tauri plugin.
 */
object BluetoothPairing {
    private const val BLUETOOTH_PERMISSION_REQUEST_CODE = 4210

    private fun requiredPermissions(): Array<String> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            arrayOf(
                Manifest.permission.BLUETOOTH_SCAN,
                Manifest.permission.BLUETOOTH_CONNECT,
                Manifest.permission.BLUETOOTH_ADVERTISE,
            )
        } else {
            arrayOf(Manifest.permission.ACCESS_FINE_LOCATION)
        }

    @JvmStatic
    fun hasPermissions(context: Context): Boolean =
        requiredPermissions().all {
            ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED
        }

    /**
     * Fires the OS permission dialog if not already granted. Fire-and-forget
     * from Rust's side: there is no synchronous grant result, only the
     * eventual system dialog outcome once the user responds to it.
     *
     * Must only be called from a genuine user action -- today, the
     * Bluetooth toggle in Device settings
     * (`device_connection_set_bluetooth_transport`) -- never at app
     * startup. Requesting unprompted on every launch is what this was
     * originally wired up as in `MainActivity.onCreate`; moved here and
     * made call-site-triggered instead, so the OS prompt only appears when
     * the user has actually opted into Bluetooth.
     */
    @JvmStatic
    fun requestPermissionsIfNeeded(context: Context) {
        if (hasPermissions(context)) return
        val activity = context as? Activity ?: return
        activity.runOnUiThread {
            ActivityCompat.requestPermissions(
                activity,
                requiredPermissions(),
                BLUETOOTH_PERMISSION_REQUEST_CODE,
            )
        }
    }

    /**
     * Tri-state on purpose: `null` means the check itself couldn't be
     * completed (no adapter, permission denied, `bondedDevices` itself
     * unavailable) -- distinct from a confirmed `false`, which means the
     * query ran and this address genuinely isn't in the bonded set. Callers
     * that just want a plain fail-closed bool should collapse `null` to
     * `false` themselves (mirroring
     * `services::android_context::call_static_context_string_to_bool`'s own
     * contract) rather than this method doing it silently -- collapsing it
     * here is exactly what let a transient permission/adapter hiccup look
     * identical to "definitely not bonded" to callers that need to tell the
     * difference (see `device_connection::commands::persist_bluetooth_address_and_maybe_enable`).
     */
    @JvmStatic
    fun isBonded(context: Context, address: String): Boolean? {
        return try {
            val manager = context.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager
            val adapter = manager?.adapter ?: return null
            adapter.bondedDevices?.any { it.address.equals(address, ignoreCase = true) }
        } catch (e: SecurityException) {
            // BLUETOOTH_CONNECT not granted (API 31+) -- inconclusive, not
            // a confirmed "not bonded".
            null
        }
    }

    /**
     * Fires the OS-level bonding request for [address]. Fire-and-forget from
     * Rust's side, same as [requestPermissionsIfNeeded]: `createBond()` only
     * reports whether the request was *accepted*, not whether bonding
     * eventually succeeds -- that plays out via the system's own pairing
     * UI/broadcast on its own schedule, well after this call returns.
     * [isBonded] is the only source of truth callers should poll afterward.
     *
     * Requires [BLUETOOTH_CONNECT]/[ACCESS_FINE_LOCATION] (whichever
     * [requiredPermissions] resolves to on this SDK level); like [isBonded],
     * fails closed on `SecurityException` rather than crashing.
     */
    @JvmStatic
    fun createBond(context: Context, address: String) {
        try {
            val manager = context.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager
            val adapter = manager?.adapter ?: return
            val device = adapter.getRemoteDevice(address)
            device.createBond()
        } catch (e: SecurityException) {
            // Permission not granted -- nothing more to do here;
            // requestPermissionsIfNeeded is the caller's job, not this one's.
        } catch (e: IllegalArgumentException) {
            // Malformed address -- nothing to bond.
        }
    }
}
