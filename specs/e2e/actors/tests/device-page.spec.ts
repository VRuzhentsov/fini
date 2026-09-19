import { test, expect } from '../fixtures.ts';
import { ensureSyncedActors } from '../helpers/device-sync.ts';
import { ensurePersonalSpaceSync, openDeviceDetailsFromSettings } from '../helpers/personal-sync.ts';
import {
  channelReason,
  toggleChannel,
  waitForChannelState,
  waitForSyncQueue,
} from '../helpers/device-page.ts';

/**
 * The Device page, as ADR-0008 rebuilt it: channels with their own
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

  const transport = await actorA.invoke<string | null>('device_connection_session_transport', {
    peerDeviceId: syncedB.identity.device_id,
  });
  expect(transport, 'network session must be closed once the channel is off').not.toBe('tcp_ws');

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
