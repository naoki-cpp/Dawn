//! Local Time Dilation controller (ADR-0018 / INV-TiDi).
//!
//! Time Dilation is the **last-resort** load response for a single dense
//! hotspot that cannot be split (after fission and LoD). It stretches *real
//! time only* — the wall-clock pace at which logical ticks are emitted — and
//! never touches logical-tick content: no event is reordered, dropped, or
//! changed (INV-005 is untouched; dilation is pure real-time pacing).
//!
//! # Invariants this type upholds (ADR-0018)
//!
//! - **Decision is deterministic** (not wall-clock): the dilation factor is a
//!   pure function of a *logical* cost estimate (`tick_cost`, e.g. entity /
//!   interaction count) versus a fixed `budget`. Physical time is never read
//!   here — that would break determinism (FBD-003). Measuring how long a tick
//!   *took* must not drive this.
//! - **Local**: one controller per Sector. It holds no global state, so one
//!   Sector dilating cannot affect another (INV-TiDi (a)).
//! - **Observable**: it records the current factor and how long dilation has
//!   been active, for SLA metrics (INV-TiDi (b)).
//! - **Auto-recovering**: when cost falls back within budget the factor returns
//!   to `1.0` (INV-TiDi (d)).
//! - **Bounded**: the factor never drops below [`MIN_DILATION`] — past that we
//!   fall through to the next backstop (admission control), not a freeze.

/// Lowest dilation factor: real time is stretched at most 10× (EVE-like floor).
/// Below this, admission control — not further slowdown — is the backstop.
pub const MIN_DILATION: f64 = 0.1;

/// Per-Sector Time Dilation controller (ADR-0018).
#[derive(Debug, Clone)]
pub struct DilationController {
    /// Max logical cost per tick before dilation engages.
    budget: f64,
    /// Current factor in `[MIN_DILATION, 1.0]`. `1.0` = real-time.
    dilation: f64,
    /// Consecutive ticks dilation has been active (`< 1.0`). For observability.
    active_ticks: u64,
}

impl DilationController {
    /// Create a controller that engages once a tick's cost exceeds `budget`.
    pub fn new(budget: f64) -> Self {
        assert!(budget > 0.0, "budget must be positive, got {budget}");
        Self { budget, dilation: 1.0, active_ticks: 0 }
    }

    /// Update from this tick's deterministic logical cost; return the dilation
    /// factor to pace the *next* real-time tick with.
    ///
    /// `tick_cost` must be a logical quantity (entity / interaction count), not a
    /// measured duration — see the module invariants.
    pub fn update(&mut self, tick_cost: f64) -> f64 {
        if tick_cost > self.budget {
            self.dilation = (self.budget / tick_cost).max(MIN_DILATION);
            self.active_ticks += 1;
        } else {
            self.dilation = 1.0;
            self.active_ticks = 0;
        }
        self.dilation
    }

    /// Current dilation factor (`1.0` = real-time).
    pub fn dilation(&self) -> f64 {
        self.dilation
    }

    /// Whether dilation is currently engaged.
    pub fn is_dilated(&self) -> bool {
        self.dilation < 1.0
    }

    /// Consecutive ticks dilation has been active (`0` when at real-time).
    pub fn active_ticks(&self) -> u64 {
        self.active_ticks
    }

    /// Real-time delay for one tick: the base tick duration stretched by the
    /// current factor (factor `0.5` → twice the wall-clock time). This is the
    /// *only* thing dilation changes — logical tick processing is untouched.
    pub fn paced_tick_ms(&self, base_ms: f64) -> f64 {
        base_ms / self.dilation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_at_real_time_while_cost_is_within_budget() {
        let mut c = DilationController::new(1_000.0);
        assert_eq!(c.update(500.0), 1.0);
        assert!(!c.is_dilated());
        assert_eq!(c.active_ticks(), 0);
    }

    #[test]
    fn engages_proportionally_when_cost_exceeds_budget() {
        let mut c = DilationController::new(1_000.0);
        // cost 2000 vs budget 1000 → factor 0.5 (half real-time pace).
        assert_eq!(c.update(2_000.0), 0.5);
        assert!(c.is_dilated());
    }

    #[test]
    fn the_factor_is_bounded_below_by_min_dilation() {
        let mut c = DilationController::new(1_000.0);
        // Extreme overload would imply 0.01; clamped to the 0.1 floor.
        assert_eq!(c.update(100_000.0), MIN_DILATION);
    }

    #[test]
    fn auto_recovers_to_real_time_when_load_drops() {
        let mut c = DilationController::new(1_000.0);
        c.update(4_000.0);
        assert!(c.is_dilated());
        // Load subsides → factor returns to 1.0 and the active counter resets.
        assert_eq!(c.update(800.0), 1.0);
        assert_eq!(c.active_ticks(), 0);
    }

    #[test]
    fn active_tick_count_tracks_sustained_dilation_for_observability() {
        let mut c = DilationController::new(1_000.0);
        c.update(2_000.0);
        c.update(2_000.0);
        c.update(2_000.0);
        assert_eq!(c.active_ticks(), 3);
    }

    #[test]
    fn dilation_in_one_sector_does_not_affect_another() {
        // Locality (INV-TiDi (a)): controllers share no state.
        let mut hot  = DilationController::new(1_000.0);
        let mut calm = DilationController::new(1_000.0);
        hot.update(5_000.0);
        calm.update(200.0);
        assert!(hot.is_dilated());
        assert!(!calm.is_dilated());
        assert_eq!(calm.dilation(), 1.0);
    }

    #[test]
    fn paced_tick_ms_stretches_real_time_by_the_factor_only() {
        let mut c = DilationController::new(1_000.0);
        c.update(2_000.0); // factor 0.5
        assert_eq!(c.paced_tick_ms(16.0), 32.0);
    }

    #[test]
    fn the_decision_is_deterministic_for_a_given_cost_sequence() {
        // Same logical costs → same factors, every run (no wall-clock input).
        let run = || {
            let mut c = DilationController::new(1_000.0);
            [c.update(500.0), c.update(2_000.0), c.update(900.0)]
        };
        assert_eq!(run(), run());
    }
}
