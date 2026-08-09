//! Client <-> server wire schema for Dawn (ADR-0041, ADR-0042).
//!
//! [`dawn_core::ClientRequest`] is the single schema-of-record for every
//! Sector request a client can send. The same typed value is constructed by
//! the Godot binding, postcard-encoded in [`ClientMessage`], decoded and
//! validated by the server, and admitted into family-local Sector policy.
//! Market requests remain a separate stream by design.
//!
//! ```
//! use dawn_core::{ClientRequest, Position};
//! use dawn_wire::ClientMessage;
//!
//! let message = ClientMessage::Command(ClientRequest::Move {
//!     target: Position::new(10.0, 0.0, -5.0),
//! });
//! let decoded = ClientMessage::decode(&message.encode()).unwrap();
//! assert!(matches!(decoded, ClientMessage::Command(ClientRequest::Move { .. })));
//! ```
//!
//! Sector commands are encoded in a versioned envelope. The legacy command
//! variant index is permanently reserved, so pre-#273 postcard payloads cannot
//! be reinterpreted as the new typed request catalog.

mod client_request;
mod hello_resume;
mod initial_state;
mod item;
mod market;
mod motion;
mod player_loadout;
mod server_fact;

pub use client_request::client_request_json_schema;
pub use dawn_core::ClientRequest;
pub use hello_resume::{HelloMessage, ResumeTicket};
pub use initial_state::{
    AbsPosWire, BuildableShipTypeWire, CelestialBodyWire, InitialStateWire, JumpGateWire,
    ShipStateWire, StationWire, SystemWire,
};
pub use item::{ItemWire, ItemWireError};
pub use market::{
    market_command_wire_json_schema, MarketCommandWire, MarketOrderWire, MarketSnapshotWire,
};
pub use motion::VelWire;
pub use player_loadout::{
    ItemRowWire, ModuleRowWire, OwnedShipRowWire, PlayerLoadoutWire, SlotCapacityWire,
};
pub use server_fact::{
    project_domain_event, server_fact_json_schema, ServerFact, ServerFactDeactivationReason,
    ServerFactRepairLayer, ServerFactSlot,
};

use dawn_core::ClientRequestValidationError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Stable machine-readable reason for rejecting a client request before
/// gameplay policy is evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ClientRequestRejectionCode {
    InvalidEncoding,
    UnsupportedProtocol,
    UnsupportedRequest,
    NonFinitePosition,
    InvalidOrbitRadius,
    InvalidKeepAtRange,
    ZeroModuleId,
    ZeroShipTypeId,
    NoActiveShip,
}

/// Structured rejection sent back to the client for decode, validation, or
/// application-admission failures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClientRequestRejectionWire {
    pub code: ClientRequestRejectionCode,
    pub message: String,
}

impl ClientRequestRejectionWire {
    pub fn invalid_encoding(error: impl std::fmt::Display) -> Self {
        Self {
            code: ClientRequestRejectionCode::InvalidEncoding,
            message: error.to_string(),
        }
    }

    pub fn unsupported_protocol(message: impl Into<String>) -> Self {
        Self {
            code: ClientRequestRejectionCode::UnsupportedProtocol,
            message: message.into(),
        }
    }

    pub fn unsupported_request(request: &str) -> Self {
        Self {
            code: ClientRequestRejectionCode::UnsupportedRequest,
            message: format!("{request} is not currently supported"),
        }
    }

    pub fn validation(error: ClientRequestValidationError) -> Self {
        let code = match error {
            ClientRequestValidationError::NonFinitePosition => {
                ClientRequestRejectionCode::NonFinitePosition
            }
            ClientRequestValidationError::InvalidOrbitRadius { .. } => {
                ClientRequestRejectionCode::InvalidOrbitRadius
            }
            ClientRequestValidationError::InvalidKeepAtRange { .. } => {
                ClientRequestRejectionCode::InvalidKeepAtRange
            }
            ClientRequestValidationError::ZeroModuleId { .. } => {
                ClientRequestRejectionCode::ZeroModuleId
            }
            ClientRequestValidationError::ZeroShipTypeId { .. } => {
                ClientRequestRejectionCode::ZeroShipTypeId
            }
        };
        Self {
            code,
            message: error.to_string(),
        }
    }

    pub fn no_active_ship() -> Self {
        Self {
            code: ClientRequestRejectionCode::NoActiveShip,
            message: "request requires an admitted active ship".to_owned(),
        }
    }
}

/// Every message the server sends over the binary WebSocket envelope.
#[derive(Debug, Serialize, Deserialize)]
pub enum ServerMessage {
    Welcome {
        player_id: u64,
        ship_id: u64,
        resume_ticket: ResumeTicket,
    },
    Redirect {
        ws_addr: String,
        resume_ticket: ResumeTicket,
    },
    /// A client-facing fact projected from committed Sector state.
    ///
    /// This is intentionally distinct from the durable `DomainEvent` catalog:
    /// internal recovery and bookkeeping facts never need a public-protocol
    /// placeholder variant.
    Fact(ServerFact),
    PlayerLoadout(PlayerLoadoutWire),
    InitialState(InitialStateWire),
    AoiEnter(ShipStateWire),
    AoiLeave {
        ship_id: u64,
    },
    PositionSnap {
        ship_id: u64,
        position: AbsPosWire,
    },
    MotionCorrection {
        ship_id: u64,
        position: AbsPosWire,
        velocity: VelWire,
        tick: u64,
    },
    MarketSnapshot(MarketSnapshotWire),
    ClientRequestRejected(ClientRequestRejectionWire),
}

impl ServerMessage {
    /// Encode one server message, preserving serialization failure at the
    /// transport boundary instead of turning it into an empty frame or panic.
    pub fn encode(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_stdvec(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }
}

/// Magic and version for the post-#273 typed Sector request envelope.
const CLIENT_REQUEST_PROTOCOL_MAGIC: u32 = 0x4441_574E; // "DAWN"
const CLIENT_REQUEST_PROTOCOL_VERSION: u16 = 1;

/// Versioned payload carried by the new Sector-command wire variant.
///
/// Keeping this framing separate from [`ClientRequest`] lets the request enum
/// remain the single intent catalog while giving postcard a hard compatibility
/// boundary. The legacy command occupied outer enum index 1; that index remains
/// reserved in the private wire enum below.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ClientRequestEnvelope {
    protocol_magic: u32,
    protocol_version: u16,
    request: ClientRequest,
}

impl ClientRequestEnvelope {
    fn validate(&self) -> Result<(), ClientMessageDecodeError> {
        if self.protocol_magic != CLIENT_REQUEST_PROTOCOL_MAGIC
            || self.protocol_version != CLIENT_REQUEST_PROTOCOL_VERSION
        {
            return Err(ClientMessageDecodeError::UnsupportedProtocol {
                magic: self.protocol_magic,
                version: self.protocol_version,
            });
        }
        self.request.validate()?;
        Ok(())
    }
}

#[derive(Serialize)]
struct ClientRequestEnvelopeRef<'a> {
    protocol_magic: u32,
    protocol_version: u16,
    request: &'a ClientRequest,
}

impl<'a> ClientRequestEnvelopeRef<'a> {
    fn new(request: &'a ClientRequest) -> Self {
        Self {
            protocol_magic: CLIENT_REQUEST_PROTOCOL_MAGIC,
            protocol_version: CLIENT_REQUEST_PROTOCOL_VERSION,
            request,
        }
    }
}

/// Public client-message API. Its Sector command variant carries the one typed
/// request authority; version framing is an implementation detail of encode/decode.
#[derive(Debug)]
pub enum ClientMessage {
    Hello(HelloMessage),
    Command(ClientRequest),
    Market(MarketCommandWire),
}

/// Actual postcard layout. Variant index 1 is deliberately never reused:
/// legacy `ClientCommandWire` messages used that index, while the new versioned
/// command is index 3.
#[derive(Serialize, Deserialize)]
enum ClientMessageWire {
    Hello(HelloMessage),
    #[allow(dead_code)]
    LegacyCommand,
    Market(MarketCommandWire),
    Command(ClientRequestEnvelope),
}

#[derive(Serialize)]
enum ClientMessageWireRef<'a> {
    Hello(&'a HelloMessage),
    #[allow(dead_code)]
    LegacyCommand,
    Market(&'a MarketCommandWire),
    Command(ClientRequestEnvelopeRef<'a>),
}

#[derive(Debug, thiserror::Error)]
pub enum ClientMessageDecodeError {
    #[error("invalid postcard client message: {0}")]
    Postcard(#[from] postcard::Error),
    #[error("legacy Sector command protocol is unsupported")]
    LegacyCommandProtocol,
    #[error("unsupported Sector command protocol magic=0x{magic:08X} version={version}")]
    UnsupportedProtocol { magic: u32, version: u16 },
    #[error("client message has {remaining} trailing byte(s)")]
    TrailingBytes { remaining: usize },
    #[error("invalid client request: {0}")]
    RequestValidation(#[from] ClientRequestValidationError),
}

impl ClientMessageDecodeError {
    pub fn rejection(&self) -> ClientRequestRejectionWire {
        match self {
            Self::Postcard(error) => ClientRequestRejectionWire::invalid_encoding(error),
            Self::LegacyCommandProtocol | Self::UnsupportedProtocol { .. } => {
                ClientRequestRejectionWire::unsupported_protocol(self.to_string())
            }
            Self::TrailingBytes { .. } => ClientRequestRejectionWire::invalid_encoding(self),
            Self::RequestValidation(error) => ClientRequestRejectionWire::validation(*error),
        }
    }
}

impl ClientMessage {
    pub fn encode(&self) -> Vec<u8> {
        let wire = match self {
            Self::Hello(hello) => ClientMessageWireRef::Hello(hello),
            Self::Command(request) => {
                ClientMessageWireRef::Command(ClientRequestEnvelopeRef::new(request))
            }
            Self::Market(command) => ClientMessageWireRef::Market(command),
        };
        postcard::to_stdvec(&wire).expect("typed wire message serialization")
    }

    /// Decode and validate an untrusted client frame before queueing it for
    /// application admission.
    pub fn decode(bytes: &[u8]) -> Result<Self, ClientMessageDecodeError> {
        let (wire, remaining) = postcard::take_from_bytes::<ClientMessageWire>(bytes)?;
        if matches!(&wire, ClientMessageWire::LegacyCommand) {
            return Err(ClientMessageDecodeError::LegacyCommandProtocol);
        }
        if !remaining.is_empty() {
            return Err(ClientMessageDecodeError::TrailingBytes {
                remaining: remaining.len(),
            });
        }

        match wire {
            ClientMessageWire::Hello(hello) => Ok(Self::Hello(hello)),
            ClientMessageWire::LegacyCommand => unreachable!("handled above"),
            ClientMessageWire::Market(command) => Ok(Self::Market(command)),
            ClientMessageWire::Command(envelope) => {
                envelope.validate()?;
                Ok(Self::Command(envelope.request))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{ApproachTarget, EntityId, ModuleId, NodeId, Position, ShipId, SlotKind};

    fn ship_id(counter: u64) -> ShipId {
        ShipId(EntityId::new(NodeId(0), counter))
    }

    fn roundtrip(message: &ClientMessage) -> ClientMessage {
        ClientMessage::decode(&message.encode()).expect("postcard ClientMessage round trip")
    }

    #[allow(dead_code)]
    #[derive(Serialize)]
    enum LegacyClientMessage {
        Hello(HelloMessage),
        Command(LegacyClientCommand),
        Market(MarketCommandWire),
    }

    #[derive(Serialize)]
    enum LegacyClientCommand {
        MoveCommand { target: LegacyPosition },
    }

    #[derive(Serialize)]
    struct LegacyPosition {
        x: f64,
        y: f64,
        z: f64,
    }

    #[test]
    fn legacy_command_variant_is_rejected_before_payload_reinterpretation() {
        let legacy = LegacyClientMessage::Command(LegacyClientCommand::MoveCommand {
            target: LegacyPosition {
                x: 10.0,
                y: 0.0,
                z: -5.0,
            },
        });
        let bytes = postcard::to_stdvec(&legacy).expect("encode legacy command");
        let error = ClientMessage::decode(&bytes).expect_err("legacy command must be rejected");

        assert!(matches!(
            &error,
            ClientMessageDecodeError::LegacyCommandProtocol
        ));
        assert_eq!(
            error.rejection().code,
            ClientRequestRejectionCode::UnsupportedProtocol
        );
    }

    #[test]
    fn wrong_command_envelope_version_is_rejected_structurally() {
        let wire = ClientMessageWire::Command(ClientRequestEnvelope {
            protocol_magic: CLIENT_REQUEST_PROTOCOL_MAGIC,
            protocol_version: CLIENT_REQUEST_PROTOCOL_VERSION + 1,
            request: ClientRequest::Stop,
        });
        let bytes = postcard::to_stdvec(&wire).expect("encode versioned command");
        let error = ClientMessage::decode(&bytes).expect_err("unknown version must be rejected");

        assert!(matches!(
            &error,
            ClientMessageDecodeError::UnsupportedProtocol { .. }
        ));
        assert_eq!(
            error.rejection().code,
            ClientRequestRejectionCode::UnsupportedProtocol
        );
    }

    #[test]
    fn motion_correction_round_trips_through_the_binary_envelope() {
        let message = ServerMessage::MotionCorrection {
            ship_id: 7,
            position: AbsPosWire {
                x: 1.0e12,
                y: -2.0,
                z: 3.5,
            },
            velocity: VelWire {
                dx: 4.0,
                dy: 5.0,
                dz: -6.0,
            },
            tick: 42,
        };
        let decoded =
            ServerMessage::decode(&message.encode().expect("valid postcard message encoding"))
                .expect("valid postcard message");
        assert!(matches!(
            decoded,
            ServerMessage::MotionCorrection {
                ship_id: 7,
                tick: 42,
                ..
            }
        ));
    }

    #[test]
    fn typed_client_request_round_trips_without_conversion_catalog() {
        let message = ClientMessage::Command(ClientRequest::ActivateModule {
            module: ModuleId(3),
            slot: SlotKind::High,
            target: Some(ship_id(9)),
        });
        assert!(matches!(
            roundtrip(&message),
            ClientMessage::Command(ClientRequest::ActivateModule {
                module: ModuleId(3),
                slot: SlotKind::High,
                target: Some(target),
            }) if target == ship_id(9)
        ));
    }

    #[test]
    fn navigation_target_round_trips_as_domain_typed_value() {
        let message = ClientMessage::Command(ClientRequest::Approach {
            target: ApproachTarget::Ship(ship_id(7)),
        });
        assert!(matches!(
            roundtrip(&message),
            ClientMessage::Command(ClientRequest::Approach {
                target: ApproachTarget::Ship(target),
            }) if target == ship_id(7)
        ));
    }

    #[test]
    fn non_finite_request_is_rejected_during_decode() {
        let message = ClientMessage::Command(ClientRequest::Move {
            target: Position::new(f64::NAN, 0.0, 0.0),
        });
        let error = ClientMessage::decode(&message.encode()).expect_err("NaN must be rejected");
        assert!(matches!(
            error,
            ClientMessageDecodeError::RequestValidation(
                ClientRequestValidationError::NonFinitePosition
            )
        ));
        assert_eq!(
            error.rejection().code,
            ClientRequestRejectionCode::NonFinitePosition
        );
    }

    #[test]
    fn market_command_preserves_typed_item_identity() {
        let message = ClientMessage::Market(MarketCommandWire::PlaceMarketOrderCommand {
            ship_id: 42,
            item_id: ItemWire::Module { module_id: 5 },
            side: "Ask".to_owned(),
            price: 100,
            quantity: 3,
        });
        assert!(matches!(
            roundtrip(&message),
            ClientMessage::Market(MarketCommandWire::PlaceMarketOrderCommand {
                ship_id: 42,
                item_id: ItemWire::Module { module_id: 5 },
                side,
                price: 100,
                quantity: 3,
            }) if side == "Ask"
        ));
    }

    #[test]
    fn hello_preserves_resume_ticket() {
        let ticket = ResumeTicket::from_bytes([7; ResumeTicket::BYTE_LEN]);
        let message = ClientMessage::Hello(HelloMessage {
            resume: Some(ticket),
        });
        assert!(matches!(
            roundtrip(&message),
            ClientMessage::Hello(HelloMessage { resume: Some(decoded) }) if decoded == ticket
        ));
    }

    #[test]
    fn wire_schema_doc_is_up_to_date() {
        assert_schema_file_matches(
            &server_fact_json_schema(),
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../docs/architecture/wire-protocol.schema.json"
            ),
        );
        assert_schema_file_matches(
            &client_request_json_schema(),
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../docs/architecture/wire-protocol-commands.schema.json"
            ),
        );
        assert_schema_file_matches(
            &market_command_wire_json_schema(),
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../docs/architecture/wire-protocol-market.schema.json"
            ),
        );
    }

    fn assert_schema_file_matches(schema: &schemars::Schema, path: &str) {
        let current = serde_json::to_string_pretty(schema).unwrap() + "\n";
        let checked_in = std::fs::read_to_string(path)
            .unwrap_or_else(|_| panic!("{path} must exist"))
            .replace("\r\n", "\n");
        assert_eq!(
            current, checked_in,
            "{path} is stale -- regenerate with `cargo run -p dawn-actor --example gen_wire_schema`"
        );
    }
}
