---
name: fini-test
description: "The Fini testing process: the stabilize phase of Spike and Stabilize. Use after a feature is done and manually verified, or when the user asks to run tests, write a test, add coverage, debug a failing test, choose a test category, set up a multi-device scenario (TestDevice fixture, virtual Android phones), or understand fixtures and harnesses. For Android-only behavior, load `fini-android-testing` instead. For CLI semantics referenced from CLI e2e tests, also load `fini-cli`. For Makefile/CI/script changes around tests, also load `fini-scripting`."
---

# Fini testing

This is the stabilize phase of Spike and Stabilize (`dev`, step 5). Start it
only after the feature is done and you verified it manually against the
requirements. Until then, prove behaviour on the running HMR apps (`dev`,
steps 3 and 4).

The test surfaces, the commands, and how to write each kind of test are in
`docs/testing.md`. The e2e harness is in `docs/e2e.md` and
`docs/e2e-android.md`.

## Stabilize

1. Choose the test category for each behavior with `docs/testing.md`, "Choose
   the test category".
2. Update or add tests so that they describe the final, verified behaviour.
   Write them characterization-style, from the requirements and the manual
   checks with their results: assert what the feature does, not how the code
   does it. Delete the tests for behaviour that you removed. Follow the
   conventions for the surface in `docs/testing.md`, "Writing …".
3. Run only the affected tests: the spec files and Playwright projects that
   cover the changed code (`docs/testing.md`, "Running tests" and "Targeting
   a single Playwright project"). Leave the full suite to CI, which runs on
   the PR.
4. Device or hardware testing needs the authorization of the user. Get the
   approval of the user before each run on a physical device, and follow
   `fini-android-testing` and `docs/android.md`.
5. Report the software checks and the device or hardware results as
   separate claims, each as "Evidence requirement" says.

A behavior that fits no surface in `docs/testing.md` needs a harness change,
not a test change. Load `fini-scripting` before adding new Makefile targets,
npm scripts, Playwright projects, or CI jobs, and add the new surface to
`docs/testing.md` once it lands.

## Evidence requirement

Before claiming a test passes, present:

1. **Command** — the exact command invoked, including project flags or env vars.
2. **Outcome** — observed exit code or pass/fail summary line from the runner.
3. **Artifacts** — for e2e, the `test-results/` directory path produced by Playwright (videos, traces, screenshots); for unit tests, the runner's printed test count.
4. **Limits** — any test skipped, any harness not exercised, any flake suspected.

Failures must include the same four items plus the failing test name and the assertion message.
