# Skill Feature E2E

This feature defines ideal end-to-end QA coverage for skill-driven interactions in Fini.

## Goal

Validate that user intent expressed in natural language is translated into correct, deterministic product actions.

## Ideal Skill Contract

- Skill resolves user intent into explicit actions with no hidden side effects.
- Skill selects the correct space and lifecycle action for each request.
- Skill confirms outcomes with structured, user-visible results.
- Skill reports validation and failure states clearly.

## Core Scenarios

1. Create quest with default space behavior.
2. Create quest with explicit space selection.
3. Update quest fields and verify persistence after reload.
4. Complete, abandon, and delete lifecycle transitions.
5. Create and remove reminders.
6. Handle cross-device side effects when action scope spans paired devices.

## Assertions

- Generated action sequence matches user intent.
- Resulting records match requested state changes.
- Errors are explicit and actionable.
- No unrelated data changes occur.

## Evidence

- Action transcript from the test harness.
- Structured outputs for created/updated/deleted records.
- DOM/state evidence for user-visible claims.
- Screenshot evidence only as rare fallback when DOM/state evidence is unavailable.

## Cleanup

- Remove all skill-created test data from all participating devices.
- Restore baseline focus, mapping, and pairing state when changed by the test.
- Verify cleanup completion before closing the test case.
