//! Server-issued reconnect capability used by client admission.

use serde::{Deserialize, Serialize};

/// Opaque capability for resuming one server-authorized client handoff.
///
/// The bytes have no meaning to a client. The authoritative binding lives in
/// the Sector admission store, which associates the ticket with a Player,
/// Ship, destination Sector, and one-time admission state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResumeTicket(pub [u8; 32]);

impl ResumeTicket {
    pub const BYTE_LEN: usize = 32;

    pub const fn from_bytes(bytes: [u8; Self::BYTE_LEN]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> [u8; Self::BYTE_LEN] {
        self.0
    }
}
