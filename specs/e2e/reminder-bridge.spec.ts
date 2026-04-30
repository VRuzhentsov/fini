/**
 * E2E tests for the reminder bridge via the CLI surface.
 *
 * Tests drive the Fini CLI against an isolated temp SQLite DB,
 * so no display, GUI, or OS notification delivery is required.
 *
 * Wall-clock note: the CLI process inherits TZ=UTC inside Docker, which makes
 * due_at_utc exactly equal to the local due date+time — stable for assertions.
 */
import { test, expect } from '@playwright/test';
import { CliClient, createCliClient } from './fixtures/cli.ts';

interface Quest {
  id: string;
  status: string;
  due: string | null;
  due_time: string | null;
}

interface Reminder {
  id: string;
  quest_id: string;
  due_at_utc: string | null;
  kind: string;
}

// Each test gets its own CLI client (and its own temp DB).
let cli: CliClient;

test.beforeEach(() => {
  cli = createCliClient();
});

test.afterEach(() => {
  cli.close();
});

async function createReminderQuest(title: string): Promise<Quest> {
  return cli.run<Quest>(['quest', 'create', '--title', title]);
}

async function reminders(questId: string): Promise<Reminder[]> {
  return cli.run<Reminder[]>(['reminder', 'list', '--quest-id', questId]);
}

async function updateQuest(id: string, ...args: string[]): Promise<Quest> {
  return cli.run<Quest>(['quest', 'update', '--id', id, ...args]);
}

async function completeQuest(id: string): Promise<Quest> {
  return cli.run<Quest>(['quest', 'complete', '--id', id]);
}

async function abandonQuest(id: string): Promise<Quest> {
  return cli.run<Quest>(['quest', 'abandon', '--id', id]);
}

// ── Core bridge tests ─────────────────────────────────────────────────────────

test('setting due+due_time auto-creates a reminder row', async () => {
  const q = await createReminderQuest('bridge: create on due set');

  const before = await reminders(q.id);
  expect(before).toHaveLength(0);

  await updateQuest(q.id, '--due', '2099-06-15', '--due-time', '14:30');

  const after = await reminders(q.id);
  expect(after).toHaveLength(1);
  expect(after[0].quest_id).toBe(q.id);
  expect(after[0].kind).toBe('absolute');
  // In UTC (Docker container TZ=UTC): local 14:30 == 14:30 UTC
  expect(after[0].due_at_utc).toBe('2099-06-15T14:30:00Z');
});

test('date-only quest defaults reminder to 09:00 local', async () => {
  const q = await createReminderQuest('bridge: date-only defaults to 09:00');

  await updateQuest(q.id, '--due', '2099-06-15');

  const r = await reminders(q.id);
  expect(r).toHaveLength(1);
  expect(r[0].due_at_utc).toBe('2099-06-15T09:00:00Z');
});

test('changing due_time reschedules the same reminder row', async () => {
  const q = await createReminderQuest('bridge: reschedule on due_time change');
  await updateQuest(q.id, '--due', '2099-06-15', '--due-time', '08:00');

  const first = await reminders(q.id);
  expect(first).toHaveLength(1);
  expect(first[0].due_at_utc).toBe('2099-06-15T08:00:00Z');
  const reminderId = first[0].id;

  await updateQuest(q.id, '--due-time', '17:45');

  const second = await reminders(q.id);
  expect(second).toHaveLength(1);
  expect(second[0].id).toBe(reminderId);           // same row — upsert, not insert
  expect(second[0].due_at_utc).toBe('2099-06-15T17:45:00Z');
});

test('completing a quest deletes its reminder', async () => {
  const q = await createReminderQuest('bridge: delete on complete');
  await updateQuest(q.id, '--due', '2099-06-15', '--due-time', '10:00');
  expect(await reminders(q.id)).toHaveLength(1);

  await completeQuest(q.id);

  expect(await reminders(q.id)).toHaveLength(0);
});

test('abandoning a quest deletes its reminder', async () => {
  const q = await createReminderQuest('bridge: delete on abandon');
  await updateQuest(q.id, '--due', '2099-06-15', '--due-time', '10:00');
  expect(await reminders(q.id)).toHaveLength(1);

  await abandonQuest(q.id);

  expect(await reminders(q.id)).toHaveLength(0);
});

test('quest with no due date has no reminder', async () => {
  const q = await createReminderQuest('bridge: no reminder without due');

  await updateQuest(q.id, '--title', 'updated title');

  expect(await reminders(q.id)).toHaveLength(0);
});

test('changing due date updates due_at_utc', async () => {
  const q = await createReminderQuest('bridge: due date change');
  await updateQuest(q.id, '--due', '2099-06-15', '--due-time', '10:00');

  await updateQuest(q.id, '--due', '2099-07-20');

  const r = await reminders(q.id);
  expect(r).toHaveLength(1);
  expect(r[0].due_at_utc).toBe('2099-07-20T10:00:00Z');
});

test('creating a quest with due date auto-creates its reminder row', async () => {
  const q = cli.run<Quest>(['quest', 'create', '--title', 'bridge: create seeds reminder', '--due', '2099-06-15']);

  const r = await reminders(q.id);
  expect(r).toHaveLength(1);
  expect(r[0].due_at_utc).toBe('2099-06-15T09:00:00Z');
});
