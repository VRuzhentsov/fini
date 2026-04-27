export async function waitForText(
  page: { waitForFunction: (script: string, timeout?: number) => Promise<unknown> },
  selector: string,
  text: string,
  timeout = 30_000,
) {
  await page.waitForFunction(`(() => {
    return Array.from(document.querySelectorAll(${JSON.stringify(selector)}))
      .some((node) => node.textContent?.includes(${JSON.stringify(text)}));
  })()`, timeout);
}

export async function allTextContents(
  page: { evaluate: <T>(script: string) => Promise<T> },
  selector: string,
): Promise<string[]> {
  return page.evaluate<string[]>(`(() => {
    return Array.from(document.querySelectorAll(${JSON.stringify(selector)}))
      .map((node) => node.textContent?.trim() ?? '');
  })()`);
}

export async function pollUntil<T>(
  description: string,
  probe: () => Promise<T | null | undefined | false>,
  timeoutMs = 30_000,
  intervalMs = 500,
): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  let lastError: unknown;

  while (Date.now() < deadline) {
    try {
      const result = await probe();
      if (result) {
        return result;
      }
    } catch (error) {
      lastError = error;
    }

    await new Promise((resolve) => setTimeout(resolve, intervalMs));
  }

  const suffix = lastError instanceof Error ? ` Last error: ${lastError.message}` : '';
  throw new Error(`Timed out waiting for ${description}.${suffix}`);
}
