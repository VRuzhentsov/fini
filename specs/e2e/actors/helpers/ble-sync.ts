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

export async function waitForBleSession(actor: E2EActor, timeoutMs = 60_000): Promise<void> {
  await pollUntil(`${actor.slug} session established over BLE transport`, async () => {
    await actor.invoke('space_sync_tick');
    const status = await actor.invoke<PeerSessionDebugStatus>('device_connection_debug_status');
    return status.peer_session_count > 0 || false;
  }, timeoutMs, 1_000);
}

export async function expectNetworkTransportUnavailable(actor: E2EActor): Promise<void> {
  const presence = await actor.invoke<unknown[]>('device_connection_presence_snapshot');
  expect(presence, `${actor.slug} should have no network presence (FINI_DISCOVERY_DISABLED)`).toHaveLength(0);
}

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
