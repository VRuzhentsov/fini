import { expect, test as base } from '@playwright/test';
import { PluginClient, TauriPage } from '@srsholmes/tauri-playwright';
import { appendFileSync, existsSync, mkdirSync, readFileSync, rmSync } from 'fs';
import { spawn, type ChildProcessWithoutNullStreams } from 'child_process';
import { dirname, join } from 'path';
import { fileURLToPath } from 'url';
import { resetActorsUi } from './helpers/teardown.ts';

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '../../..');
const DEFAULT_APP_BINARY = join(REPO_ROOT, 'src-tauri/target/debug-e2e/debug/fini-app');
const DEFAULT_RUN_ROOT = join(REPO_ROOT, 'tmp', 'fini-e2e-actors');
const DEFAULT_ACTOR_WAIT_SECS = 60;
const DEFAULT_BASE_DISCOVERY_PORT = 46_000 + Math.floor(Math.random() * 500);

export interface E2EActor {
  slug: string;
  page: TauriPage;
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
}

interface ActorProcessState {
  slug: string;
  binaryPath: string;
  child: ChildProcessWithoutNullStreams;
  logPath: string;
  socketPath: string;
  dataDir: string;
  discoveryPort: number;
  wsPort: number;
  spawnError: Error | null;
}

interface ActorSession {
  actors: Record<string, E2EActor>;
  stop(preserve?: boolean): Promise<void>;
}

interface ActorFixtures {
  actorSession: ActorSession;
  actors: Record<string, E2EActor>;
  actorA: E2EActor;
  actorB: E2EActor;
}

function actorSlugs(): string[] {
  return (process.env.FINI_E2E_ACTORS ?? process.env.FINI_E2E_CI_ACTORS ?? 'actor-a,actor-b')
    .split(',')
    .map((value) => value.trim())
    .filter(Boolean);
}

function resolveAppBinaryPath(): string {
  const binary = process.env.FINI_APP_BINARY ?? DEFAULT_APP_BINARY;
  if (!existsSync(binary)) {
    throw new Error(`Fini GUI binary not found: ${binary}`);
  }

  return binary;
}

function resolveRunRoot(): string {
  return process.env.FINI_E2E_ROOT ?? process.env.FINI_E2E_CI_RESULTS_DIR ?? DEFAULT_RUN_ROOT;
}

function resolveRunId(): string {
  return process.env.FINI_E2E_RUN_ID
    ?? process.env.FINI_E2E_CI_RUN_ID
    ?? `${new Date().toISOString().replace(/[:.]/g, '-')}-${process.pid}`;
}

function resolveActorWaitMs(): number {
  const raw = process.env.FINI_E2E_ACTOR_WAIT_SECS ?? process.env.FINI_E2E_CI_ACTOR_WAIT_SECS;
  const parsed = raw ? Number.parseInt(raw, 10) : DEFAULT_ACTOR_WAIT_SECS;
  return Number.isFinite(parsed) && parsed > 0 ? parsed * 1_000 : DEFAULT_ACTOR_WAIT_SECS * 1_000;
}

function resolveBaseDiscoveryPort(): number {
  const raw = process.env.FINI_E2E_BASE_DISCOVERY_PORT;
  const parsed = raw ? Number.parseInt(raw, 10) : DEFAULT_BASE_DISCOVERY_PORT;
  return Number.isFinite(parsed) && parsed > 0 ? parsed : DEFAULT_BASE_DISCOVERY_PORT;
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function tailLog(logPath: string, maxLines = 40): string {
  if (!existsSync(logPath)) {
    return '(log file missing)';
  }

  const lines = readFileSync(logPath, 'utf8').trim().split(/\r?\n/).filter(Boolean);
  if (lines.length === 0) {
    return '(log file empty)';
  }

  return lines.slice(-maxLines).join('\n');
}

function actorDebugMessage(state: ActorProcessState, headline: string): string {
  const parts = [
    headline,
    `slug: ${state.slug}`,
    `binary: ${state.binaryPath}`,
    `dataDir: ${state.dataDir}`,
    `socket: ${state.socketPath}`,
    `discoveryPort: ${state.discoveryPort}`,
    `spaceSyncWsPort: ${state.wsPort}`,
    `logPath: ${state.logPath}`,
    state.spawnError ? `spawnError: ${state.spawnError.message}` : null,
    `exitCode: ${state.child.exitCode ?? 'running'}`,
    `signalCode: ${state.child.signalCode ?? 'none'}`,
    'logTail:',
    tailLog(state.logPath),
  ].filter(Boolean);

  return parts.join('\n');
}

function spawnActorProcess(
  runId: string,
  runRoot: string,
  socketDir: string,
  slugs: string[],
  slug: string,
  index: number,
  binaryPath: string,
): ActorProcessState {
  const sessionRoot = join(runRoot, runId);
  const actorRoot = join(sessionRoot, 'actors');
  const dataDir = join(actorRoot, `${slug}-data`);
  const logPath = join(actorRoot, `${slug}.log`);
  const socketPath = join(socketDir, `${slug}.sock`);
  const baseDiscoveryPort = resolveBaseDiscoveryPort();
  const discoveryPort = baseDiscoveryPort + index * 2;
  const wsPort = discoveryPort + 1;
  const peerPorts = slugs.map((_, peerIndex) => String(baseDiscoveryPort + peerIndex * 2)).join(',');

  mkdirSync(dataDir, { recursive: true });
  rmSync(socketPath, { force: true });

  const child = spawn(binaryPath, [], {
    env: {
      ...process.env,
      FINI_ACTOR_SLUG: slug,
      FINI_APP_DATA_DIR: dataDir,
      FINI_E2E_ACTORS: slugs.join(','),
      FINI_E2E_ROOT: runRoot,
      FINI_E2E_RUN_ID: runId,
      FINI_E2E_SOCKET_DIR: socketDir,
      FINI_DISCOVERY_PEER_PORTS: peerPorts,
      FINI_DISCOVERY_PORT: String(discoveryPort),
      FINI_SPACE_SYNC_WS_PORT: String(wsPort),
      TAURI_PLAYWRIGHT_SOCKET: socketPath,
      HOSTNAME: slug,
      TZ: 'UTC',
      XDG_DATA_HOME: dataDir,
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  });

  const state: ActorProcessState = {
    slug,
    binaryPath,
    child,
    logPath,
    socketPath,
    dataDir,
    discoveryPort,
    wsPort,
    spawnError: null,
  };

  appendFileSync(logPath, `[${new Date().toISOString()}] spawn actor ${slug}\n`);
  child.stdout.on('data', (chunk) => appendFileSync(logPath, chunk));
  child.stderr.on('data', (chunk) => appendFileSync(logPath, chunk));
  child.once('error', (error) => {
    state.spawnError = error;
    appendFileSync(logPath, `[${new Date().toISOString()}] spawn error: ${error.message}\n`);
  });

  return state;
}

async function waitForActorSocket(state: ActorProcessState, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    if (state.spawnError) {
      throw new Error(actorDebugMessage(state, 'Actor spawn failed before socket appeared'));
    }

    if (existsSync(state.socketPath)) {
      return;
    }

    if (state.child.exitCode !== null || state.child.signalCode !== null) {
      throw new Error(actorDebugMessage(state, 'Actor exited before socket appeared'));
    }

    await delay(200);
  }

  throw new Error(actorDebugMessage(state, `Actor socket did not appear within ${timeoutMs}ms`));
}

async function stopActorProcess(state: ActorProcessState): Promise<void> {
  if (state.child.exitCode !== null || state.child.signalCode !== null) {
    return;
  }

  state.child.kill('SIGTERM');

  await new Promise<void>((resolve) => {
    const timer = setTimeout(() => {
      if (state.child.exitCode === null && state.child.signalCode === null) {
        state.child.kill('SIGKILL');
      }
      resolve();
    }, 5_000);

    state.child.once('exit', () => {
      clearTimeout(timer);
      resolve();
    });
  });
}

async function createActorSession(): Promise<ActorSession> {
  const slugs = actorSlugs();
  if (slugs.length < 2) {
    throw new Error(`actors fixture requires at least two actors, got ${slugs.length}`);
  }

  const binaryPath = resolveAppBinaryPath();
  const runRoot = resolveRunRoot();
  const runId = resolveRunId();
  const sessionRoot = join(runRoot, runId);
  const actorRoot = join(sessionRoot, 'actors');
  const socketDir = join(sessionRoot, 'sockets');
  const waitMs = resolveActorWaitMs();

  mkdirSync(actorRoot, { recursive: true });
  mkdirSync(socketDir, { recursive: true });

  console.log(`FINI_E2E_RUN_ROOT=${sessionRoot}`);
  console.log(`FINI_E2E_APP_BINARY=${binaryPath}`);

  const actorStates = slugs.map((slug, index) =>
    spawnActorProcess(runId, runRoot, socketDir, slugs, slug, index, binaryPath),
  );

  const clients: PluginClient[] = [];
  const actorEntries: Array<[string, E2EActor]> = [];
  let stopped = false;

  async function stop(preserve = false): Promise<void> {
    if (stopped) {
      return;
    }
    stopped = true;

    const keepArtifacts = preserve || process.env.FINI_E2E_KEEP === '1' || (process.exitCode ?? 0) !== 0;

    for (const client of clients.reverse()) {
      try {
        client.disconnect();
      } catch {
        // Best-effort disconnect only.
      }
    }

    for (const state of actorStates.reverse()) {
      try {
        await stopActorProcess(state);
      } catch {
        // Best-effort process shutdown only.
      }
    }

    if (keepArtifacts) {
      console.log(`Keeping E2E actor run dir for debugging: ${sessionRoot}`);
      return;
    }

    try {
      rmSync(sessionRoot, { recursive: true, force: true });
    } catch {
      // Best-effort cleanup only.
    }
  }

  try {
    for (const state of actorStates) {
      await waitForActorSocket(state, waitMs);

      const client = new PluginClient(state.socketPath);
      clients.push(client);
      await client.connect();

      const ping = await client.send({ type: 'ping' });
      if (!ping.ok) {
        throw new Error(actorDebugMessage(state, 'Plugin ping failed'));
      }

      const page = new TauriPage(client);
      page.setDefaultTimeout(15_000);

      actorEntries.push([
        state.slug,
        {
          slug: state.slug,
          page,
          invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
            return invokeTauri<T>(page, command, args);
          },
        },
      ]);
    }
  } catch (error) {
    await stop(true);
    throw error;
  }

  return {
    actors: Object.fromEntries(actorEntries),
    stop,
  };
}

async function invokeTauri<T>(page: TauriPage, command: string, args?: Record<string, unknown>): Promise<T> {
  return page.evaluate<T>(`(async () => {
    const invoke = window.__TAURI_INTERNALS__?.invoke;
    if (!invoke) throw new Error('Tauri invoke is unavailable');
    return await invoke(${JSON.stringify(command)}, ${JSON.stringify(args ?? {})});
  })()`);
}

export const test = base.extend<ActorFixtures>({
  actorSession: [async ({}, use) => {
    const session = await createActorSession();
    try {
      await use(session);
    } finally {
      await session.stop();
    }
  }, { scope: 'worker' }],

  actors: async ({ actorSession }, use) => {
    const actorEntries = Object.values(actorSession.actors);
    try {
      await use(actorSession.actors);
    } finally {
      await resetActorsUi(actorEntries);
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
