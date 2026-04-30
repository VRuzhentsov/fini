import type { E2EActor } from '../fixtures.ts';

export async function resetActorUi(actor: E2EActor): Promise<void> {
  try {
    const dialog = actor.page.locator('[data-testid="incoming-space-sync-dialog"]');
    if (await dialog.isVisible().catch(() => false)) {
      const dismissButton = dialog.getByRole('button', { name: 'Not now' });
      if (await dismissButton.isVisible().catch(() => false)) {
        await dismissButton.click();
      } else {
        const overlay = dialog.getByRole('button', { name: 'Close dialog' });
        if (await overlay.isVisible().catch(() => false)) {
          await overlay.click();
        }
      }

      await actor.page.waitForFunction(() => {
        return !document.querySelector('[data-testid="incoming-space-sync-dialog"]');
      });
    }

    await actor.page.click('nav.nav a[href="#/main"]');
    await actor.page.waitForSelector('[data-testid="chat-input"]', { timeout: 10_000 });
  } catch {
    // Best-effort cleanup. Teardown should not turn a failed test into a stuck one.
  }
}

export async function resetActorsUi(actors: E2EActor[]): Promise<void> {
  for (const actor of actors) {
    await resetActorUi(actor);
  }
}
