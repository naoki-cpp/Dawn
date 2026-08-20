use crate::item_identity_gd::ItemIdentity;
use dawn_core::{
    ApproachTarget, CelestialBodyId, ClientRequest, EntityId, ItemId, JumpGateId, ModuleId,
    Position, ShipId, ShipTypeId, SlotKind, StationId, TransferDirection, WarpTarget,
};
use dawn_protocol::{
    ClientMessage, HelloMessage, ItemWire, MarketCommandWire, MarketOrderSide, ResumeTicket,
};
use godot::prelude::*;

#[derive(Debug)]
struct RequestBuildError {
    code: &'static str,
    message: String,
}

impl RequestBuildError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Typed result for every client message builder. Construction and encoding
/// failures remain observable without using an empty byte array sentinel.
#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct ClientCommandResult {
    #[var]
    ok: bool,
    #[var]
    bytes: PackedByteArray,
    #[var]
    error_code: GString,
    #[var]
    error_message: GString,
}

impl ClientCommandResult {
    fn success(bytes: Vec<u8>) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            ok: true,
            bytes: PackedByteArray::from(bytes.as_slice()),
            error_code: GString::new(),
            error_message: GString::new(),
        })
    }

    pub(crate) fn failure(code: &str, message: impl Into<String>) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            ok: false,
            bytes: PackedByteArray::new(),
            error_code: GString::from(code),
            error_message: GString::from(message.into().as_str()),
        })
    }
}

#[godot_api]
impl ClientCommandResult {}

fn encode_message(message: ClientMessage) -> Result<Vec<u8>, RequestBuildError> {
    message
        .encode()
        .map_err(|error| RequestBuildError::new("encode_failed", error.to_string()))
}

fn request_build_bytes(
    request: Result<ClientRequest, RequestBuildError>,
) -> Result<Vec<u8>, RequestBuildError> {
    match request {
        Ok(request) => request
            .validate()
            .map_err(|error| RequestBuildError::new("request_validation", error.to_string()))
            .and_then(|()| encode_message(ClientMessage::Command(request))),
        Err(error) => Err(error),
    }
}

fn request_result(request: Result<ClientRequest, RequestBuildError>) -> Gd<ClientCommandResult> {
    match request_build_bytes(request) {
        Ok(bytes) => ClientCommandResult::success(bytes),
        Err(error) => ClientCommandResult::failure(error.code, error.message),
    }
}

pub(crate) fn request_result_from_request(request: ClientRequest) -> Gd<ClientCommandResult> {
    request_result(Ok(request))
}

fn market_result(command: Result<MarketCommandWire, RequestBuildError>) -> Gd<ClientCommandResult> {
    match command.and_then(|command| encode_message(ClientMessage::Market(command))) {
        Ok(bytes) => ClientCommandResult::success(bytes),
        Err(error) => ClientCommandResult::failure(error.code, error.message),
    }
}

fn optional_positive(value: f64, field: &str) -> Result<Option<f64>, RequestBuildError> {
    if !value.is_finite() {
        Err(RequestBuildError::new(
            "non_finite_number",
            format!("{field} must be finite"),
        ))
    } else if value > 0.0 {
        Ok(Some(value))
    } else {
        Ok(None)
    }
}

fn ship_id(value: i64, field: &str) -> Result<ShipId, RequestBuildError> {
    u64::try_from(value)
        .map(|value| ShipId(EntityId::from_raw(value)))
        .map_err(|_| RequestBuildError::new("invalid_id", format!("{field} must be non-negative")))
}

fn u32_id(value: i64, field: &str) -> Result<u32, RequestBuildError> {
    u32::try_from(value)
        .map_err(|_| RequestBuildError::new("invalid_id", format!("{field} must fit u32")))
}

fn nonzero_u32_id(value: i64, field: &str) -> Result<u32, RequestBuildError> {
    let value = u32_id(value, field)?;
    if value == 0 {
        Err(RequestBuildError::new(
            "zero_id",
            format!("{field} must be non-zero"),
        ))
    } else {
        Ok(value)
    }
}

pub(crate) fn slot_kind_from_str(value: &str) -> Option<SlotKind> {
    match value {
        "High" => Some(SlotKind::High),
        "Mid" => Some(SlotKind::Mid),
        "Low" => Some(SlotKind::Low),
        "Rig" => Some(SlotKind::Rig),
        _ => None,
    }
}

fn slot_kind(value: &GString) -> Result<SlotKind, RequestBuildError> {
    let value = value.to_string();
    slot_kind_from_str(&value).ok_or_else(|| {
        RequestBuildError::new("invalid_slot_kind", format!("unknown slot kind '{value}'"))
    })
}

fn market_side(value: &GString) -> Result<MarketOrderSide, RequestBuildError> {
    match value.to_string().as_str() {
        "Bid" => Ok(MarketOrderSide::Bid),
        "Ask" => Ok(MarketOrderSide::Ask),
        other => Err(RequestBuildError::new(
            "invalid_market_side",
            format!("unknown market side '{other}'"),
        )),
    }
}

fn item_wire(item_id: &Gd<ItemIdentity>) -> ItemWire {
    item_id.bind().get().into()
}

/// Typed builder for client -> server postcard messages.
///
/// Sector and Market methods construct their protocol enums directly and
/// return a typed build result. There is no JSON reconstruction path or
/// empty-byte sentinel.
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct ClientCommand {}

#[godot_api]
impl ClientCommand {
    #[func]
    fn move_command(&self, x: f64, y: f64, z: f64) -> Gd<ClientCommandResult> {
        request_result(Ok(ClientRequest::Move {
            target: Position::new(x, y, z),
        }))
    }

    #[func]
    fn lock_on_command(&self, target_id: i64) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::LockOn {
                target: ship_id(target_id, "target_id")?,
            })
        })())
    }

    #[func]
    fn attack_command(&self, target_id: i64) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::Attack {
                target: ship_id(target_id, "target_id")?,
            })
        })())
    }

    #[func]
    fn activate_module_command(
        &self,
        module_id: i64,
        slot: GString,
        target_ship_id: i64,
    ) -> Gd<ClientCommandResult> {
        request_result((|| {
            let target = if target_ship_id < 0 {
                None
            } else {
                Some(ship_id(target_ship_id, "target_ship_id")?)
            };
            Ok(ClientRequest::ActivateModule {
                module: ModuleId(nonzero_u32_id(module_id, "module_id")?),
                slot: slot_kind(&slot)?,
                target,
            })
        })())
    }

    #[func]
    fn deactivate_module_command(&self, module_id: i64, slot: GString) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::DeactivateModule {
                module: ModuleId(nonzero_u32_id(module_id, "module_id")?),
                slot: slot_kind(&slot)?,
            })
        })())
    }

    #[func]
    fn stop_command(&self) -> Gd<ClientCommandResult> {
        request_result(Ok(ClientRequest::Stop))
    }

    #[func]
    fn jump_command(&self, gate_id: i64) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::Jump {
                gate: JumpGateId(u32_id(gate_id, "gate_id")?),
            })
        })())
    }

    #[func]
    fn approach_command(&self, target_id: i64) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::Approach {
                target: ApproachTarget::Ship(ship_id(target_id, "target_id")?),
            })
        })())
    }

    #[func]
    fn approach_gate_command(&self, gate_id: i64) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::Approach {
                target: ApproachTarget::Gate(JumpGateId(u32_id(gate_id, "gate_id")?)),
            })
        })())
    }

    #[func]
    fn warp_command(&self, gate_id: i64) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::Warp {
                target: WarpTarget::Gate(JumpGateId(u32_id(gate_id, "gate_id")?)),
            })
        })())
    }

    #[func]
    fn warp_to_body_command(&self, body_id: i64) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::Warp {
                target: WarpTarget::Body(CelestialBodyId(u32_id(body_id, "body_id")?)),
            })
        })())
    }

    #[func]
    fn warp_to_station_command(&self, station_id: i64) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::Warp {
                target: WarpTarget::Station(StationId(u32_id(station_id, "station_id")?)),
            })
        })())
    }

    #[func]
    fn orbit_command(&self, target_id: i64, radius: f64) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::Orbit {
                target: ApproachTarget::Ship(ship_id(target_id, "target_id")?),
                radius: optional_positive(radius, "radius")?,
            })
        })())
    }

    #[func]
    fn orbit_gate_command(&self, gate_id: i64, radius: f64) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::Orbit {
                target: ApproachTarget::Gate(JumpGateId(u32_id(gate_id, "gate_id")?)),
                radius: optional_positive(radius, "radius")?,
            })
        })())
    }

    #[func]
    fn keep_at_range_command(&self, target_id: i64, range: f64) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::KeepAtRange {
                target: ApproachTarget::Ship(ship_id(target_id, "target_id")?),
                range: optional_positive(range, "range")?,
            })
        })())
    }

    #[func]
    fn keep_at_range_gate_command(&self, gate_id: i64, range: f64) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::KeepAtRange {
                target: ApproachTarget::Gate(JumpGateId(u32_id(gate_id, "gate_id")?)),
                range: optional_positive(range, "range")?,
            })
        })())
    }

    #[func]
    fn fit_module_command(&self, ship: i64, module: i64, slot: GString) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::FitModule {
                ship: ship_id(ship, "ship_id")?,
                module: ModuleId(nonzero_u32_id(module, "module_id")?),
                slot: slot_kind(&slot)?,
            })
        })())
    }

    #[func]
    fn unfit_module_command(
        &self,
        ship: i64,
        module: i64,
        slot: GString,
    ) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::UnfitModule {
                ship: ship_id(ship, "ship_id")?,
                module: ModuleId(nonzero_u32_id(module, "module_id")?),
                slot: slot_kind(&slot)?,
            })
        })())
    }

    #[func]
    fn reorder_fitted_module_command(
        &self,
        ship: i64,
        slot: GString,
        from_index: i64,
        to_index: i64,
    ) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::ReorderFittedModule {
                ship: ship_id(ship, "ship_id")?,
                slot: slot_kind(&slot)?,
                from_index: u32_id(from_index, "from_index")?,
                to_index: u32_id(to_index, "to_index")?,
            })
        })())
    }

    #[func]
    fn dock_command(&self, station: i64) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::Dock {
                station: StationId(u32_id(station, "station_id")?),
            })
        })())
    }

    #[func]
    fn undock_command(&self) -> Gd<ClientCommandResult> {
        request_result(Ok(ClientRequest::Undock))
    }

    #[func]
    fn build_packaged_ship_command(
        &self,
        ship: i64,
        station: i64,
        ship_type: i64,
    ) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::BuildPackagedShip {
                ship: ship_id(ship, "ship_id")?,
                station: StationId(u32_id(station, "station_id")?),
                ship_type: ShipTypeId(nonzero_u32_id(ship_type, "ship_type_id")?),
            })
        })())
    }

    #[func]
    fn disassemble_ship_command(&self, ship: i64, station: i64) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::DisassembleShip {
                ship: ship_id(ship, "ship_id")?,
                station: StationId(u32_id(station, "station_id")?),
            })
        })())
    }

    #[func]
    fn select_active_ship_command(&self, ship: i64) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::SelectActiveShip {
                ship: ship_id(ship, "ship_id")?,
            })
        })())
    }

    #[func]
    fn assemble_command(&self, station: i64, ship_type: i64) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::Assemble {
                station: StationId(u32_id(station, "station_id")?),
                ship_type: ShipTypeId(nonzero_u32_id(ship_type, "ship_type_id")?),
            })
        })())
    }

    #[func]
    fn disembark_command(&self) -> Gd<ClientCommandResult> {
        request_result(Ok(ClientRequest::Disembark))
    }

    #[func]
    fn transfer_to_station_command(
        &self,
        ship: i64,
        station: i64,
        item_id: Gd<ItemIdentity>,
    ) -> Gd<ClientCommandResult> {
        self.transfer_command(
            ship,
            station,
            item_id.bind().get(),
            TransferDirection::ToStation,
        )
    }

    #[func]
    fn transfer_from_station_command(
        &self,
        ship: i64,
        station: i64,
        item_id: Gd<ItemIdentity>,
    ) -> Gd<ClientCommandResult> {
        self.transfer_command(
            ship,
            station,
            item_id.bind().get(),
            TransferDirection::ToShip,
        )
    }

    #[func]
    fn market_refresh_command(&self) -> Gd<ClientCommandResult> {
        market_result(Ok(MarketCommandWire::RefreshMarketCommand {}))
    }

    #[func]
    fn market_place_order_command(
        &self,
        ship_id: i64,
        item_id: Gd<ItemIdentity>,
        side: GString,
        price: i64,
        quantity: i64,
    ) -> Gd<ClientCommandResult> {
        market_result((|| {
            Ok(MarketCommandWire::PlaceMarketOrderCommand {
                ship_id: u64::try_from(ship_id).map_err(|_| {
                    RequestBuildError::new("invalid_id", "ship_id must be non-negative")
                })?,
                item_id: item_wire(&item_id),
                side: market_side(&side)?,
                price: u64::try_from(price).map_err(|_| {
                    RequestBuildError::new("invalid_amount", "price must be non-negative")
                })?,
                quantity: u64::try_from(quantity).map_err(|_| {
                    RequestBuildError::new("invalid_amount", "quantity must be non-negative")
                })?,
            })
        })())
    }

    #[func]
    fn market_cancel_order_command(&self, order_id: i64) -> Gd<ClientCommandResult> {
        market_result((|| {
            Ok(MarketCommandWire::CancelMarketOrderCommand {
                order_id: u64::try_from(order_id).map_err(|_| {
                    RequestBuildError::new("invalid_id", "order_id must be non-negative")
                })?,
            })
        })())
    }

    #[func]
    fn hello_command(&self, resume_ticket: PackedByteArray) -> Gd<ClientCommandResult> {
        let resume = match resume_ticket.to_vec().try_into() {
            Ok(bytes) => Some(ResumeTicket::from_bytes(bytes)),
            Err(bytes) if bytes.is_empty() => None,
            Err(_) => {
                return ClientCommandResult::failure(
                    "invalid_resume_ticket",
                    "resume ticket must be empty or 32 bytes",
                );
            }
        };
        match encode_message(ClientMessage::Hello(HelloMessage { resume })) {
            Ok(bytes) => ClientCommandResult::success(bytes),
            Err(error) => ClientCommandResult::failure(error.code, error.message),
        }
    }
}

impl ClientCommand {
    fn transfer_command(
        &self,
        ship: i64,
        station: i64,
        item: ItemId,
        direction: TransferDirection,
    ) -> Gd<ClientCommandResult> {
        request_result((|| {
            Ok(ClientRequest::TransferCargo {
                ship: ship_id(ship, "ship_id")?,
                station: StationId(u32_id(station, "station_id")?),
                item,
                direction,
            })
        })())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_sector_builder_returns_structured_error_not_empty_bytes() {
        let result = request_build_bytes(
            ship_id(-1, "ship_id").map(|ship| ClientRequest::SelectActiveShip { ship }),
        );
        let error = result.expect_err("negative IDs must be rejected");
        assert_eq!(error.code, "invalid_id");
        assert!(error.message.contains("ship_id"));
    }

    #[test]
    fn valid_sector_builder_returns_postcard_bytes() {
        let bytes = request_build_bytes(Ok(ClientRequest::Stop)).expect("valid request");
        assert!(!bytes.is_empty());
    }
}
