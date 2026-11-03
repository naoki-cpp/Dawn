//! Connection policy without sockets, credentials, engine objects, or clocks.
//! Callers advance elapsed time and execute the returned transport effects.
//!
//! ```
//! use dawn_client_core::ConnectionLifecycle;
//! let mut connection = ConnectionLifecycle::default();
//! connection.start();
//! connection.socket_opened();
//! assert!(connection.blocks_gameplay());
//! connection.welcome_received();
//! assert!(!connection.blocks_gameplay());
//! ```

const ATTEMPT_TIMEOUT: f64 = 90.0;
const RETRY_INTERVAL: f64 = 2.0;
const LOG_INTERVAL: f64 = 30.0;

/// Observable lifecycle shared by transport and presentation adapters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConnectionPhase {
    #[default]
    Stopped,
    Connecting,
    AwaitingWelcome,
    Connected,
    Reconnecting,
    TimedOut,
}

/// One transition's effects. Credential values stay with the socket adapter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConnectionEffect {
    #[default]
    None,
    Connect {
        attempt: u32,
        log: bool,
    },
    SendHello,
    Retrying {
        disconnected: bool,
    },
    Ready,
    Stop {
        timed_out: bool,
    },
}

/// Owns retry timing, Welcome gating and each connection attempt's deadline.
#[derive(Debug, Default)]
pub struct ConnectionLifecycle {
    phase: ConnectionPhase,
    waiting_elapsed: f64,
    retry_elapsed: f64,
    log_elapsed: f64,
    attempts: u32,
}

impl ConnectionLifecycle {
    #[must_use]
    pub const fn phase(&self) -> ConnectionPhase {
        self.phase
    }

    #[must_use]
    pub const fn is_connected(&self) -> bool {
        matches!(self.phase, ConnectionPhase::Connected)
    }

    #[must_use]
    pub const fn is_running(&self) -> bool {
        !matches!(
            self.phase,
            ConnectionPhase::Stopped | ConnectionPhase::TimedOut
        )
    }

    #[must_use]
    pub const fn can_edit(&self) -> bool {
        !self.is_running()
    }

    #[must_use]
    pub const fn can_cancel(&self) -> bool {
        self.is_running() && !self.is_connected()
    }

    #[must_use]
    pub const fn blocks_gameplay(&self) -> bool {
        !self.is_connected()
    }

    /// Begin a fresh attempt only while stopped. Concurrent starts are ignored.
    pub fn start(&mut self) -> ConnectionEffect {
        if self.is_running() {
            return ConnectionEffect::None;
        }
        self.waiting_elapsed = 0.0;
        self.retry_elapsed = 0.0;
        self.log_elapsed = LOG_INTERVAL;
        self.attempts = 0;
        self.connect()
    }

    /// Ignore invalid elapsed time; socket opens never reset the deadline.
    pub fn advance(&mut self, delta: f64) -> ConnectionEffect {
        if !self.is_running() || !delta.is_finite() || delta < 0.0 {
            return ConnectionEffect::None;
        }
        if self.is_connected() {
            return ConnectionEffect::None;
        }
        self.waiting_elapsed += delta;
        if self.waiting_elapsed >= ATTEMPT_TIMEOUT {
            self.phase = ConnectionPhase::TimedOut;
            return ConnectionEffect::Stop { timed_out: true };
        }
        self.log_elapsed = (self.log_elapsed + delta).min(LOG_INTERVAL);
        if self.phase == ConnectionPhase::Reconnecting {
            self.retry_elapsed += delta;
            if self.retry_elapsed >= RETRY_INTERVAL {
                return self.connect();
            }
        }
        ConnectionEffect::None
    }

    pub fn socket_opened(&mut self) -> ConnectionEffect {
        if self.phase != ConnectionPhase::Connecting {
            return ConnectionEffect::None;
        }
        self.phase = ConnectionPhase::AwaitingWelcome;
        ConnectionEffect::SendHello
    }

    pub fn socket_closed(&mut self) -> ConnectionEffect {
        let disconnected = match self.phase {
            ConnectionPhase::Connected => {
                self.waiting_elapsed = 0.0;
                self.attempts = 0;
                true
            }
            ConnectionPhase::AwaitingWelcome => true,
            ConnectionPhase::Connecting => false,
            _ => return ConnectionEffect::None,
        };
        self.phase = ConnectionPhase::Reconnecting;
        self.retry_elapsed = 0.0;
        ConnectionEffect::Retrying { disconnected }
    }

    /// A socket open is insufficient: only Welcome enables gameplay.
    pub fn welcome_received(&mut self) -> ConnectionEffect {
        if self.phase != ConnectionPhase::AwaitingWelcome {
            return ConnectionEffect::None;
        }
        self.phase = ConnectionPhase::Connected;
        self.waiting_elapsed = 0.0;
        ConnectionEffect::Ready
    }

    /// Redirect may arrive instead of Welcome during an admission handshake.
    pub fn redirect(&mut self) -> ConnectionEffect {
        if !matches!(
            self.phase,
            ConnectionPhase::Connected | ConnectionPhase::AwaitingWelcome
        ) {
            return ConnectionEffect::None;
        }
        self.phase = ConnectionPhase::Reconnecting;
        self.waiting_elapsed = 0.0;
        self.retry_elapsed = RETRY_INTERVAL;
        self.attempts = 0;
        ConnectionEffect::Retrying { disconnected: true }
    }

    pub fn cancel(&mut self) -> ConnectionEffect {
        self.phase = ConnectionPhase::Stopped;
        ConnectionEffect::Stop { timed_out: false }
    }

    fn connect(&mut self) -> ConnectionEffect {
        self.phase = ConnectionPhase::Connecting;
        self.retry_elapsed = 0.0;
        self.attempts = self.attempts.saturating_add(1);
        let log = self.attempts == 1 || self.log_elapsed >= LOG_INTERVAL;
        if log {
            self.log_elapsed = 0.0;
        }
        ConnectionEffect::Connect {
            attempt: self.attempts,
            log,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected() -> ConnectionLifecycle {
        let mut state = ConnectionLifecycle::default();
        state.start();
        assert_eq!(state.socket_opened(), ConnectionEffect::SendHello);
        assert_eq!(state.welcome_received(), ConnectionEffect::Ready);
        state
    }

    #[test]
    fn only_welcome_enables_gameplay_and_duplicate_observations_do_nothing() {
        let mut state = ConnectionLifecycle::default();
        assert!(!state.is_running());
        assert_eq!(state.welcome_received(), ConnectionEffect::None);
        assert_eq!(
            state.start(),
            ConnectionEffect::Connect {
                attempt: 1,
                log: true
            }
        );
        assert_eq!(state.start(), ConnectionEffect::None);
        assert_eq!(state.advance(30.0), ConnectionEffect::None);
        assert_eq!(state.socket_opened(), ConnectionEffect::SendHello);
        assert!(!state.is_connected());
        assert_eq!(state.socket_opened(), ConnectionEffect::None);
        assert_eq!(state.welcome_received(), ConnectionEffect::Ready);
        assert!(state.is_connected());
        assert_eq!(state.advance(1000.0), ConnectionEffect::None);
        assert_eq!(state.welcome_received(), ConnectionEffect::None);
    }

    #[test]
    fn connecting_and_opening_without_welcome_share_one_deadline() {
        let mut state = ConnectionLifecycle::default();
        state.start();
        state.advance(89.0);
        state.socket_opened();
        assert_eq!(
            state.advance(1.0),
            ConnectionEffect::Stop { timed_out: true }
        );
        assert_eq!(state.phase(), ConnectionPhase::TimedOut);
        assert!(!state.is_running());
        assert_eq!(state.advance(100.0), ConnectionEffect::None);
        assert_eq!(state.welcome_received(), ConnectionEffect::None);
    }

    #[test]
    fn repeated_open_close_failures_cannot_extend_the_deadline() {
        let mut state = ConnectionLifecycle::default();
        state.start();
        for _ in 0..44 {
            state.socket_opened();
            state.socket_closed();
            state.advance(2.0);
        }
        assert_eq!(
            state.advance(2.0),
            ConnectionEffect::Stop { timed_out: true }
        );
    }

    #[test]
    fn disconnect_gets_a_fresh_budget_without_clearing_resume_credentials() {
        let mut state = connected();
        state.advance(1000.0);
        assert_eq!(
            state.socket_closed(),
            ConnectionEffect::Retrying { disconnected: true }
        );
        assert_eq!(state.advance(1.0), ConnectionEffect::None);
        assert_eq!(
            state.advance(1.0),
            ConnectionEffect::Connect {
                attempt: 1,
                log: true
            }
        );
        state.socket_opened();
        assert_eq!(
            state.advance(88.0),
            ConnectionEffect::Stop { timed_out: true }
        );
    }

    #[test]
    fn redirect_before_or_after_welcome_starts_a_new_budget() {
        for welcomed in [false, true] {
            let mut state = ConnectionLifecycle::default();
            state.start();
            state.advance(80.0);
            state.socket_opened();
            if welcomed {
                state.welcome_received();
            }
            assert_eq!(
                state.redirect(),
                ConnectionEffect::Retrying { disconnected: true }
            );
            assert_eq!(
                state.advance(0.0),
                ConnectionEffect::Connect {
                    attempt: 1,
                    log: true
                }
            );
            state.socket_opened();
            assert_eq!(state.advance(89.0), ConnectionEffect::None);
            assert_eq!(
                state.advance(1.0),
                ConnectionEffect::Stop { timed_out: true }
            );
        }
    }

    #[test]
    fn retries_wait_two_seconds_and_logs_wait_thirty_seconds() {
        let mut state = ConnectionLifecycle::default();
        state.start();
        for attempt in 2..=16 {
            state.socket_closed();
            assert_eq!(state.advance(1.0), ConnectionEffect::None);
            assert_eq!(
                state.advance(1.0),
                ConnectionEffect::Connect {
                    attempt,
                    log: attempt == 16
                }
            );
        }
    }

    #[test]
    fn cancel_blocks_late_observations_and_allows_a_new_start() {
        let mut state = connected();
        assert_eq!(state.cancel(), ConnectionEffect::Stop { timed_out: false });
        assert!(!state.is_running());
        assert_eq!(state.socket_closed(), ConnectionEffect::None);
        assert_eq!(state.socket_opened(), ConnectionEffect::None);
        assert_eq!(state.welcome_received(), ConnectionEffect::None);
        assert_eq!(state.redirect(), ConnectionEffect::None);
        assert_eq!(state.advance(100.0), ConnectionEffect::None);
        assert_eq!(
            state.start(),
            ConnectionEffect::Connect {
                attempt: 1,
                log: true
            }
        );
    }

    #[test]
    fn invalid_time_cannot_change_the_lifecycle() {
        let mut state = ConnectionLifecycle::default();
        state.start();
        for delta in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            assert_eq!(state.advance(delta), ConnectionEffect::None);
        }
        assert_eq!(state.phase(), ConnectionPhase::Connecting);
        assert_eq!(
            state.advance(90.0),
            ConnectionEffect::Stop { timed_out: true }
        );
    }

    #[test]
    fn presentation_permissions_come_from_the_same_phase_as_transport() {
        let mut state = ConnectionLifecycle::default();
        assert!(state.can_edit() && state.blocks_gameplay() && !state.can_cancel());
        state.start();
        assert!(!state.can_edit() && state.blocks_gameplay() && state.can_cancel());
        state.socket_opened();
        assert!(!state.can_edit() && state.blocks_gameplay() && state.can_cancel());
        state.welcome_received();
        assert!(!state.can_edit() && !state.blocks_gameplay() && !state.can_cancel());
        state.socket_closed();
        assert!(!state.can_edit() && state.blocks_gameplay() && state.can_cancel());
        state.advance(90.0);
        assert!(state.can_edit() && state.blocks_gameplay() && !state.can_cancel());
    }
}
