//! Transit state mutation for `SimulationNode` (ADR-0014).
//!
//! The private child modules keep live lifecycle mutation and handoff
//! materialization separate while preserving the existing `SimulationNode`
//! interface and Transit protocol semantics.

mod lifecycle;
mod materialization;

#[cfg(test)]
mod tests;
