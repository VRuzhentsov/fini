package dev.blegatt

import android.bluetooth.BluetoothAdapter
import java.util.concurrent.ConcurrentHashMap
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothGattServer
import android.bluetooth.BluetoothGattServerCallback
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.content.pm.PackageManager
import android.os.Handler
import android.os.Looper
import android.os.ParcelUuid
import android.util.Log
import java.util.UUID

private const val TAG = "BleGattBridge"

/**
 * Real Android BLE GATT central + peripheral implementation. Called from
 * Rust via raw JNI (`env.call_method(...)`) — not through Tauri's mobile
 * plugin `Invoke`/`@Command` machinery, so this class has no `Plugin`
 * base class or `@TauriPlugin` annotation. That is deliberate: it is what
 * keeps a hypothetical non-Tauri Android Rust consumer able to reuse this
 * file (copy it + the manifest permissions into their own app, no Tauri
 * Android runtime required) rather than being structurally locked into
 * Tauri's IPC path. See docs/adr/0002 for the full rationale.
 *
 * Permission checks are deliberately not performed here — declaring
 * `<uses-permission>` in this module's AndroidManifest.xml is this
 * bridge's responsibility; requesting the *runtime* grant from the user is
 * the consuming app's. An operation attempted without a granted permission
 * throws `SecurityException`, which Rust surfaces as a `BleError::Gatt`.
 */
class BleGattBridge(private val context: Context, private val nativeHandle: Long) {

    private val bluetoothManager =
        context.getSystemService(Context.BLUETOOTH_SERVICE) as BluetoothManager
    private val adapter: BluetoothAdapter? = bluetoothManager.adapter

    init {
        try {
            val hasBleFeature = context.packageManager.hasSystemFeature(PackageManager.FEATURE_BLUETOOTH_LE)
            Log.d(
                TAG,
                "init: FEATURE_BLUETOOTH_LE=$hasBleFeature adapter=${adapter != null} " +
                    "adapter.isEnabled=${adapter?.isEnabled} " +
                    "bluetoothLeScanner=${adapter?.bluetoothLeScanner != null} " +
                    "isMultipleAdvertisementSupported=${adapter?.isMultipleAdvertisementSupported} " +
                    "bluetoothLeAdvertiser=${adapter?.bluetoothLeAdvertiser != null}"
            )
        } catch (e: Exception) {
            Log.e(TAG, "init: capability probe threw", e)
        }
    }

    // --- Central role state ---
    // Concurrency note: JNI calls arrive on whichever Tokio worker thread
    // made them, and Bluetooth callbacks arrive on Binder threads. The JVM
    // does not serialize calls on a shared object, so every mutable
    // collection below is concurrent — plain HashMaps here could lose a GATT
    // entry or corrupt their internal state outright, leaving a Rust
    // connection that had connected successfully unable to read, write or
    // disconnect. Concurrent collections rather than a lock because several
    // of these are touched around blocking Android calls, where holding one
    // invites deadlock.
    @Volatile
    private var scanCallback: ScanCallback? = null
    /// Guards the check-and-publish transitions on client GATT state.
    ///
    /// `ConcurrentHashMap` makes each individual access safe but cannot make
    /// "verify this GATT still owns the address, then publish" atomic — and
    /// that pair is exactly where a cancellation and retry can interleave.
    private val gattLock = Any()
    /// Schedules GATT-busy retries (see `GATT_BUSY_MAX_RETRIES`) off
    /// whichever thread the rejected call happened to run on -- often a
    /// Tokio worker thread arriving via JNI, which must not itself block on
    /// a sleep. `gattLock` is bridge-wide (covers every connected peer), so
    /// a retry's backoff must also happen without holding it, or one peer's
    /// busy backoff would stall every other peer's GATT operations too.
    private val retryHandler = Handler(Looper.getMainLooper())
    private val connectedGatts = ConcurrentHashMap<String, BluetoothGatt>()
    /// Rust's session id for the GATT currently owning each address.
    ///
    /// Rust cannot make its own check-and-act atomic: it must not hold the
    /// `connections` mutex across a JNI call, since the Kotlin callbacks
    /// take `gattLock` and then call back into Rust for that same mutex —
    /// holding it here would close that cycle into a deadlock. So ownership
    /// is verified on *this* side instead, under the same lock that resolves
    /// the GATT, which makes validation and initiation one transition.
    private val gattSessions = ConcurrentHashMap<String, Long>()

    /// Session id per central attached to our GATT server.
    ///
    /// Minted here rather than in Rust so that validating a session and
    /// acting on it can be one transition under this monitor. Rust cannot
    /// hold its own map across the JNI call — a Binder thread inside
    /// `onDescriptorWriteRequest` already holds this monitor and calls into
    /// Rust for that map, so holding it here would close the cycle.
    private val serverSessions = ConcurrentHashMap<String, Long>()
    private var nextServerSession: Long = 1
    /// Request ids for the one outstanding read and write Android allows per
    /// connection, echoed back on completion.
    ///
    /// Needed because Android's completion callbacks carry no identity of
    /// their own: routing on address alone let a delayed callback from a
    /// *cancelled* operation resolve whichever operation replaced it, and
    /// the characteristic UUID is no help — every datagram fragment targets
    /// the same characteristic.
    /// FIFO per address, not a single slot. A slot is replaceable: if read A
    /// is cancelled and read B starts before A's delayed callback arrives, B
    /// overwrites the slot and A's callback then echoes *B's* id — handing
    /// Rust A's bytes as B's result, which is the corruption the ids were
    /// added to prevent. Android completes these in issue order, so popping
    /// the oldest outstanding id matches each callback to its own operation.
    /// Keyed by address+characteristic, since a connection may have several
    /// subscriptions outstanding — unlike reads and writes, where Android
    /// permits one at a time.
    private val pendingSubscribeIds = ConcurrentHashMap<String, ArrayDeque<Long>>()
    private val pendingReadIds = ConcurrentHashMap<String, ArrayDeque<Long>>()
    private val pendingWriteIds = ConcurrentHashMap<String, ArrayDeque<Long>>()
    private val pendingCharacteristics =
        ConcurrentHashMap<String, ConcurrentHashMap<String, BluetoothGattCharacteristic>>()

    // --- Peripheral role state ---
    @Volatile
    private var gattServer: BluetoothGattServer? = null
    @Volatile
    private var advertiseCallback: AdvertiseCallback? = null
    private val serverCharacteristics = ConcurrentHashMap<String, BluetoothGattCharacteristic>()
    /// Values are concurrent sets: a CCCD write and a notify can touch the
    /// same characteristic's subscriber set from different threads.
    private val subscribedDevices = ConcurrentHashMap<String, MutableSet<BluetoothDevice>>()

    /// Android allows exactly one outstanding server notification at a time:
    /// `notifyCharacteristicChanged` only reports that a send was
    /// *initiated*, and the real status arrives on `onNotificationSent`.
    /// Issuing the next fragment before that lands makes the stack either
    /// reject it or transmit the characteristic's already-overwritten value,
    /// which silently corrupts a multi-fragment datagram. So notifications
    /// are queued and drained strictly one at a time.
    private class PendingNotify(
        val device: BluetoothDevice,
        val characteristicUuid: String,
        val value: ByteArray,
        val requestId: Long,
    )

    /// Chunks of an ATT Prepare Write, keyed by device+characteristic and
    /// ordered by offset. A central uses this for values larger than one MTU
    /// and for reliable writes; applying each chunk on arrival published a
    /// series of partial values as if each were a complete write.
    private val preparedWrites =
        ConcurrentHashMap<String, java.util.TreeMap<Int, ByteArray>>()

    private val notifyQueue = ArrayDeque<PendingNotify>()
    private var notifyInFlight: PendingNotify? = null

    /// Incremented every time a GATT server is opened. A notification
    /// callback from a previous server can still be executing when a new one
    /// is advertising; without this it would consume the *new* server's
    /// in-flight entry, report that request's outcome using the old
    /// request's status, and restart the pump while the new send is still
    /// active — defeating the one-in-flight rule that keeps fragmented
    /// datagrams intact.
    @Volatile
    private var serverGeneration: Long = 0

    fun hasCentralSupport(): Boolean = adapter?.bluetoothLeScanner != null

    fun hasPeripheralSupport(): Boolean =
        adapter?.isMultipleAdvertisementSupported == true && adapter.bluetoothLeAdvertiser != null

    // ---------------------------------------------------------------
    // Central role
    // ---------------------------------------------------------------

    /// Returns false if a scan is already running. There is one
    /// `scanCallback` and one discovery sender, so a second concurrent scan
    /// would silently replace the first — and dropping either stream would
    /// then cancel the wrong one. Refusing is honest; per-scan ownership can
    /// come later if a caller genuinely needs concurrent scans.
    /// `generation` is echoed back on every callback so Rust can discard
    /// results from a scan it has already replaced. A callback already
    /// executing when a scan is stopped can otherwise land after the next
    /// scan installs itself, and be attributed to it.
    fun startScan(serviceUuid: String, generation: Long): Boolean {
        if (scanCallback != null) {
            Log.w(TAG, "startScan refused: a scan is already active")
            return false
        }
        val scanner = adapter?.bluetoothLeScanner ?: return false
        val target = UUID.fromString(serviceUuid)
        val settings = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
            .build()
        val callback = object : ScanCallback() {
            override fun onScanResult(callbackType: Int, result: ScanResult) {
                val record = result.scanRecord
                val uuids = record?.serviceUuids
                Log.d(TAG, "onScanResult: ${result.device.address} name=${result.device.name} advertisedUuids=$uuids")
                if (uuids == null || !uuids.contains(ParcelUuid(target))) {
                    return
                }

                // Flatten both advertisement maps into parallel arrays — see
                // Native.kt for why the JNI boundary takes them this way.
                val manufacturer = record.manufacturerSpecificData
                val manufacturerIds = IntArray(manufacturer?.size() ?: 0)
                val manufacturerValues = arrayOfNulls<ByteArray>(manufacturer?.size() ?: 0)
                if (manufacturer != null) {
                    for (i in 0 until manufacturer.size()) {
                        manufacturerIds[i] = manufacturer.keyAt(i)
                        manufacturerValues[i] = manufacturer.valueAt(i) ?: ByteArray(0)
                    }
                }

                val serviceData = record.serviceData ?: emptyMap()
                val serviceDataUuids = arrayOfNulls<String>(serviceData.size)
                val serviceDataValues = arrayOfNulls<ByteArray>(serviceData.size)
                serviceData.entries.forEachIndexed { i, entry ->
                    serviceDataUuids[i] = entry.key.uuid.toString()
                    serviceDataValues[i] = entry.value ?: ByteArray(0)
                }

                @Suppress("UNCHECKED_CAST")
                onPeerDiscovered(
                    nativeHandle,
                    generation,
                    // Already parsed to match the scan filter; passing it on
                    // costs nothing and is what Linux and the mock report.
                    (uuids ?: emptyList()).map { it.uuid.toString() }.toTypedArray(),
                    result.device.address,
                    result.device.name,
                    result.rssi,
                    manufacturerIds,
                    manufacturerValues as Array<ByteArray>,
                    serviceDataUuids as Array<String>,
                    serviceDataValues as Array<ByteArray>,
                )
            }

            override fun onScanFailed(errorCode: Int) {
                // Reported asynchronously, after Rust already has a
                // discovery stream. Without forwarding it, a scan that never
                // started is indistinguishable from one that found nothing.
                Log.e(TAG, "onScanFailed: errorCode=$errorCode")
                onScanFailed(nativeHandle, generation, errorCode)
            }
        }
        Log.d(TAG, "startScan: serviceUuid=$serviceUuid scanner=${scanner != null}")
        scanCallback = callback
        // Matching against the raw advertised UUID list in onScanResult
        // above, rather than a native android.bluetooth.le.ScanFilter — a
        // filtered scan against a Root Canal virtual controller (Android
        // emulator's Bluetooth simulator) delivered zero results in testing
        // even when the target peer was confirmed advertising and the
        // filter parameters were accepted (status=0); an unfiltered scan
        // with matching done here works identically on real adapters and
        // sidesteps whatever that native-filter gap is. See docs/adr/0002.
        scanner.startScan(emptyList(), settings, callback)
        return true
    }

    fun stopScan() {
        // Clear ownership unconditionally. Returning early when the scanner
        // is gone (Bluetooth switched off before the scan stream was
        // dropped) left `scanCallback` set forever, and every later
        // `startScan` then rejected the scan as already active — scanning
        // stayed dead for the lifetime of this bridge even after Bluetooth
        // came back.
        val scanner = adapter?.bluetoothLeScanner
        val callback = scanCallback
        scanCallback = null
        if (scanner != null && callback != null) {
            try {
                scanner.stopScan(callback)
            } catch (e: Exception) {
                Log.w(TAG, "stopScan: error stopping scan", e)
            }
        }
    }

    fun connect(address: String, session: Long) {
        val device = adapter?.getRemoteDevice(address)
        if (device == null) {
            // No adapter at all. Rust has already stored connected_tx, so
            // returning silently strands it forever — the same failure the
            // advertiser path had. Report the link as never established.
            Log.w(TAG, "connect: no Bluetooth adapter available for $address")
            onDisconnected(nativeHandle, address, false)
            return
        }
        val callback = object : BluetoothGattCallback() {
            override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
                when (newState) {
                    BluetoothProfile.STATE_CONNECTED -> {
                        // Ask for the largest MTU the spec allows before
                        // discovering services. The peer decides the real
                        // value and reports it via onMtuChanged; until then
                        // Rust keeps the conservative 23-byte default.
                        gatt.requestMtu(MAX_ATT_MTU)
                        if (!gatt.discoverServices()) {
                            // No callback is guaranteed when this returns
                            // false, so the pending connect would hang
                            // forever. Report the link as gone instead.
                            Log.w(TAG, "discoverServices did not start for $address")
                            onDisconnected(nativeHandle, address, false)
                            gatt.close()
                        }
                    }
                    BluetoothProfile.STATE_DISCONNECTED -> {
                        // A cancelled connect can be retried before its old
                        // GATT's disconnect callback arrives. Removing by
                        // address alone would delete the *replacement* and
                        // then clear its Rust state, tearing down a
                        // connection that had just succeeded — so only the
                        // GATT still owning this address may do so.
                        // The removal *and* the Rust publication happen
                        // under `gattLock`. Deciding ownership, releasing,
                        // then acting leaves a window in which a retry
                        // installs a replacement — after which this callback
                        // deletes that replacement and reports it
                        // disconnected. Holding the lock across the JNI call
                        // is safe here: no Rust path calls into Kotlin while
                        // holding the `connections` mutex these callbacks
                        // take, so there is no cycle.
                        synchronized(gattLock) {
                            if (connectedGatts[address] === gatt) {
                                connectedGatts.remove(address)
                                gattSessions.remove(address)
                                pendingCharacteristics.remove(address)
                                clearPendingOperations(address)
                                // Fires for unsolicited drops too (out of
                                // range, peer powered off), which is the
                                // whole point of surfacing this to Rust
                                // rather than only reporting disconnects we
                                // initiated.
                                onDisconnected(nativeHandle, address, false)
                            } else {
                                Log.d(TAG, "onConnectionStateChange: $address superseded, not clearing state")
                            }
                        }
                        // Closed either way: a superseded GATT still needs
                        // releasing, it just must not take the live one's
                        // bookkeeping with it.
                        gatt.close()
                    }
                }
            }

            override fun onMtuChanged(gatt: BluetoothGatt, mtu: Int, status: Int): Unit = synchronized(gattLock) {
                if (connectedGatts[address] !== gatt) return
                if (status == BluetoothGatt.GATT_SUCCESS) {
                    Log.d(TAG, "onMtuChanged: $address negotiated mtu=$mtu")
                    onMtuChanged(nativeHandle, address, mtu)
                }
            }

            override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
                // Ownership and the state change it authorises happen under
                // one lock. Checking first and acting afterwards left a
                // window in which a cancellation could close this GATT and a
                // retry install its replacement, after which this callback
                // would publish its service map and resolve the
                // *replacement's* connect before that connection had
                // discovered anything. `closeConnection` and the disconnect
                // path take the same lock.
                // Ownership *and* the Rust publication under one lock.
                // Releasing between them let a cancellation plus retry
                // interleave, after which this callback resolved the
                // replacement's connect with its own service map — marking a
                // connection live before its services had been discovered.
                val superseded = synchronized(gattLock) {
                    when {
                        connectedGatts[address] !== gatt -> true
                        status != BluetoothGatt.GATT_SUCCESS -> {
                            // Previously ignored, so a failed discovery still
                            // resolved connect() successfully and handed back
                            // a connection with no usable characteristics.
                            Log.w(TAG, "onServicesDiscovered failed for $address status=$status")
                            connectedGatts.remove(address)
                            gattSessions.remove(address)
                            pendingCharacteristics.remove(address)
                            clearPendingOperations(address)
                            onDisconnected(nativeHandle, address, false)
                            false
                        }
                        else -> {
                            val byUuid = ConcurrentHashMap<String, BluetoothGattCharacteristic>()
                            for (service in gatt.services) {
                                for (characteristic in service.characteristics) {
                                    byUuid[characteristic.uuid.toString()] = characteristic
                                }
                            }
                            pendingCharacteristics[address] = byUuid
                            onConnected(nativeHandle, address, false)
                            false
                        }
                    }
                }
                if (superseded) {
                    Log.d(TAG, "onServicesDiscovered: $address superseded, ignoring")
                }
                // A superseded or failed GATT still needs releasing; only a
                // published, owned one stays open.
                if (superseded || status != BluetoothGatt.GATT_SUCCESS) {
                    gatt.close()
                }
            }

            @Suppress("DEPRECATION")
            override fun onCharacteristicRead(
                gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, status: Int
            ) {
                // Ownership first, and held: a superseded GATT's callback
                // must not even consume a queue entry, or it would pop the
                // live operation's id and strand it. Checking and then
                // popping as two steps left that window open.
                synchronized(gattLock) {
                if (connectedGatts[address] !== gatt) return
                val queue = pendingReadIds[address] ?: return
                val requestId = synchronized(queue) { queue.removeFirstOrNull() } ?: return
                onCharacteristicRead(
                    nativeHandle, requestId, address, characteristic.uuid.toString(),
                    characteristic.value ?: ByteArray(0), status == BluetoothGatt.GATT_SUCCESS
                )
                }
            }

            override fun onCharacteristicWrite(
                gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, status: Int
            ) {
                synchronized(gattLock) {
                if (connectedGatts[address] !== gatt) return
                val queue = pendingWriteIds[address] ?: return
                val requestId = synchronized(queue) { queue.removeFirstOrNull() } ?: return
                onCharacteristicWriteResult(
                    nativeHandle, requestId, address, characteristic.uuid.toString(),
                    status == BluetoothGatt.GATT_SUCCESS
                )
                }
            }

            @Suppress("DEPRECATION")
            override fun onCharacteristicChanged(
                gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic
            ) {
                // Ownership *and* the forwarding under one lock. Checking
                // and then publishing let a cancellation remove this GATT in
                // between, after which the payload lands in the replacement
                // connection's address-keyed stream and an old datagram
                // fragment is reassembled as current data.
                synchronized(gattLock) {
                    if (connectedGatts[address] !== gatt) {
                        Log.d(TAG, "onCharacteristicChanged: $address superseded, dropping payload")
                        return
                    }
                    onCharacteristicChanged(
                        nativeHandle,
                        address,
                        characteristic.uuid.toString(),
                        characteristic.value ?: ByteArray(0),
                    )
                }
            }

            override fun onDescriptorWrite(
                gatt: BluetoothGatt, descriptor: BluetoothGattDescriptor, status: Int
            ) {
                // A superseded GATT's CCCD completion would otherwise resolve
                // the *replacement's* pending subscribe with the old
                // operation's status — handing back a stream before
                // notifications are enabled, or failing a valid new setup.
                synchronized(gattLock) {
                if (connectedGatts[address] !== gatt) return
                val characteristicUuid = descriptor.characteristic.uuid.toString()
                // Tagged like reads and writes: a cancelled subscribe's
                // delayed callback would otherwise resolve the retry that
                // replaced it, handing back a stream before the CCCD write
                // completed or failing a valid attempt.
                val queue = pendingSubscribeIds["$address/$characteristicUuid"] ?: return
                val requestId = synchronized(queue) { queue.removeFirstOrNull() } ?: return
                onSubscribed(
                    nativeHandle,
                    requestId,
                    address,
                    characteristicUuid,
                    status == BluetoothGatt.GATT_SUCCESS,
                )
                }
            }
        }
        val gatt = device.connectGatt(context, false, callback)
        synchronized(gattLock) {
            connectedGatts[address] = gatt
            gattSessions[address] = session
        }
    }

    fun disconnect(address: String, session: Long): Unit = synchronized(gattLock) {
        // The operation that most needs this: a stale handle's disconnect
        // would otherwise drop the link that replaced it.
        if (gattSessions[address] != session) {
            Log.d(TAG, "disconnect: $address session superseded, ignoring")
            return
        }
        connectedGatts[address]?.disconnect()
    }

    private fun findCharacteristic(address: String, characteristicUuid: String): BluetoothGattCharacteristic? =
        pendingCharacteristics[address]?.get(characteristicUuid)

    /// Runnables for a GATT-busy retry that has been scheduled but has not
    /// fired yet -- present *only* for that window, never for an id whose
    /// underlying platform call already returned `true` and is genuinely
    /// in flight awaiting its real `onCharacteristicRead`/`onCharacteristicWrite`/
    /// `onDescriptorWrite` callback. Removed either by `cancelPendingOperation`
    /// or by the retry itself -- both under `gattLock`, see
    /// `cancelPendingOperation`'s own comment for why that matters.
    private val pendingRetries = ConcurrentHashMap<Long, Runnable>()

    /// Called by Rust when a pending read/write/subscribe future is dropped
    /// before its result arrived (a `timeout`, a losing `select!` branch) --
    /// see `PendingOpGuard`/`SubscribeGuard`'s Drop impls on the Rust side.
    /// Without this, a GATT-busy retry already scheduled via `retryHandler`
    /// when the rejection happened has no way to learn its request was
    /// abandoned: it fires anyway once the stack stops being busy,
    /// transmitting the abandoned payload over the air.
    ///
    /// Three P1 review findings shaped this method, in order:
    /// 1. Removing the id from the pending-id queue unconditionally
    ///    corrupts FIFO matching for an id whose call had *already* been
    ///    accepted by the platform and was genuinely in flight -- that
    ///    entry must survive untouched for the real callback to find.
    /// 2. Cancelling only the `Runnable` without also removing the queue
    ///    entry leaves a permanent orphan for an id that really was only
    ///    scheduled, never issued -- no real callback is ever coming for
    ///    it, so left in place it silently swallows the *next* same-kind
    ///    request's callback instead.
    /// 3. Removing the `pendingRetries` entry from inside the retry's own
    ///    `Runnable`, before it entered `synchronized(gattLock)`, left a
    ///    window where a concurrent cancellation could see nothing left
    ///    to cancel and return early while the retry -- already past that
    ///    point -- fired anyway. `gattLock` is the actual correctness
    ///    mechanism now: both this method and each `attempt*` (for
    ///    `attempt > 0`, see their own first line) perform their
    ///    `pendingRetries.remove` as the very first thing *inside* the
    ///    lock, so whichever side acquires it first is unconditionally
    ///    the one that gets to act. `Handler.removeCallbacks` below is
    ///    only an optimisation that skips a wasted dispatch in the common
    ///    case -- if the retry's `Runnable` ran anyway, its own
    ///    `pendingRetries.remove` would find nothing and abort the same
    ///    way `cancelPendingOperation` finding nothing here does.
    fun cancelPendingOperation(requestId: Long): Unit = synchronized(gattLock) {
        val retry = pendingRetries.remove(requestId) ?: return
        retryHandler.removeCallbacks(retry)
        for (queue in pendingWriteIds.values) synchronized(queue) { queue.remove(requestId) }
        for (queue in pendingReadIds.values) synchronized(queue) { queue.remove(requestId) }
        for (queue in pendingSubscribeIds.values) synchronized(queue) { queue.remove(requestId) }
    }

    /// Every failure path must report back. An early `return` here leaves
    /// the Rust side's oneshot unresolved and its caller awaiting forever —
    /// and unknown UUIDs or incomplete service discovery are ordinary
    /// errors, not exotic ones.
    fun readCharacteristic(
        address: String, characteristicUuid: String, requestId: Long, session: Long,
    ) = attemptRead(address, characteristicUuid, requestId, session, attempt = 0)

    /// Same GATT-busy retry as `attemptWrite`/`attemptSubscribe` — see
    /// `GATT_BUSY_MAX_RETRIES`'s doc comment. `gatt.readCharacteristic()` is
    /// exactly as subject to the one-outstanding-op rule as the write path.
    private fun attemptRead(
        address: String, characteristicUuid: String, requestId: Long, session: Long, attempt: Int,
    ): Unit = synchronized(gattLock) {
        // Must be the very first thing done under gattLock -- see
        // cancelPendingOperation's doc comment (finding 3). A `null` here
        // means cancellation already claimed this attempt; Rust has
        // stopped listening, so there is nothing left to do.
        if (attempt > 0 && pendingRetries.remove(requestId) == null) return
        if (gattSessions[address] != session) {
            if (attempt > 0) {
                pendingReadIds[address]?.let { queue -> synchronized(queue) { queue.remove(requestId) } }
            }
            onCharacteristicRead(nativeHandle, requestId, address, characteristicUuid, ByteArray(0), false)
            return
        }
        val gatt = connectedGatts[address]
        val characteristic = findCharacteristic(address, characteristicUuid)
        val queue = pendingReadIds.getOrPut(address) { ArrayDeque() }
        if (attempt == 0) {
            synchronized(queue) { queue.addLast(requestId) }
        }
        if (gatt == null || characteristic == null) {
            Log.w(TAG, "readCharacteristic could not start: $address/$characteristicUuid")
            // Remove this id specifically: another operation's may be queued
            // ahead of it and must not be consumed by this failure.
            synchronized(queue) { queue.remove(requestId) }
            onCharacteristicRead(nativeHandle, requestId, address, characteristicUuid, ByteArray(0), false)
            return
        }
        if (!gatt.readCharacteristic(characteristic)) {
            if (attempt < GATT_BUSY_MAX_RETRIES) {
                Log.w(
                    TAG,
                    "readCharacteristic rejected by the stack, retrying " +
                        "(${attempt + 1}/$GATT_BUSY_MAX_RETRIES): $address/$characteristicUuid",
                )
                val retry = Runnable { attemptRead(address, characteristicUuid, requestId, session, attempt + 1) }
                pendingRetries[requestId] = retry
                retryHandler.postDelayed(retry, gattBusyRetryDelayMs(attempt + 1))
            } else {
                Log.w(TAG, "readCharacteristic rejected by the stack after $attempt retries: $address/$characteristicUuid")
                synchronized(queue) { queue.remove(requestId) }
                onCharacteristicRead(nativeHandle, requestId, address, characteristicUuid, ByteArray(0), false)
            }
        }
    }

    /// `withoutResponse` selects ATT Write Command over Write Request —
    /// materially faster for bulk transfer, at the cost of the peer silently
    /// dropping writes it can't keep up with. See `models::WriteType`.
    fun writeCharacteristic(
        address: String, characteristicUuid: String, value: ByteArray, withoutResponse: Boolean,
        requestId: Long, session: Long,
    ) = attemptWrite(address, characteristicUuid, value, withoutResponse, requestId, session, attempt = 0)

    /// See `GATT_BUSY_MAX_RETRIES`'s doc comment for why a `false` return
    /// here gets retried rather than reported as failure immediately.
    /// `requestId` is queued once, on `attempt == 0` — a retry reuses the
    /// same slot rather than adding a second entry, since it's still the
    /// same logical write as far as `onCharacteristicWrite`'s FIFO matching
    /// is concerned; the platform never actually started the earlier
    /// attempt, so there is nothing for it to complete out of order with.
    @Suppress("DEPRECATION")
    private fun attemptWrite(
        address: String, characteristicUuid: String, value: ByteArray, withoutResponse: Boolean,
        requestId: Long, session: Long, attempt: Int,
    ): Unit = synchronized(gattLock) {
        // Must be the very first thing done under gattLock -- see
        // cancelPendingOperation's doc comment (finding 3). A `null` here
        // means cancellation already claimed this attempt; Rust has
        // stopped listening, so there is nothing left to do.
        if (attempt > 0 && pendingRetries.remove(requestId) == null) return
        if (gattSessions[address] != session) {
            if (attempt > 0) {
                pendingWriteIds[address]?.let { queue -> synchronized(queue) { queue.remove(requestId) } }
            }
            onCharacteristicWriteResult(nativeHandle, requestId, address, characteristicUuid, false)
            return
        }
        val gatt = connectedGatts[address]
        val characteristic = findCharacteristic(address, characteristicUuid)
        val queue = pendingWriteIds.getOrPut(address) { ArrayDeque() }
        if (attempt == 0) {
            synchronized(queue) { queue.addLast(requestId) }
        }
        if (gatt == null || characteristic == null) {
            Log.w(TAG, "writeCharacteristic could not start: $address/$characteristicUuid")
            synchronized(queue) { queue.remove(requestId) }
            onCharacteristicWriteResult(nativeHandle, requestId, address, characteristicUuid, false)
            return
        }
        characteristic.writeType = if (withoutResponse) {
            BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
        } else {
            BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
        }
        characteristic.value = value
        if (!gatt.writeCharacteristic(characteristic)) {
            if (attempt < GATT_BUSY_MAX_RETRIES) {
                Log.w(
                    TAG,
                    "writeCharacteristic rejected by the stack, retrying " +
                        "(${attempt + 1}/$GATT_BUSY_MAX_RETRIES): $address/$characteristicUuid",
                )
                val retry = Runnable {
                    attemptWrite(address, characteristicUuid, value, withoutResponse, requestId, session, attempt + 1)
                }
                pendingRetries[requestId] = retry
                retryHandler.postDelayed(retry, gattBusyRetryDelayMs(attempt + 1))
            } else {
                Log.w(TAG, "writeCharacteristic rejected by the stack after $attempt retries: $address/$characteristicUuid")
                synchronized(queue) { queue.remove(requestId) }
                onCharacteristicWriteResult(nativeHandle, requestId, address, characteristicUuid, false)
            }
        }
    }

    fun subscribeCharacteristic(
        address: String, characteristicUuid: String, requestId: Long, session: Long,
    ) = attemptSubscribe(address, characteristicUuid, requestId, session, attempt = 0)

    /// Same GATT-busy retry as `attemptWrite` — see `GATT_BUSY_MAX_RETRIES`'s
    /// doc comment. Only the descriptor write (`gatt.writeDescriptor`) is a
    /// real over-the-air GATT operation subject to the platform's
    /// one-outstanding-op rule; `setCharacteristicNotification` is purely
    /// local bookkeeping and re-running it on a retry is harmless.
    @Suppress("DEPRECATION")
    private fun attemptSubscribe(
        address: String, characteristicUuid: String, requestId: Long, session: Long, attempt: Int,
    ): Unit = synchronized(gattLock) {
        // Must be the very first thing done under gattLock -- see
        // cancelPendingOperation's doc comment (finding 3). A `null` here
        // means cancellation already claimed this attempt; Rust has
        // stopped listening, so there is nothing left to do.
        if (attempt > 0 && pendingRetries.remove(requestId) == null) return
        if (gattSessions[address] != session) {
            if (attempt > 0) {
                pendingSubscribeIds["$address/$characteristicUuid"]?.let { queue ->
                    synchronized(queue) { queue.remove(requestId) }
                }
            }
            onSubscribed(nativeHandle, requestId, address, characteristicUuid, false)
            return
        }
        // A queue, not a slot: a cancelled attempt whose callback has not
        // arrived must not have its identity overwritten by the retry, or
        // that callback reports the retry's id and Rust accepts the old
        // status as the new attempt's completion.
        val subscribeQueue =
            pendingSubscribeIds.getOrPut("$address/$characteristicUuid") { ArrayDeque() }
        if (attempt == 0) {
            synchronized(subscribeQueue) { subscribeQueue.addLast(requestId) }
        }
        val gatt = connectedGatts[address]
        val characteristic = findCharacteristic(address, characteristicUuid)
        val cccd = characteristic?.getDescriptor(CLIENT_CHARACTERISTIC_CONFIG_UUID)
        if (gatt == null || characteristic == null || cccd == null) {
            Log.w(TAG, "subscribeCharacteristic could not start: $address/$characteristicUuid")
            synchronized(subscribeQueue) { subscribeQueue.remove(requestId) }
            onSubscribed(nativeHandle, requestId, address, characteristicUuid, false)
            return
        }
        if (!gatt.setCharacteristicNotification(characteristic, true)) {
            // Local delivery was never enabled, so onCharacteristicChanged
            // can never fire. Writing the remote CCCD anyway would let the
            // descriptor callback report success and hand Rust a live-looking
            // stream that stays silent forever.
            Log.w(TAG, "setCharacteristicNotification failed: $address/$characteristicUuid")
            synchronized(subscribeQueue) { subscribeQueue.remove(requestId) }
            onSubscribed(nativeHandle, requestId, address, characteristicUuid, false)
            return
        }
        cccd.value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
        if (!gatt.writeDescriptor(cccd)) {
            if (attempt < GATT_BUSY_MAX_RETRIES) {
                Log.w(
                    TAG,
                    "subscribe descriptor write rejected by the stack, retrying " +
                        "(${attempt + 1}/$GATT_BUSY_MAX_RETRIES): $address/$characteristicUuid",
                )
                val retry = Runnable { attemptSubscribe(address, characteristicUuid, requestId, session, attempt + 1) }
                pendingRetries[requestId] = retry
                retryHandler.postDelayed(retry, gattBusyRetryDelayMs(attempt + 1))
            } else {
                Log.w(
                    TAG,
                    "subscribe descriptor write rejected by the stack after $attempt retries: $address/$characteristicUuid",
                )
                synchronized(subscribeQueue) { subscribeQueue.remove(requestId) }
                onSubscribed(nativeHandle, requestId, address, characteristicUuid, false)
            }
        }
    }

    // ---------------------------------------------------------------
    // Peripheral role
    // ---------------------------------------------------------------

    fun startAdvertising(
        serviceUuid: String, characteristicUuids: Array<String>, readable: BooleanArray, writable: BooleanArray,
        notifiable: BooleanArray, initialValues: Array<ByteArray>,
        // Same parallel-array flattening `onPeerDiscovered` uses on the read
        // side, mirrored here for the write side: a `java.util.HashMap`
        // across JNI costs several reflective calls per entry, and the Rust
        // side would have to walk it back out again either way.
        manufacturerIds: IntArray, manufacturerValues: Array<ByteArray>,
        serviceDataUuids: Array<String>, serviceDataValues: Array<ByteArray>,
    ) {
        // Tear down any predecessor first. This bridge holds a single
        // `gattServer`/`advertiseCallback` pair, so starting without
        // stopping silently overwrote them — leaving an abandoned attempt's
        // server open and its advertisement discoverable, with nothing left
        // able to close it.
        //
        // Doing it here rather than in the abandoned attempt's own cleanup
        // is deliberate: that cleanup must no-op once superseded, or it
        // would stop the very advertisement that replaced it. Making the
        // retry responsible serialises the two without either racing.
        stopAdvertising()

        val advertiser = adapter?.bluetoothLeAdvertiser
        if (advertiser == null) {
            // Central-only device, or Bluetooth switched off. Returning
            // silently left Rust's oneshot pending forever, which is worse
            // than the unsupported error the backend contract asks for.
            Log.w(TAG, "startAdvertising: no BLE advertiser available")
            failAdvertise(ADVERTISE_ERROR_UNAVAILABLE)
            return
        }

        // Captured by every callback below, so a callback outliving its
        // server can tell that it has been superseded.
        val generation = synchronized(this) { ++serverGeneration }

        val serverCallback = object : BluetoothGattServerCallback() {
            /// `addService` completes asynchronously. Advertising used to
            /// start immediately after the call, so a delayed or rejected
            /// registration still resolved Rust's `advertise()` as success
            /// while centrals could see the advertisement but not the
            /// service it promised.
            override fun onServiceAdded(status: Int, service: BluetoothGattService) {
                // `addService` completes asynchronously, so this can run
                // after `stopAdvertising()` has already closed the server.
                // Advertising anyway would violate a stop request that had
                // completed, and during a rapid stop/start it would start
                // the *old* service and resolve the new generation's
                // advertise waiter with the old attempt's outcome.
                // Validation *and* effects under the monitor. Sampling the
                // generation and then acting let this callback resume after
                // a restart and either tear the replacement down via
                // `failAdvertise` or overwrite `advertiseCallback` and start
                // the old advertisement.
                synchronized(this@BleGattBridge) {
                    if (serverGeneration != generation) {
                        Log.d(TAG, "onServiceAdded: stale server generation, ignoring")
                        return
                    }
                    if (status != BluetoothGatt.GATT_SUCCESS) {
                        Log.e(TAG, "onServiceAdded failed: status=$status")
                        failAdvertise(status)
                        return
                    }
                    Log.d(TAG, "onServiceAdded ok, starting advertisement")
                    beginAdvertising(
                        advertiser, serviceUuid, generation,
                        manufacturerIds, manufacturerValues, serviceDataUuids, serviceDataValues,
                    )
                }
            }

            /// Real server-side lifecycle. Without this the peripheral role
            /// had no genuine connect/disconnect signal at all — it was
            /// synthesized from write traffic, so a central that connected
            /// and then left was never reported as gone.
            override fun onConnectionStateChange(device: BluetoothDevice, status: Int, newState: Int) {
                when (newState) {
                    BluetoothProfile.STATE_CONNECTED -> synchronized(this@BleGattBridge) {
                        if (serverGeneration != generation) {
                            Log.d(TAG, "server: stale generation connect, ignoring")
                            return
                        }
                        Log.d(TAG, "server: central connected ${device.address}")
                        onConnected(nativeHandle, device.address, true)
                    }
                    BluetoothProfile.STATE_DISCONNECTED -> {
                        // A queued callback from a stopped server can run
                        // after the same central has subscribed to its
                        // replacement. Without this it would strip that
                        // central from the *new* generation's subscriber set
                        // and report it disconnected, tearing down a served
                        // channel that had only just been established.
                        // Check and effects together: otherwise this can
                        // resume after the same device has subscribed to the
                        // replacement server, strip it from that generation's
                        // subscriber sets and report it gone — terminating a
                        // channel that was just established.
                        synchronized(this@BleGattBridge) {
                            if (serverGeneration != generation) {
                                Log.d(TAG, "server: stale generation disconnect, ignoring")
                                return
                            }
                            Log.d(TAG, "server: central disconnected ${device.address}")
                            // Discard anything this peer had prepared but not
                            // executed. Left behind, a later connection from
                            // the same address could prepare the remaining
                            // offsets and execute — publishing a value spliced
                            // from two different sessions.
                            discardPreparedWrites(device.address)
                            serverSessions.remove(device.address)
                            for (subscribers in subscribedDevices.values) {
                                subscribers.remove(device)
                            }
                            onDisconnected(nativeHandle, device.address, true)
                        }
                    }
                }
            }

            override fun onCharacteristicReadRequest(
                device: BluetoothDevice, requestId: Int, offset: Int, characteristic: BluetoothGattCharacteristic
            ) {
                synchronized(this@BleGattBridge) {
                    if (serverGeneration != generation) return
                    val value = characteristic.value ?: ByteArray(0)
                    // A central continuing a long read sends Read Blob with a
                    // nonzero offset. Returning the whole value again while
                    // echoing that offset duplicates data or fails outright;
                    // the response must carry the suffix from `offset`.
                    if (offset > value.size) {
                        gattServer?.sendResponse(
                            device, requestId, BluetoothGatt.GATT_INVALID_OFFSET, offset, null
                        )
                        return
                    }
                    gattServer?.sendResponse(
                        device,
                        requestId,
                        BluetoothGatt.GATT_SUCCESS,
                        offset,
                        value.copyOfRange(offset, value.size),
                    )
                }
            }

            @Suppress("DEPRECATION")
            override fun onCharacteristicWriteRequest(
                device: BluetoothDevice, requestId: Int, characteristic: BluetoothGattCharacteristic,
                preparedWrite: Boolean, responseNeeded: Boolean, offset: Int, value: ByteArray,
            ) {
                // A write queued against a stopped server would otherwise be
                // published as current. If it is a datagram fragment, the
                // replacement session reassembles stale bytes into its own
                // message — corruption, not just a late delivery.
                //
                // The generation is held across the *effects*, not merely
                // sampled: sampling alone let a callback pass the check,
                // pause, and forward after a restart had already begun.
                // `stopAdvertising` takes the same lock to increment it, so
                // a callback that gets here either completes entirely before
                // the restart or is rejected.
                // Publication inside the monitor as well. Leaving it outside
                // let a callback decide it was current, pause while
                // `stopAdvertising` incremented the generation and a
                // replacement server started, and then publish its stale
                // payload into the new session — where a datagram fragment
                // is reassembled as current data.
                synchronized(this@BleGattBridge) {
                    if (serverGeneration != generation) return

                    // Prepared chunks are staged, not applied: the value is
                    // only complete once the central sends Execute Write.
                    if (preparedWrite) {
                        val key = "${device.address}/${characteristic.uuid}"
                        preparedWrites
                            .getOrPut(key) { java.util.TreeMap() }[offset] = value
                        if (responseNeeded) {
                            gattServer?.sendResponse(
                                device, requestId, BluetoothGatt.GATT_SUCCESS, offset, value
                            )
                        }
                        return
                    }

                    characteristic.value = value
                    if (responseNeeded) {
                        gattServer?.sendResponse(
                            device, requestId, BluetoothGatt.GATT_SUCCESS, offset, value
                        )
                    }
                    // Minted here if this write is the first sign of the
                    // peer, so Kotlin stays the single source of server
                    // session ids — Rust minting its own would give the same
                    // map two independent id spaces.
                    val session = serverSessions.getOrPut(device.address) { nextServerSession++ }
                    onServerCharacteristicWritten(
                        nativeHandle, device.address, characteristic.uuid.toString(), value, session
                    )
                }
            }

            /// Apply or discard everything staged by Prepare Write.
            ///
            /// Without this the central's execute went unacknowledged, so a
            /// reliable or long write never completed from its point of view
            /// even though the chunks had already been published.
            override fun onExecuteWrite(device: BluetoothDevice, requestId: Int, execute: Boolean) {
                synchronized(this@BleGattBridge) {
                    if (serverGeneration != generation) return
                    val prefix = "${device.address}/"
                    val keys = preparedWrites.keys.filter { it.startsWith(prefix) }

                    // ATT execute is transactional: validate and assemble
                    // *everything* first, and only then mutate and publish.
                    // Publishing as we went meant a gap in a later
                    // characteristic returned failure to the central after
                    // earlier characteristics had already taken effect.
                    val assembled = ArrayList<Pair<String, ByteArray>>()
                    if (execute) {
                        for (key in keys) {
                            val staged = preparedWrites[key] ?: continue
                            val uuid = key.removePrefix(prefix)
                            if (serverCharacteristics[uuid] == null) continue
                            var end = 0
                            for ((chunkOffset, chunk) in staged) {
                                if (chunkOffset > end) {
                                    Log.w(TAG, "onExecuteWrite: gap at $chunkOffset for $uuid")
                                    // Nothing has been applied yet, so the
                                    // central's failure is total, as ATT
                                    // requires. The staged data stays put so
                                    // it can be cancelled explicitly.
                                    gattServer?.sendResponse(
                                        device, requestId, BluetoothGatt.GATT_INVALID_OFFSET, 0, null
                                    )
                                    return
                                }
                                end = maxOf(end, chunkOffset + chunk.size)
                            }
                            val value = ByteArray(end)
                            for ((chunkOffset, chunk) in staged) {
                                chunk.copyInto(value, chunkOffset)
                            }
                            assembled.add(uuid to value)
                        }
                    }

                    for (key in keys) {
                        preparedWrites.remove(key)
                    }
                    // Same session as a plain write: an executed prepared
                    // write is this peer's traffic and must carry the same
                    // identity, or `serve` records a sessionless channel.
                    val session = serverSessions.getOrPut(device.address) { nextServerSession++ }
                    for ((uuid, value) in assembled) {
                        val characteristic = serverCharacteristics[uuid] ?: continue
                        @Suppress("DEPRECATION")
                        characteristic.value = value
                        onServerCharacteristicWritten(
                            nativeHandle, device.address, uuid, value, session
                        )
                    }
                    gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, 0, null)
                }
            }

            override fun onNotificationSent(device: BluetoothDevice, status: Int) {
                val completed = synchronized(this@BleGattBridge) {
                    if (serverGeneration != generation) {
                        Log.d(TAG, "onNotificationSent: stale server generation, ignoring")
                        return
                    }
                    val done = notifyInFlight
                    notifyInFlight = null
                    done
                }
                if (completed != null) {
                    onNotifySent(
                        nativeHandle,
                        completed.requestId,
                        status == BluetoothGatt.GATT_SUCCESS,
                    )
                }
                pumpNotifications()
            }

            override fun onDescriptorWriteRequest(
                device: BluetoothDevice, requestId: Int, descriptor: BluetoothGattDescriptor,
                preparedWrite: Boolean, responseNeeded: Boolean, offset: Int, value: ByteArray,
            ) {
                // `subscribedDevices` is global, not per-generation, so a
                // CCCD callback queued against a stopped server could strip
                // a subscription the *replacement* server had just accepted
                // and report that central disconnected.
                synchronized(this@BleGattBridge) {
                if (serverGeneration != generation) {
                    Log.d(TAG, "onDescriptorWriteRequest: stale server generation, ignoring")
                    return
                }
                val characteristicUuid = descriptor.characteristic.uuid.toString()
                if (value.contentEquals(BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE)) {
                    val added = subscribedDevices
                        .getOrPut(characteristicUuid) { ConcurrentHashMap.newKeySet() }
                        .add(device)
                    // Report only the transition. This is what tells Rust the
                    // peer is reachable by notify — announcing at connection
                    // time was too early, since a greeting sent before the
                    // CCCD write has nowhere to go.
                    if (added) {
                        val session = serverSessions.getOrPut(device.address) {
                            nextServerSession++
                        }
                        onServerSubscribed(nativeHandle, device.address, session)
                    }
                } else {
                    val removed = subscribedDevices[characteristicUuid]?.remove(device) ?: false
                    // Unsubscribing ends the served session even though the
                    // link is still up. Rust's `serve` keys its single-central
                    // slot on connect/disconnect, so without this the peer
                    // stays "served" while being unreachable by notify —
                    // sends fail, `recv()` never closes, and every other
                    // central is refused until it physically disconnects.
                    if (removed && !isSubscribedAnywhere(device)) {
                        onDisconnected(nativeHandle, device.address, true)
                    }
                }
                if (responseNeeded) {
                    gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, value)
                }
                }
            }
        }
        val server = bluetoothManager.openGattServer(context, serverCallback)
        gattServer = server

        val service = BluetoothGattService(
            UUID.fromString(serviceUuid), BluetoothGattService.SERVICE_TYPE_PRIMARY
        )
        for (i in characteristicUuids.indices) {
            var properties = 0
            var permissions = 0
            if (readable[i]) {
                properties = properties or BluetoothGattCharacteristic.PROPERTY_READ
                permissions = permissions or BluetoothGattCharacteristic.PERMISSION_READ
            }
            if (writable[i]) {
                // Both modes, matching linux.rs (`write` and
                // `write_without_response`). Advertising only PROPERTY_WRITE
                // let a peer using WriteType::WithoutResponse discover no
                // such capability and reject the write command locally,
                // before it ever reached onCharacteristicWriteRequest.
                properties = properties or
                    BluetoothGattCharacteristic.PROPERTY_WRITE or
                    BluetoothGattCharacteristic.PROPERTY_WRITE_NO_RESPONSE
                permissions = permissions or BluetoothGattCharacteristic.PERMISSION_WRITE
            }
            if (notifiable[i]) {
                properties = properties or BluetoothGattCharacteristic.PROPERTY_NOTIFY
            }
            val characteristic = BluetoothGattCharacteristic(
                UUID.fromString(characteristicUuids[i]), properties, permissions
            )
            @Suppress("DEPRECATION")
            characteristic.value = initialValues[i]
            if (notifiable[i]) {
                val cccd = BluetoothGattDescriptor(
                    CLIENT_CHARACTERISTIC_CONFIG_UUID,
                    BluetoothGattDescriptor.PERMISSION_READ or BluetoothGattDescriptor.PERMISSION_WRITE
                )
                characteristic.addDescriptor(cccd)
            }
            service.addCharacteristic(characteristic)
            serverCharacteristics[characteristicUuids[i]] = characteristic
        }
        if (!server.addService(service)) {
            Log.e(TAG, "addService was rejected outright for $serviceUuid")
            failAdvertise(ADVERTISE_ERROR_SERVICE_REJECTED)
            return
        }
        // Advertising now starts from onServiceAdded, once registration has
        // actually succeeded.
    }

    private fun beginAdvertising(
        advertiser: android.bluetooth.le.BluetoothLeAdvertiser, serviceUuid: String,
        generation: Long,
        manufacturerIds: IntArray, manufacturerValues: Array<ByteArray>,
        serviceDataUuids: Array<String>, serviceDataValues: Array<ByteArray>,
    ) {
        val settings = AdvertiseSettings.Builder()
            .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_BALANCED)
            .setConnectable(true)
            .build()
        val dataBuilder = AdvertiseData.Builder()
            .addServiceUuid(ParcelUuid(UUID.fromString(serviceUuid)))
            .setIncludeDeviceName(false)
        // Legacy advertisements are capped at 31 bytes total, and the
        // service UUID above already spends most of that -- callers are
        // responsible for keeping this small (see GattServiceSpec's own
        // doc comment on manufacturer_data/service_data).
        for (i in manufacturerIds.indices) {
            dataBuilder.addManufacturerData(manufacturerIds[i], manufacturerValues[i])
        }
        for (i in serviceDataUuids.indices) {
            dataBuilder.addServiceData(ParcelUuid(UUID.fromString(serviceDataUuids[i])), serviceDataValues[i])
        }
        val data = dataBuilder.build()
        val callback = object : AdvertiseCallback() {
            override fun onStartSuccess(settingsInEffect: AdvertiseSettings) {
                // A stop/restart can leave this callback running against a
                // replacement generation. Resolving the new server's advertise
                // waiter with this attempt's outcome would report a success
                // that belongs to an advertisement already torn down.
                synchronized(this@BleGattBridge) {
                    if (serverGeneration != generation) {
                        Log.d(TAG, "advertise onStartSuccess: stale generation, ignoring")
                        return
                    }
                    Log.d(TAG, "advertise onStartSuccess: serviceUuid=$serviceUuid")
                    onAdvertiseResult(nativeHandle, true, 0)
                }
            }

            override fun onStartFailure(errorCode: Int) {
                // Worse than a misreported success: `failAdvertise` tears the
                // server down, so a stale failure would destroy the
                // advertisement that replaced this one.
                synchronized(this@BleGattBridge) {
                if (serverGeneration != generation) {
                    Log.d(TAG, "advertise onStartFailure: stale generation, ignoring")
                    return
                }
                // Android reports this asynchronously — advertising already
                // in progress, controller out of resources. Reporting it
                // back is what stops Rust claiming the service is reachable
                // when no advertisement exists.
                Log.e(TAG, "advertise onStartFailure: errorCode=$errorCode serviceUuid=$serviceUuid")
                failAdvertise(errorCode)
                }
            }
        }
        advertiseCallback = callback
        Log.d(TAG, "startAdvertising: serviceUuid=$serviceUuid advertiser=${advertiser != null}")
        advertiser.startAdvertising(settings, data, callback)
    }

    /// Close every open GATT and the server, so no Kotlin object is left
    /// holding a callback that captures `nativeHandle`. Rust calls this
    /// before the last `Arc<Inner>` can be freed; without it a later
    /// link-state callback would reconstruct an `Arc` from freed memory.
    /// Close and forget one client GATT, for a connect attempt Rust
    /// abandoned. Leaving it open would keep the address occupied here while
    /// no Rust handle owns it.
    fun closeConnection(address: String) {
        val gatt = synchronized(gattLock) {
            val existing = connectedGatts.remove(address) ?: return
            gattSessions.remove(address)
            pendingCharacteristics.remove(address)
            existing
        } ?: return
        clearPendingOperations(address)
        try {
            gatt.disconnect()
            gatt.close()
        } catch (e: Exception) {
            Log.w(TAG, "closeConnection: error closing gatt for $address", e)
        }
    }

    /// Forget outstanding operation ids for an address whose GATT is going
    /// away.
    ///
    /// The queues are keyed by address but the ids belong to a specific
    /// `BluetoothGatt`. An operation cancelled before its callback arrived
    /// leaves its id behind; after a reconnect the replacement's first
    /// callback would pop that stale id, Rust would reject it as belonging
    /// to a finished operation, and the live one would hang forever.
    private fun clearPendingOperations(address: String) {
        pendingReadIds.remove(address)
        pendingWriteIds.remove(address)
        pendingSubscribeIds.keys.removeAll { it.startsWith("$address/") }
    }

    fun closeAll() {
        for (gatt in connectedGatts.values) {
            try {
                gatt.disconnect()
                gatt.close()
            } catch (e: Exception) {
                Log.w(TAG, "closeAll: error closing gatt", e)
            }
        }
        connectedGatts.clear()
        gattSessions.clear()
        pendingCharacteristics.clear()
        pendingReadIds.clear()
        pendingWriteIds.clear()
        pendingSubscribeIds.clear()
        preparedWrites.clear()
        stopScan()
        stopAdvertising()
    }

    fun stopAdvertising() {
        val advertiser = adapter?.bluetoothLeAdvertiser

        // The generation increment comes before the callback is even read.
        // Capturing and clearing `advertiseCallback` outside the monitor let
        // an old `onServiceAdded` run in the gap, install its callback after
        // the clear and start an advertisement this stop was meant to end —
        // or a queued start callback resolve the *replacement's*
        // `advertise_tx`, which Rust installs before calling in here.
        val stopping = synchronized(this) {
            ++serverGeneration
            val previous = advertiseCallback
            advertiseCallback = null
            previous
        }
        stopping?.let { advertiser?.stopAdvertising(it) }

        // Teardown shares the same monitor as the invalidation above, so a
        // CCCD-enable callback cannot acquire it in between, pass the
        // old-generation check, and add a peer back after the snapshot —
        // whose eventual disconnect would then be discarded as stale,
        // leaving Rust's channel holding the single-central slot forever.
        //
        // `failPendingNotifications` re-enters this monitor; Java monitors
        // are reentrant, so that is safe.
        synchronized(this) {
            failPendingNotifications()
            // Nothing staged survives the server that was collecting it.
            preparedWrites.clear()
            serverSessions.clear()

            // Report every served central as gone before dropping the
            // server. Closing a GATT server delivers no
            // onConnectionStateChange for the centrals attached to it, so
            // without this Rust keeps them in its single-central map with
            // their channels live.
            val served = subscribedDevices.values.flatten().map { it.address }.distinct()
            subscribedDevices.clear()
            for (address in served) {
                onDisconnected(nativeHandle, address, true)
            }
        }

        gattServer?.close()
        gattServer = null
        serverCharacteristics.clear()
    }

    /// Report an advertise failure *and* release everything the attempt
    /// opened.
    ///
    /// Reporting alone left `gattServer` and its registered service live. A
    /// retry then overwrote the sole `gattServer` reference, so the first
    /// server could never be closed by `closeAll()` — leaking the Bluetooth
    /// resource and, worse, leaving its callback holding `nativeHandle`
    /// after Rust had freed that state.
    private fun failAdvertise(errorCode: Int) {
        stopAdvertising()
        onAdvertiseResult(nativeHandle, false, errorCode)
    }

    /// Whether this device still holds any notify subscription. Checked
    /// before reporting a disable as the end of the session, since a peer may
    /// be subscribed to several characteristics.
    private fun isSubscribedAnywhere(device: BluetoothDevice): Boolean =
        subscribedDevices.values.any { peers -> peers.any { it.address == device.address } }

    /// Addresses currently subscribed to a characteristic. Rust expresses a
    /// broadcast as one addressed send per subscriber, so every payload goes
    /// through the same completion-confirmed queue.
    fun subscribedAddresses(characteristicUuid: String): Array<String> =
        subscribedDevices[characteristicUuid]
            ?.map { it.address }
            ?.distinct()
            ?.toTypedArray()
            ?: emptyArray()

    /// Drain the notification queue, one outstanding send at a time.
    ///
    /// Failures are reported to Rust here rather than swallowed: a caller
    /// that believes a fragment was sent when it was not will wait for a
    /// reply that can never come.
    private fun pumpNotifications() {
        while (true) {
            val next = synchronized(this) {
                if (notifyInFlight != null) return
                val candidate = notifyQueue.removeFirstOrNull() ?: return
                notifyInFlight = candidate
                candidate
            }
            val server = gattServer
            val characteristic = serverCharacteristics[next.characteristicUuid]
            val initiated = if (server != null && characteristic != null) {
                @Suppress("DEPRECATION")
                characteristic.value = next.value
                @Suppress("DEPRECATION")
                server.notifyCharacteristicChanged(next.device, characteristic, false)
            } else {
                false
            }
            if (initiated) {
                // onNotificationSent resumes the pump.
                return
            }
            synchronized(this) { notifyInFlight = null }
            onNotifySent(nativeHandle, next.requestId, false)
        }
    }

    /// Queue a notification for exactly one subscribed central, resolved
    /// asynchronously through `onNotifySent(requestId, ...)`.
    ///
    /// Returns false only for a synchronous impossibility (no server, or the
    /// peer is not subscribed); anything queued is answered by the callback.
    fun notifyCharacteristicTo(
        address: String, characteristicUuid: String, value: ByteArray, requestId: Long,
        session: Long,
    ): Boolean {
        // Validation and admission under the same monitor `stopAdvertising`
        // uses. Checking outside it let a request be enqueued *after* the
        // teardown drained pending notifications, so the pump would start it
        // on an invalidated server whose completion callback is then
        // discarded as stale — and Rust's `notify_peer` waits forever for an
        // `onNotifySent` that will never be delivered.
        synchronized(this) {
            // Session validated here, inside the same monitor that selects
            // the subscriber and enqueues — checking it in Rust and then
            // calling would let the old subscription drop and the same
            // address resubscribe in between, after which a retained
            // channel's fragment is queued to the replacement device and
            // accepted there as current application data.
            if (session != 0L && serverSessions[address] != session) {
                Log.d(TAG, "notifyCharacteristicTo: $address session superseded, dropping")
                return false
            }
            if (gattServer == null || serverCharacteristics[characteristicUuid] == null) {
                return false
            }
            val device = subscribedDevices[characteristicUuid]
                ?.firstOrNull { it.address == address }
                ?: return false
            notifyQueue.addLast(PendingNotify(device, characteristicUuid, value, requestId))
        }
        pumpNotifications()
        return true
    }

    /// Fail every queued and in-flight notification. Called when the server
    /// goes away, so Rust callers are not left waiting on sends that can no
    /// longer complete.
    /// Drop every prepared-write chunk staged for one peer.
    private fun discardPreparedWrites(address: String) {
        val prefix = "$address/"
        preparedWrites.keys.removeAll { it.startsWith(prefix) }
    }

    private fun failPendingNotifications() {
        val abandoned = synchronized(this) {
            val all = notifyQueue.toList() + listOfNotNull(notifyInFlight)
            notifyQueue.clear()
            notifyInFlight = null
            all
        }
        for (pending in abandoned) {
            onNotifySent(nativeHandle, pending.requestId, false)
        }
    }


    /// Drop a central's connection to our GATT server, and forget its
    /// subscriptions so `notifyCharacteristic` stops broadcasting to it even
    /// before `onConnectionStateChange` lands.
    fun disconnectServerPeer(address: String, session: Long) = synchronized(this) {
        // Validated and acted on under one monitor: a Binder callback could
        // otherwise remove this session and install a replacement
        // subscription between Rust's check and this call, after which the
        // cancellation would drop the replacement.
        if (session != 0L && serverSessions[address] != session) {
            Log.d(TAG, "disconnectServerPeer: $address session superseded, ignoring")
            return
        }
        serverSessions.remove(address)
        val server = gattServer ?: return
        val device = subscribedDevices.values
            .asSequence()
            .flatten()
            .firstOrNull { it.address == address }
            ?: adapter?.getRemoteDevice(address)
            ?: return
        for (peers in subscribedDevices.values) {
            peers.removeAll { it.address == address }
        }
        server.cancelConnection(device)
    }

    companion object {

        private val CLIENT_CHARACTERISTIC_CONFIG_UUID: UUID =
            UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")

        /// Largest ATT MTU the Bluetooth spec permits. Requested on every
        /// connection; the peer negotiates it down to whatever it supports.
        private const val MAX_ATT_MTU: Int = 517

        /// Locally-assigned advertise error codes, chosen above the range
        /// Android's AdvertiseCallback uses (1..5) so they cannot be
        /// confused with a real controller error.
        private const val ADVERTISE_ERROR_UNAVAILABLE: Int = 100
        private const val ADVERTISE_ERROR_SERVICE_REJECTED: Int = 101

        /// Android allows exactly one outstanding GATT operation per
        /// connection (an MTU request, a characteristic read/write, a
        /// descriptor write); calling the platform API for a new one before
        /// the previous op's callback has landed makes it return `false`
        /// immediately -- before anything is attempted over the air, not a
        /// protocol-level rejection. Confirmed on real hardware (2026-08-17,
        /// see ble-gatt/docs/hardware-verification.md) as reliably
        /// reproducible under external radio contention -- a classic-profile
        /// reconnect sharing the same radio -- even with a correctly
        /// declared, fully-writable characteristic and zero bytes ever sent
        /// on the air. That's exactly the transient condition the platform
        /// expects callers to retry, not a hard failure, so `attemptWrite`/
        /// `attemptRead`/`attemptSubscribe` back off and retry a bounded
        /// number of times before reporting failure and tearing down the
        /// link.
        ///
        /// A P1 review finding on the original flat 3 x 200ms schedule
        /// (~600ms total): the busy window measured on that same real
        /// hardware run was 3.7-3.8 seconds, not milliseconds -- every one
        /// of those three retries fired while the stack was still busy, so
        /// the write failed exactly as it would have with no retry at all.
        /// `gattBusyRetryDelayMs` below instead backs off exponentially,
        /// capped, giving a ~7s cumulative window (200+400+800+1600+2000+
        /// 2000ms) that comfortably covers the measured worst case with
        /// margin, while still giving up well short of the ~35-45s full
        /// reconnect-and-reauthenticate cycle this exists to avoid paying
        /// for on every transient stall.
        private const val GATT_BUSY_MAX_RETRIES = 6
        private const val GATT_BUSY_RETRY_BASE_DELAY_MS = 200L
        private const val GATT_BUSY_RETRY_MAX_DELAY_MS = 2000L

        /// `attempt` is the *upcoming* attempt number (1-indexed: the first
        /// retry is 1, matching `attempt + 1` at each call site) -- delay
        /// doubles each time, capped, so it's cheap to retry a genuinely
        /// brief stall quickly without spending the whole budget on retries
        /// fired too early to have any chance during a sustained one.
        private fun gattBusyRetryDelayMs(attempt: Int): Long =
            (GATT_BUSY_RETRY_BASE_DELAY_MS shl (attempt - 1)).coerceAtMost(GATT_BUSY_RETRY_MAX_DELAY_MS)
    }
}
