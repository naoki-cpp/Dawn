//! Server-issued reconnect capability used by client admission.

use serde::{Deserialize, Serialize};

/// Opaque capability for resuming one server-authorized client handoff.
///
/// The bytes have no meaning to a client. The authoritative binding lives in
/// the Sector admission store, which associates the ticket with a Player,
/// Ship, destination Sector, and one-time admission state.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResumeTicket(pub [u8; 32]);

impl std::fmt::Debug for ResumeTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ResumeTicket(<redacted>)")
    }
}

impl ResumeTicket {
    pub const BYTE_LEN: usize = 32;

    pub const fn from_bytes(bytes: [u8; Self::BYTE_LEN]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> [u8; Self::BYTE_LEN] {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::ResumeTicket;

    #[test]
    fn debug_redacts_ticket_bytes() {
        let rendered = format!(
            "{:?}",
            ResumeTicket::from_bytes([7; ResumeTicket::BYTE_LEN])
        );

        assert_eq!(rendered, "ResumeTicket(<redacted>)");
    }
}
