//! Standalone broker for `ble-gatt`'s `mock-broker` feature: the shared
//! "radio" state that separate `fini-app` e2e actor processes connect to
//! over a socket instead of a real Bluetooth adapter, so the `actors-ble`
//! Playwright lane can prove `ble.rs`'s dial/peripheral/session-claim code
//! path without hardware. See `specs/e2e/actors/helpers/ble-sync.ts` and
//! `ble-gatt`'s `docs/adr/0004-mock-broker-for-cross-process-e2e.md`.
//!
//! Reads `FINI_BLE_MOCK_BROKER_LISTEN` (host:port, e.g. `127.0.0.1:47600`)
//! and serves until the process is killed. The harness assigns this port
//! deterministically (see `fixtures.ts`'s `bleBrokerPort`) rather than
//! letting the OS pick one, since every actor needs to know it up front.

#[cfg(any(target_os = "linux", target_os = "android"))]
#[tokio::main]
async fn main() {
    let listen = std::env::var("FINI_BLE_MOCK_BROKER_LISTEN")
        .unwrap_or_else(|_| panic!("ble-mock-broker: FINI_BLE_MOCK_BROKER_LISTEN is not set"));
    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .unwrap_or_else(|err| panic!("ble-mock-broker: failed to bind {listen}: {err}"));
    // Plain eprintln!, not `log::` -- this standalone binary installs no
    // logger (only `fini-app`'s `run()` wires up `tauri-plugin-log`), so a
    // `log::` call here would be silently discarded by the facade.
    eprintln!("[ble-mock-broker] listening on {listen}");
    if let Err(err) = ble_gatt::backend::mock::MockNetwork::serve(listener).await {
        eprintln!("[ble-mock-broker] serve exited: {err}");
        std::process::exit(1);
    }
}

// `ble-gatt` is only a dependency on Linux/Android (see `Cargo.toml`'s
// target-gated `[dependencies]` tables) -- this binary still needs to build
// (just do nothing useful) on other targets so `cargo build --features
// devtools` doesn't break Windows/macOS builds that never run this lane.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn main() {
    panic!("ble-mock-broker is only supported on Linux/Android (the actors-ble e2e lane runs Linux-only)");
}
