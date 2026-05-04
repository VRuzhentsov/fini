import { test, expect } from '../fixtures.ts';

interface Quest {
  id: string;
  title: string;
  status: string;
  due: string | null;
  due_time: string | null;
}

interface Reminder {
  id: string;
  quest_id: string;
  due_at_utc: string | null;
  scheduled_notification_id: string | null;
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

async function invokeTauri<T>(tauriPage: { evaluate: <R>(script: string) => Promise<R> }, cmd: string, args?: Record<string, unknown>): Promise<T> {
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
    if (!(el instanceof HTMLTextAreaElement)) {
      throw new Error('textarea not found for selector: ' + ${JSON.stringify(selector)});
    }
    el.focus();
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
    if (!setter) throw new Error('textarea setter is unavailable');
    setter.call(el, ${JSON.stringify(value)});
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  })()`);
}

async function createQuestFromFocusInput(
  tauriPage: { waitForSelector: (selector: string, timeout?: number) => Promise<unknown>; press: (selector: string, key: string) => Promise<void>; evaluate: <R>(script: string) => Promise<R> },
  title: string,
): Promise<void> {
  await tauriPage.waitForSelector('[data-testid="chat-input"]', 30_000);
  await fillTextarea(tauriPage, '[data-testid="chat-input"]', title);
  await tauriPage.press('[data-testid="chat-input"]', 'Enter');
}

function nearFutureReminderTarget(): { due: string; dueTime: string; quickPick: 'today' | 'tomorrow'; waitMs: number } {
  const now = new Date();
  const target = new Date(now);
  target.setSeconds(0, 0);
  target.setMinutes(target.getMinutes() + (now.getSeconds() > 35 ? 2 : 1));

  const due = `${target.getFullYear()}-${String(target.getMonth() + 1).padStart(2, '0')}-${String(target.getDate()).padStart(2, '0')}`;
  const today = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`;
  const dueTime = `${String(target.getHours()).padStart(2, '0')}:${String(target.getMinutes()).padStart(2, '0')}`;
  return {
    due,
    dueTime,
    quickPick: due === today ? 'today' : 'tomorrow',
    waitMs: Math.max(0, target.getTime() - now.getTime()) + 15_000,
  };
}

test('creating a quest and setting due time schedules its reminder path', async ({ tauriPage }) => {
  const title = `e2e reminder ${Date.now()}`;

  await tauriPage.waitForSelector('nav.nav a[href="#/main"]', 30_000);
  await tauriPage.evaluate(`(() => { window.location.hash = '#/quests'; })()`);
  if (!(await tauriPage.isVisible('[data-testid="chat-input"]'))) {
    await tauriPage.click('nav.nav a[href="#/main"]');
    await tauriPage.evaluate(`(() => { window.location.hash = '#/quests'; })()`);
  }
  await tauriPage.waitForSelector('[data-testid="chat-input"]', 30_000);

  await invokeTauri<void>(tauriPage, 'e2e_clear_notification_events');

  await fillTextarea(tauriPage, '[data-testid="chat-input"]', title);
  await tauriPage.press('[data-testid="chat-input"]', 'Enter');

  const tomorrow = await tauriPage.evaluate<string>(`(() => {
    const d = new Date();
    d.setDate(d.getDate() + 1);
    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, '0');
    const day = String(d.getDate()).padStart(2, '0');
    return y + '-' + m + '-' + day;
  })()`);

  await tauriPage.waitForSelector('.quest-row-surface', 10_000);
  await tauriPage.evaluate(`(() => {
    const rows = Array.from(document.querySelectorAll('.quest-row-surface'));
    const row = rows.find((candidate) => candidate.textContent?.includes(${JSON.stringify(title)}));
    if (!(row instanceof HTMLElement)) {
      throw new Error('quest row not found for title: ' + ${JSON.stringify(title)});
    }
    row.click();
  })()`);
  await tauriPage.click('[data-testid="quest-reminder"]');
  await tauriPage.click('[data-testid="reminder-tomorrow"]');
  await tauriPage.click('[data-testid="reminder-toggle-time"]');
  await tauriPage.fill('[data-testid="reminder-hour"]', '14');
  await tauriPage.fill('[data-testid="reminder-minute"]', '30');
  await tauriPage.click('[data-testid="reminder-done"]');

  await tauriPage.waitForFunction(`(() => {
    const button = document.querySelector('[data-testid="quest-reminder"]');
    return !!button && button.textContent?.includes('14:30');
  })()`, 10_000);

  const quests = await invokeTauri<Quest[]>(tauriPage, 'get_quests');
  const quest = quests.find((item) => item.title === title);
  expect(quest).toBeTruthy();
  expect(quest?.due).toBe(tomorrow);
  expect(quest?.due_time).toBe('14:30');

  const reminders = await invokeTauri<Reminder[]>(tauriPage, 'get_reminders', { questId: quest!.id });
  expect(reminders).toHaveLength(1);
  expect(reminders[0].quest_id).toBe(quest!.id);
  expect(reminders[0].due_at_utc).toBe(`${tomorrow}T14:30:00Z`);

  const events = await invokeTauri<NotificationEvent[]>(tauriPage, 'e2e_list_notification_events');
  const scheduled = events.filter((event) => event.phase === 'scheduled' && event.quest_id === quest!.id);
  expect(scheduled.length).toBeGreaterThan(0);
  expect(scheduled[scheduled.length - 1]).toMatchObject({
    delivery_path: 'desktop_in_process',
    due_at_utc: `${tomorrow}T14:30:00Z`,
    reminder_id: reminders[0].id,
  });
});

test('near-future reminder becomes Focus only after its due time arrives', async ({ tauriPage }) => {
  const manualTitle = `e2e focus current ${Date.now()}`;
  const reminderTitle = `e2e focus reminder ${Date.now()}`;

  await tauriPage.waitForSelector('nav.nav a[href="#/main"]', 30_000);
  await tauriPage.click('nav.nav a[href="#/main"]');
  await createQuestFromFocusInput(tauriPage, manualTitle);

  await tauriPage.waitForFunction(`(async () => {
    const invoke = window.__TAURI_INTERNALS__?.invoke;
    const quests = await invoke('get_quests');
    return quests.some((quest) => quest.title === ${JSON.stringify(manualTitle)});
  })()`, 10_000);
  const manualQuest = (await invokeTauri<Quest[]>(tauriPage, 'get_quests')).find((quest) => quest.title === manualTitle);
  expect(manualQuest).toBeTruthy();

  await invokeTauri<Quest>(tauriPage, 'set_focus', { id: manualQuest!.id });
  await tauriPage.evaluate(`(() => { window.location.reload(); })()`);
  await tauriPage.waitForSelector('nav.nav a[href="#/main"]', 30_000);
  await tauriPage.click('nav.nav a[href="#/main"]');
  await tauriPage.waitForFunction(`(() => document.body.textContent?.includes(${JSON.stringify(manualTitle)}))()`, 30_000);

  await createQuestFromFocusInput(tauriPage, reminderTitle);
  await tauriPage.waitForFunction(`(() => document.body.textContent?.includes(${JSON.stringify(reminderTitle)}))()`, 10_000);

  const target = nearFutureReminderTarget();

  await tauriPage.evaluate(`(() => {
    const rows = Array.from(document.querySelectorAll('.quest-row-surface'));
    const row = rows.find((candidate) => candidate.textContent?.includes(${JSON.stringify(reminderTitle)}));
    if (!(row instanceof HTMLElement)) {
      throw new Error('quest row not found for title: ' + ${JSON.stringify(reminderTitle)});
    }
    row.click();
  })()`);
  await tauriPage.click('[data-testid="quest-reminder"]');
  await tauriPage.click(`[data-testid="reminder-${target.quickPick}"]`);
  await tauriPage.click('[data-testid="reminder-toggle-time"]');
  await tauriPage.fill('[data-testid="reminder-hour"]', target.dueTime.slice(0, 2));
  await tauriPage.fill('[data-testid="reminder-minute"]', target.dueTime.slice(3, 5));
  await tauriPage.click('[data-testid="reminder-done"]');

  await tauriPage.waitForFunction(`(() => {
    const button = document.querySelector('[data-testid="quest-reminder"]');
    return !!button && button.textContent?.includes(${JSON.stringify(target.dueTime)});
  })()`, 10_000);

  const immediateFocus = await invokeTauri<Quest | null>(tauriPage, 'get_active_focus');
  expect(immediateFocus?.id).toBe(manualQuest!.id);
  expect(await tauriPage.textContent('.active-quest-title')).toContain(manualTitle);

  await tauriPage.waitForFunction(`(() => {
    const title = document.querySelector('.active-quest-title');
    return title?.textContent?.includes(${JSON.stringify(reminderTitle)});
  })()`, target.waitMs);

  const dueFocus = await invokeTauri<Quest | null>(tauriPage, 'get_active_focus');
  expect(dueFocus?.title).toBe(reminderTitle);
  expect(dueFocus?.due).toBe(target.due);
  expect(dueFocus?.due_time).toBe(target.dueTime);
});
