import { expect } from '../fixtures.ts';
import type { E2EActor } from '../fixtures.ts';
import { pollUntil } from './dom.ts';
import { waitForActorsReady, type SyncedActor } from './device-sync.ts';

/**
 * Readiness + pairing for the Sim-transport actor suite
 * (`FINI_E2E_TRANSPORT=sim`). Those actors are spawned with
 * `FINI_DISCOVERY_DISABLED=1` — no mDNS, no UDP presence — so the normal
 * `ensureSyncedActors` flow (which pairs via the discovered-nearby-devices
 * UI and waits on `device_connection_presence_snapshot`) cannot apply here:
 * there is nothing to discover by design, the same way there would be
 * nothing to discover over Bluetooth before an OS-level pairing exists.
 * Pairing is done directly via `device_connection_save_paired_device` (the
 * same command the pairing UI calls at the end of its flow), then readiness
 * is driven by `space_sync_tick` until a session claims itself over Sim.
 */

interface PeerSessionDebugStatus {
  peer_session_count: number;
}

export async function ensureSimPairedActors(
  actors: E2EActor[],
  timeoutMs = 60_000,
): Promise<SyncedActor[]> {
  if (actors.length !== 2) {
    throw new Error(`ensureSimPairedActors expects exactly two actors, got ${actors.length}`);
  }

  const [a, b] = await waitForActorsReady(actors, timeoutMs);

  await a.actor.invoke('device_connection_save_paired_device', {
    peerDeviceId: b.identity.device_id,
    displayName: b.identity.hostname,
  });
  await b.actor.invoke('device_connection_save_paired_device', {
    peerDeviceId: a.identity.device_id,
    displayName: a.identity.hostname,
  });

  return [a, b];
}

export async function waitForSimSession(actor: E2EActor, timeoutMs = 60_000): Promise<void> {
  await pollUntil(`${actor.slug} session established over Sim transport`, async () => {
    await actor.invoke('space_sync_tick');
    const status = await actor.invoke<PeerSessionDebugStatus>('device_connection_debug_status');
    return status.peer_session_count > 0 || false;
  }, timeoutMs, 1_000);
}

export async function expectNetworkTransportUnavailable(actor: E2EActor): Promise<void> {
  const presence = await actor.invoke<unknown[]>('device_connection_presence_snapshot');
  expect(presence, `${actor.slug} should have no network presence (FINI_DISCOVERY_DISABLED)`).toHaveLength(0);
}
