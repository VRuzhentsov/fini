//! Standalone broker for `ble-gatt`'s `mock-broker` feature: the shared
//! "radio" state that separate `fini-app` e2e actor processes connect to
//! over a socket instead of a real Bluetooth adapter, so the `actors-ble`
//! Playwright lane can prove `ble.rs`'s dial/peripheral/session-claim code
//! path without hardware. See `specs/e2e/actors/helpers/ble-sync.ts` and
//! `ble-gatt`'s `docs/adr/0004-mock-broker-for-cross-process-e2e.md`.
//!
//! Its own crate, not a `src-tauri` `[[bin]]` -- see this crate's
//! `Cargo.toml` for why.
//!
//! Reads `FINI_BLE_MOCK_BROKER_LISTEN` (host:port, e.g. `127.0.0.1:47600`)
//! and serves until the process is killed. The harness assigns this port
//! deterministically (see `fixtures.ts`'s `bleBrokerPort`) rather than
//! letting the OS pick one, since every actor needs to know it up front.

#[tokio::main]
async fn main() {
    let listen = std::env::var("FINI_BLE_MOCK_BROKER_LISTEN")
        .unwrap_or_else(|_| panic!("ble-mock-broker: FINI_BLE_MOCK_BROKER_LISTEN is not set"));
    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .unwrap_or_else(|err| panic!("ble-mock-broker: failed to bind {listen}: {err}"));
    // Plain eprintln!, not `log::` -- this standalone binary installs no
    // logger, so a `log::` call here would be silently discarded.
    eprintln!("[ble-mock-broker] listening on {listen}");
    if let Err(err) = ble_gatt::backend::mock::MockNetwork::serve(listener).await {
        eprintln!("[ble-mock-broker] serve exited: {err}");
        std::process::exit(1);
    }
}
