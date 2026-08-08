package dev.blegatt

/**
 * Native callbacks implemented in Rust (`ble-gatt/src/backend/android.rs`,
 * `#[no_mangle] extern "system" fn Java_dev_blegatt_NativeKt_<name>`).
 * Declared as top-level functions (not class members) so Kotlin compiles
 * them as static methods on `NativeKt` — a fixed, unmangled JNI symbol name
 * `BleGattBridge`'s own instance methods would not have.
 *
 * Every callback carries `nativeHandle`, the `jlong` a `BleGattBridge` was
 * constructed with: a raw pointer to the Rust-side channel state
 * (`Box::into_raw` on the Rust side; reconstructed with
 * `&*(native_handle as *const _)` in each callback, never taking ownership).
 * This is the standard stateful-JNI-callback pattern — see the module doc
 * comment on `android.rs` for the full contract.
 */
/// `manufacturerData` and `serviceData` are passed pre-flattened rather than
/// as Java maps: building a `java.util.HashMap` across JNI costs several
/// reflective calls per entry, and the Rust side has to walk it back out
/// again. Parallel key/value arrays keep the boundary to one array copy per
/// side. `serviceDataUuids` holds UUID strings; the value arrays line up
/// index-for-index with their key arrays.
external fun onPeerDiscovered(
    nativeHandle: Long,
    generation: Long, advertisedServiceUuids: Array<String>, address: String, name: String?, rssi: Int,
    manufacturerIds: IntArray, manufacturerValues: Array<ByteArray>,
    serviceDataUuids: Array<String>, serviceDataValues: Array<ByteArray>,
)

/// `fromServer` distinguishes an inbound central (our GATT server) from our
/// own outbound connection. Without it Rust cannot tell the two apart, and a
/// backend used in both roles treats its own dial-out as an arriving peer.
external fun onConnected(nativeHandle: Long, address: String, fromServer: Boolean)

external fun onDisconnected(nativeHandle: Long, address: String, fromServer: Boolean)

/// Reports the ATT MTU actually negotiated for a connection. Fires from
/// `BluetoothGattCallback.onMtuChanged` after the explicit `requestMtu`
/// issued on connect — the peer decides the final value, so this is the
/// only trustworthy source for it.
external fun onMtuChanged(nativeHandle: Long, address: String, mtu: Int)

external fun onCharacteristicRead(
    nativeHandle: Long,
    requestId: Long, address: String, characteristicUuid: String, value: ByteArray, success: Boolean
)

external fun onCharacteristicWriteResult(
    nativeHandle: Long,
    requestId: Long, address: String, characteristicUuid: String, success: Boolean
)

external fun onCharacteristicChanged(
    nativeHandle: Long, address: String, characteristicUuid: String, value: ByteArray
)

external fun onServerCharacteristicWritten(
    // `session` identifies the server-side peer; see `onServerSubscribed`.
    nativeHandle: Long, address: String, characteristicUuid: String, value: ByteArray,
    session: Long,
)

/// Asynchronous advertise outcome. `startAdvertising` returns before Android
/// has decided, so this is the only signal that the advertisement is really
/// live.
external fun onAdvertiseResult(nativeHandle: Long, success: Boolean, errorCode: Int)

/// Asynchronous scan-start failure, for the same reason.
external fun onScanFailed(nativeHandle: Long, generation: Long, errorCode: Int)

/// A remote central enabled notifications on one of our server
/// characteristics. This — not the physical connection — is when the
/// peripheral-role peer becomes usable, because it is the point at which
/// the notify path back to it exists.
/// Completion of a queued server notification. Android reports the real
/// send status here, not from `notifyCharacteristicChanged`.
external fun onNotifySent(nativeHandle: Long, requestId: Long, success: Boolean)

external fun onServerSubscribed(nativeHandle: Long, address: String, session: Long)

external fun onSubscribed(
    nativeHandle: Long,
    requestId: Long, address: String, characteristicUuid: String, success: Boolean,
)
