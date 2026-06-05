//! Ship identity component.
//!
//! Stores the domain `ShipId` alongside the hecs `Entity` handle so that
//! systems can produce `DomainEvent`s with correct identifiers without
//! maintaining a separate lookup table.

use dawn_core::ShipId;

/// ECS component that binds the hecs `Entity` to its domain `ShipId`.
///
/// Every spawned Ship entity must carry exactly one `ShipIdComp`.
#[derive(Debug, Clone, Copy)]
pub struct ShipIdComp(pub ShipId);

/// マーカーコンポーネント: NPC 船であることを示す。
///
/// NPC はロックオンを自動で行う。
/// プレイヤー船にはこのコンポーネントを付けない（手動 LockOnCommand のみ）。
#[derive(Debug, Clone, Copy)]
pub struct IsNpcComp;
