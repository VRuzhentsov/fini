//! The transport-neutral peer protocol engine: pairing gate, auth gate, and
//! the authenticated sync session loop. Operates purely on `PeerFrame` over
//! a `Link` trait object, so it is shared verbatim by every transport
//! adapter's accept/dial code (`transport::tcp_ws`, `transport::sim`, and
//! the future real Bluetooth adapter).

use std::path::PathBuf;
use std::time::Duration;

use diesel::prelude::*;
use tokio::sync::mpsc;

use crate::schema::{pair_space_mappings, paired_devices};
use crate::services::db::open_db_at_path;
use crate::services::device_connection::{
    DeviceConnectionState, IncomingSpaceMappingUpdate, IncomingSpaceSyncEnd, IncomingSyncAck,
};
use crate::services::space_sync::outbox::load_events_for_space;
use crate::services::space_sync::types::{PeerFrame, SessionCommand, PROTOCOL_VERSION};
use crate::services::transport::{recv_frame, send_frame, Link, TransportKind};

fn check_paired(db_path: &PathBuf, device_id: &str) -> bool {
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        paired_devices::table
            .find(device_id)
            .count()
            .get_result::<i64>(&mut conn)
            .unwrap_or(0)
            > 0
    })
}

/// Whether `device_id`'s paired-device row currently has Bluetooth enabled.
/// Checked only for `TransportKind::Bluetooth` accepts, in addition to
/// `check_paired` -- `bluetooth_dial_candidates` already enforces this on
/// the *dialing* side, but `run_peer_gate` had no equivalent on the
/// *accepting* side: a peer that still had this pair's Bluetooth enabled on
/// their end could dial in and connect to our advertising GATT server, and
/// be authenticated on `check_paired` alone, even after the local user
/// disabled Bluetooth for this pair -- contradicting
/// `specs/device-connect/README.md`'s "disabling ... prevents future
/// Bluetooth use" contract. `unwrap_or(false)` fails closed: a peer row
/// that's gone (unpaired) or unreadable must not be treated as enabled.
fn check_bluetooth_enabled(db_path: &PathBuf, device_id: &str) -> bool {
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        paired_devices::table
            .find(device_id)
            .select(paired_devices::bluetooth_enabled)
            .first::<bool>(&mut conn)
            .unwrap_or(false)
    })
}

/// Whether this device has *explicitly* disabled Bluetooth for
/// `device_id`'s pair (`device_connection_set_bluetooth_transport_impl`'s
/// disable branch) -- checked by `BluetoothProbe`'s pre-auth handler so an
/// explicit opt-out isn't bypassed by "Find via Bluetooth": that flow's
/// whole point is discovering an address for a pair that has *never* been
/// enabled (see its own doc comment), but a pair the user actively turned
/// off is a different case entirely. Replying would let the other side
/// believe discovery succeeded and persist/enable the address on its own
/// end, only for every real session attempt to then be rejected by
/// `check_bluetooth_enabled` here -- `specs/device-connect/README.md`'s
/// "disabling ... prevents future Bluetooth use" contract, silently
/// undermined via a side channel that predates it. `unwrap_or(false)`
/// fails open here on purpose (opposite of `check_bluetooth_enabled`'s
/// fail-closed): an unpaired/unreadable row has nothing to have been
/// disabled, matching a never-enabled pair.
fn check_bluetooth_disabled_by_user(db_path: &PathBuf, device_id: &str) -> bool {
    tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        paired_devices::table
            .find(device_id)
            .select(paired_devices::bluetooth_disabled_by_user)
            .first::<bool>(&mut conn)
            .unwrap_or(false)
    })
}

/// Whether the link's actual peer address matches this pair's stored
/// Bluetooth address, *and* that address is currently OS-bonded.
///
/// `check_bluetooth_enabled` alone only proves the authenticated
/// `device_id` belongs to a row with Bluetooth turned on -- it says nothing
/// about whether the central that just connected over BLE is actually the
/// specific bonded hardware the pairing metadata expects. `device_id` is an
/// app-level identifier carried inside the `Auth` frame, not tied to any
/// physical radio; `specs/device-connect/README.md` states OS Bluetooth
/// pairing as an additional, transport-level precondition specifically for
/// Bluetooth (unlike tcp_ws/sim, which have no such notion), and Android's
/// `BleGattBridge` advertises plain, unencrypted GATT characteristics (no
/// `PERMISSION_*_ENCRYPTED`), so nothing at the BLE stack level enforces
/// bonding before a connection succeeds -- that enforcement has to happen
/// here. Mirrors `bluetooth_dial_candidates`'s own dial-side eligibility
/// check. Fails closed on any missing/unreadable/mismatched data.
fn check_bluetooth_bond(db_path: &PathBuf, device_id: &str, observed_address: Option<&str>) -> bool {
    let Some(observed_address) = observed_address else {
        return false;
    };
    let stored: Option<String> = tokio::task::block_in_place(|| {
        let mut conn = open_db_at_path(db_path);
        paired_devices::table
            .find(device_id)
            .select(paired_devices::bluetooth_address)
            .first::<Option<String>>(&mut conn)
            .unwrap_or(None)
    });
    let Some(stored) = stored else {
        return false;
    };
    if !stored.eq_ignore_ascii_case(observed_address) {
        return false;
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // `bluetooth_address_is_os_paired` internally does a
        // `tauri::async_runtime::block_on` on Linux (bounding its
        // `bluetoothctl` subprocess) -- calling that directly from this
        // async fn's body would risk "cannot start a runtime from within a
        // runtime" on whichever worker thread is currently driving this
        // task. `block_in_place` (already used for the DB read above) is
        // the sanctioned way to run blocking/nested-runtime work safely
        // from inside an async task on a multi-threaded runtime.
        tokio::task::block_in_place(|| {
            crate::services::device_connection::bluetooth_address_is_os_paired(&stored)
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        false
    }
}

/// Inverse of `transport_kind_to_preference_string`'s (many-to-one) mapping
/// -- `preferred_transport` is only ever written as "network" or
/// "bluetooth" by anything that runs over a real wire (see that function's
/// own doc comment for why Sim/LoRa never appear here in practice), so this
/// reconstructs the one concrete `TransportKind` each maps back to.
fn transport_kind_from_preference_string(preference: &str) -> TransportKind {
    match preference {
        "network" => TransportKind::TcpWs,
        _ => TransportKind::Bluetooth,
    }
}

/// Client-side auth handshake: send `Auth`, await `AuthOk`/`AuthFail`.
/// Shared by every adapter's dial path. Returns the peer's reported
/// `PROTOCOL_VERSION` (`0` for a peer running a build from before that
/// field existed) so the caller's `run_session` knows which proactive
/// frames are safe to send -- see `PROTOCOL_VERSION`'s doc comment.
pub async fn perform_client_auth(
    link: &mut dyn Link,
    my_device_id: &str,
    peer_device_id: &str,
) -> Result<u32, String> {
    send_frame(
        link,
        &PeerFrame::Auth {
            device_id: my_device_id.to_string(),
            peer_device_id: peer_device_id.to_string(),
            protocol_version: PROTOCOL_VERSION,
        },
    )
    .await?;

    match recv_frame(link).await {
        Some(Ok(PeerFrame::AuthOk { protocol_version })) => Ok(protocol_version),
        Some(Ok(PeerFrame::AuthFail { reason })) => Err(format!("auth rejected: {reason}")),
        Some(Ok(_)) => Err("unexpected reply to auth".to_string()),
        Some(Err(err)) => Err(err),
        None => Err("connection closed before auth reply".to_string()),
    }
}

/// Server-side gate: read the first frame off a freshly accepted `Link` and
/// dispatch it. Pre-auth pairing messages (`PairRequest`/`PairAccept`/
/// `PairComplete`) are handled and the link is then closed — discovery and
/// pairing metadata are untrusted regardless of which transport carried
/// them. An `Auth` frame is checked against `paired_devices`; on success the
/// sticky single-session invariant is enforced via `try_claim_session`
/// before `AuthOk` is sent and the session loop starts. Transport-neutral:
/// call this from every adapter's accept loop.
///
/// `ui-plane`/`test` only: gated the same way the original `ws_server`
/// listener was — `cli-plane` dials out for sync (see the adapters'
/// `spawn_dial_loop`/`spawn_fallback_dial_loop`, ungated) but does not run
/// an inbound listener/pairing acceptor.
#[cfg(any(feature = "ui-plane", test))]
pub async fn run_peer_gate(mut link: Box<dyn Link>, state: DeviceConnectionState, db_path: PathBuf) {
    let kind = link.kind();
    let from_addr = link.peer_addr().unwrap_or_default();
    let Some(Ok(frame)) = recv_frame(link.as_mut()).await else {
        return;
    };

    let (device_id, peer_device_id, peer_protocol_version) = match frame {
        PeerFrame::PairRequest(payload) => {
            let _ = state.receive_ws_pair_request(payload, from_addr, kind == TransportKind::Bluetooth);
            return;
        }
        PeerFrame::PairAccept(payload) => {
            let _ = state.receive_ws_pair_accept(payload);
            return;
        }
        PeerFrame::PairComplete(payload) => {
            let _ =
                state.receive_ws_pair_complete(payload, from_addr, kind == TransportKind::Bluetooth);
            return;
        }
        PeerFrame::BluetoothProbe { device_id } => {
            // Deliberately `check_paired`, not `check_bluetooth_enabled`:
            // this exists precisely so "Find via Bluetooth" can confirm an
            // address for a pair that doesn't have Bluetooth enabled yet.
            // But an *explicit* disable is a different case from
            // never-enabled -- see `check_bluetooth_disabled_by_user`'s
            // doc comment for why that one must still gate the reply.
            if check_paired(&db_path, &device_id)
                && !check_bluetooth_disabled_by_user(&db_path, &device_id)
            {
                let _ = send_frame(
                    link.as_mut(),
                    &PeerFrame::BluetoothProbeReply {
                        device_id: state.identity.device_id.clone(),
                    },
                )
                .await;
            }
            return;
        }
        PeerFrame::DiscoveryHello => {
            // No reply at all when not in add-mode -- matching the
            // network-discovery equivalent (a mDNS beacon simply isn't
            // broadcast outside add-mode), rather than an explicit
            // rejection frame that would let a scanner distinguish "not in
            // add-mode" from "connection failed."
            if state.is_add_mode_enabled() {
                let _ = send_frame(
                    link.as_mut(),
                    &PeerFrame::DiscoveryHelloReply {
                        device_id: state.identity.device_id.clone(),
                        hostname: state.identity.hostname.clone(),
                    },
                )
                .await;
            }
            return;
        }
        PeerFrame::Auth {
            device_id,
            peer_device_id,
            protocol_version,
        } => (device_id, peer_device_id, protocol_version),
        _ => {
            let _ = send_frame(
                link.as_mut(),
                &PeerFrame::AuthFail {
                    reason: "expected auth first".into(),
                },
            )
            .await;
            return;
        }
    };

    if peer_device_id != state.identity.device_id {
        let _ = send_frame(
            link.as_mut(),
            &PeerFrame::AuthFail {
                reason: "wrong target device".into(),
            },
        )
        .await;
        return;
    }

    if !check_paired(&db_path, &device_id) {
        let _ = send_frame(
            link.as_mut(),
            &PeerFrame::AuthFail {
                reason: "unknown device".into(),
            },
        )
        .await;
        return;
    }

    if kind == TransportKind::Bluetooth {
        if !check_bluetooth_enabled(&db_path, &device_id) {
            let _ = send_frame(
                link.as_mut(),
                &PeerFrame::AuthFail {
                    reason: "bluetooth disabled for this pair".into(),
                },
            )
            .await;
            return;
        }
        if !check_bluetooth_bond(&db_path, &device_id, link.peer_addr().as_deref()) {
            let _ = send_frame(
                link.as_mut(),
                &PeerFrame::AuthFail {
                    reason: "bluetooth device is not currently OS-paired".into(),
                },
            )
            .await;
            return;
        }
    }

    let (tx, rx) = mpsc::channel::<SessionCommand>(64);
    if !state.try_claim_session(&device_id, kind, tx, peer_protocol_version) {
        let _ = send_frame(
            link.as_mut(),
            &PeerFrame::AuthFail {
                reason: "session already active on another transport".into(),
            },
        )
        .await;
        return;
    }

    if send_frame(
        link.as_mut(),
        &PeerFrame::AuthOk {
            protocol_version: PROTOCOL_VERSION,
        },
    )
    .await
    .is_err()
    {
        state.release_session(&device_id);
        return;
    }

    run_session(link, rx, state, db_path, device_id, peer_protocol_version).await;
}

/// The authenticated per-peer message loop. `rx` is the mailbox side of the
/// session sender already claimed via `DeviceConnectionState::try_claim_session`
/// (by `run_peer_gate` on accept, or the adapter's dial loop on outbound
/// connect) — this function never claims the session itself, only releases
/// it on exit.
pub async fn run_session(
    mut link: Box<dyn Link>,
    mut rx: mpsc::Receiver<SessionCommand>,
    state: DeviceConnectionState,
    db_path: PathBuf,
    peer_device_id: String,
    peer_protocol_version: u32,
) {
    // ADR-0003 Phase 3: whichever side just established this session,
    // relay the LOCAL device's own transport preference for this peer if
    // it mismatches the transport this session actually landed on. This
    // is what converges a pin set while the peer was offline: nothing
    // could deliver SwitchTransport at the moment the pin was set (no
    // live session existed to carry it through -- see
    // `device_connection_set_preferred_transport_impl`), and a hard
    // reject at the accept gate can't fix that either -- if the *other*
    // side is the deterministic dialer and its own stale pin blocks it
    // from ever trying the correct transport, nothing would ever
    // establish a session for this relay to ride on. Accepting normally
    // and relaying here instead means the very first session that
    // manages to form at all (inbound or outbound, on whichever
    // transport got there first) delivers the correct pin. Uses the
    // pin's *original* `requested_at`, not "now" -- the peer's own
    // last-writer-wins adoption (`adopt_peer_transport_preference`)
    // depends on comparing against its own possibly-stale timestamp
    // correctly. Gated on protocol version like the Bluetooth self-report
    // below: a peer that can't decode SwitchTransport is left alone
    // entirely rather than force-closed for something it could never
    // converge on anyway.
    //
    // Deliberately does *not* close here, even though the mismatch was
    // just detected: this side's own record can itself be the stale one
    // (both peers set conflicting pins while offline, each unaware of the
    // other). Closing unconditionally on a guess that might be wrong
    // means the peer -- if it's the deterministic dialer -- just
    // reconnects on the same transport and gets closed again forever,
    // never actually converging. `handle_inbound`'s own SwitchTransport
    // handling is what actually decides whether *this* session needs to
    // close: it always replies with whichever preference wins, so the
    // losing side (whichever that turns out to be) ends up adopting the
    // correct one and, if it now disagrees with a session already open
    // elsewhere, closing that. If this device's guess was right all
    // along, the peer's own adoption closes its end, which tears this
    // side down too via the shared link -- no separate action needed here.
    if peer_protocol_version >= PROTOCOL_VERSION {
        let mismatch = tokio::task::block_in_place(|| {
            let mut conn = open_db_at_path(&db_path);
            crate::services::device_connection::peer_transport_preference_with_timestamp(
                &mut conn,
                &peer_device_id,
            )
        })
        .filter(|(preference, _)| {
            preference.as_str()
                != crate::services::device_connection::transport_kind_to_preference_string(link.kind())
        });

        if let Some((preference, requested_at)) = mismatch {
            let to = transport_kind_from_preference_string(&preference);
            let _ = send_frame(link.as_mut(), &PeerFrame::SwitchTransport { to, requested_at }).await;
        }
    }

    // Self-report our own Bluetooth address once per network session, if
    // this platform can read one at all -- see `PeerFrame::BluetoothAddressUpdate`'s
    // doc comment. Only over the network transport: sending it over an
    // already-live Bluetooth session would be reporting an address the
    // other side already used to reach us. Gated on `peer_protocol_version`
    // (learned during the Auth/AuthOk exchange, see `PROTOCOL_VERSION`):
    // a peer on a build from before this frame existed cannot decode it and
    // would drop the whole authenticated session, so this frame is never
    // sent proactively to a peer that hasn't proven it understands it.
    let bluetooth_self_report_enabled = link.kind() == TransportKind::TcpWs && peer_protocol_version >= 1;
    let mut last_reported_bluetooth_address: Option<String> = None;
    if bluetooth_self_report_enabled {
        if let Some(address) = crate::services::device_connection::local_bluetooth_address().await {
            if send_frame(
                link.as_mut(),
                &PeerFrame::BluetoothAddressUpdate { address: address.clone() },
            )
            .await
            .is_ok()
            {
                last_reported_bluetooth_address = Some(address);
            }
        }
    }

    // Re-checked periodically, not just once at session start: a network
    // session can stay live for a long time, and if the local Bluetooth
    // controller changes underneath it (e.g. a USB dongle swap) with
    // nothing to notice, the peer is left holding a stale address with no
    // other way to refresh it -- this self-report only ever travels over
    // the network transport, so once network sync eventually breaks, the
    // Bluetooth fallback would be stuck dialing an address that no longer
    // exists. `set_missed_tick_behavior(Delay)`: a slow tick (e.g. this
    // process suspended) should never fire a burst of catch-up sends.
    let mut bluetooth_recheck = tokio::time::interval(bluetooth_recheck_interval());
    bluetooth_recheck.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    bluetooth_recheck.tick().await; // interval fires immediately on the first tick; consume it

    loop {
        tokio::select! {
            inbound = recv_frame(link.as_mut()) => {
                let Some(Ok(frame)) = inbound else { break };
                handle_inbound(frame, link.as_mut(), &state, &db_path, &peer_device_id).await;
            }
            Some(command) = rx.recv() => {
                match command {
                    SessionCommand::Forward(frame) => {
                        if send_frame(link.as_mut(), &frame).await.is_err() {
                            break;
                        }
                    }
                    // ADR-0003 Phase 3: the only thing that ever sends this
                    // -- a manual transport switch closing the session on
                    // *this* transport so the next reconnect can honor the
                    // new preference. Deliberately no frame/reason sent to
                    // the peer here: `device_connection_set_preferred_
                    // transport_impl` already sent PeerFrame::SwitchTransport
                    // over this same mailbox (ahead of this Close, so it's
                    // delivered first) if there was a session to carry it.
                    SessionCommand::Close => break,
                }
            }
            _ = bluetooth_recheck.tick(), if bluetooth_self_report_enabled => {
                if let Some(address) = crate::services::device_connection::local_bluetooth_address().await {
                    if last_reported_bluetooth_address.as_deref() != Some(address.as_str())
                        && send_frame(
                            link.as_mut(),
                            &PeerFrame::BluetoothAddressUpdate { address: address.clone() },
                        )
                        .await
                        .is_ok()
                    {
                        last_reported_bluetooth_address = Some(address);
                    }
                }
            }
        }
    }

    state.release_session(&peer_device_id);
}

/// Test/CI escape hatch, mirroring `local_bluetooth_address`'s own
/// `FINI_LOCAL_BLUETOOTH_ADDRESS`: exercising the periodic re-check
/// deterministically can't wait on the real 5-minute interval.
fn bluetooth_recheck_interval() -> Duration {
    if let Ok(value) = std::env::var("FINI_BLUETOOTH_RECHECK_INTERVAL_MS") {
        if let Ok(ms) = value.parse::<u64>() {
            return Duration::from_millis(ms);
        }
    }
    Duration::from_secs(300)
}

async fn handle_inbound(
    frame: PeerFrame,
    link: &mut dyn Link,
    state: &DeviceConnectionState,
    db_path: &PathBuf,
    peer_device_id: &str,
) {
    match frame {
        PeerFrame::SyncEvent(envelope) => {
            let event_id = envelope.event_id.clone();
            state.push_incoming_sync_event(envelope);
            let _ = send_frame(link, &PeerFrame::Ack { event_id }).await;
        }
        PeerFrame::Ack { event_id } => {
            let acked_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            state.push_incoming_sync_ack(IncomingSyncAck {
                from_device_id: peer_device_id.to_string(),
                event_id,
                acked_at,
            });
        }
        PeerFrame::SpaceMappingUpdate {
            mapped_space_ids,
            custom_spaces,
            sent_at,
        } => {
            state.push_incoming_space_mapping_update(IncomingSpaceMappingUpdate {
                from_device_id: peer_device_id.to_string(),
                mapped_space_ids,
                custom_spaces,
                sent_at,
            });
        }
        PeerFrame::SpaceSyncEnd { space_id, ended_at } => {
            state.push_incoming_space_sync_end(IncomingSpaceSyncEnd {
                from_device_id: peer_device_id.to_string(),
                space_id,
                ended_at,
            });
        }
        PeerFrame::BootstrapStart { space_id } => {
            let db = db_path.clone();
            let sid = space_id.clone();
            let events = tokio::task::block_in_place(|| {
                let mut conn = open_db_at_path(&db);
                load_events_for_space(&mut conn, &sid).unwrap_or_default()
            });
            for event in events {
                if send_frame(link, &PeerFrame::SyncEvent(event)).await.is_err() {
                    return;
                }
            }
            let _ = send_frame(
                link,
                &PeerFrame::BootstrapEnd {
                    space_id,
                    completed_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                },
            )
            .await;
        }
        PeerFrame::BootstrapEnd {
            space_id,
            completed_at,
        } => {
            let db = db_path.clone();
            let peer = peer_device_id.to_string();
            tokio::task::block_in_place(|| {
                let mut conn = open_db_at_path(&db);
                let _ = diesel::update(
                    pair_space_mappings::table
                        .filter(pair_space_mappings::peer_device_id.eq(&peer))
                        .filter(pair_space_mappings::space_id.eq(&space_id)),
                )
                .set(pair_space_mappings::last_synced_at.eq(Some(completed_at)))
                .execute(&mut conn);
            });
        }
        PeerFrame::SwitchTransport { to, requested_at } => {
            use crate::services::device_connection::TransportPreferenceAdoption;

            let db = db_path.clone();
            let peer = peer_device_id.to_string();
            let (outcome, winning) = tokio::task::block_in_place(|| {
                let mut conn = open_db_at_path(&db);
                let outcome = crate::services::device_connection::adopt_peer_transport_preference(
                    &mut conn, &peer, to, &requested_at,
                );
                let winning = match outcome {
                    TransportPreferenceAdoption::Adopted => None,
                    TransportPreferenceAdoption::Stale => {
                        crate::services::device_connection::peer_transport_preference_with_timestamp(
                            &mut conn, &peer,
                        )
                    }
                    // Whatever the timestamp says, this device can't
                    // support `to` right now, and may have no preference
                    // of its own recorded at all to fall back on offering
                    // -- Network is the one transport every peer
                    // understands, so it's always a safe, concrete
                    // counter-proposal here. Stamped "now" so it's
                    // guaranteed to win over the rejected proposal instead
                    // of silently going unanswered (see this match arm's
                    // caller-side handling below for why silence would
                    // otherwise strand the sender).
                    TransportPreferenceAdoption::Ineligible => {
                        Some(("network".to_string(), crate::services::db::utc_now()))
                    }
                };
                (outcome, winning)
            });
            match outcome {
                TransportPreferenceAdoption::Adopted => {
                    // Only ever closes *this* session if the adopted
                    // preference actually won the race and doesn't already
                    // match what's live -- an older/losing `requested_at`
                    // changes nothing here, same as `device_connection_
                    // set_preferred_transport_impl`'s own local force-switch.
                    if state.session_kind(&peer) != Some(to) {
                        state.request_session_close(&peer);
                    }
                }
                TransportPreferenceAdoption::Stale | TransportPreferenceAdoption::Ineligible => {
                    // Rejected -- either the sender's info was stale, or it
                    // proposed a transport this device can't actually adopt
                    // (Bluetooth disabled locally). Reply with whichever
                    // preference actually applies so the sender can
                    // self-correct instead of repeatedly offering (and this
                    // device repeatedly rejecting) the same stale or
                    // unsupportable proposal -- see `run_session`'s startup
                    // relay for why it deliberately doesn't just close
                    // unconditionally on a guess that might itself be
                    // wrong. Only reply when there's an actual correction
                    // to offer: if the winning target already matches what
                    // was proposed (just a tied/older timestamp for an
                    // otherwise-agreed transport), staying silent avoids an
                    // endless reply-to-a-reply loop between two sides that
                    // already agree.
                    if let Some((preference, winning_requested_at)) = winning {
                        let winning_kind = transport_kind_from_preference_string(&preference);
                        if winning_kind != to {
                            let _ = send_frame(
                                link,
                                &PeerFrame::SwitchTransport {
                                    to: winning_kind,
                                    requested_at: winning_requested_at,
                                },
                            )
                            .await;
                        }
                    }
                }
            }
        }
        PeerFrame::BluetoothAddressUpdate { address } => {
            let Some(address) = crate::services::device_connection::normalize_bluetooth_address(&address)
            else {
                return;
            };
            let db = db_path.clone();
            let peer = peer_device_id.to_string();
            tokio::task::block_in_place(|| {
                let mut conn = open_db_at_path(&db);
                if let Err(err) =
                    crate::services::device_connection::persist_bluetooth_address_and_maybe_enable(
                        &mut conn, &peer, &address,
                    )
                {
                    eprintln!("[space-sync] persist bluetooth self-report failed: {err}");
                }
            });
        }
        // Pre-auth only (handled earlier in run_peer_gate's first-frame
        // dispatch) or sent by this side -- never expected inbound here.
        PeerFrame::Auth { .. }
        | PeerFrame::AuthOk { .. }
        | PeerFrame::AuthFail { .. }
        | PeerFrame::PairRequest(_)
        | PeerFrame::PairAccept(_)
        | PeerFrame::PairComplete(_)
        | PeerFrame::DiscoveryHello
        | PeerFrame::DiscoveryHelloReply { .. }
        | PeerFrame::BluetoothProbe { .. }
        | PeerFrame::BluetoothProbeReply { .. }
        // A tag this build doesn't recognize -- see `PeerFrame::Unknown`'s
        // doc comment. Ignoring it is the whole point: the session must
        // keep running rather than treat it as a decode failure.
        | PeerFrame::Unknown => {}
    }
}
