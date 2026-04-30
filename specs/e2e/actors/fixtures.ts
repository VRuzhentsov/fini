import { expect, test as base } from '@playwright/test';
import { PluginClient, TauriPage } from '@srsholmes/tauri-playwright';
import { existsSync } from 'fs';
import path from 'path';
import { resetActorsUi } from './helpers/teardown.ts';

export interface E2EActor {
  slug: string;
  page: TauriPage;
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
}

interface ActorFixtures {
  actors: Record<string, E2EActor>;
  actorA: E2EActor;
  actorB: E2EActor;
}

function actorSlugs(): string[] {
  return (process.env.FINI_E2E_ACTORS ?? 'actor-a,actor-b')
    .split(',')
    .map((value) => value.trim())
    .filter(Boolean);
}

function actorSocketPath(slug: string): string {
  const socketDir = process.env.FINI_E2E_SOCKET_DIR ?? '/var/run/fini-e2e';
  return path.join(socketDir, `${slug}.sock`);
}

async function waitForSocket(socketPath: string, timeoutMs = 30_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    if (existsSync(socketPath)) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }

  throw new Error(`Socket ${socketPath} did not appear within ${timeoutMs}ms`);
}

async function invokeTauri<T>(page: TauriPage, command: string, args?: Record<string, unknown>): Promise<T> {
  return page.evaluate<T>(`(async () => {
    const invoke = window.__TAURI_INTERNALS__?.invoke;
    if (!invoke) throw new Error('Tauri invoke is unavailable');
    return await invoke(${JSON.stringify(command)}, ${JSON.stringify(args ?? {})});
  })()`);
}

export const test = base.extend<ActorFixtures>({
  actors: async ({}, use) => {
    const slugs = actorSlugs();
    const clients: PluginClient[] = [];
    const actorEntries: Array<[string, E2EActor]> = [];

    try {
      for (const slug of slugs) {
        const socketPath = actorSocketPath(slug);
        await waitForSocket(socketPath);

        const client = new PluginClient(socketPath);
        clients.push(client);
        await client.connect();

        const ping = await client.send({ type: 'ping' });
        if (!ping.ok) {
          throw new Error(`Plugin ping failed for ${slug}`);
        }

        const page = new TauriPage(client);
        page.setDefaultTimeout(15_000);

        actorEntries.push([
          slug,
          {
            slug,
            page,
            invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
              return invokeTauri<T>(page, command, args);
            },
          },
        ]);
      }

      await use(Object.fromEntries(actorEntries));
    } finally {
      await resetActorsUi(actorEntries.map(([, actor]) => actor));

      for (const client of clients.reverse()) {
        client.disconnect();
      }
    }
  },

  actorA: async ({ actors }, use) => {
    const actor = actors['actor-a'] ?? Object.values(actors)[0];
    if (!actor) {
      throw new Error('actor-a is not available');
    }
    await use(actor);
  },

  actorB: async ({ actors }, use) => {
    const actor = actors['actor-b'] ?? Object.values(actors)[1];
    if (!actor) {
      throw new Error('actor-b is not available');
    }
    await use(actor);
  },
});

export { expect };
