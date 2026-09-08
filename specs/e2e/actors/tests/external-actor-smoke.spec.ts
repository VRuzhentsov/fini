import { expect, test } from '../fixtures.ts';

/**
 * Proves the external-actor path end to end: one actor this harness spawned
 * and owns, one it merely connects to over TCP (a real Android device, via
 * `adb forward`). Both are driven through the identical `invoke` surface, so
 * anything a spec can ask of a desktop actor it can ask of the phone.
 *
 * Deliberately asserts only what holds for *any* pair of live actors --
 * identities exist and differ. Pairing/sync state on an external device is
 * whatever that device already has (the harness neither configures nor
 * resets it, see `externalActorPorts`' doc comment), so asserting anything
 * about it here would make this smoke test depend on the operator's phone.
 *
 * Run (phone side prepared with `make android-debug-deploy` + adb forward):
 *   FINI_E2E_ACTORS=actor-a,phone \
 *   FINI_E2E_EXTERNAL_ACTORS=phone=9223 \
 *   npx playwright test --config specs/e2e/playwright.config.ts \
 *     --project actors -g "external actor"
 */
// Addresses actors positionally (actorA/actorB) rather than by slug: this
// same spec has to hold whether the pair is two spawned processes, a spawned
// desktop plus a phone, or two external devices -- naming a slug would tie it
// to one of those wirings.
test('both actors answer the same invoke surface', async ({ actorA, actorB }) => {
  const identityA = await actorA.invoke<{ device_id: string }>('device_connection_get_identity');
  const identityB = await actorB.invoke<{ device_id: string }>('device_connection_get_identity');

  expect(identityA.device_id).toBeTruthy();
  expect(identityB.device_id).toBeTruthy();
  expect(identityB.device_id).not.toBe(identityA.device_id);
});
