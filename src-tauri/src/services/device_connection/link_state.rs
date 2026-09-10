//! The owned state machine for one peer's link on one transport (ADR-0005).
//!
//! **Load-bearing. Read `docs/adr/0005-transport-link-state-machine.md` before
//! changing anything here.** Everything a user sees about connectivity is
//! downstream of this file: the grey/amber/green row, whether a dial happens,
//! whether an incoming connection is accepted. It is listed among the
//! load-bearing files in `src-tauri/README.md` for that reason.
//!
//! Three properties below are deliberate and easy to erode by accident --
//! purity, effects being a bare minimum rather than a command channel, and
//! unhandled pairs being no-ops. Each has its own note at the point it
//! matters; none of them is incidental.
//!
//! Everything here is pure: `LinkState::apply` takes the current state, an
//! event and the current time, and returns the next state plus whatever must
//! happen as a consequence. No clock reads, no locks, no I/O. That is what
//! makes the whole transition table testable without a peer, a radio or a
//! database -- the three things whose absence in CI let every defect ADR-0005
//! lists reach real hardware instead.
//!
//! The machine deliberately does *not* decide policy it cannot see. Whether
//! this device is the designated dialer for a pair depends on both device ids
//! (`transport::ble::should_dial_peer`), so the caller owns that decision and
//! reports the outcome as `DialStarted`. The machine owns what the *state* is,
//! never who is allowed to act.

use std::time::{Duration, Instant};

use super::transport::TransportStatusCode;

/// How long a lapsed liveness proof is tolerated before the link is torn down.
///
/// Two ping cycles (`APP_PING_INTERVAL`, 15s). This is a *second* tolerance
/// stacked on the proof's own: the proof already survives 3 missed cycles
/// (~45s) before lapsing at all, so a link is torn down after ~75s of genuine
/// silence. Long enough to ride out a radio glitch without reconnect churn,
/// short enough that nothing sits dead for an hour -- which is exactly what
/// happened when this edge did not exist (ADR-0005's opening evidence).
pub(crate) const FADE_GRACE: Duration = Duration::from_secs(30);

/// How long a dial or an auth handshake may run before it is given up on.
/// Matches `transport::ble::AUTO_RETRY_WINDOW`, whose own doc comment records
/// the real-device evidence for it: a flaky link that connects, negotiates
/// MTU and completes service discovery, then dies before the app-level auth
/// reply -- repeatedly, for minutes, with an indefinite "Still connecting..."
/// as the only visible symptom.
pub(super) const ATTEMPT_WINDOW: Duration = Duration::from_secs(60);

/// One peer's link on one transport.
///
/// The invariant this type exists to hold, asserted in `has_session` and
/// property-tested below:
///
/// > A session exists **iff** the state is `Authenticating`, `Proving`,
/// > `Live` or `Fading`.
///
/// Every "the row says connected but nothing is" defect in this area's history
/// is a violation of that sentence. Under the previous design it could not
/// even be stated, because "is there a session" and "is it alive" lived in
/// separate maps that nothing reconciled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LinkState {
    /// Preconditions for this transport aren't met: no adapter, disabled for
    /// this pair, no stored address, not OS-bonded, no network presence.
    /// Nothing to attempt until that changes.
    Unavailable { reason: TransportStatusCode },
    /// Eligible, nothing in flight. `retry_at` carries dial backoff: `Some`
    /// means "eligible, but not before this instant".
    Idle { retry_at: Option<Instant> },
    /// An outbound attempt is in flight.
    Dialing { since: Instant },
    /// The link is up and the auth exchange is running.
    Authenticating { since: Instant },
    /// A session is claimed; the first ping/ack round has not completed yet.
    Proving { since: Instant },
    /// Liveness proven, currently. The only green state.
    Live,
    /// The proof lapsed. A grace window is running toward teardown; a proof
    /// that completes again before it expires returns to `Live`.
    Fading { since: Instant },
    /// Automatic retries were exhausted. Left only by an explicit user retry
    /// or by the peer dialling in -- never on its own, so a peer that is
    /// genuinely gone stops costing radio time.
    GaveUp { since: Instant },
}

/// Something that happened to the link. Submitted by whichever component
/// observed it; the machine decides what it means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LinkEvent {
    /// Preconditions are no longer met (and why).
    PreconditionsLost { reason: TransportStatusCode },
    /// Preconditions are met again.
    PreconditionsMet,
    /// The caller decided to dial and has started doing so.
    DialStarted,
    /// The dial attempt failed. `retry_at` is the caller's backoff decision.
    DialFailed { retry_at: Option<Instant> },
    /// A link is up -- ours after a successful dial, or the peer's after they
    /// dialled us -- and auth is about to run.
    LinkEstablished,
    /// Auth completed and a session is claimed.
    AuthSucceeded,
    /// Auth was rejected or the link died before it completed.
    AuthFailed { retry_at: Option<Instant> },
    /// The bidirectional ping/ack proof is currently complete.
    ProofComplete,
    /// The proof lapsed (3 missed cycles on either direction).
    ProofLapsed,
    /// The session ended, from either side, for any reason.
    SessionEnded,
    /// The user asked to try again -- the Device row's click affordance.
    UserRetryRequested,
    /// Time passed. Drives every deadline in the machine; nothing else here
    /// reads a clock.
    Tick,
}

/// What must happen as a consequence of a transition.
///
/// Deliberately one variant. Effects are for things the machine *must* cause
/// to keep its own invariant true -- not a general command channel. Dialing,
/// for instance, is not an effect: the caller owns that decision and reports
/// it back as `DialStarted`, so the state never claims an attempt nobody made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LinkEffect {
    /// Close the transport link and release the session slot. Emitted only
    /// where honesty demands it: the state is about to stop being a session
    /// state, so the session must actually stop existing.
    TearDownSession,
}

impl LinkState {
    /// The starting point for a pair whose preconditions are already met.
    pub(crate) fn new() -> Self {
        LinkState::Idle { retry_at: None }
    }

    /// The invariant from this type's doc comment, in code. Callers use it to
    /// answer "is a session claimed" instead of consulting a separate map --
    /// that separation is what allowed the two to disagree.
    pub(crate) fn has_session(&self) -> bool {
        matches!(
            self,
            LinkState::Authenticating { .. }
                | LinkState::Proving { .. }
                | LinkState::Live
                | LinkState::Fading { .. }
        )
    }

    /// Whether a dial may be started now. Necessary, not sufficient: the
    /// caller still applies the pair's designated-dialer tiebreak.
    pub(super) fn may_dial(&self, now: Instant) -> bool {
        match self {
            LinkState::Idle { retry_at } => retry_at.is_none_or(|at| now >= at),
            _ => false,
        }
    }

    /// The pure transition. Returns the next state and any effects.
    ///
    /// Unhandled `(state, event)` pairs are no-ops by design rather than
    /// errors: events arrive from concurrent components (a ping tick, a
    /// dropped link, a user click) and can legitimately race a transition
    /// that already moved past them. A late `ProofLapsed` for a session that
    /// has already ended must be ignored, not panic.
    pub(super) fn apply(&self, event: LinkEvent, now: Instant) -> (LinkState, Vec<LinkEffect>) {
        use LinkEvent as E;
        use LinkState as S;

        match (self, event) {
            // Preconditions gone: anything live must actually stop, or the
            // state would outlive the thing it describes.
            (state, E::PreconditionsLost { reason }) => {
                let effects = if state.has_session() {
                    vec![LinkEffect::TearDownSession]
                } else {
                    vec![]
                };
                (S::Unavailable { reason }, effects)
            }
            (S::Unavailable { .. }, E::PreconditionsMet) => (S::new(), vec![]),

            (S::Idle { .. }, E::DialStarted) => (S::Dialing { since: now }, vec![]),
            (S::Dialing { .. }, E::DialFailed { retry_at }) => (S::Idle { retry_at }, vec![]),

            // Accepted from Idle and GaveUp alike: the peer dialling us is
            // exactly the evidence that makes giving up wrong.
            (S::Idle { .. } | S::Dialing { .. } | S::GaveUp { .. }, E::LinkEstablished) => {
                (S::Authenticating { since: now }, vec![])
            }

            (S::Authenticating { .. }, E::AuthSucceeded) => (S::Proving { since: now }, vec![]),
            (S::Authenticating { .. }, E::AuthFailed { retry_at }) => {
                (S::Idle { retry_at }, vec![LinkEffect::TearDownSession])
            }

            (S::Proving { .. } | S::Fading { .. }, E::ProofComplete) => (S::Live, vec![]),
            (S::Live, E::ProofLapsed) => (S::Fading { since: now }, vec![]),

            // The edge that did not exist. Without it the row merely turned
            // amber while a dead session kept the transport's only slot, and
            // every subsequent connection from that peer was refused.
            (S::Fading { since }, E::Tick) if now.duration_since(*since) >= FADE_GRACE => {
                (S::Idle { retry_at: None }, vec![LinkEffect::TearDownSession])
            }

            (S::Dialing { since }, E::Tick) if now.duration_since(*since) >= ATTEMPT_WINDOW => {
                (S::GaveUp { since: now }, vec![])
            }
            // A hung auth is given up on the same way a hung dial is: it holds
            // a session slot while proving nothing.
            (S::Authenticating { since }, E::Tick)
                if now.duration_since(*since) >= ATTEMPT_WINDOW =>
            {
                (S::GaveUp { since: now }, vec![LinkEffect::TearDownSession])
            }
            // A session that never completes its first proof is not connected
            // in any useful sense, and must not hold the slot forever.
            (S::Proving { since }, E::Tick) if now.duration_since(*since) >= ATTEMPT_WINDOW => {
                (S::Idle { retry_at: None }, vec![LinkEffect::TearDownSession])
            }

            (state, E::SessionEnded) if state.has_session() => {
                // No teardown effect: the session is already gone, which is
                // what this event reports. Emitting one would double-close.
                (S::Idle { retry_at: None }, vec![])
            }

            (S::GaveUp { .. }, E::UserRetryRequested) => (S::new(), vec![]),
            (S::Idle { .. }, E::UserRetryRequested) => (S::new(), vec![]),

            (state, _) => (state.clone(), vec![]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn fading_tears_the_session_down_once_grace_expires() {
        // ADR-0005's opening evidence, as a test: a session went amber and was
        // still claimed 1h44m later, refusing every incoming connection.
        let start = t0();
        let live = LinkState::Live;

        let (fading, effects) = live.apply(LinkEvent::ProofLapsed, start);
        assert_eq!(fading, LinkState::Fading { since: start });
        assert!(effects.is_empty(), "lapsing alone must not tear down");
        assert!(fading.has_session(), "still connected during grace");

        let (still_fading, effects) = fading.apply(LinkEvent::Tick, start + FADE_GRACE / 2);
        assert_eq!(still_fading, fading, "grace not expired yet");
        assert!(effects.is_empty());

        let (idle, effects) = fading.apply(LinkEvent::Tick, start + FADE_GRACE);
        assert_eq!(idle, LinkState::Idle { retry_at: None });
        assert_eq!(effects, vec![LinkEffect::TearDownSession]);
        assert!(!idle.has_session());
    }

    #[test]
    fn a_recovered_proof_cancels_the_fade() {
        let start = t0();
        let fading = LinkState::Fading { since: start };
        let (state, effects) = fading.apply(LinkEvent::ProofComplete, start + FADE_GRACE / 2);
        assert_eq!(state, LinkState::Live);
        assert!(effects.is_empty());
    }

    #[test]
    fn an_inbound_link_rescues_a_given_up_peer() {
        // The peer dialling in is the evidence that makes giving up wrong.
        let start = t0();
        let gave_up = LinkState::GaveUp { since: start };
        let (state, _) = gave_up.apply(LinkEvent::LinkEstablished, start);
        assert_eq!(state, LinkState::Authenticating { since: start });
    }

    #[test]
    fn losing_preconditions_tears_down_a_live_session() {
        let start = t0();
        let (state, effects) = LinkState::Live.apply(
            LinkEvent::PreconditionsLost {
                reason: TransportStatusCode::BluetoothDisabled,
            },
            start,
        );
        assert_eq!(
            state,
            LinkState::Unavailable {
                reason: TransportStatusCode::BluetoothDisabled
            }
        );
        assert_eq!(effects, vec![LinkEffect::TearDownSession]);
    }

    #[test]
    fn losing_preconditions_without_a_session_tears_nothing_down() {
        let start = t0();
        let (_, effects) = LinkState::Idle { retry_at: None }.apply(
            LinkEvent::PreconditionsLost {
                reason: TransportStatusCode::BluetoothNoAddress,
            },
            start,
        );
        assert!(effects.is_empty(), "nothing to tear down");
    }

    #[test]
    fn session_ended_does_not_double_close() {
        let start = t0();
        let (state, effects) = LinkState::Live.apply(LinkEvent::SessionEnded, start);
        assert_eq!(state, LinkState::Idle { retry_at: None });
        assert!(effects.is_empty(), "the session is already gone");
    }

    #[test]
    fn a_hung_attempt_is_given_up_on() {
        let start = t0();
        let (state, _) = LinkState::Dialing { since: start }
            .apply(LinkEvent::Tick, start + ATTEMPT_WINDOW);
        assert!(matches!(state, LinkState::GaveUp { .. }));

        let (state, effects) = LinkState::Authenticating { since: start }
            .apply(LinkEvent::Tick, start + ATTEMPT_WINDOW);
        assert!(matches!(state, LinkState::GaveUp { .. }));
        assert_eq!(
            effects,
            vec![LinkEffect::TearDownSession],
            "a hung auth holds a slot and must release it"
        );
    }

    #[test]
    fn a_session_that_never_proves_itself_releases_the_slot() {
        let start = t0();
        let (state, effects) =
            LinkState::Proving { since: start }.apply(LinkEvent::Tick, start + ATTEMPT_WINDOW);
        assert_eq!(state, LinkState::Idle { retry_at: None });
        assert_eq!(effects, vec![LinkEffect::TearDownSession]);
    }

    #[test]
    fn backoff_gates_dialling_but_expires() {
        let start = t0();
        let backing_off = LinkState::Idle {
            retry_at: Some(start + Duration::from_secs(10)),
        };
        assert!(!backing_off.may_dial(start));
        assert!(backing_off.may_dial(start + Duration::from_secs(10)));
        assert!(LinkState::new().may_dial(start));
        assert!(!LinkState::Live.may_dial(start), "already connected");
        assert!(
            !LinkState::GaveUp { since: start }.may_dial(start),
            "giving up must stop costing radio time"
        );
    }

    #[test]
    fn late_events_are_ignored_rather_than_fatal() {
        // Events arrive from concurrent components and can race a transition
        // that already moved past them.
        let start = t0();
        for state in [
            LinkState::Idle { retry_at: None },
            LinkState::GaveUp { since: start },
            LinkState::Unavailable {
                reason: TransportStatusCode::NetworkUnavailable,
            },
        ] {
            let (next, effects) = state.apply(LinkEvent::ProofLapsed, start);
            assert_eq!(next, state, "a stale proof event must not move a dead link");
            assert!(effects.is_empty());
        }
    }

    /// The invariant, over every reachable (state, event) pair: a teardown
    /// effect is emitted only when a session actually stops existing, and a
    /// session never stops existing silently.
    #[test]
    fn teardown_is_emitted_exactly_when_a_session_stops_existing() {
        let start = t0();
        let states = [
            LinkState::Unavailable {
                reason: TransportStatusCode::BluetoothDisabled,
            },
            LinkState::Idle { retry_at: None },
            LinkState::Dialing { since: start },
            LinkState::Authenticating { since: start },
            LinkState::Proving { since: start },
            LinkState::Live,
            LinkState::Fading { since: start },
            LinkState::GaveUp { since: start },
        ];
        let events = [
            LinkEvent::PreconditionsLost {
                reason: TransportStatusCode::BluetoothDisabled,
            },
            LinkEvent::PreconditionsMet,
            LinkEvent::DialStarted,
            LinkEvent::DialFailed { retry_at: None },
            LinkEvent::LinkEstablished,
            LinkEvent::AuthSucceeded,
            LinkEvent::AuthFailed { retry_at: None },
            LinkEvent::ProofComplete,
            LinkEvent::ProofLapsed,
            LinkEvent::SessionEnded,
            LinkEvent::UserRetryRequested,
            LinkEvent::Tick,
        ];

        // Far enough ahead that every deadline in the machine has expired, so
        // the timed edges are covered too rather than silently skipped.
        let now = start + ATTEMPT_WINDOW + FADE_GRACE;

        for state in &states {
            for event in &events {
                let (next, effects) = state.apply(event.clone(), now);
                let tore_down = effects.contains(&LinkEffect::TearDownSession);
                let lost_session = state.has_session() && !next.has_session();

                if tore_down {
                    assert!(
                        lost_session,
                        "tore down without leaving a session state: {state:?} + {event:?} -> {next:?}"
                    );
                }
                // The one legitimate silent loss is `SessionEnded`, which
                // reports a session that has *already* gone.
                if lost_session && !tore_down {
                    assert_eq!(
                        *event,
                        LinkEvent::SessionEnded,
                        "session vanished with no teardown: {state:?} + {event:?} -> {next:?}"
                    );
                }
            }
        }
    }
}
