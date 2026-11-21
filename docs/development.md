# Development

The development guidelines of Fini: the project map, the commands, the repo
automation, and the rules for each change. The agent process that uses them
is the `dev` skill (`.agents/skills/dev/SKILL.md`).

## Project map

- Frontend: `src/`, Vue 3, TypeScript, Vite, Tailwind CSS, DaisyUI, Pinia.
- Backend: `src-tauri/`, Rust, Tauri 2, Diesel, SQLite.
- Domain specs and companion specs: `specs/`, folder `README.md` files, and sidecar `.md` files next to source files. They move to `docs/` when they change ([`documentation.md`](documentation.md)).
- Product semantics and decision history: `specs/` and `docs/adr/`.
- Repo automation: `Makefile` is the primary human execution entrypoint, `npm run` owns JS/TS package tasks, and `xtask/` owns non-trivial automation logic ("Repo automation" below).

Before changing a significant source file, read its companion `.md` spec when present. Write code to match the spec, or update docs/specs deliberately when the behavior changes.

How to write code: [`code-style.md`](code-style.md) and
[`architecture/code-design.md`](architecture/code-design.md). Tests:
[`testing.md`](testing.md).

## No legacy in alpha

Fini is in open alpha and has not been publicly released. Treat the current source and schema as the only supported contract: do not add, preserve, or test legacy compatibility for prior Energy/Priority values, sync payloads, backup formats, migrations, or rollback paths unless the user explicitly directs it.

When a contract changes during alpha, implement the new contract directly. Remove obsolete adapters and mappings rather than carrying a compatibility layer for an unreleased predecessor. For the database: [`db.md`](db.md), "Schema migrations".

## Commands

### Development runs on HMR; the user builds artefacts

For any development or iteration run, start the **HMR processes** and leave
them running:

| Side | Use | Not |
|---|---|---|
| Desktop | `make desktop-debug-dev` | `make desktop-debug` (builds a binary first) |
| Phone | `make android-dev` | `make android-debug-deploy` (builds and installs an APK) |

A frontend edit then reaches both apps immediately. Building an artefact for
an iteration costs minutes per change and is the user's call, not a default:
reach for one only when the artefact itself is the thing under test — a
release check, or something that must run against an installed package.

Two limits to state rather than work around silently:

- **HMR covers the frontend only.** A Rust change needs a rebuild, and
  `tauri dev` does not always pick one up on its own. After editing Rust,
  confirm the running app actually has the change (query the behaviour, do
  not assume) and restart the dev process when it does not.
- **Bluetooth-only proof can retain Wi-Fi and HMR.** Keep both paired Network
  channels off and verify that Bluetooth carries the session and test data
  (`make e2e-devices-ble`). Use an installed package only when the test needs
  independence from HMR. Change radio settings only with specific user approval.

### Make targets and what CI runs

On your own computer, prefer these Makefile targets over raw `npm`, `tauri`, or container commands. Make is for local development only ("Make is local only" below): for CI, a release, a container or CI-parity evidence, use what the last column names.

| Need | Local command | What CI runs |
|---|---|---|
| Desktop dev app | `make dev` | – |
| Local release-profile desktop build (not a release) | `make build` | `npm run tauri build` in the `linux` `build` image (`release-tag.yml`, `build-linux`) |
| Visible local two-app E2E | `make e2e` or `make e2e-headed` | – |
| Containerized E2E, as CI runs it | `make e2e-ci` | `npm run gate:e2e:run` in the `linux` `e2e` image (job Quality Gates) |
| Build/update E2E images | `make e2e-image` | `.github/actions/container-stage`, `linux` `e2e` |
| Full containerized E2E | `make pr-gate-e2e` | `npm run gate:e2e:run` (job Quality Gates) |
| Runtime container image | `make runtime-image` | `linux` `runtime` stage (job Quality Gates; `release-tag.yml`, `build-docker-image`) |
| Runtime CLI smoke | `make runtime-smoke` | `<engine> run --rm <image> --help` (job Quality Gates) |
| Android device list | `make android-devices` | – |
| Android Wi-Fi connect | `make android-connect` once after `make android-devices`; if it times out, stop and ask the user to re-authorize wireless debugging or use USB/emulator | – |
| Android hot reload | `make android-dev` | – |
| Android build | `make android-build` | `npm run tauri android build` in the `android` `build` image (`release-tag.yml`, `build-android`) |
| Android debug-signed release deploy (shares `com.fini.app`'s id) | `make android-release-deploy-debugsigned` | – |
| Android debug deploy (separate `com.fini.app.debug` id, Tauri logs enabled) | `make android-debug-deploy` | – |
| Android local release deploy | `make android-release-deploy-local` | – |

A release has no Make target. Releases run only in CI: the user starts the Release Button GitHub Action (`.github/workflows/release-button.yml`), and its tag push starts `release-tag.yml` ([`release.md`](release.md)).

Use package scripts directly only when no Makefile target exists or when a narrower check is clearly better, such as `npm run build` for frontend type/build validation.

Generic targets such as `make test`, `make lint`, or `make check` do not exist unless they are in the current `Makefile`. If a desired target is missing, name the closest existing target from the table or state that no Makefile target exists for that check.

The test commands: [`testing.md`](testing.md), "Running tests".

## Repo automation

Keep automation predictable and discoverable:

- Makefile is the entrypoint for developers on their own computers only (ADR-0013 D9).
- `npm run` owns JavaScript, TypeScript, frontend, package, and JS/TS test scripts, and every CI step.
- `cargo xtask` owns non-trivial Rust repo automation.
- CI calls `npm run ...` (or `cargo xtask ...`) directly. CI never calls `make`.
- One workflow has one implementation: the npm script. The Make target calls the same script.

App version policy belongs to [`release.md`](release.md), "Versioning".

### Make is local only

`make` is a shortcut for a developer at their own computer. It is never the
entry point of these:

- CI jobs and workflows;
- release steps (releases run only in CI: [`release.md`](release.md));
- commands in a container: the Containerfiles, `containers/run.sh`, and all
  that runs in a container;
- e2e runners and Playwright;
- the evidence of an agent for CI parity.

For these, use the npm script, `cargo xtask`, the workflow job, or
`containers/run.sh`. A Make target calls one of them; it holds no logic of
its own that CI needs.

### Command surface

Use this default hierarchy:

| Surface | Role |
|---|---|
| `Makefile` | Local developer shortcuts with memorable names; each recipe calls an npm script or a local-only tool (adb, a dev server) |
| `npm run` | JS/TS/frontend/package scripts, Vite/Tauri package scripts, JS/TS tests, and the CI gate steps (`gate:*`, `test:e2e:*`, `build:android:e2e`) |
| `scripts/*.mts` | The logic behind an npm script that is more than one command (`ci-gate.mts`, `e2e-android.mts`, `with-devtools-capability.mts`). Node 24 runs them as they are (type stripping): use only types that can be erased, so no `enum` and no parameter properties. Older scripts are still `.mjs` |
| `cargo xtask` | Non-trivial Rust repo automation, file parsing/editing, manifest updates, packaging/release orchestration |
| CI workflow steps | One `run: npm run <script>` (or `cargo xtask ...`) for each step; no `make` |

Document a command for developers as `make <target>` when a target exists; document the same command for CI as `npm run <script>`.

Playwright runs from npm scripts, not from Make or xtask.

### Makefile rules

Makefile targets should:

- Provide stable, memorable names for humans.
- Validate required inputs and environment variables.
- Orchestrate existing commands across npm, Cargo, containers, Android tooling, or git.
- Call `npm run` for JS/TS/package tasks when appropriate.
- Call `cargo xtask` for non-trivial automation logic when appropriate.

Keep Makefile recipes simple. Move parsing, file mutation, and complex branching into `xtask` or the domain's native command runner.

### npm run rules

Use `npm run` for:

- Frontend development and builds.
- Vite and Vue TypeScript checks.
- Tauri CLI package scripts exposed through `package.json`.
- Jest, Playwright, and other JS/TS test scripts.
- Package-manager behavior where `package.json` is the natural owner.

Do not force every `npm run` command behind Makefile. Add a Makefile wrapper when the command becomes a broader repo workflow, needs guard checks, or is intended as a common human entrypoint.

### cargo xtask rules

Use `cargo xtask` for automation that needs to:

- Parse or rewrite JSON, TOML, lockfiles, manifests, generated metadata, or release files.
- Validate git or release state beyond simple shell checks.
- Share identical logic between local commands and CI.
- Coordinate packaging, release, build metadata, or multi-step repo maintenance.

Keep `xtask` at the repo root:

```text
xtask/
  Cargo.toml
  src/main.rs
```

Run directly when needed:

```bash
cargo run --manifest-path xtask/Cargo.toml -- <command> [args]
```

Prefer Makefile wrappers for human-facing `xtask` workflows.

### Script policy

Node scripts in `scripts/*.mts` (TypeScript that Node 24 runs as it is) are the adopted runner for the logic behind npm scripts and CI steps (ADR-0013 D9): they need no install beyond Node, which every CI job and developer has. Avoid other ad hoc runners (shell, PHP, Python) for repo automation.

If adding a new automation runner, make it explicit in project dependencies and update this section with when to use it.

Shell in Makefile is acceptable for simple local guards. Logic that CI also runs goes into `scripts/*.mts` (or `xtask` for Rust-side work), never into a Make recipe.

## Fini CLI

`fini` is the single app binary for CLI work (`README.md`, "CLI";
`specs/cli/README.md`). Its behavior depends on invocation:

| Invocation | Use |
|---|---|
| `fini` | Return the current Focus quest using CLI default behavior |
| `fini focus get` | Explicitly return the current Focus quest |
| `fini quest ...` | List, get, create, update, complete, abandon, delete, or inspect quest history |
| `fini space ...` | List, create, update, or delete spaces |
| `fini reminder ...` | List, create, or delete reminders |

The desktop launcher or `fini-app` opens the graphical app.

Built-in spaces default to these IDs. Users may rename built-ins or create custom spaces, so a fresh `fini space list --json` result wins over these names.

| ID | Default name |
|---|---|
| `1` | Personal |
| `2` | Family |
| `3` | Work |

Make targets that validate the binary from this repo:

| Need | Command |
|---|---|
| Build a local release-profile app binary (not a release; releases run in CI) | `make build` |
| Validate runtime container exposes CLI | `make runtime-smoke` |
| Build/update runtime container | `make runtime-image` |

## Dev data

- The CLI is the only supported automation surface against the running app. Use `fini quest create`, `fini reminder create`, `fini space list`, etc. to seed and query state. Do not drive data population through the UI or through webview/IPC MCP tools (`mcp___hypothesi_tauri-mcp-server__*`). Those tools are not available in this repo's workflow.
- Use CLI for: seeding a reminder due soon to trigger a notification, building a history dataset across multiple spaces, completing or abandoning quests to verify state transitions, listing state as evidence after a write.
- Use the UI (via `make dev`) for visual confirmation of the result only — not for setup.

## Portable files

### No user-specific paths

Never write absolute paths containing a username, home directory, or machine-specific prefix into any source file, config, permission entry, or generated artifact. This includes paths like `/home/<user>/`, `/var/home/<user>/`, or `/Users/<user>/`.

- Use paths relative to the project root for anything inside the repo (`src-tauri/target/debug/fini`, not `/home/<user>/projects/fini/src-tauri/...`).
- Use `~` for paths relative to the user's home directory (`~/.local/share/fini/fini.db`, not `/home/<user>/.local/...`).
- Use a task-appropriate scoped scratch path, such as the repository's ignored `tmp/` directory or a platform-provided runtime directory. Do not prescribe a global scratch path.
- When adding permission entries to `.claude/settings.local.json`, use relative or `~`-anchored paths only. The local config is project-specific and minimal; do not accumulate machine-specific one-off entries there.

### User-agnostic agent instructions

Keep repo-local skills, scripts, prompts, and generated artifacts portable across Fini users and agent hosts.

- Do not hard-code a person's name, an agent nickname, a bot username or handle, a chat ID, a topic ID, or a repository owner in source-controlled instructions.
- Refer to configured values instead, such as `FINI_REPO`, `FINI_DAILY_RECIPIENT`, `FINI_DAILY_TG_TARGET`, `FINI_PROGRESS_TG_TARGET`, and `FINI_DEV_TG_CHANNEL_ID`.
- If a script needs a repository, prefer `FINI_REPO`; otherwise infer the GitHub `owner/repo` from the current checkout's `origin` remote.
- If a prompt needs a greeting or addressee, use `FINI_DAILY_RECIPIENT` when it is set; otherwise write a neutral report with no personal salutation.
- Mention bot behavior generically. Do not include concrete bot handles in repo-local skills.
