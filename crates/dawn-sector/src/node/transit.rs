//! Transit state mutation for `SimulationNode` (ADR-0014).
//!
//! The private child modules keep live lifecycle mutation, shared handoff
//! materialization, and public-event replay separate while preserving the
//! existing `SimulationNode` interface and Transit protocol semantics.

mod lifecycle;
mod materialization;
mod replay;

#[cfg(test)]
mod tests;
