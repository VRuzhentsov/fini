import { expect } from '../fixtures.ts';
import type { E2EActor } from '../fixtures.ts';
import { pollUntil } from './dom.ts';
import { waitForActorsReady, type SyncedActor } from './device-sync.ts';
import { openDeviceDetailsFromSettings } from './personal-sync.ts';

/**
 * Readiness + pairing for the BLE-transport actor suite
 * (`FINI_E2E_TRANSPORT=ble`). Mirrors `sim-sync.ts` almost exactly -- same
 * reason: actors are spawned with `FINI_DISCOVERY_DISABLED=1`, so there is
 * nothing to discover by design and the normal `ensureSyncedActors` UI-pair
 * flow doesn't apply. The difference from Sim is what's underneath: these
 * actors dial the real `ble.rs` code path (dial loop, peripheral accept,
 * session claim) against a cross-process mock radio instead of a plain TCP
 * stand-in, and pairing is marked `viaBluetooth: true` with a real (fake)
 * address so `bluetooth_dial_candidates` actually has something to find.
 * See `fixtures.ts`'s `fakeBluetoothAddress`/`FINI_BLUETOOTH_PAIRED_ADDRESSES`
 * wiring and `docs/adr/0004-mock-broker-for-cross-process-e2e.md` in
 * `ble-gatt`.
 */

interface PeerSessionDebugStatus {
  peer_session_count: number;
}

interface TransportStatusCode {
  code: string;
}

interface TransportStatus {
  kind: 'network' | 'bluetooth';
  primary: boolean;
  state: { state: 'unconfigured'; code: TransportStatusCode } | { state: 'configured'; code: TransportStatusCode | null };
}

/**
 * Every actor's fake address is deterministic from its index alone (see
 * `fixtures.ts`'s `fakeBluetoothAddress`) -- duplicated here rather than
 * exported/imported so this helper stays a pure consumer of what the
 * harness already put in each actor's environment (`FINI_LOCAL_BLUETOOTH_
 * ADDRESS`), not a second source of truth for it. Tests never need to
 * compute an address themselves; they only need to hand the *other*
 * actor's known address to `device_connection_save_paired_device`.
 */
function fakeBluetoothAddress(index: number): string {
  return `AA:BB:CC:00:00:${(index + 1).toString(16).padStart(2, '0').toUpperCase()}`;
}

export async function ensureBlePairedActors(
  actors: E2EActor[],
  timeoutMs = 60_000,
): Promise<SyncedActor[]> {
  if (actors.length !== 2) {
    throw new Error(`ensureBlePairedActors expects exactly two actors, got ${actors.length}`);
  }

  const [a, b] = await waitForActorsReady(actors, timeoutMs);

  // External actors are real apps on real devices carrying a real pairing.
  // Seeding a fake address over that would overwrite the thing the run
  // depends on, so the hardware path asserts the pairing rather than
  // manufacturing it -- and only ensures the per-pair Bluetooth toggle is on,
  // which since ADR-0006 costs one boolean and no bond.
  if (a.actor.kind === 'external' || b.actor.kind === 'external') {
    await ensureBluetoothEnabledForPeer(a, b.identity.device_id);
    await ensureBluetoothEnabledForPeer(b, a.identity.device_id);
    // Resume dialling on both sides before the run starts. A real device may
    // arrive already in `bluetooth_dial_exhausted` from earlier activity, and
    // that state is left only by an explicit user retry -- so without this the
    // spec waits out its whole timeout on a pair that has simply stopped
    // trying. This is the same call the Device row's "tap to try again"
    // affordance makes; it resumes the automatic path rather than standing in
    // for it, so what the spec then measures is still the real dial.
    await a.actor.invoke('device_connection_retry_bluetooth_dial', {
      peerDeviceId: b.identity.device_id,
    });
    await b.actor.invoke('device_connection_retry_bluetooth_dial', {
      peerDeviceId: a.identity.device_id,
    });
    return [a, b];
  }

  await a.actor.invoke('device_connection_save_paired_device', {
    peerDeviceId: b.identity.device_id,
    displayName: b.identity.hostname,
    bluetoothAddress: fakeBluetoothAddress(1),
    viaBluetooth: true,
  });
  await b.actor.invoke('device_connection_save_paired_device', {
    peerDeviceId: a.identity.device_id,
    displayName: a.identity.hostname,
    bluetoothAddress: fakeBluetoothAddress(0),
    viaBluetooth: true,
  });

  return [a, b];
}

interface PairedDeviceRow {
  peer_device_id: string;
  bluetooth_enabled: boolean;
  bluetooth_address: string | null;
}

/**
 * The hardware precondition, stated as an assertion rather than set up by the
 * test: this actor already knows the peer, has Bluetooth enabled for it, and
 * holds an address. A failure here means the devices were never paired over
 * Bluetooth, which is a setup problem the run cannot fix for itself -- and a
 * far clearer message than the timeout it would otherwise become.
 */
/**
 * Asserts the pairing this run depends on, then makes sure Bluetooth is
 * actually switched on for it.
 *
 * The pairing itself is still only asserted, never manufactured: these are
 * real apps on real devices, and inventing a pair would replace the thing the
 * run is supposed to exercise.
 *
 * Enablement is different, and ADR-0006 is why. It used to be an assertion
 * too, on the reasoning that the flag stood for a real OS-level bond that a
 * test had no business fabricating. There is no bond any more -- enabling
 * writes one boolean and needs no address -- so asserting it only made the
 * lane fail for a reason the lane could fix. It also failed for real: a
 * device that had run an older build could arrive with the flag cleared by
 * the self-report path that used to switch Bluetooth *off* when it found no
 * bond, leaving a permanently red lane and a peer reporting
 * `auth rejected: bluetooth disabled for this pair` with nothing naming the
 * cause.
 *
 * A stored address is deliberately not required. Since ADR-0006 nothing
 * dials it, and a phone that advertises under a rotating address will not
 * have one that means anything.
 */
async function ensureBluetoothEnabledForPeer(
  actor: SyncedActor,
  peerDeviceId: string,
): Promise<void> {
  const paired = await actor.actor.invoke<PairedDeviceRow[]>('device_connection_get_paired_devices');
  const row = paired.find((entry) => entry.peer_device_id === peerDeviceId);
  expect(row, `${actor.actor.slug} should already be paired with ${peerDeviceId}`).toBeTruthy();

  if (row?.bluetooth_enabled) {
    return;
  }
  await actor.actor.invoke('device_connection_set_bluetooth_transport', {
    input: { peerDeviceId, enabled: true },
  });

  const after = await actor.actor.invoke<PairedDeviceRow[]>('device_connection_get_paired_devices');
  expect(
    after.find((entry) => entry.peer_device_id === peerDeviceId)?.bluetooth_enabled,
    `${actor.actor.slug} should have Bluetooth enabled for ${peerDeviceId}`,
  ).toBe(true);
}

export async function waitForBleSession(actor: E2EActor, timeoutMs = 60_000): Promise<void> {
  await pollUntil(`${actor.slug} session established over BLE transport`, async () => {
    await actor.invoke('space_sync_tick');
    const status = await actor.invoke<PeerSessionDebugStatus>('device_connection_debug_status');
    return status.peer_session_count > 0 || false;
  }, timeoutMs, 1_000);
}

/**
 * Proves the network transport genuinely cannot carry this session, so a
 * Bluetooth result is not one the network quietly produced.
 *
 * Two shapes, because the two lanes make the network unavailable in different
 * ways. Spawned actors are launched with `FINI_DISCOVERY_DISABLED=1`, so they
 * see no presence at all and the global assertion is the strongest one
 * available. External actors are real apps that cannot be relaunched with that
 * flag: there, the network is made unavailable by switching the phone's Wi-Fi
 * off, and what must hold is that *this peer* is absent -- another device on
 * the desktop's network is irrelevant and must not fail the run.
 */
export async function expectNetworkTransportUnavailable(
  actor: E2EActor,
  peerDeviceId?: string,
): Promise<void> {
  const presence = await actor.invoke<{ device_id?: string; last_seen_at?: string }[]>(
    'device_connection_presence_snapshot',
  );

  // A spawned actor was launched with FINI_DISCOVERY_DISABLED=1 and can see
  // nothing at all, so assert exactly that -- it is the stronger claim, and
  // weakening it to "not this peer" for both lanes would quietly stop proving
  // the flag works.
  if (actor.kind === 'spawned' || peerDeviceId === undefined) {
    expect(presence, `${actor.slug} should have no network presence (FINI_DISCOVERY_DISABLED)`).toHaveLength(0);
    return;
  }

  // Freshness, not mere membership. The snapshot is not TTL-filtered -- it
  // keeps every peer it has ever heard from, so a device that dropped off the
  // network still appears in it indefinitely. Observed on this pair: entries
  // fifteen hours stale sitting alongside live ones. Asking "is this peer in
  // the list" would therefore never pass on hardware, however unreachable the
  // peer actually is; asking "has it been heard from recently" is the question
  // that matches what the assertion means.
  const cutoff = Date.now() - PRESENCE_FRESHNESS_MS;
  const freshPeerPresence = presence.filter((entry) => {
    if (entry.device_id !== peerDeviceId) {
      return false;
    }
    const seenAt = entry.last_seen_at ? Date.parse(entry.last_seen_at) : Number.NaN;
    return Number.isNaN(seenAt) ? true : seenAt >= cutoff;
  });

  expect(
    freshPeerPresence,
    `${actor.slug} still sees live network presence for ${peerDeviceId} -- ` +
      "turn the phone's Wi-Fi off so Bluetooth is the only path left",
  ).toHaveLength(0);
}

/**
 * How recent a presence beacon has to be to count as "the network can still
 * reach this peer". Four times `DISCOVERY_TTL_SECS` (15s, see
 * `device_connection`), so a peer that is genuinely live is never mistaken for
 * a stale record on a slow beacon cycle.
 */
const PRESENCE_FRESHNESS_MS = 60_000;

/**
 * "Green" is `state: "configured"` with `code: null` -- the ping/ack
 * liveness proof has completed, not merely that a session exists. This is
 * the exact signal the "Still connecting..." investigation found the
 * frontend was getting wrong (see `src/stores/device.ts`'s
 * `refreshLiveConnectedState` and its regression test), so asserting it
 * here directly guards that class of bug, not just "a session exists
 * somewhere."
 */
export async function waitForGreenTransport(
  actor: E2EActor,
  peerDeviceId: string,
  timeoutMs = 60_000,
): Promise<void> {
  await pollUntil(`${actor.slug} bluetooth transport reports green`, async () => {
    await actor.invoke('space_sync_tick');
    const statuses = await actor.invoke<TransportStatus[]>('device_connection_transport_statuses', {
      peerDeviceId,
    });
    const bluetooth = statuses.find((status) => status.kind === 'bluetooth');
    return (bluetooth?.state.state === 'configured' && bluetooth.state.code === null) || false;
  }, timeoutMs, 1_000);
}

/**
 * The UI-facing half of "green": not just that the backend reports a live
 * session, but that `DeviceView.vue`'s Bluetooth row actually renders
 * "Connected now" -- and, just as importantly, never renders "Still
 * connecting..." along the way. Throws the moment that text appears
 * (mirrors `waitForApproveDialogToClose`'s `state.error` pattern above) so
 * `pollUntil`'s timeout message names the actual regression instead of a
 * generic "timed out" -- a transient "Still connecting..." blip before
 * settling green is exactly the class of regression this e2e lane exists
 * to catch.
 */
export async function waitForBluetoothRowConnectedInUi(
  actor: E2EActor,
  peerDeviceId: string,
  timeoutMs = 60_000,
): Promise<void> {
  await pollUntil(`${actor.slug} bluetooth row shows Connected now in the UI`, async () => {
    await openDeviceDetailsFromSettings(actor, peerDeviceId);
    await actor.invoke('space_sync_tick');
    const text = await actor.page.textContent(
      '[data-testid="transport-status-row"][data-transport-kind="bluetooth"]',
    );
    const value = text?.trim() ?? '';
    if (value.includes('Still connecting')) {
      throw new Error(`${actor.slug} bluetooth row shows "Still connecting..." -- regression guard tripped`);
    }
    return value.includes('Connected now') ? value : false;
  }, timeoutMs, 1_000);
}
