import { test, expect } from '../fixtures.ts';
import { ensureSyncedActors } from '../helpers/device-sync.ts';
import { ensurePersonalSpaceSync, openDeviceDetailsFromSettings } from '../helpers/personal-sync.ts';
import {
  anyChannelConnected,
  channelReason,
  deviceDotConnected,
  toggleChannel,
  waitForChannelState,
  waitForSyncQueue,
} from '../helpers/device-page.ts';

/**
 * The Device page, as ADR-0007 rebuilt it: channels with their own
 * switches, a sync queue, and an unlink that says what it costs.
 *
 * These cover what unit tests structurally cannot. `DeviceView.spec.ts`
 * mocks the store, so it proves the page renders what it is handed --
 * these prove two real apps actually behave that way, which is the part
 * that matters for a switch whose whole job is to stop traffic.
 */

test('turning a channel off stops the traffic, not just the colour of the row', async ({
  actorA,
  actorB,
}) => {
  const [syncedA, syncedB] = await ensureSyncedActors([actorA, actorB], { pairViaUi: true });
  await ensurePersonalSpaceSync(
    actorA,
    syncedB.identity.device_id,
    actorB,
    syncedA.identity.device_id,
  );

  await waitForChannelState(actorA, syncedB.identity.device_id, 'network', 'connected');

  // Off. A greyed row over a live session would be the exact lie this
  // design exists to remove, so assert the session itself is gone rather
  // than trusting the row.
  await toggleChannel(actorA, 'network');
  await waitForChannelState(actorA, syncedB.identity.device_id, 'network', 'off');

  // Sampled repeatedly, not once. A single check moments after the toggle
  // passes even when the switch does nothing: the peer's own dial loop simply
  // has not come back round yet. That is exactly how this assertion passed
  // while the session was in fact being re-established seconds later --
  // caught on hardware, where the peer dials on its own schedule rather than
  // the test's. Closing our dial loop is only half the switch; refusing the
  // peer's inbound dial is the other half (`check_network_enabled`).
  for (let attempt = 0; attempt < 10; attempt += 1) {
    const transport = await actorA.invoke<string | null>('device_connection_session_channel', {
      peerDeviceId: syncedB.identity.device_id,
    });
    expect(
      transport,
      'network session must stay closed while the channel is off, including against an inbound dial',
    ).not.toBe('tcp_ws');
    await actorA.invoke('space_sync_tick');
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }

  // And back on again: the switch has to be a switch, not a one-way door.
  await toggleChannel(actorA, 'network');
  await waitForChannelState(actorA, syncedB.identity.device_id, 'network', 'connected', 60_000);
});

/**
 * "On, waiting": the channel is on, and this machine's own radio is what's
 * missing. The e2e container runs a D-Bus system bus with no `bluetoothd`
 * on it (see `scripts/e2e-runner.sh`), so `LinuxBackend::new()` genuinely
 * fails here -- the same condition as Bluetooth switched off on a laptop,
 * arrived at honestly rather than mocked.
 *
 * The row must say so about *this* device. Reporting the peer as "not
 * nearby" would be a claim with no evidence behind it, about the wrong
 * machine, and the ordering in `bluetooth_unconfigured_code` exists
 * specifically to prevent it.
 */
test('a channel switched on with no usable radio waits, and blames this device', async ({
  actorA,
  actorB,
}) => {
  const [, syncedB] = await ensureSyncedActors([actorA, actorB], { pairViaUi: true });

  await openDeviceDetailsFromSettings(actorA, syncedB.identity.device_id);
  await waitForChannelState(actorA, syncedB.identity.device_id, 'bluetooth', 'off');

  await toggleChannel(actorA, 'bluetooth');

  // Not "down", and specifically not a failure: the switch stays on and
  // the channel is expected to start by itself once a radio appears.
  await waitForChannelState(actorA, syncedB.identity.device_id, 'bluetooth', 'waiting', 60_000);

  const reason = await channelReason(actorA, 'bluetooth');
  expect(reason, 'the reason must name this computer').toContain('this computer');
  expect(reason, 'must not blame the peer for a local radio problem').not.toContain("isn't nearby");
});

/**
 * The sync queue answers "has my stuff synced?".
 *
 * Made deterministic by closing the channel first: with nothing able to
 * reach the peer, the queue has to report a backlog rather than racing a
 * tick that would drain it. Turning the channel back on then proves the
 * queue actually drains rather than merely displaying a number.
 */
test('the sync queue reports what cannot move, then that it has moved', async ({
  actorA,
  actorB,
}) => {
  const [syncedA, syncedB] = await ensureSyncedActors([actorA, actorB], { pairViaUi: true });
  await ensurePersonalSpaceSync(
    actorA,
    syncedB.identity.device_id,
    actorB,
    syncedA.identity.device_id,
  );

  await waitForChannelState(actorA, syncedB.identity.device_id, 'network', 'connected');
  await toggleChannel(actorA, 'network');
  await waitForChannelState(actorA, syncedB.identity.device_id, 'network', 'off');

  const title = `queued quest ${Date.now()}`;
  await actorA.invoke('create_quest', {
    input: { title, description: null, space_id: '1', is_checklist: false },
  });

  const stalled = await waitForSyncQueue(
    actorA,
    syncedB.identity.device_id,
    (text) => text.includes('waiting'),
    'reports a backlog while nothing can reach the peer',
  );
  expect(stalled).toContain('waiting');

  await toggleChannel(actorA, 'network');
  await waitForChannelState(actorA, syncedB.identity.device_id, 'network', 'connected', 60_000);

  const drained = await waitForSyncQueue(
    actorA,
    syncedB.identity.device_id,
    (text) => text.includes('Everything synced'),
    'drains once the channel is back',
    60_000,
  );
  expect(drained).toContain('Everything synced');
});

/**
 * Unlinking states what actually stops -- the spaces, by name -- and that
 * nothing is deleted. A generic "are you sure?" gives the person nothing
 * to decide on.
 */
test('unlinking names the spaces that stop syncing and promises nothing is deleted', async ({
  actorA,
  actorB,
}) => {
  const [syncedA, syncedB] = await ensureSyncedActors([actorA, actorB], { pairViaUi: true });
  await ensurePersonalSpaceSync(
    actorA,
    syncedB.identity.device_id,
    actorB,
    syncedA.identity.device_id,
  );

  await openDeviceDetailsFromSettings(actorA, syncedB.identity.device_id);
  await actorA.page.waitForSelector('[data-testid="unlink-device"]', 30_000);
  await actorA.page.click('[data-testid="unlink-device"]');

  const confirmation = await actorA.page.evaluate<string>(`(() => {
    const el = document.querySelector('[data-testid="unlink-dialog"]');
    return el ? (el.textContent ?? '').replace(/\\s+/g, ' ').trim() : '';
  })()`);

  expect(confirmation).toContain('Personal');
  expect(confirmation).toContain('Nothing is deleted');
});

/**
 * The devices list's dot, and the line that is no longer under it.
 *
 * The dot used to read presence -- a beacon heard on the LAN -- so a machine
 * that was merely discoverable showed green while no session existed. Found
 * by using the app, not by a test, which is why this one asserts the two
 * surfaces against *each other* rather than hard-coding both: the list's dot
 * and the device page's rows are two renderings of one fact, and the bug was
 * them disagreeing. Hard-coding each separately would let them drift apart
 * again and still pass.
 */
test('the devices list dot agrees with the channel rows', async ({ actorA, actorB }) => {
  const [, syncedB] = await ensureSyncedActors([actorA, actorB], { pairViaUi: true });
  const peerId = syncedB.identity.device_id;

  await waitForChannelState(actorA, peerId, 'network', 'connected');
  expect(await anyChannelConnected(actorA, peerId), 'a channel row is connected').toBe(true);
  expect(await deviceDotConnected(actorA, peerId), 'so the dot is the connected colour').toBe(true);

  // Nothing connected. The dot has to follow the session, not the peer's
  // continued presence on the network -- which is unchanged here, and is
  // exactly what the old implementation was reading.
  await openDeviceDetailsFromSettings(actorA, peerId);
  await toggleChannel(actorA, 'network');
  await waitForChannelState(actorA, peerId, 'network', 'off');

  expect(await anyChannelConnected(actorA, peerId), 'no channel row is connected').toBe(false);
  expect(await deviceDotConnected(actorA, peerId), 'so the dot must not be green').toBe(false);

  // And the row says nothing on its own. The detail exists, but only for
  // someone who presses the button for it.
  const detail = await actorA.page.evaluate<{ shown: boolean; hasInfo: boolean }>(`(() => {
    const row = document.querySelector('[data-testid="paired-device-row"]');
    return {
      shown: !!row?.querySelector('[data-testid="paired-device-detail"]'),
      hasInfo: !!row?.querySelector('[data-testid="paired-device-info"]'),
    };
  })()`);
  expect(detail.shown, 'the list must not explain each device unasked').toBe(false);
  expect(detail.hasInfo, 'but the detail must still be reachable').toBe(true);

  await openDeviceDetailsFromSettings(actorA, peerId);
  await toggleChannel(actorA, 'network');
  await waitForChannelState(actorA, peerId, 'network', 'connected', 60_000);
});
