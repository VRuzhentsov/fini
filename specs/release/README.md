# Release Workflow

## Desktop updater signing

### Observations

- Release `v0.1.40` succeeded with `bundle.createUpdaterArtifacts=false` and an empty updater public key.
- Release `v0.1.41` enabled updater artifacts while `src-tauri/tauri.conf.json` still had an empty updater `pubkey` and no release endpoint.
- The failed release logs stopped after desktop bundles were produced, when Tauri attempted to generate signed updater artifacts and reported `failed to decode pubkey: failed to convert updater pubkey: Missing comment in public key`.
- The updater public key is public verification material; the private signing key and password are the only secret release inputs.

### Expectations

- Prefer the smallest correct release fix: populate the missing public updater configuration instead of adding CI-time configuration mutation or normalization layers.
- Commit the Tauri updater public key and release endpoint in `src-tauri/tauri.conf.json` so the release source tree is the source of truth.
- Keep the matching private signing key in GitHub Actions secrets as `TAURI_SIGNING_PRIVATE_KEY`; use `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` only when the key is password-protected.
- Keep `FINI_TAURI_UPDATER_PUBKEY` aligned with the committed public key while Rust runtime update code still reads the compiled updater key from that environment variable.
- When the fix direction is ambiguous, present the small config-population option before proposing automation-heavy fallback options.
