import { test, expect } from '../fixtures.js';

interface Quest {
  id: string;
  title: string;
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
