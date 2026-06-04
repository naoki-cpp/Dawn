//! # dawn-ecs
//!
//! ECS (Entity-Component-System) layer for the dawn simulation.
//!
//! This crate wraps `hecs` and provides:
//! - Typed component bundles for Ship entities.
//! - A `SimWorld` that hides raw `hecs::World` internals.
//! - Systems as pure functions that take `&mut SimWorld` and return events.
//!
//! ## Design contract (ADR-0004)
//!
//! - Systems are pure functions: same inputs → same outputs.
//! - Systems never write to I/O.  They return `Vec<DomainEvent>` to the caller.
//! - `SimWorld` is the single mutable owner of all ECS state.

pub mod components;
pub mod systems;
pub mod world;

pub use world::SimWorld;
// Re-export so upper crates don't need a direct hecs dependency.
pub use hecs::Entity;
