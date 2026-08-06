use crate::item_identity_gd::ItemIdentity;
use dawn_core::{
    ApproachTarget, CelestialBodyId, ClientRequest, EntityId, ItemId, JumpGateId, ModuleId,
    Position, ShipId, ShipTypeId, SlotKind, StationId, TransferDirection, WarpTarget,
};
use dawn_wire::{ClientMessage, HelloMessage, ItemWire, MarketCommandWire, ResumeTicket};
use godot::prelude::*;

// Godot-facing result for Sector request construction:
// { ok: bool, bytes: PackedByteArray, error_code: String, error_message: String }.
type Dict = Dictionary<Variant, Variant>;

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

fn to_wire_bytes(message: &ClientMessage) -> Vec<u8> {
    message.encode()
}

#[derive(Debug, PartialEq, Eq)]
struct RequestBuildResult {
    ok: bool,
    bytes: Vec<u8>,
    error_code: String,
    error_message: String,
}

fn request_build_result(request: Result<ClientRequest, RequestBuildError>) -> RequestBuildResult {
    match request {
        Ok(request) => match request.validate() {
            Ok(()) => RequestBuildResult {
                ok: true,
                bytes: to_wire_bytes(&ClientMessage::Command(request)),
                error_code: String::new(),
                error_message: String::new(),
            },
            Err(error) => RequestBuildResult {
                ok: false,
                bytes: Vec::new(),
                error_code: "request_validation".to_owned(),
                error_message: error.to_string(),
            },
        },
        Err(error) => RequestBuildResult {
            ok: false,
            bytes: Vec::new(),
            error_code: error.code.to_owned(),
            error_message: error.message,
        },
    }
}

fn request_result(request: Result<ClientRequest, RequestBuildError>) -> Dict {
    let result = request_build_result(request);
    build_result(
        result.ok,
        PackedByteArray::from(result.bytes.as_slice()),
        &result.error_code,
        &result.error_message,
    )
}

fn build_result(ok: bool, bytes: PackedByteArray, error_code: &str, error_message: &str) -> Dict {
    let mut result = Dict::new();
    result.set("ok", ok);
    result.set("bytes", &bytes.to_variant());
    result.set("error_code", &GString::from(error_code).to_variant());
    result.set("error_message", &GString::from(error_message).to_variant());
    result
}

fn market_command_bytes(command: MarketCommandWire) -> PackedByteArray {
    PackedByteArray::from(to_wire_bytes(&ClientMessage::Market(command)).as_slice())
}

fn scalar_dict_to_json_object(fields: &Dict) -> Option<serde_json::Map<String, serde_json::Value>> {
    let mut map = serde_json::Map::with_capacity(fields.len());
    for (key, value) in fields.iter_shared() {
        let key: String = key.to::<GString>().to_string();
        let value = match value.get_type() {
            VariantType::INT => serde_json::Value::from(value.to::<i64>()),
            VariantType::FLOAT => serde_json::Value::from(value.to::<f64>()),
            VariantType::STRING | VariantType::STRING_NAME => {
                serde_json::Value::from(value.to::<GString>().to_string())
            }
            VariantType::BOOL => serde_json::Value::from(value.to::<bool>()),
            _ => {
                godot_error!(
                    "ClientCommand.market_build: field '{key}' has unsupported type {:?}",
                    value.get_type()
                );
                return None;
            }
        };
        map.insert(key, value);
    }
    Some(map)
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

fn slot_kind(value: &GString) -> Result<SlotKind, RequestBuildError> {
    match value.to_string().as_str() {
        "High" => Ok(SlotKind::High),
        "Mid" => Ok(SlotKind::Mid),
        "Low" => Ok(SlotKind::Low),
        "Rig" => Ok(SlotKind::Rig),
        other => Err(RequestBuildError::new(
            "invalid_slot_kind",
            format!("unknown slot kind '{other}'"),
        )),
    }
}

fn item_wire(item_id: &Gd<ItemIdentity>) -> ItemWire {
    item_id.bind().get().into()
}

/// Typed builder for client -> server postcard messages.
///
/// Sector methods construct [`ClientRequest`] directly and return a structured
/// build result. There is no JSON builder, mirrored wire enum, or empty-byte
/// sentinel for Sector requests. Market remains a separate envelope and
/// retains its schema-driven helper.
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct ClientCommand {}

#[godot_api]
impl ClientCommand {
    #[func]
    fn move_command(&self, x: f64, y: f64, z: f64) -> Dict {
        request_result(Ok(ClientRequest::Move {
            target: Position::new(x, y, z),
        }))
    }

    #[func]
    fn lock_on_command(&self, target_id: i64) -> Dict {
        request_result((|| {
            Ok(ClientRequest::LockOn {
                target: ship_id(target_id, "target_id")?,
            })
        })())
    }

    #[func]
    fn attack_command(&self, target_id: i64) -> Dict {
        request_result((|| {
            Ok(ClientRequest::Attack {
                target: ship_id(target_id, "target_id")?,
            })
        })())
    }

    #[func]
    fn activate_module_command(&self, module_id: i64, slot: GString, target_ship_id: i64) -> Dict {
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
    fn deactivate_module_command(&self, module_id: i64, slot: GString) -> Dict {
        request_result((|| {
            Ok(ClientRequest::DeactivateModule {
                module: ModuleId(nonzero_u32_id(module_id, "module_id")?),
                slot: slot_kind(&slot)?,
            })
        })())
    }

    #[func]
    fn stop_command(&self) -> Dict {
        request_result(Ok(ClientRequest::Stop))
    }

    #[func]
    fn jump_command(&self, gate_id: i64) -> Dict {
        request_result((|| {
            Ok(ClientRequest::Jump {
                gate: JumpGateId(u32_id(gate_id, "gate_id")?),
            })
        })())
    }

    #[func]
    fn approach_command(&self, target_id: i64) -> Dict {
        request_result((|| {
            Ok(ClientRequest::Approach {
                target: ApproachTarget::Ship(ship_id(target_id, "target_id")?),
            })
        })())
    }

    #[func]
    fn approach_gate_command(&self, gate_id: i64) -> Dict {
        request_result((|| {
            Ok(ClientRequest::Approach {
                target: ApproachTarget::Gate(JumpGateId(u32_id(gate_id, "gate_id")?)),
            })
        })())
    }

    #[func]
    fn warp_command(&self, gate_id: i64) -> Dict {
        request_result((|| {
            Ok(ClientRequest::Warp {
                target: WarpTarget::Gate(JumpGateId(u32_id(gate_id, "gate_id")?)),
            })
        })())
    }

    #[func]
    fn warp_to_body_command(&self, body_id: i64) -> Dict {
        request_result((|| {
            Ok(ClientRequest::Warp {
                target: WarpTarget::Body(CelestialBodyId(u32_id(body_id, "body_id")?)),
            })
        })())
    }

    #[func]
    fn orbit_command(&self, target_id: i64, radius: f64) -> Dict {
        request_result((|| {
            Ok(ClientRequest::Orbit {
                target: ApproachTarget::Ship(ship_id(target_id, "target_id")?),
                radius: optional_positive(radius, "radius")?,
            })
        })())
    }

    #[func]
    fn orbit_gate_command(&self, gate_id: i64, radius: f64) -> Dict {
        request_result((|| {
            Ok(ClientRequest::Orbit {
                target: ApproachTarget::Gate(JumpGateId(u32_id(gate_id, "gate_id")?)),
                radius: optional_positive(radius, "radius")?,
            })
        })())
    }

    #[func]
    fn keep_at_range_command(&self, target_id: i64, range: f64) -> Dict {
        request_result((|| {
            Ok(ClientRequest::KeepAtRange {
                target: ApproachTarget::Ship(ship_id(target_id, "target_id")?),
                range: optional_positive(range, "range")?,
            })
        })())
    }

    #[func]
    fn keep_at_range_gate_command(&self, gate_id: i64, range: f64) -> Dict {
        request_result((|| {
            Ok(ClientRequest::KeepAtRange {
                target: ApproachTarget::Gate(JumpGateId(u32_id(gate_id, "gate_id")?)),
                range: optional_positive(range, "range")?,
            })
        })())
    }

    #[func]
    fn fit_module_command(&self, ship: i64, module: i64, slot: GString) -> Dict {
        request_result((|| {
            Ok(ClientRequest::FitModule {
                ship: ship_id(ship, "ship_id")?,
                module: ModuleId(nonzero_u32_id(module, "module_id")?),
                slot: slot_kind(&slot)?,
            })
        })())
    }

    #[func]
    fn unfit_module_command(&self, ship: i64, module: i64, slot: GString) -> Dict {
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
    ) -> Dict {
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
    fn dock_command(&self, station: i64) -> Dict {
        request_result((|| {
            Ok(ClientRequest::Dock {
                station: StationId(u32_id(station, "station_id")?),
            })
        })())
    }

    #[func]
    fn undock_command(&self) -> Dict {
        request_result(Ok(ClientRequest::Undock))
    }

    #[func]
    fn build_packaged_ship_command(&self, ship: i64, station: i64, ship_type: i64) -> Dict {
        request_result((|| {
            Ok(ClientRequest::BuildPackagedShip {
                ship: ship_id(ship, "ship_id")?,
                station: StationId(u32_id(station, "station_id")?),
                ship_type: ShipTypeId(nonzero_u32_id(ship_type, "ship_type_id")?),
            })
        })())
    }

    #[func]
    fn disassemble_ship_command(&self, ship: i64, station: i64) -> Dict {
        request_result((|| {
            Ok(ClientRequest::DisassembleShip {
                ship: ship_id(ship, "ship_id")?,
                station: StationId(u32_id(station, "station_id")?),
            })
        })())
    }

    #[func]
    fn select_active_ship_command(&self, ship: i64) -> Dict {
        request_result((|| {
            Ok(ClientRequest::SelectActiveShip {
                ship: ship_id(ship, "ship_id")?,
            })
        })())
    }

    #[func]
    fn assemble_command(&self, station: i64, ship_type: i64) -> Dict {
        request_result((|| {
            Ok(ClientRequest::Assemble {
                station: StationId(u32_id(station, "station_id")?),
                ship_type: ShipTypeId(nonzero_u32_id(ship_type, "ship_type_id")?),
            })
        })())
    }

    #[func]
    fn disembark_command(&self) -> Dict {
        request_result(Ok(ClientRequest::Disembark))
    }

    #[func]
    fn transfer_to_station_command(
        &self,
        ship: i64,
        station: i64,
        item_id: Gd<ItemIdentity>,
    ) -> Dict {
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
    ) -> Dict {
        self.transfer_command(
            ship,
            station,
            item_id.bind().get(),
            TransferDirection::ToShip,
        )
    }

    /// Schema-driven builder for the Market-only request envelope. Sector
    /// requests intentionally have no equivalent JSON builder.
    #[func]
    fn market_build(&self, kind: GString, fields: Dict) -> PackedByteArray {
        let Some(fields) = scalar_dict_to_json_object(&fields) else {
            return PackedByteArray::new();
        };
        let mut wrapper = serde_json::Map::with_capacity(1);
        wrapper.insert(kind.to_string(), serde_json::Value::Object(fields));
        match serde_json::from_value::<MarketCommandWire>(serde_json::Value::Object(wrapper)) {
            Ok(command) => market_command_bytes(command),
            Err(error) => {
                godot_error!("ClientCommand.market_build({kind}): {error}");
                PackedByteArray::new()
            }
        }
    }

    #[func]
    fn market_place_order_command(
        &self,
        ship_id: i64,
        item_id: Gd<ItemIdentity>,
        side: GString,
        price: i64,
        quantity: i64,
    ) -> PackedByteArray {
        let (Ok(ship_id), Ok(price), Ok(quantity)) = (
            u64::try_from(ship_id),
            u64::try_from(price),
            u64::try_from(quantity),
        ) else {
            godot_error!(
                "ClientCommand.market_place_order_command: ship, price, and quantity must be non-negative"
            );
            return PackedByteArray::new();
        };
        market_command_bytes(MarketCommandWire::PlaceMarketOrderCommand {
            ship_id,
            item_id: item_wire(&item_id),
            side: side.to_string(),
            price,
            quantity,
        })
    }

    #[func]
    fn hello_command(&self, resume_ticket: PackedByteArray) -> PackedByteArray {
        let resume = match resume_ticket.to_vec().try_into() {
            Ok(bytes) => Some(ResumeTicket::from_bytes(bytes)),
            Err(bytes) if bytes.is_empty() => None,
            Err(_) => {
                godot_error!(
                    "ClientCommand.hello_command: resume ticket must be empty or 32 bytes"
                );
                return PackedByteArray::new();
            }
        };
        to_wire_bytes(&ClientMessage::Hello(HelloMessage { resume })).into()
    }
}

impl ClientCommand {
    fn transfer_command(
        &self,
        ship: i64,
        station: i64,
        item: ItemId,
        direction: TransferDirection,
    ) -> Dict {
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
        let result = request_build_result(
            ship_id(-1, "ship_id").map(|ship| ClientRequest::SelectActiveShip { ship }),
        );
        assert!(!result.ok);
        assert_eq!(result.error_code, "invalid_id");
        assert!(result.bytes.is_empty());
        assert!(!result.error_message.is_empty());
    }

    #[test]
    fn valid_sector_builder_returns_postcard_bytes() {
        let result = request_build_result(Ok(ClientRequest::Stop));
        assert!(result.ok);
        assert!(!result.bytes.is_empty());
        assert!(result.error_code.is_empty());
        assert!(result.error_message.is_empty());
    }
}
