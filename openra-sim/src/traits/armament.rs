//! Armament trait component — per-armament firing state.
//!
//! Phase-3 typed component carrying a weapon's stats (range, damage,
//! reload delay) and the actor's per-tick firing cooldown. The weapon
//! definition is `openra_data::rules::WeaponStats`, the typed view
//! Phase 4 surfaces over the parsed `weapons.yaml` ruleset; the
//! `Armament` instance owns a clone so it can be carried around the
//! actor's component graph without lifetime grief.
//!
//! Phase 6 adds `MultiArmament` for actors that mount more than one
//! weapon (e3 has primary RedEye + secondary Dragon, 4tnk hull-MG +
//! turret cannon). Existing single-weapon callers can keep using
//! `Armament` directly. `MultiArmament::select_for_target` returns
//! the first armament whose weapon range covers a Chebyshev-distance
//! target, mirroring C# `AttackBase.ChooseArmamentsForTarget`.
//!
//! Still out of scope: per-armor-class damage multipliers, burst
//! patterns and splash radii (deferred to Phase 8).

use openra_data::rules::WeaponStats;

/// One weapon mounted on an actor. Tracks the cooldown remaining
/// before the next shot.
#[derive(Debug, Clone)]
pub struct Armament {
    /// Weapon definition (range / damage / reload). Cloned from the
    /// ruleset so the component is `'static`.
    pub weapon: WeaponStats,
    /// Ticks remaining until the next shot. `0` means ready to fire.
    pub current_cooldown_ticks: u32,
}

impl Armament {
    /// Build a fresh armament with the cooldown at `0`.
    pub fn new(weapon: WeaponStats) -> Self {
        Armament { weapon, current_cooldown_ticks: 0 }
    }

    /// Decrement the cooldown by one tick (saturating at zero).
    /// Should be called once per game tick by the world loop.
    pub fn tick(&mut self) {
        if self.current_cooldown_ticks > 0 {
            self.current_cooldown_ticks -= 1;
        }
    }

    /// True when the armament is ready to fire this tick.
    pub fn is_ready(&self) -> bool {
        self.current_cooldown_ticks == 0
    }

    /// Mark the armament as having fired — resets the cooldown to
    /// `weapon.reload_delay` ticks. Negative reload delays clamp to
    /// zero (defensive against malformed YAML).
    pub fn mark_fired(&mut self) {
        self.current_cooldown_ticks = self.weapon.reload_delay.max(0) as u32;
    }

    /// True iff `chebyshev_cells` is within the weapon's range.
    /// We compare in cells so the activity layer never has to convert
    /// `WDist` itself.
    pub fn in_range(&self, chebyshev_cells: i32) -> bool {
        let weapon_range_cells = (self.weapon.range.length / 1024).max(0);
        chebyshev_cells <= weapon_range_cells
    }
}

/// Multi-weapon armament for vehicles with hull + turret guns, or for
/// e3 with primary + secondary anti-armor / anti-air rockets.
///
/// Each entry has an optional name (`Armament@PRIMARY`, `Armament@SECONDARY`,
/// `Armament@GARRISONED`, …) so callers can pick by tag. Most callers
/// will use `select_for_target` which picks the first in-range armament
/// in declaration order, matching C# `AttackBase.ChooseArmamentsForTarget`
/// for Phase 6's purposes (no `Versus`-driven priority yet).
#[derive(Debug, Clone, Default)]
pub struct MultiArmament {
    pub armaments: Vec<NamedArmament>,
}

/// One named entry in a `MultiArmament`. The `name` is the C# instance
/// suffix after `@` — `"primary"` / `"secondary"` / `"garrisoned"` /
/// `""` for the unnamed default. We lowercase on insertion for stable
/// lookups.
#[derive(Debug, Clone)]
pub struct NamedArmament {
    pub name: String,
    pub armament: Armament,
    /// True if this armament is mounted on the actor's turret (i.e. its
    /// muzzle facing is determined by the `Turret` component, not the
    /// chassis facing). Phase 6 reads this from the `WithSpriteTurret`
    /// + `Armament` association in YAML; for actors without a turret
    /// every entry is `false`.
    pub turret_mounted: bool,
}

impl MultiArmament {
    pub fn new() -> Self {
        MultiArmament { armaments: Vec::new() }
    }

    /// Add an armament. `name` is normalised to lowercase; pass `""`
    /// for an unnamed primary.
    pub fn push(&mut self, name: &str, armament: Armament, turret_mounted: bool) {
        self.armaments.push(NamedArmament {
            name: name.trim().to_ascii_lowercase(),
            armament,
            turret_mounted,
        });
    }

    /// True if the actor has any armament at all.
    pub fn is_empty(&self) -> bool {
        self.armaments.is_empty()
    }

    /// Tick every armament's cooldown by one game-tick.
    pub fn tick_all(&mut self) {
        for n in self.armaments.iter_mut() {
            n.armament.tick();
        }
    }

    /// Look up a named armament (case-insensitive). Empty string returns
    /// the first unnamed entry.
    pub fn get(&self, name: &str) -> Option<&NamedArmament> {
        let needle = name.trim().to_ascii_lowercase();
        self.armaments.iter().find(|n| n.name == needle)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut NamedArmament> {
        let needle = name.trim().to_ascii_lowercase();
        self.armaments.iter_mut().find(|n| n.name == needle)
    }

    /// Select an armament suitable for a target at `chebyshev_cells`
    /// distance. Returns the first one whose weapon range covers the
    /// target AND that is currently ready to fire. If nothing is ready
    /// but at least one weapon is in range, returns that one (so the
    /// caller can decide whether to wait). Returns `None` if the
    /// target is out of range of every armament.
    pub fn select_for_target(&self, chebyshev_cells: i32) -> Option<&NamedArmament> {
        // First pass: ready and in range.
        for n in &self.armaments {
            if n.armament.in_range(chebyshev_cells) && n.armament.is_ready() {
                return Some(n);
            }
        }
        // Second pass: in range but cooling down.
        for n in &self.armaments {
            if n.armament.in_range(chebyshev_cells) {
                return Some(n);
            }
        }
        None
    }

    /// Mutable variant of `select_for_target`.
    pub fn select_for_target_mut(
        &mut self,
        chebyshev_cells: i32,
    ) -> Option<&mut NamedArmament> {
        // We can't easily share the in-range filter with the immutable
        // path because of the borrow checker; keep it readable.
        let mut idx_ready: Option<usize> = None;
        let mut idx_inrange: Option<usize> = None;
        for (i, n) in self.armaments.iter().enumerate() {
            if n.armament.in_range(chebyshev_cells) {
                if idx_inrange.is_none() {
                    idx_inrange = Some(i);
                }
                if n.armament.is_ready() && idx_ready.is_none() {
                    idx_ready = Some(i);
                }
            }
        }
        let idx = idx_ready.or(idx_inrange)?;
        self.armaments.get_mut(idx)
    }

    /// Maximum range across all armaments (in cells). Returns 0 if
    /// the actor has no armaments.
    pub fn max_range_cells(&self) -> i32 {
        self.armaments
            .iter()
            .map(|n| (n.armament.weapon.range.length / 1024).max(0))
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openra_data::rules::WDist;

    fn m1carbine() -> WeaponStats {
        WeaponStats {
            name: "M1Carbine".into(),
            range: WDist::from_cells(5),
            reload_delay: 20,
            damage: 1000,
            ..Default::default()
        }
    }

    #[test]
    fn fresh_armament_is_ready() {
        let arm = Armament::new(m1carbine());
        assert!(arm.is_ready());
        assert_eq!(arm.current_cooldown_ticks, 0);
    }

    #[test]
    fn mark_fired_resets_cooldown_to_reload_delay() {
        let mut arm = Armament::new(m1carbine());
        arm.mark_fired();
        assert_eq!(arm.current_cooldown_ticks, 20);
        assert!(!arm.is_ready());
    }

    #[test]
    fn tick_decrements_cooldown() {
        let mut arm = Armament::new(m1carbine());
        arm.mark_fired();
        for _ in 0..20 {
            arm.tick();
        }
        assert!(arm.is_ready());
    }

    #[test]
    fn tick_saturates_at_zero() {
        let mut arm = Armament::new(m1carbine());
        // Tick from idle: must not underflow.
        for _ in 0..5 {
            arm.tick();
        }
        assert!(arm.is_ready());
    }

    #[test]
    fn negative_reload_delay_clamps_to_zero() {
        let mut bad = m1carbine();
        bad.reload_delay = -5;
        let mut arm = Armament::new(bad);
        arm.mark_fired();
        assert_eq!(arm.current_cooldown_ticks, 0);
    }

    fn dragon() -> WeaponStats {
        WeaponStats {
            name: "Dragon".into(),
            range: WDist::from_cells(8),
            reload_delay: 60,
            damage: 4500,
            ..Default::default()
        }
    }

    fn redeye() -> WeaponStats {
        WeaponStats {
            name: "RedEye".into(),
            range: WDist::from_cells(6),
            reload_delay: 45,
            damage: 2000,
            ..Default::default()
        }
    }

    #[test]
    fn multi_armament_select_in_range() {
        let mut multi = MultiArmament::new();
        multi.push("primary", Armament::new(redeye()), false);
        multi.push("secondary", Armament::new(dragon()), false);
        // Target at 4 cells: primary covers (range 6)
        let pick = multi.select_for_target(4).unwrap();
        assert_eq!(pick.name, "primary");
        // Target at 7 cells: secondary covers (range 8)
        let pick = multi.select_for_target(7).unwrap();
        assert_eq!(pick.name, "secondary");
        // Target at 10 cells: nothing covers
        assert!(multi.select_for_target(10).is_none());
    }

    #[test]
    fn multi_armament_prefers_ready_armament() {
        let mut multi = MultiArmament::new();
        // Both armaments cover cell-distance 4. Mark primary as
        // cooling-down; secondary should be picked.
        let mut a1 = Armament::new(redeye());
        a1.mark_fired();
        multi.push("primary", a1, false);
        multi.push("secondary", Armament::new(dragon()), false);
        let pick = multi.select_for_target(4).unwrap();
        assert_eq!(pick.name, "secondary");
    }

    #[test]
    fn multi_armament_falls_back_to_in_range_if_none_ready() {
        let mut multi = MultiArmament::new();
        let mut a1 = Armament::new(redeye());
        a1.mark_fired();
        multi.push("primary", a1, false);
        let pick = multi.select_for_target(4).unwrap();
        assert_eq!(pick.name, "primary");
        assert!(!pick.armament.is_ready());
    }

    #[test]
    fn multi_armament_tick_decrements_all() {
        let mut multi = MultiArmament::new();
        let mut a1 = Armament::new(redeye());
        a1.mark_fired();
        let mut a2 = Armament::new(dragon());
        a2.mark_fired();
        multi.push("p", a1, false);
        multi.push("s", a2, false);
        multi.tick_all();
        assert_eq!(multi.get("p").unwrap().armament.current_cooldown_ticks, 44);
        assert_eq!(multi.get("s").unwrap().armament.current_cooldown_ticks, 59);
    }

    #[test]
    fn multi_armament_max_range() {
        let mut multi = MultiArmament::new();
        multi.push("primary", Armament::new(redeye()), false);
        multi.push("secondary", Armament::new(dragon()), false);
        assert_eq!(multi.max_range_cells(), 8);
    }

    #[test]
    fn armament_in_range_check() {
        let arm = Armament::new(m1carbine());
        // M1Carbine range is 5 cells.
        assert!(arm.in_range(0));
        assert!(arm.in_range(5));
        assert!(!arm.in_range(6));
    }
}
