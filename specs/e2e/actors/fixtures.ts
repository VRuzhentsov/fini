import { expect, test as base } from '@playwright/test';
import { PluginClient, TauriPage } from '@srsholmes/tauri-playwright';
import { appendFileSync, existsSync, mkdirSync, readFileSync, rmSync } from 'fs';
import { spawn, type ChildProcessWithoutNullStreams } from 'child_process';
import { createConnection } from 'net';
import { dirname, join } from 'path';
import { fileURLToPath } from 'url';
import { resetActorsUi } from './helpers/teardown.ts';

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '../../..');
const DEFAULT_APP_BINARY = join(REPO_ROOT, 'src-tauri/target/debug-e2e/debug/fini-app');
const DEFAULT_BLE_BROKER_BINARY = join(REPO_ROOT, 'src-tauri/target/debug-e2e/debug/ble-mock-broker');
const DEFAULT_RUN_ROOT = join(REPO_ROOT, 'tmp', 'fini-e2e-actors');
const DEFAULT_ACTOR_WAIT_SECS = 60;
const DEFAULT_BASE_DISCOVERY_PORT = 46_000 + Math.floor(Math.random() * 500);

export interface E2EActor {
  slug: string;
  page: TauriPage;
  /**
   * Whether this harness owns the app behind the actor.
   *
   * Exposed because a few guarantees only hold for 'spawned' actors: the
   * harness gives those a hostname, a fresh data directory and known ports,
   * none of which it can impose on an app already running on someone's
   * device. A spec asserting such a value should say so explicitly rather
   * than silently assume it -- and specs that assert nothing of the sort run
   * unchanged against either kind.
   */
  kind: 'spawned' | 'external';
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

/**
 * How one actor gets backed. The harness only ever asks a provider to
 * `prepare` it, hand back a connected `PluginClient`, and `dispose` it -- it
 * deliberately knows nothing about *what* is on the other end.
 *
 * That indirection is the point: a spawned desktop process, an app already
 * running on a real phone, an emulator, or a stub that answers the plugin
 * protocol without any app at all are all substitutable here. Specs address
 * every actor through the same `invoke` surface regardless, so what a test
 * runs against becomes a matter of configuration rather than of which code
 * path the harness takes.
 */
interface ActorProvider {
  slug: string;
  /** Bring the backing app to a state where `connect` can succeed. */
  prepare(): Promise<void>;
  connect(): Promise<PluginClient>;
  /** Release only what this provider itself created. */
  dispose(preserve: boolean): Promise<void>;
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

/**
 * Actors this harness connects to instead of spawning, as `slug=tcpPort`
 * pairs (e.g. `FINI_E2E_EXTERNAL_ACTORS=phone=9223`).
 *
 * Every other actor is a local child process the harness owns end to end: it
 * spawns the binary, hands it its whole configuration through the
 * environment (data dir, discovery/ws ports, transport selection), and talks
 * to it over a unix socket whose path it chose. None of that is possible for
 * an app already running on a real Android device -- `am start` cannot set
 * environment variables, and the plugin's unix socket lives inside the app
 * sandbox where the host cannot reach it. Such a build instead exposes the
 * same plugin protocol over loopback TCP (see `DEVTOOLS_ANDROID_TCP_PORT` in
 * src-tauri/src/lib.rs), reachable here through `adb forward`.
 *
 * So an external actor is deliberately the harness's *unmanaged* case: it is
 * not spawned, not configured, and not torn down here -- only driven. Its
 * pairing, data and transport state are whatever that device already has,
 * which is exactly the point when the thing under test is real radio
 * behaviour that no emulator reproduces.
 */
function externalActorPorts(): Map<string, number> {
  const raw = process.env.FINI_E2E_EXTERNAL_ACTORS?.trim();
  if (!raw) return new Map();

  return new Map(
    raw
      .split(',')
      .map((entry) => entry.trim())
      .filter(Boolean)
      .map((entry) => {
        const [slug, port] = entry.split('=').map((part) => part.trim());
        const parsed = Number(port);
        if (!slug || !Number.isInteger(parsed) || parsed <= 0) {
          throw new Error(
            `FINI_E2E_EXTERNAL_ACTORS entry must be "slug=port", got "${entry}"`,
          );
        }
        return [slug, parsed] as const;
      }),
  );
}

/**
 * Which transport actors use to sync. 'network' (default) is the real
 * WebSocket transport used by every other actor suite. 'sim' spawns actors
 * with the network transport made genuinely unavailable
 * (`FINI_DISCOVERY_DISABLED=1` — no mDNS, no UDP presence) and the Sim
 * transport configured instead, so fallback/selection is proven against a
 * real second transport rather than raced against presence timing. 'ble'
 * is the same idea one layer deeper: network disabled the same way, but
 * actors dial the real `ble.rs` code path against a cross-process mock
 * radio (`ble-gatt`'s `mock-broker` feature) instead of the Sim transport's
 * plain TCP stand-in — see `helpers/ble-sync.ts` and
 * `docs/adr/0004-mock-broker-for-cross-process-e2e.md` in `ble-gatt`. See
 * `specs/e2e/transports.md`.
 */
export type ActorTransport = 'network' | 'sim' | 'ble';

function actorTransport(): ActorTransport {
  if (process.env.FINI_E2E_TRANSPORT === 'sim') return 'sim';
  if (process.env.FINI_E2E_TRANSPORT === 'ble') return 'ble';
  return 'network';
}

/**
 * Deterministic per-actor fake Bluetooth address for the `ble` lane's mock
 * radio. Doesn't need to look like a real BLE address (`ble-gatt`'s
 * `PeerAddress` is a plain string) — just stable and unique per actor index
 * so `FINI_LOCAL_BLUETOOTH_ADDRESS`/`FINI_BLUETOOTH_PAIRED_ADDRESSES` agree
 * on who's who without any coordination step between actor processes.
 */
function fakeBluetoothAddress(index: number): string {
  return `AA:BB:CC:00:00:${(index + 1).toString(16).padStart(2, '0').toUpperCase()}`;
}

/**
 * Clear of the discovery/ws range (`baseDiscoveryPort + index*2` and
 * `+1`) and the sim range (`baseDiscoveryPort + slugCount*2 + 1000..
 * +1000+slugCount-1`) computed above -- one broker shared by every actor,
 * not one per actor, so this doesn't take an index.
 */
function bleBrokerPort(baseDiscoveryPort: number, slugCount: number): number {
  return baseDiscoveryPort + slugCount * 2 + 2000;
}

function resolveBleBrokerBinaryPath(): string {
  const binary = process.env.FINI_BLE_MOCK_BROKER_BINARY ?? DEFAULT_BLE_BROKER_BINARY;
  if (!existsSync(binary)) {
    throw new Error(`ble-mock-broker binary not found: ${binary}`);
  }

  return binary;
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
  const simBasePort = baseDiscoveryPort + slugs.length * 2 + 1000;
  const simPort = simBasePort + index;
  const peerSimPorts = slugs.map((_, peerIndex) => String(simBasePort + peerIndex)).join(',');
  const transport = actorTransport();

  mkdirSync(dataDir, { recursive: true });
  rmSync(socketPath, { force: true });

  const transportEnv: Record<string, string> =
    transport === 'sim'
      ? {
          FINI_DISCOVERY_DISABLED: '1',
          FINI_SIM_TRANSPORT_PORT: String(simPort),
          FINI_SIM_PEER_PORTS: peerSimPorts,
        }
      : transport === 'ble'
        ? {
            FINI_DISCOVERY_DISABLED: '1',
            FINI_BLE_MOCK_BROKER: `127.0.0.1:${bleBrokerPort(baseDiscoveryPort, slugs.length)}`,
            FINI_LOCAL_BLUETOOTH_ADDRESS: fakeBluetoothAddress(index),
            // Everyone *else's* fake address -- what `bluetooth_dial_candidates`'
            // OS-bond check needs to treat every peer as already bonded (see
            // `FINI_BLUETOOTH_PAIRED_ADDRESSES`'s own doc comment in
            // `device_connection::commands`).
            FINI_BLUETOOTH_PAIRED_ADDRESSES: slugs
              .map((_, peerIndex) => peerIndex)
              .filter((peerIndex) => peerIndex !== index)
              .map(fakeBluetoothAddress)
              .join(','),
          }
        : {};

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
      // WebKitGTK's DMABUF renderer fails on some GPU/compositor combinations
      // (notably NVIDIA), where it logs "Failed to create GBM buffer" and
      // paints nothing -- the window comes up blank and every `eval` against
      // it times out, which looks exactly like a hung actor rather than a
      // rendering problem. Forcing it off costs nothing headless and keeps a
      // headed run working without the operator having to know this.
      WEBKIT_DISABLE_DMABUF_RENDERER: '1',
      HOSTNAME: slug,
      TZ: 'UTC',
      XDG_DATA_HOME: dataDir,
      ...transportEnv,
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

async function stopActorProcess(state: { child: ChildProcessWithoutNullStreams }): Promise<void> {
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

interface BleBrokerState {
  child: ChildProcessWithoutNullStreams;
  logPath: string;
  port: number;
}

/**
 * The `ble` lane's shared radio -- one broker process per worker, spawned
 * before any actor so the dial loop has somewhere to connect to from the
 * start (not strictly required: `ble.rs`'s `backend()` OnceCell leaves
 * itself empty and retries on failure, same as a real unavailable adapter,
 * so a broker that comes up a beat late self-heals on the next tick -- but
 * starting it first avoids the retry noise). See `helpers/ble-sync.ts`.
 */
function spawnBleBroker(runRoot: string, runId: string, port: number): BleBrokerState {
  const logPath = join(runRoot, runId, 'ble-mock-broker.log');
  const binaryPath = resolveBleBrokerBinaryPath();

  const child = spawn(binaryPath, [], {
    env: { ...process.env, FINI_BLE_MOCK_BROKER_LISTEN: `127.0.0.1:${port}` },
    stdio: ['ignore', 'pipe', 'pipe'],
  });

  appendFileSync(logPath, `[${new Date().toISOString()}] spawn ble-mock-broker on 127.0.0.1:${port}\n`);
  child.stdout.on('data', (chunk) => appendFileSync(logPath, chunk));
  child.stderr.on('data', (chunk) => appendFileSync(logPath, chunk));

  return { child, logPath, port };
}

async function waitForBleBrokerReady(state: BleBrokerState, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    if (state.child.exitCode !== null || state.child.signalCode !== null) {
      throw new Error(`ble-mock-broker exited before it was ready -- see ${state.logPath}`);
    }

    const listening = await new Promise<boolean>((resolve) => {
      const socket = createConnection({ host: '127.0.0.1', port: state.port }, () => {
        socket.destroy();
        resolve(true);
      });
      socket.once('error', () => {
        socket.destroy();
        resolve(false);
      });
    });
    if (listening) {
      return;
    }

    await delay(100);
  }

  throw new Error(
    `ble-mock-broker did not start listening on 127.0.0.1:${state.port} within ${timeoutMs}ms -- see ${state.logPath}`,
  );
}

/**
 * What an actor is backed by. `Actor.create` turns one of these into a live
 * actor, so choosing between a spawned desktop process, a real device, or a
 * stub is a matter of which spec is passed in -- never of which branch the
 * harness takes internally.
 */
type ActorSpec =
  | {
      kind: 'spawned';
      slug: string;
      spawnState: () => ActorProcessState;
      waitMs: number;
      onState: (state: ActorProcessState) => void;
    }
  | { kind: 'external'; slug: string; port: number };

/**
 * One actor, independent of what backs it.
 *
 * The lifecycle (`prepare` -> `connect` -> `dispose`) and the surface specs
 * use (`invoke`, `page`) are identical for every backing, which is what lets
 * a real phone, an emulator, or a stub be substituted for a spawned process
 * without a spec noticing. `ActorService` owns instances of this; tests can
 * be handed pre-built ones directly (see `ActorService`'s constructor).
 */
class Actor implements E2EActor {
  readonly slug: string;
  readonly kind: 'spawned' | 'external';
  private readonly provider: ActorProvider;
  private client: PluginClient | null = null;
  private tauriPage: TauriPage | null = null;

  private constructor(slug: string, kind: 'spawned' | 'external', provider: ActorProvider) {
    this.slug = slug;
    this.kind = kind;
    this.provider = provider;
  }

  /** The one place that maps a spec to a backing implementation. */
  static create(spec: ActorSpec): Actor {
    const provider =
      spec.kind === 'spawned'
        ? spawnedActorProvider(spec.slug, spec.spawnState, spec.waitMs, spec.onState)
        : externalActorProvider(spec.slug, spec.port);
    return new Actor(spec.slug, spec.kind, provider);
  }

  /**
   * Escape hatch for a fully custom backing (a stub, a remote runner, ...).
   * Defaults to 'external' because anything reaching for this is, by
   * definition, not a process this harness spawned and configured.
   */
  static fromProvider(provider: ActorProvider, kind: 'spawned' | 'external' = 'external'): Actor {
    return new Actor(provider.slug, kind, provider);
  }

  get page(): TauriPage {
    if (!this.tauriPage) throw new Error(`Actor "${this.slug}" is not started`);
    return this.tauriPage;
  }

  async start(): Promise<PluginClient> {
    await this.provider.prepare();
    this.client = await this.provider.connect();
    this.tauriPage = new TauriPage(this.client);
    this.tauriPage.setDefaultTimeout(15_000);
    return this.client;
  }

  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    return invokeTauri<T>(this.page, command, args);
  }

  async dispose(preserve: boolean): Promise<void> {
    await this.provider.dispose(preserve);
  }
}

/**
 * Backs an actor with a process this harness spawns and owns end to end:
 * it picks the binary, hands the app its whole configuration through the
 * environment, and talks to it over a unix socket at a path it chose.
 */
function spawnedActorProvider(
  slug: string,
  spawnState: () => ActorProcessState,
  waitMs: number,
  onState: (state: ActorProcessState) => void,
): ActorProvider {
  let state: ActorProcessState | null = null;

  return {
    slug,
    async prepare() {
      state = spawnState();
      onState(state);
      await waitForActorSocket(state, waitMs);
    },
    async connect() {
      if (!state) throw new Error(`Actor "${slug}" was not prepared`);
      const client = new PluginClient(state.socketPath);
      await client.connect();
      const ping = await client.send({ type: 'ping' });
      if (!ping.ok) {
        throw new Error(actorDebugMessage(state, 'Plugin ping failed'));
      }
      return client;
    },
    async dispose() {
      if (state) await stopActorProcess(state);
    },
  };
}

/**
 * Backs an actor with an app that is *already running* somewhere this
 * harness does not control, reached over loopback TCP.
 *
 * Nothing here spawns, configures or tears anything down -- deliberately.
 * A real Android device cannot take the spawned provider's approach at all:
 * `am start` cannot set environment variables, and the plugin's unix socket
 * lives inside the app sandbox where the host cannot reach it. Such a build
 * exposes the same plugin protocol over TCP instead (see
 * `DEVTOOLS_ANDROID_TCP_PORT` in src-tauri/src/lib.rs), reachable here via
 * `adb forward`. The device's pairing, data and transport state are whatever
 * it already has, which is the point when the thing under test is real radio
 * behaviour no emulator reproduces.
 */
function externalActorProvider(slug: string, port: number): ActorProvider {
  return {
    slug,
    async prepare() {
      // Nothing to prepare -- whoever owns this app already started it.
    },
    async connect() {
      const client = new PluginClient(undefined, port);
      await client.connect();
      const ping = await client.send({ type: 'ping' });
      if (!ping.ok) {
        throw new Error(
          `External actor "${slug}" did not answer ping on tcp:${port}. `
            + 'Is the app running, built with the `devtools` feature, and the port forwarded '
            + `(e.g. \`adb forward tcp:${port} tcp:${port}\`)?`,
        );
      }
      return client;
    },
    async dispose() {
      // Never stop what we did not start.
    },
  };
}

/**
 * Owns a set of actors and their shared lifecycle, without knowing what any
 * of them is backed by.
 *
 * Constructed with actor instances rather than with configuration on
 * purpose: that is the injection seam. `ActorService.fromEnv` is just the
 * default wiring (read the env, build one `Actor` per slug); a caller that
 * wants a stubbed actor, a second phone, or a mixed set builds those
 * instances itself and passes them here instead.
 */
class ActorService {
  private readonly actors: Actor[];
  private readonly clients: PluginClient[] = [];
  private stopped = false;

  constructor(actors: Actor[]) {
    this.actors = actors;
  }

  async startAll(): Promise<Record<string, E2EActor>> {
    for (const actor of this.actors) {
      this.clients.push(await actor.start());
    }
    return Object.fromEntries(this.actors.map((actor) => [actor.slug, actor]));
  }

  async stopAll(preserve: boolean): Promise<void> {
    if (this.stopped) return;
    this.stopped = true;

    for (const client of [...this.clients].reverse()) {
      try {
        client.disconnect();
      } catch {
        // Best-effort disconnect only.
      }
    }

    // Each actor releases only what its own provider created -- an external
    // actor's dispose is deliberately a no-op, so a real device is never
    // killed by a run that merely borrowed it.
    for (const actor of [...this.actors].reverse()) {
      try {
        await actor.dispose(preserve);
      } catch {
        // Best-effort teardown only.
      }
    }
  }
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

  let bleBroker: BleBrokerState | null = null;
  if (actorTransport() === 'ble') {
    bleBroker = spawnBleBroker(runRoot, runId, bleBrokerPort(resolveBaseDiscoveryPort(), slugs.length));
    await waitForBleBrokerReady(bleBroker, waitMs);
  }

  const externalPorts = externalActorPorts();
  for (const slug of externalPorts.keys()) {
    if (!slugs.includes(slug)) {
      throw new Error(
        `FINI_E2E_EXTERNAL_ACTORS names "${slug}", which is not in FINI_E2E_ACTORS (${slugs.join(', ')})`,
      );
    }
  }

  // Default wiring: one Actor per slug, backed by whatever its spec says.
  // Everything downstream goes through ActorService and never learns which.
  const actorStates: ActorProcessState[] = [];
  const service = new ActorService(
    slugs.map((slug, index) => {
      const externalPort = externalPorts.get(slug);
      return Actor.create(
        externalPort === undefined
          ? {
              kind: 'spawned',
              slug,
              spawnState: () =>
                spawnActorProcess(runId, runRoot, socketDir, slugs, slug, index, binaryPath),
              waitMs,
              onState: (state) => actorStates.push(state),
            }
          : { kind: 'external', slug, port: externalPort },
      );
    }),
  );

  let stopped = false;

  async function stop(preserve = false): Promise<void> {
    if (stopped) {
      return;
    }
    stopped = true;

    const keepArtifacts = preserve || process.env.FINI_E2E_KEEP === '1' || (process.exitCode ?? 0) !== 0;

    await service.stopAll(keepArtifacts);

    if (bleBroker) {
      try {
        // After the actors, not before -- nothing left needs the radio by
        // this point, and stopping it first would just make their own
        // shutdown noisier (in-flight dial attempts failing to connect).
        await stopActorProcess(bleBroker);
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

  let actors: Record<string, E2EActor>;
  try {
    actors = await service.startAll();
  } catch (error) {
    await stop(true);
    throw error;
  }

  return { actors, stop };
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
