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
            // Use the f64 anchor source (ADR-0029), not the f32 `position`, so
            // anchors stay precise at true-AU distances.
            abs.insert(id, b.abs_m.into());
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
        Some([
            a[0] + offset.x as f64,
            a[1] + offset.y as f64,
            a[2] + offset.z as f64,
        ])
    }

    /// Re-express a Sector-frame absolute point relative to `anchor`, i.e. the
    /// inverse of [`Self::absolute`]: `abs - anchor_abs`, computed in f64 then
    /// cast to f32 once. Exact when `abs` is near `anchor` (ship-scale
    /// offsets); ADR-0029's precision guarantee does not extend to points far
    /// from `anchor`.
    pub fn to_relative(&self, anchor: AnchorId, abs: [f64; 3]) -> Option<Position> {
        let a = self.abs(anchor)?;
        Some(Position::new(
            (abs[0] - a[0]) as f32,
            (abs[1] - a[1]) as f32,
            (abs[2] - a[2]) as f32,
        ))
    }

    /// Re-express a ship currently anchored at `from` (with `offset`) relative
    /// to a new anchor `to`. The new offset is `(from_abs + offset) - to_abs`,
    /// computed in f64 then stored as f32 — exact when the ship is near `to`
    /// (ADR-0029 §2: rebase at warp arrival, where the new offset is small).
    pub fn rebase(&self, from: AnchorId, offset: Position, to: AnchorId) -> Option<Position> {
        self.to_relative(to, self.absolute(from, offset)?)
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
            assert!(
                t.abs(AnchorId::from(b.id)).is_some(),
                "missing anchor for {:?}",
                b.id
            );
        }
    }

    #[test]
    fn anchor_abs_comes_from_the_f64_source_not_the_f32_position() {
        // The anchor table must use CelestialBodyDef.abs_m (f64), which stays
        // precise where the f32 `position` would not (ADR-0029). Forge is
        // authored at 0.8 AU; its anchor x must match the f64 computation
        // (0.8 * UNITS_PER_AU) exactly, and equal abs_m, not the f32 position.
        let g = Galaxy::demo();
        let t = AnchorTable::from_galaxy(&g);
        let forge = g
            .bodies
            .iter()
            .find(|b| b.id == CelestialBodyId(1))
            .unwrap();
        let anchor_abs = t.abs(AnchorId(1)).unwrap();
        assert_eq!(
            anchor_abs,
            forge.abs_m.as_array(),
            "anchor must use the f64 abs_m source"
        );
    }

    #[test]
    fn star_is_the_sector_origin_anchor() {
        let t = table();
        // Helios (id 0) sits at [0,0,0] in Sector 0 — the origin anchor.
        assert_eq!(t.sector_origin_anchor(SectorId(0)), Some(AnchorId(0)));
    }

    #[test]
    fn to_relative_is_the_inverse_of_absolute() {
        let t = table();
        let forge = AnchorId(1);
        let offset = Position::new(2000.0, 0.0, -1500.0);
        let abs = t.absolute(forge, offset).unwrap();
        let recovered = t.to_relative(forge, abs).unwrap();
        assert!((recovered.x - offset.x).abs() < 1e-3);
        assert!((recovered.z - offset.z).abs() < 1e-3);
    }

    #[test]
    fn rebase_preserves_absolute_position() {
        let t = table();
        let star = AnchorId(0); // Helios at origin
        let planet = AnchorId(1); // Forge
                                  // Rebasing across a FAR anchor (star <-> Forge ~1 AU) is inherently
                                  // f32-lossy: the offset on the far side is ~10^11 m, whose f32 ulp is
                                  // ~16 km. This is exactly why anchors must stay local and real warps
                                  // rebase via the f64 arrival point (rebase_arrival_event), not via
                                  // AnchorTable::rebase. Here we only assert the loss is bounded by the
                                  // f32 ulp at that magnitude (ADR-0029).
        let offset = Position::new(1000.0, 0.0, 0.0);
        let world_before = t.absolute(star, offset).unwrap();
        let new_off = t.rebase(star, offset, planet).unwrap();
        let world_after = t.absolute(planet, new_off).unwrap();
        let err = sq_dist(world_before, world_after).sqrt();
        assert!(
            err < 30_000.0,
            "rebase error {err} m exceeds the f32 ulp bound at ~1 AU"
        );
    }

    #[test]
    fn absolute_compose_is_exact_for_a_near_anchor_offset() {
        // The precise, realistic operation: a ship near an anchor (small offset)
        // composes to an exact absolute position in f64, even though the anchor
        // itself is at true-AU distance (ADR-0029).
        let t = table();
        let forge_abs = t.abs(AnchorId(1)).unwrap();
        let off = Position::new(2000.0, 0.0, -1500.0);
        let abs = t.absolute(AnchorId(1), off).unwrap();
        assert!((abs[0] - (forge_abs[0] + 2000.0)).abs() < 1e-3);
        assert!((abs[2] - (forge_abs[2] - 1500.0)).abs() < 1e-3);
    }

    /// ADR-0029 / spike S1–S2 (now permanent): the whole point of method B is
    /// that two ships near two *different* anchors that sit at true astronomical
    /// distances still get a sub-millimetre-accurate separation, because the
    /// composition happens in f64. This is independent of the compressed/real
    /// galaxy scale toggle — it builds anchors at real AU directly — so it keeps
    /// guarding the precision guarantee even while the live galaxy runs
    /// compressed. A naive f32 distance at 1 AU loses ~16 km (one f32 ulp).
    #[test]
    fn cross_anchor_distance_is_sub_mm_accurate_at_true_au() {
        const AU_M: f64 = 1.495978707e11;
        // Two anchors ~4.2 AU apart (Earth-ish and Jupiter-ish along x), in two
        // sectors' worth of magnitude — well into f32's lossy regime.
        let mut abs = HashMap::new();
        abs.insert(AnchorId(10), [1.0 * AU_M, 0.0, 0.0]);
        abs.insert(AnchorId(11), [5.2 * AU_M, 3.0e10, -2.0e10]);
        let mut sector = HashMap::new();
        sector.insert(AnchorId(10), SectorId(0));
        sector.insert(AnchorId(11), SectorId(0));
        let t = AnchorTable { abs, sector };

        // Small, ship-scale offsets near each anchor (metres).
        let off_a = Position::new(1234.0, -567.0, 89.0);
        let off_b = Position::new(-4321.0, 765.0, -98.0);

        // Analytic separation from the same f64 components.
        let pa = t.absolute(AnchorId(10), off_a).unwrap();
        let pb = t.absolute(AnchorId(11), off_b).unwrap();
        let expected = sq_dist(pa, pb).sqrt();

        let d = t
            .distance((AnchorId(10), off_a), (AnchorId(11), off_b))
            .unwrap();
        assert!(
            (d - expected).abs() < 1e-3,
            "cross-anchor distance off by {} m at true AU",
            (d - expected).abs()
        );

        // And the offsets themselves survive the compose to sub-mm (the property
        // f32-at-absolute would destroy): recover off_a from the composed point.
        let recovered_x = pa[0] - 1.0 * AU_M;
        assert!(
            (recovered_x - off_a.x as f64).abs() < 1e-3,
            "near-anchor offset lost precision: {} vs {}",
            recovered_x,
            off_a.x
        );
    }

    #[test]
    fn distance_across_anchors_matches_body_separation() {
        let g = Galaxy::demo();
        let t = AnchorTable::from_galaxy(&g);
        let helios = g
            .bodies
            .iter()
            .find(|b| b.id == CelestialBodyId(0))
            .unwrap();
        let forge = g
            .bodies
            .iter()
            .find(|b| b.id == CelestialBodyId(1))
            .unwrap();
        // Expected from the f64 anchor source (positions are ~10^11 m, where the
        // f32 `position` distance would be ~km off).
        let d_abs = [
            helios.abs_m[0] - forge.abs_m[0],
            helios.abs_m[1] - forge.abs_m[1],
            helios.abs_m[2] - forge.abs_m[2],
        ];
        let expected = (d_abs[0] * d_abs[0] + d_abs[1] * d_abs[1] + d_abs[2] * d_abs[2]).sqrt();
        // Two ships at their anchors' origins (zero offset).
        let d = t
            .distance(
                (AnchorId(0), Position::ORIGIN),
                (AnchorId(1), Position::ORIGIN),
            )
            .unwrap();
        assert!(
            (d - expected).abs() < 1.0,
            "anchor distance {d} != body sep {expected}"
        );
    }
}
