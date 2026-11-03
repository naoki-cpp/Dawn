use dawn_client_core::{ConnectionEffect, ConnectionLifecycle as CoreLifecycle, ConnectionPhase};
use godot::prelude::*;

/// Transport effects of a single Rust-owned lifecycle transition.
#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct ConnectionStep {
    effect: ConnectionEffect,
}

impl ConnectionStep {
    fn wrap(effect: ConnectionEffect) -> Gd<Self> {
        Gd::from_object(Self { effect })
    }
}

#[godot_api]
impl ConnectionStep {
    #[func]
    fn is_empty(&self) -> bool {
        self.effect == ConnectionEffect::None
    }

    #[func]
    fn should_connect(&self) -> bool {
        matches!(self.effect, ConnectionEffect::Connect { .. })
    }

    #[func]
    fn should_send_hello(&self) -> bool {
        self.effect == ConnectionEffect::SendHello
    }

    #[func]
    fn should_reset_session(&self) -> bool {
        matches!(
            self.effect,
            ConnectionEffect::Retrying { disconnected: true } | ConnectionEffect::Stop { .. }
        )
    }

    #[func]
    fn should_clear_credentials(&self) -> bool {
        matches!(self.effect, ConnectionEffect::Stop { .. })
    }

    #[func]
    fn should_log_attempt(&self) -> bool {
        matches!(self.effect, ConnectionEffect::Connect { log: true, .. })
    }

    #[func]
    fn attempt(&self) -> i64 {
        match self.effect {
            ConnectionEffect::Connect { attempt, .. } => i64::from(attempt),
            _ => 0,
        }
    }
}

/// Thin adapter; socket operations and credential bytes stay in GDScript.
#[derive(Debug, GodotClass)]
#[class(init, base=RefCounted)]
pub struct ClientConnectionLifecycle {
    core: CoreLifecycle,
}

#[godot_api]
impl ClientConnectionLifecycle {
    #[func]
    fn start(&mut self) -> Gd<ConnectionStep> {
        ConnectionStep::wrap(self.core.start())
    }

    #[func]
    fn advance(&mut self, delta: f64) -> Gd<ConnectionStep> {
        ConnectionStep::wrap(self.core.advance(delta))
    }

    #[func]
    fn socket_opened(&mut self) -> Gd<ConnectionStep> {
        ConnectionStep::wrap(self.core.socket_opened())
    }

    #[func]
    fn socket_closed(&mut self) -> Gd<ConnectionStep> {
        ConnectionStep::wrap(self.core.socket_closed())
    }

    #[func]
    fn welcome_received(&mut self) -> Gd<ConnectionStep> {
        ConnectionStep::wrap(self.core.welcome_received())
    }

    #[func]
    fn redirect(&mut self) -> Gd<ConnectionStep> {
        ConnectionStep::wrap(self.core.redirect())
    }

    #[func]
    fn cancel(&mut self) -> Gd<ConnectionStep> {
        ConnectionStep::wrap(self.core.cancel())
    }

    #[func]
    fn is_running(&self) -> bool {
        self.core.is_running()
    }

    #[func]
    fn is_connected(&self) -> bool {
        self.core.is_connected()
    }

    #[func]
    fn blocks_gameplay(&self) -> bool {
        self.core.blocks_gameplay()
    }

    #[func]
    fn can_edit(&self) -> bool {
        self.core.can_edit()
    }

    #[func]
    fn can_cancel(&self) -> bool {
        self.core.can_cancel()
    }

    #[func]
    fn status_text(&self) -> GString {
        match self.core.phase() {
            ConnectionPhase::Stopped => "Not connected.",
            ConnectionPhase::Connecting => "Connecting...",
            ConnectionPhase::AwaitingWelcome => "Waiting for Welcome...",
            ConnectionPhase::Connected => "",
            ConnectionPhase::Reconnecting => "Reconnecting...",
            ConnectionPhase::TimedOut => "Connection timed out. Check the server and invite code.",
        }
        .into()
    }
}
