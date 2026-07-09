import { test, expect } from '../fixtures.ts';

interface Quest {
  id: string;
  title: string;
  status: string;
}

interface Reminder {
  id: string;
  quest_id: string;
  due_at_utc: string | null;
}

interface NotificationEvent {
  phase: string;
  delivery_path: string;
  reminder_id: string;
  quest_id: string;
  body: string;
  due_at_utc: string | null;
  scheduled_notification_id: string | null;
}

async function invokeTauri<T>(
  tauriPage: { evaluate: <R>(script: string) => Promise<R> },
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  return tauriPage.evaluate<T>(`(async () => {
    const invoke = window.__TAURI_INTERNALS__?.invoke;
    if (!invoke) throw new Error('Tauri invoke is unavailable');
    return await invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args ?? {})});
  })()`);
}

async function fillTextarea(
  tauriPage: { evaluate: <R>(script: string) => Promise<R> },
  selector: string,
  value: string,
): Promise<void> {
  await tauriPage.evaluate(`(() => {
    const el = document.querySelector(${JSON.stringify(selector)});
    if (!(el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement)) {
      throw new Error('text control not found: ' + ${JSON.stringify(selector)});
    }
    el.focus();
    const prototype = el instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(prototype, 'value')?.set;
    if (!setter) throw new Error('text control setter is unavailable');
    setter.call(el, ${JSON.stringify(value)});
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  })()`);
}

async function createQuestWithTodayReminder(
  tauriPage: { waitForSelector: (selector: string, t?: number) => Promise<unknown>; waitForFunction: (script: string, t?: number) => Promise<unknown>; press: (selector: string, key: string) => Promise<void>; click: (selector: string) => Promise<void>; evaluate: <R>(script: string) => Promise<R> },
  title: string,
): Promise<{ quest: Quest; reminder: Reminder }> {
  await tauriPage.waitForSelector('[data-testid="chat-input"]', 30_000);
  await fillTextarea(tauriPage, '[data-testid="chat-input"]', title);
  await tauriPage.press('[data-testid="chat-input"]', 'Enter');

  await tauriPage.waitForSelector('.quest-row-surface', 10_000);
  await tauriPage.waitForFunction(`(() => {
    const rows = Array.from(document.querySelectorAll('.quest-row-surface'));
    return rows.some((r) => r.textContent?.includes(${JSON.stringify(title)}));
  })()`, 10_000);
  await tauriPage.evaluate(`(() => {
    const rows = Array.from(document.querySelectorAll('.quest-row-surface'));
    const row = rows.find((r) => r.textContent?.includes(${JSON.stringify(title)}));
    if (!(row instanceof HTMLElement)) throw new Error('quest row not found: ' + ${JSON.stringify(title)});
    row.click();
  })()`);

  await tauriPage.click('[data-testid="quest-reminder"]');
  await tauriPage.click('[data-testid="reminder-today"]');
  await tauriPage.click('[data-testid="reminder-done"]');

  const quests = await invokeTauri<Quest[]>(tauriPage, 'get_quests');
  const quest = quests.find((q) => q.title === title);
  if (!quest) throw new Error(`quest not found after creation: ${title}`);

  const reminders = await invokeTauri<Reminder[]>(tauriPage, 'get_reminders', { questId: quest.id });
  if (reminders.length === 0) throw new Error(`no reminder created for quest: ${title}`);

  return { quest, reminder: reminders[0] };
}

test('complete action marks quest as completed', async ({ tauriPage }) => {
  const title = `e2e notif complete ${Date.now()}`;

  await tauriPage.waitForSelector('nav.nav a[href="#/main"]', 30_000);
  await tauriPage.click('nav.nav a[href="#/main"]');
  await invokeTauri<void>(tauriPage, 'e2e_clear_notification_events');

  const { quest, reminder } = await createQuestWithTodayReminder(tauriPage, title);
  expect(quest.status).toBe('active');

  await invokeTauri<void>(tauriPage, 'e2e_dispatch_notification_action', {
    reminderId: reminder.id,
    actionId: 'complete',
  });

  const updatedQuests = await invokeTauri<Quest[]>(tauriPage, 'get_quests');
  const updated = updatedQuests.find((q) => q.id === quest.id);
  expect(updated?.status).toBe('completed');
});

test('snooze_30m action creates a snooze record without completing the quest', async ({ tauriPage }) => {
  const title = `e2e notif snooze30 ${Date.now()}`;

  await tauriPage.waitForSelector('nav.nav a[href="#/main"]', 30_000);
  await tauriPage.click('nav.nav a[href="#/main"]');
  await invokeTauri<void>(tauriPage, 'e2e_clear_notification_events');

  const { quest, reminder } = await createQuestWithTodayReminder(tauriPage, title);

  await invokeTauri<void>(tauriPage, 'e2e_dispatch_notification_action', {
    reminderId: reminder.id,
    actionId: 'snooze_30m',
  });

  const quests = await invokeTauri<Quest[]>(tauriPage, 'get_quests');
  const after = quests.find((q) => q.id === quest.id);
  expect(after?.status).toBe('active');

  const reminders = await invokeTauri<Reminder[]>(tauriPage, 'get_reminders', { questId: quest.id });
  expect(reminders.length).toBeGreaterThan(0);
});

test('snooze_1d action keeps quest active', async ({ tauriPage }) => {
  const title = `e2e notif snooze1d ${Date.now()}`;

  await tauriPage.waitForSelector('nav.nav a[href="#/main"]', 30_000);
  await tauriPage.click('nav.nav a[href="#/main"]');
  await invokeTauri<void>(tauriPage, 'e2e_clear_notification_events');

  const { quest, reminder } = await createQuestWithTodayReminder(tauriPage, title);

  await invokeTauri<void>(tauriPage, 'e2e_dispatch_notification_action', {
    reminderId: reminder.id,
    actionId: 'snooze_1d',
  });

  const quests = await invokeTauri<Quest[]>(tauriPage, 'get_quests');
  const after = quests.find((q) => q.id === quest.id);
  expect(after?.status).toBe('active');
});

test('schedule event is recorded when reminder is created', async ({ tauriPage }) => {
  const title = `e2e notif schedule ${Date.now()}`;

  await tauriPage.waitForSelector('nav.nav a[href="#/main"]', 30_000);
  await invokeTauri<void>(tauriPage, 'e2e_clear_notification_events');

  // Use IPC to create quest with an explicit future due date (tomorrow at 09:00 UTC).
  // The "today" UI shortcut defaults to 09:00 UTC which is already past in most CI runs,
  // causing schedule_reminder to early-return with delay_ms <= 0 and record no event.
  const tomorrow = new Date();
  tomorrow.setUTCDate(tomorrow.getUTCDate() + 1);
  const dueDate = tomorrow.toISOString().slice(0, 10); // "YYYY-MM-DD"
  const dueAtUtc = `${dueDate}T09:00:00Z`;

  const quest = await invokeTauri<Quest>(tauriPage, 'create_quest', {
    input: { title, due: dueDate, due_time: '09:00' },
  });
  const reminder = await invokeTauri<Reminder>(tauriPage, 'create_reminder', {
    input: { quest_id: quest.id, kind: 'absolute', due_at_utc: dueAtUtc },
  });

  // schedule_reminder is synchronous within create_reminder — event should be immediate.
  const events = await invokeTauri<NotificationEvent[]>(tauriPage, 'e2e_list_notification_events');
  const scheduled = events.filter((e) => e.phase === 'scheduled' && e.quest_id === quest.id);

  expect(scheduled.length).toBeGreaterThan(0);
  expect(scheduled[scheduled.length - 1].reminder_id).toBe(reminder.id);
});
