use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Velocity components carried by server-authoritative wire messages.
#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema, Clone, Copy)]
pub struct VelWire {
    pub dx: f64,
    pub dy: f64,
    pub dz: f64,
}

impl From<dawn_core::Velocity> for VelWire {
    fn from(value: dawn_core::Velocity) -> Self {
        Self {
            dx: value.dx,
            dy: value.dy,
            dz: value.dz,
        }
    }
}
