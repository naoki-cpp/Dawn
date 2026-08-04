pub use dawn_core::ResumeTicket;
use serde::{Deserialize, Serialize};

/// The client's Hello message (ADR-0007 §2), carried by
/// `ClientMessage::Hello` in the binary envelope (ADR-0042).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloMessage {
    pub resume: Option<ResumeTicket>,
}
