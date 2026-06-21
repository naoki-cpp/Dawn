//! Coordinate anchors — per-body local origins for ship positions (ADR-0029).
//!
//! A ship's authoritative position is `(AnchorId, f32 offset)`. The offset is a
//! small displacement (the ship stays near its anchor), so f32 stays precise
//! even when the anchor sits at a true astronomical distance from the Sector
//! origin. The anchor's *absolute* position (Sector-local frame, star at origin)
//! is held here as an `[f64; 3]` constant built from the static [`Galaxy`] map.
//!
//! This is the f64 in "method B": it is a static constant (identical on every
//! node, never recomputed), so it does not affect replay determinism — replay
//! still only recomputes the f32 offset integration (ADR-0029 §5, ADR-0028
//! Investigation B).

use crate::galaxy::Galaxy;
use dawn_core::{AnchorId, CelestialBodyId, Position, SectorId};
use std::collections::HashMap;

/// Maps each anchor (celestial body) to its absolute position in the
/// Sector-local frame, in metres, as `[f64; 3]`. Built once from the static
/// galaxy map; never persisted (snapshots store the `AnchorId`, and the table
/// is reconstructed deterministically from the galaxy data — ADR-0029 §3).
#[derive(Debug, Clone, Default)]
pub struct AnchorTable {
    /// Anchor absolute position (Sector-local, metres) keyed by anchor.
    abs: HashMap<AnchorId, [f64; 3]>,
    /// Which Sector each anchor belongs to (for nearest-anchor scoping).
    sector: HashMap<AnchorId, SectorId>,
}

impl AnchorTable {
    /// Build the anchor table from the galaxy's celestial bodies. Every body is
    /// an anchor (ADR-0029 §2 — anchors are per-body).
    pub fn from_galaxy(galaxy: &Galaxy) -> Self {
        let mut abs = HashMap::new();
        let mut sector = HashMap::new();
        for b in &galaxy.bodies {
            let id = AnchorId::from(b.id);
            abs.insert(id, [b.position.x as f64, b.position.y as f64, b.position.z as f64]);
            sector.insert(id, b.sector);
        }
        Self { abs, sector }
    }

    /// Absolute position (Sector-local, metres) of an anchor, or `None` if
    /// unknown.
    pub fn abs(&self, anchor: AnchorId) -> Option<[f64; 3]> {
        self.abs.get(&anchor).copied()
    }

    /// The full anchor → absolute-position map (Sector-local, metres). Passed to
    /// the Combat System so it can resolve ships' absolute positions across
    /// different anchors (ADR-0029 step 3).
    pub fn abs_map(&self) -> &HashMap<AnchorId, [f64; 3]> {
        &self.abs
    }

    /// Absolute position of a ship given its anchor and f32 offset.
    /// `anchor_abs + offset`, computed in f64.
    pub fn absolute(&self, anchor: AnchorId, offset: Position) -> Option<[f64; 3]> {
        let a = self.abs(anchor)?;
        Some([a[0] + offset.x as f64, a[1] + offset.y as f64, a[2] + offset.z as f64])
    }

    /// Re-express a ship currently anchored at `from` (with `offset`) relative
    /// to a new anchor `to`. The new offset is `(from_abs + offset) - to_abs`,
    /// computed in f64 then stored as f32 — exact when the ship is near `to`
    /// (ADR-0029 §2: rebase at warp arrival, where the new offset is small).
    pub fn rebase(&self, from: AnchorId, offset: Position, to: AnchorId) -> Option<Position> {
        let world = self.absolute(from, offset)?;
        let t = self.abs(to)?;
        Some(Position::new(
            (world[0] - t[0]) as f32,
            (world[1] - t[1]) as f32,
            (world[2] - t[2]) as f32,
        ))
    }

    /// True distance (metres) between two anchored positions, computed by
    /// composing each in f64 (no f32 ulp loss even across distant anchors —
    /// ADR-0028 spike B-3).
    pub fn distance(&self, a: (AnchorId, Position), b: (AnchorId, Position)) -> Option<f64> {
        let pa = self.absolute(a.0, a.1)?;
        let pb = self.absolute(b.0, b.1)?;
        let d = [pa[0] - pb[0], pa[1] - pb[1], pa[2] - pb[2]];
        Some((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt())
    }

    /// The anchor in `sector` nearest to the absolute point `world` (metres).
    /// Used to pick a ship's current anchor (e.g. on warp arrival).
    pub fn nearest_anchor(&self, sector: SectorId, world: [f64; 3]) -> Option<AnchorId> {
        self.abs
            .iter()
            .filter(|(id, _)| self.sector.get(id) == Some(&sector))
            .min_by(|(_, p), (_, q)| {
                let dp = sq_dist(**p, world);
                let dq = sq_dist(**q, world);
                dp.partial_cmp(&dq).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(id, _)| *id)
    }

    /// The default (Sector-origin) anchor: the body sitting closest to the
    /// origin in `sector` — the star. Ships start anchored here (ADR-0029 §4
    /// step 2: all ships initially anchor on the star, a semantic no-op).
    pub fn sector_origin_anchor(&self, sector: SectorId) -> Option<AnchorId> {
        self.nearest_anchor(sector, [0.0, 0.0, 0.0])
    }

    /// Anchor for a specific body (convenience over `AnchorId::from`).
    pub fn for_body(body: CelestialBodyId) -> AnchorId {
        AnchorId::from(body)
    }
}

fn sq_dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> AnchorTable {
        AnchorTable::from_galaxy(&Galaxy::demo())
    }

    #[test]
    fn every_body_becomes_an_anchor() {
        let g = Galaxy::demo();
        let t = AnchorTable::from_galaxy(&g);
        for b in &g.bodies {
            assert!(t.abs(AnchorId::from(b.id)).is_some(), "missing anchor for {:?}", b.id);
        }
    }

    #[test]
    fn star_is_the_sector_origin_anchor() {
        let t = table();
        // Helios (id 0) sits at [0,0,0] in Sector 0 — the origin anchor.
        assert_eq!(t.sector_origin_anchor(SectorId(0)), Some(AnchorId(0)));
    }

    #[test]
    fn rebase_preserves_absolute_position() {
        let t = table();
        let star = AnchorId(0); // Helios at origin
        let planet = AnchorId(1); // Forge
        // A ship 1000 m from the star, re-expressed relative to Forge, must
        // keep the same absolute position.
        let offset = Position::new(1000.0, 0.0, 0.0);
        let world_before = t.absolute(star, offset).unwrap();
        let new_off = t.rebase(star, offset, planet).unwrap();
        let world_after = t.absolute(planet, new_off).unwrap();
        let err = sq_dist(world_before, world_after).sqrt();
        assert!(err < 1.0, "rebase moved the ship by {err} m");
    }

    #[test]
    fn distance_across_anchors_matches_body_separation() {
        let g = Galaxy::demo();
        let t = AnchorTable::from_galaxy(&g);
        let helios = g.bodies.iter().find(|b| b.id == CelestialBodyId(0)).unwrap();
        let forge = g.bodies.iter().find(|b| b.id == CelestialBodyId(1)).unwrap();
        let expected = helios.position.distance(forge.position) as f64;
        // Two ships at their anchors' origins (zero offset).
        let d = t
            .distance((AnchorId(0), Position::ORIGIN), (AnchorId(1), Position::ORIGIN))
            .unwrap();
        assert!((d - expected).abs() < 1.0, "anchor distance {d} != body sep {expected}");
    }
}
