import { expect } from '../fixtures.ts';
import type { E2EActor } from '../fixtures.ts';
import { pollUntil } from './dom.ts';
import { waitForActorsReady, type SyncedActor } from './device-sync.ts';
import { openDeviceDetailsFromSettings } from './personal-sync.ts';
import { ChannelColor, ChannelKind, ChannelState } from '../../../../src/utils/channel.ts';

/**
 * Readiness + pairing for the BLE-transport actor suite
 * (`FINI_E2E_TRANSPORT=ble`). Actors are spawned with
 * `FINI_DISCOVERY_DISABLED=1`, so there is nothing to discover by design and
 * the normal `ensureSyncedActors` UI-pair flow doesn't apply. What is
 * underneath is the real thing: these
 * actors dial the real `ble.rs` code path (dial loop, peripheral accept,
 * session claim) against a cross-process mock radio instead of a plain TCP
 * stand-in, and pairing is marked `viaBluetooth: true` so the Bluetooth
 * channel is switched on for the pair -- which is what
 * `bluetooth_dial_candidates` actually reads. See `fixtures.ts`'s
 * `fakeBluetoothAddress` wiring and
 * `docs/adr/0004-mock-broker-for-cross-process-e2e.md` in `ble-gatt`.
 */

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
    return [a, b];
  }

  await a.actor.invoke('device_connection_save_paired_device', {
    peerDeviceId: b.identity.device_id,
    displayName: b.identity.hostname,
    bluetoothAddress: fakeBluetoothAddress(1),
    viaBluetooth: true,
    // The key a real pairing pins (ADR-0009 D8); without it every
    // Bluetooth session is refused.
    endpointId: b.identity.endpoint_id,
  });
  await b.actor.invoke('device_connection_save_paired_device', {
    peerDeviceId: a.identity.device_id,
    displayName: a.identity.hostname,
    bluetoothAddress: fakeBluetoothAddress(0),
    viaBluetooth: true,
    // The key a real pairing pins (ADR-0009 D8); without it every
    // Bluetooth session is refused.
    endpointId: a.identity.endpoint_id,
  });

  return [a, b];
}

interface PairedDeviceRow {
  peer_device_id: string;
}

interface ChannelStatusRow {
  kind: ChannelKind;
  state: ChannelState;
  color: ChannelColor;
}

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
  expect(
    paired.some((entry) => entry.peer_device_id === peerDeviceId),
    `${actor.actor.slug} should already be paired with ${peerDeviceId}`,
  ).toBe(true);

  // The switch lives in `channels` now, not on the paired row: a pair made
  // over the network has no Bluetooth row at all, so asking the paired row
  // whether Bluetooth is on can only ever answer "no".
  const statuses = await actor.actor.invoke<ChannelStatusRow[]>(
    'device_connection_channel_statuses',
    { peerDeviceId },
  );
  const bluetooth = statuses.find((status) => status.kind === ChannelKind.Bluetooth);
  if (bluetooth?.state === ChannelState.On) {
    return;
  }
  // A channel that does not exist can only be set up with the peer, in the
  // setup dialog on both devices (ADR-0008 D1); a test cannot switch it into
  // being.
  expect(
    bluetooth?.state,
    `${actor.actor.slug} has no Bluetooth channel with ${peerDeviceId} -- set it up in the app first`,
  ).toBe(ChannelState.Off);

  const after = await actor.actor.invoke<ChannelStatusRow[]>(
    'device_connection_set_channel_enabled',
    { peerDeviceId, kind: ChannelKind.Bluetooth, enabled: true },
  );
  expect(
    after.find((status) => status.kind === ChannelKind.Bluetooth)?.state,
    `${actor.actor.slug} should have Bluetooth on for ${peerDeviceId}`,
  ).toBe(ChannelState.On);
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
export async function expectNetworkChannelUnavailable(
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
 * Green is the backend's own verdict (ADR-0008 D9): the peer was heard on
 * this channel within its timeout, or an exchange with it is open.
 */
export async function waitForGreenChannel(
  actor: E2EActor,
  peerDeviceId: string,
  timeoutMs = 60_000,
): Promise<void> {
  await pollUntil(`${actor.slug} bluetooth transport reports green`, async () => {
    await actor.invoke('space_sync_tick');
    const statuses = await actor.invoke<ChannelStatusRow[]>('device_connection_channel_statuses', {
      peerDeviceId,
    });
    const bluetooth = statuses.find((status) => status.kind === ChannelKind.Bluetooth);
    return bluetooth?.color === ChannelColor.Green || false;
  }, timeoutMs, 1_000);
}

/**
 * The UI-facing half of "green": the Device page's Bluetooth row draws the
 * colour the backend reports. Reads `data-channel-color` rather than the
 * row's text, which is copy and moves with the design.
 *
 * Orange aborts the poll at once: it is a problem on this device (the radio)
 * that no amount of waiting in the test will clear.
 */
export async function waitForBluetoothRowConnectedInUi(
  actor: E2EActor,
  peerDeviceId: string,
  timeoutMs = 60_000,
): Promise<void> {
  const selector = `[data-testid="channel-status-row"][data-channel-kind="${ChannelKind.Bluetooth}"]`;

  // Opened once and left open: the open page is what runs the status search
  // (ADR-0008 D12), and it re-reads its rows every few seconds. Re-opening it
  // on every poll restarted the search each time, too briefly to hear anyone.
  await openDeviceDetailsFromSettings(actor, peerDeviceId);
  await pollUntil(`${actor.slug} bluetooth row is green in the UI`, async () => {
    await actor.invoke('space_sync_tick');

    const row = actor.page.locator(selector);
    const color = (await row.count()) > 0 ? ((await row.getAttribute('data-channel-color')) ?? '') : '';
    if (color === '') {
      await openDeviceDetailsFromSettings(actor, peerDeviceId);
      return false;
    }

    if (color === ChannelColor.Orange) {
      throw new Error(`${actor.slug} bluetooth row reports a problem on this device`);
    }

    return color === ChannelColor.Green ? color : false;
  }, timeoutMs, 1_000);
}
