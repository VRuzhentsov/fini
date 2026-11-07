import type { TestDevice } from '../test-device.ts';

export async function resetTestDeviceUi(testDevice: TestDevice): Promise<void> {
  try {
    const dialog = testDevice.page.locator('[data-testid="incoming-space-sync-dialog"]');
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

      await testDevice.page.waitForFunction(() => {
        return !document.querySelector('[data-testid="incoming-space-sync-dialog"]');
      });
    }

    await testDevice.page.click('nav.nav a[href="#/main"]');
    await testDevice.page.waitForSelector('[data-testid="chat-input"]', { timeout: 10_000 });
  } catch {
    // Best-effort cleanup. Teardown should not turn a failed test into a stuck one.
  }
}

export async function resetTestDevicesUi(testDevices: TestDevice[]): Promise<void> {
  for (const testDevice of testDevices) {
    await resetTestDeviceUi(testDevice);
  }
}
