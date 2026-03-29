# E2E Testing Stack PRD (Low Priority)

Date: 2026-03-22
Status: Draft
Priority: Low
Owner: TBD

> Terminology note: newer specs use `Focus` instead of `Main`, and MCP focus tool naming is expected to move from `get_active_quest` to `get_active_focus`.

## Background

Fini now has a stable backend foundation for quest identity and Main resolver behavior, and MCP is explicitly MVP-critical. Current automated checks cover Rust unit/integration coverage, but there is no dedicated end-to-end harness for:

- MCP contract behavior over stdio.
- End-user UI workflows across Main/History/Settings.
- Native desktop runtime smoke checks.

This PRD defines a practical, staged e2e stack that matches the current codebase and delivery constraints. It prioritizes deterministic and maintainable tests first, then layers exploratory AI-assisted testing as a secondary signal.

## Problem Statement

Without an e2e stack:

- MCP contract drift can break external clients silently.
- Cross-layer regressions (Rust + frontend integration) are detected late.
- Manual smoke testing becomes a bottleneck for each feature.

## Goals

1. Add a reliable e2e harness for MCP tool contracts using real stdio process execution.
2. Add fast UI smoke coverage for critical user journeys.
3. Keep the stack developer-friendly and runnable locally.
4. Fit into CI in two modes: fast gate and full/nightly gate.
5. Keep this initiative low priority and parallel to critical path feature work.

## Non-Goals

- Full cross-platform desktop matrix in phase 1.
- Visual regression baseline pipeline.
- Network sync/multi-device e2e (MVP.1 workstream).
- Replacing existing Rust tests.

## Success Criteria

- MCP e2e suite validates structured JSON contract for critical tools.
- At least one deterministic smoke path for quest lifecycle is automated.
- A documented local workflow exists for running e2e lanes.
- CI includes at least one required e2e lane for merge safety.

## Options Considered

### Option A: Playwright-only (web harness)

Pros:

- Fast setup and strong DX.
- Rich reporting, traces, retries, parallelism.

Cons:

- Does not natively validate Tauri desktop runtime behavior.
- Can miss native integration issues.

### Option B: WebDriver-only with `tauri-driver`

Pros:

- Official Tauri desktop e2e approach on Linux/Windows.
- Better signal for native runtime issues.

Cons:

- Slower setup and rougher DX compared with Playwright.
- Harder to scale broad scenario suites quickly.

### Option C: Hybrid stack (selected)

Use Playwright as orchestration/test framework plus dedicated MCP stdio harness first, then add a small native desktop smoke lane through `tauri-driver`.

Pros:

- Fast time-to-value for highest-risk area (MCP contract).
- Keeps room for native runtime validation.
- Supports modern agent-assisted test authoring workflows.

Cons:

- Two e2e technologies to maintain eventually.

## Chosen Stack

### 1) MCP contract e2e (first lane)

- Runner: Playwright Test (Node project mode).
- Transport: spawn `fini mcp` over stdio in test process.
- Assertions: strict structured JSON payload checks for:
  - `list_quests`
  - `list_history`
  - `get_active_quest`
  - plus create/update/complete/abandon/delete flows.
- Data isolation: per-test temporary data directory via env override.

### 2) Browser UI smoke e2e (second lane)

- Runner: Playwright (Chromium only initially).
- Scope: critical smoke only:
  - create quest
  - set Main
  - complete quest
  - confirm history visibility

### 3) Native desktop smoke (third lane, small)

- Driver: `tauri-driver` + WebDriver client.
- Scope: 3-5 tests max in first iteration.
- Purpose: verify desktop runtime path, not full scenario matrix.

### 4) Agentic exploratory lane (advisory)

- Tooling: Playwright agents (`planner`, `generator`, `healer`) and project AI workflows.
- Policy: non-blocking in CI until stability is proven.

## Proposed Repository Layout

```
e2e/
  mcp/
    mcp-client.ts
    contract.spec.ts
    lifecycle.spec.ts
  web/
    smoke-main.spec.ts
    smoke-history.spec.ts
  desktop/
    smoke-native.spec.ts
playwright.config.ts
```

## Workflow Details

### Local developer workflow

1. Run targeted lane while developing:
   - `npm run test:e2e:mcp`
   - `npm run test:e2e:web`
2. Run full local e2e before merge when touching contract/runtime:
   - `npm run test:e2e`

### CI workflow

- Fast required checks:
  - `cargo test --manifest-path src-tauri/Cargo.toml`
  - `cargo check --manifest-path src-tauri/Cargo.toml`
  - `npm run test:e2e:mcp`
- Optional/parallel checks initially:
  - `npm run test:e2e:web`
- Nightly/full checks:
  - desktop smoke lane with `tauri-driver`
  - agentic exploratory lane (report-only)

### Flake policy

- Required lanes cannot rely on retries to pass by default.
- If a test flakes twice in one week, quarantine with issue and owner.
- Keep smoke suites intentionally small.

## Data and Environment Strategy

- Each test run uses isolated temp storage paths.
- No shared mutable DB across parallel workers.
- No secrets in test fixtures.
- Stable deterministic timestamps where behavior depends on ordering.

## Implementation Plan (phased)

### Phase 0: scaffolding and docs

- Add Playwright config and npm scripts.
- Add e2e README and runbook.

### Phase 1: MCP harness (priority)

- Build stdio JSON-RPC helper.
- Add contract and lifecycle specs.
- Wire into fast CI gate.

### Phase 2: web smoke

- Add minimal browser smoke for critical flows.
- Add traces/report upload in CI.

### Phase 3: desktop smoke

- Add `tauri-driver` workflow and a tiny native suite.
- Keep scope narrow and stable.

### Phase 4: agentic exploratory lane

- Generate Playwright agent definitions.
- Run exploratory job nightly and collect findings.

## Risks and Mitigations

- Risk: browser harness diverges from native runtime.
  - Mitigation: maintain native smoke lane as a backstop.
- Risk: e2e suites become slow/noisy.
  - Mitigation: strict smoke scope, phased rollout, flake policy.
- Risk: agent-generated tests are brittle.
  - Mitigation: advisory-only status until stable over time.

## Acceptance Criteria for Ticket Completion

- Playwright scaffolding committed with documented run commands.
- MCP e2e suite exists and validates structured JSON contract.
- Fast CI includes MCP e2e lane.
- At least one UI smoke flow automated.
- Follow-up ticket exists for native desktop smoke lane if not included now.

## Unresolved questions

- none
