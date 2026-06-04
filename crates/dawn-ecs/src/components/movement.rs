//! Movement-related ECS components.

use dawn_core::{Position, Velocity};

/// Current world-space position of a Ship.
#[derive(Debug, Clone, Copy)]
pub struct PositionComp(pub Position);

/// Per-tick displacement vector.  The movement system applies this to
/// `PositionComp` every tick and performs wall-bounce on Sector boundaries.
#[derive(Debug, Clone, Copy)]
pub struct VelocityComp(pub Velocity);
