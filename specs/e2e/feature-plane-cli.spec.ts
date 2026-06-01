import { test, expect } from '@playwright/test';
import { CliClient, createCliClient } from './fixtures/cli.ts';

interface PairedDevice {
  peer_device_id: string;
  display_name: string;
  last_seen_at: string | null;
}

interface SpaceSyncStatus {
  peer_device_id: string | null;
  mapped_space_ids: string[];
  outbox_event_count: number;
}

let cli: CliClient;

test.beforeEach(() => {
  cli = createCliClient();
});

test.afterEach(() => {
  cli.close();
});

test('device paired CRUD and sync mappings are available from CLI', async () => {
  const saved = cli.run<PairedDevice>([
    'device', 'paired', 'save',
    '--peer-device-id', 'peer-a',
    '--display-name', 'Peer A',
  ]);
  expect(saved.peer_device_id).toBe('peer-a');

  cli.run(['device', 'paired', 'update-last-seen', '--peer-device-id', 'peer-a', '--last-seen-at', '2026-05-28T12:00:00Z']);

  const devices = cli.run<PairedDevice[]>(['device', 'paired', 'list']);
  expect(devices).toHaveLength(1);
  expect(devices[0].last_seen_at).toBe('2026-05-28T12:00:00Z');

  const mapped = cli.run<string[]>([
    'sync', 'mappings', 'update',
    '--peer-device-id', 'peer-a',
    '--mapped-space-id', '1',
  ]);
  expect(mapped).toEqual(['1']);

  const status = cli.run<SpaceSyncStatus>(['sync', 'status', '--peer-device-id', 'peer-a']);
  expect(status.peer_device_id).toBe('peer-a');
  expect(status.mapped_space_ids).toEqual(['1']);

  cli.run(['device', 'paired', 'unpair', '--peer-device-id', 'peer-a']);
  expect(cli.run<PairedDevice[]>(['device', 'paired', 'list'])).toHaveLength(0);
});

test('CLI quest changes emit sync outbox entries for shared sync core', async () => {
  const before = cli.run<SpaceSyncStatus>(['sync', 'status']);
  expect(before.outbox_event_count).toBe(0);

  cli.run(['quest', 'create', '--title', 'sync from cli']);

  const after = cli.run<SpaceSyncStatus>(['sync', 'status']);
  expect(after.outbox_event_count).toBe(1);
});

test('persistent settings core is available from CLI', async () => {
  expect(cli.run<{ mode: string }>(['settings', 'theme-get']).mode).toBe('system');
  expect(cli.run<{ mode: string }>(['settings', 'theme-set', '--mode', 'dark']).mode).toBe('dark');
  expect(cli.run<{ mode: string }>(['settings', 'theme-get']).mode).toBe('dark');
});
