use dawn_client_core::{ConnectionEffect, ConnectionLifecycle as CoreLifecycle, ConnectionPhase};
use godot::prelude::*;

/// Transport effects of a single Rust-owned lifecycle transition.
#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct ConnectionStep {
    effect: ConnectionEffect,
}

impl ConnectionStep {
    fn wrap(effect: ConnectionEffect) -> Option<Gd<Self>> {
        (effect != ConnectionEffect::None).then(|| Gd::from_object(Self { effect }))
    }
}

#[godot_api]
impl ConnectionStep {
    #[func]
    fn dispatch(&self, mut target: Gd<Node>) {
        match self.effect {
            ConnectionEffect::Connect { attempt, log } => {
                target.call("_connect_to_server", vslice![i64::from(attempt), log]);
            }
            ConnectionEffect::SendHello => {
                target.emit_signal("connection_changed", vslice![true]);
                target.call("_send_hello", &[]);
            }
            ConnectionEffect::Retrying { disconnected: true } => {
                target.emit_signal("connection_changed", vslice![false]);
            }
            ConnectionEffect::Stop { .. } => {
                target.call("_stop_transport", &[]);
                target.emit_signal("connection_changed", vslice![false]);
            }
            ConnectionEffect::Ready
            | ConnectionEffect::Retrying {
                disconnected: false,
            } => {}
            ConnectionEffect::None => return,
        }
        target.emit_signal("lifecycle_changed", &[]);
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
    fn start(&mut self) -> Option<Gd<ConnectionStep>> {
        ConnectionStep::wrap(self.core.start())
    }

    #[func]
    fn advance(&mut self, delta: f64) -> Option<Gd<ConnectionStep>> {
        ConnectionStep::wrap(self.core.advance(delta))
    }

    #[func]
    fn socket_opened(&mut self) -> Option<Gd<ConnectionStep>> {
        ConnectionStep::wrap(self.core.socket_opened())
    }

    #[func]
    fn socket_closed(&mut self) -> Option<Gd<ConnectionStep>> {
        ConnectionStep::wrap(self.core.socket_closed())
    }

    #[func]
    fn welcome_received(&mut self) -> Option<Gd<ConnectionStep>> {
        ConnectionStep::wrap(self.core.welcome_received())
    }

    #[func]
    fn redirect(&mut self) -> Option<Gd<ConnectionStep>> {
        ConnectionStep::wrap(self.core.redirect())
    }

    #[func]
    fn cancel(&mut self) -> Option<Gd<ConnectionStep>> {
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
