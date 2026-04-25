//! In-flight projectile entity — Phase-8 typed component.
//!
//! Models the rocket / missile travel phase between firing and impact.
//! Per-tick `advance` adds the projectile's velocity vector to its
//! position; once the remaining distance to the target drops below
//! `velocity.length()` the projectile is considered to have impacted
//! and the world dispatches a damage warhead at the impact point.
//!
//! Splash damage uses the sorted-distance rule for determinism: when
//! impact occurs, every actor whose center lies within `splash_radius`
//! of the impact point takes damage. The list of victims is sorted by
//! `(distance_to_impact, actor_id)` so two actors at the same distance
//! always get debited in id order.
//!
//! Versus damage multipliers (`Versus: { Heavy: 80, Light: 100, ... }`)
//! are applied per-victim using the victim's `armor_class` field. The
//! formula is:
//!
//! ```text
//! dealt = base_damage × versus[victim.armor_class] / 100
//! ```
//!
//! When the victim has no entry in the weapon's `versus` map, the
//! multiplier defaults to 100 (no modifier). When the victim's
//! `armor_class` is empty, "none" is assumed.
//!
//! Determinism notes
//! -----------------
//! Projectiles live in a `BTreeMap<u32, Projectile>` keyed by a
//! monotonically-increasing id, so iteration order is fixed. Velocities
//! are integer fixed-point (`WVec`) — no floating-point math anywhere
//! in the per-tick advance. The "did we impact?" check uses integer
//! Euclidean distance compared against integer speed.

use crate::math::{WPos, WVec};

/// One in-flight projectile from a fired armament.
#[derive(Debug, Clone)]
pub struct Projectile {
    /// Stable id (assigned at spawn time, monotonic).
    pub id: u32,
    /// Player that fired this projectile (for kill credit).
    pub attacker_id: u32,
    /// Target actor — used for "lock-on" steering and to check death
    /// without re-resolving by position.
    pub target_id: u32,
    /// Current world position.
    pub position: WPos,
    /// Most-recent target world position (refreshed every tick the
    /// target is alive, so the missile chases a moving unit).
    pub target_position: WPos,
    /// Speed in world units per tick (positive scalar).
    pub speed: i32,
    /// Base damage to apply at impact (before versus multiplier).
    pub damage: i32,
    /// Splash radius in world units. Zero = single-target hit.
    pub splash_radius: i32,
    /// Per-armor-class damage multipliers (percent). Lower-cased keys.
    /// Pulled from the weapon's `versus` table. Empty map = always 100%.
    pub versus: std::collections::BTreeMap<String, i32>,
}

impl Projectile {
    /// Create a fresh in-flight projectile aimed at `target_position`
    /// from `origin`. Speed is clamped to a minimum of 1 so a
    /// degenerate weapon (Speed: 0 mis-tagged as Missile) still
    /// progresses each tick rather than freezing.
    pub fn new(
        id: u32,
        attacker_id: u32,
        target_id: u32,
        origin: WPos,
        target_position: WPos,
        speed: i32,
        damage: i32,
        splash_radius: i32,
        versus: std::collections::BTreeMap<String, i32>,
    ) -> Self {
        Projectile {
            id,
            attacker_id,
            target_id,
            position: origin,
            target_position,
            speed: speed.max(1),
            damage,
            splash_radius: splash_radius.max(0),
            versus,
        }
    }

    /// Vector from current position to target.
    pub fn to_target(&self) -> WVec {
        self.target_position - self.position
    }

    /// Step the projectile one tick toward `target_position`. Returns
    /// `true` if the projectile has reached (or overshot) the target
    /// and should be dispatched for impact this tick. Pure integer
    /// math; no FP.
    ///
    /// `target_position` may be passed as the latest cached target
    /// location so a moving target stays "locked-on".
    pub fn advance(&mut self, target_position: WPos) -> bool {
        self.target_position = target_position;
        let delta = self.to_target();
        let dist_sq = delta.horizontal_length_squared();
        // Compare squared distance to squared step to avoid sqrt.
        let speed_sq = (self.speed as i64) * (self.speed as i64);
        if dist_sq <= speed_sq {
            // Snap to target — impact this tick.
            self.position = target_position;
            return true;
        }
        // Advance by `speed` units along the direction vector. We use
        // integer scaling: step = delta * speed / dist. dist is
        // approximated via the integer sqrt of dist_sq; on x86_64 and
        // aarch64 `i64::isqrt` is bit-deterministic.
        let dist = isqrt_i64(dist_sq).max(1);
        let nx = (delta.x as i64) * (self.speed as i64) / dist;
        let ny = (delta.y as i64) * (self.speed as i64) / dist;
        self.position = WPos::new(
            self.position.x.saturating_add(nx as i32),
            self.position.y.saturating_add(ny as i32),
            self.position.z,
        );
        false
    }
}

/// Integer square root for a non-negative `i64`. Newton-Raphson on
/// `u64`, then cast back. Rust 1.84 stabilised `i64::isqrt` but we
/// keep a portable shim here to avoid the MSRV bump.
fn isqrt_i64(n: i64) -> i64 {
    if n < 2 {
        return n.max(0);
    }
    let n_u = n as u64;
    // Initial guess: 2^(bit_count/2)
    let mut x = 1u64 << ((64 - n_u.leading_zeros()) / 2 + 1);
    loop {
        let y = (x + n_u / x) / 2;
        if y >= x {
            return x as i64;
        }
        x = y;
    }
}

/// Apply the `Versus` multiplier to a base damage value for a
/// specific armor class.
///
/// Formula: `damage_dealt = base × versus[armor_class] / 100`
///
/// If `armor_class` is empty, treat as `"none"`. If no entry exists
/// (neither for the specific class nor `"none"`), no modifier is
/// applied (multiplier = 100). Negative results are clamped to 0 to
/// keep the rest of the engine's HP arithmetic non-negative.
pub fn apply_versus(base_damage: i32, armor_class: &str, versus: &std::collections::BTreeMap<String, i32>) -> i32 {
    if base_damage == 0 {
        return 0;
    }
    let key = if armor_class.trim().is_empty() {
        "none".to_string()
    } else {
        armor_class.trim().to_ascii_lowercase()
    };
    let pct = match versus.get(&key) {
        Some(p) => *p,
        // Some weapons (M1Carbine via ^LightMG) only define `Versus:
        // None: ...` so the lookup for "heavy" misses. The C# behaviour
        // is to apply 100% (no modifier) when the class is unspecified.
        None => 100,
    };
    let scaled = (base_damage as i64) * (pct as i64) / 100;
    scaled.max(0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn isqrt_known_values() {
        assert_eq!(isqrt_i64(0), 0);
        assert_eq!(isqrt_i64(1), 1);
        assert_eq!(isqrt_i64(4), 2);
        assert_eq!(isqrt_i64(15), 3);
        assert_eq!(isqrt_i64(16), 4);
        assert_eq!(isqrt_i64(1_000_000), 1000);
    }

    #[test]
    fn projectile_advances_toward_target() {
        // Origin at (0,0), target at (10000, 0), speed 200 → first tick
        // should advance 200 units east.
        let p0 = WPos::new(0, 0, 0);
        let target = WPos::new(10_000, 0, 0);
        let mut proj = Projectile::new(1, 100, 200, p0, target, 200, 4500, 128, BTreeMap::new());
        assert!(!proj.advance(target));
        assert_eq!(proj.position.x, 200);
        assert_eq!(proj.position.y, 0);
    }

    #[test]
    fn projectile_impacts_when_within_one_step() {
        // Distance < speed → impact in one tick.
        let p0 = WPos::new(0, 0, 0);
        let target = WPos::new(50, 0, 0);
        let mut proj = Projectile::new(1, 100, 200, p0, target, 200, 4500, 128, BTreeMap::new());
        assert!(proj.advance(target));
        assert_eq!(proj.position, target);
    }

    #[test]
    fn projectile_diagonal_travel() {
        // Equal-axis target → projectile moves diagonally with integer
        // approximation.
        let p0 = WPos::new(0, 0, 0);
        let target = WPos::new(10_000, 10_000, 0);
        let mut proj = Projectile::new(1, 100, 200, p0, target, 200, 4500, 0, BTreeMap::new());
        assert!(!proj.advance(target));
        // Approx 200/sqrt(2) ≈ 141 in each axis.
        assert!(proj.position.x > 130 && proj.position.x < 150);
        assert!(proj.position.y > 130 && proj.position.y < 150);
    }

    #[test]
    fn versus_heavy_scales_damage() {
        let mut v: BTreeMap<String, i32> = BTreeMap::new();
        v.insert("heavy".into(), 80);
        v.insert("light".into(), 100);
        // Heavy → 80%
        assert_eq!(apply_versus(1000, "Heavy", &v), 800);
        assert_eq!(apply_versus(1000, "heavy", &v), 800);
        // Light → 100%
        assert_eq!(apply_versus(1000, "Light", &v), 1000);
        // Concrete (missing) → 100%
        assert_eq!(apply_versus(1000, "Concrete", &v), 1000);
        // Empty class string → "none" lookup; missing → 100%
        assert_eq!(apply_versus(1000, "", &v), 1000);
    }

    #[test]
    fn versus_clamps_negative_to_zero() {
        let mut v: BTreeMap<String, i32> = BTreeMap::new();
        v.insert("heavy".into(), -50);
        assert_eq!(apply_versus(1000, "Heavy", &v), 0);
    }
}
