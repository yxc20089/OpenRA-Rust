//! Health trait component — HP tracking, damage, death.
//!
//! This file is a Phase-1 typed view onto the existing `TraitState::Health`
//! variant in `traits/mod.rs`. The actual hash and storage live in
//! `TraitState`; this module provides ergonomic accessors and helpers.

use super::TraitState;

/// Health component holding current HP. The maximum is supplied externally
/// (from rules) since the existing trait storage tracks only `hp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Health {
    pub hp: i32,
    pub max_hp: i32,
}

impl Health {
    /// Construct a fresh full-HP component.
    pub fn full(max_hp: i32) -> Self {
        Health { hp: max_hp, max_hp }
    }

    /// Apply damage. Saturates at zero. Returns the new HP.
    pub fn take_damage(&mut self, dmg: i32) -> i32 {
        self.hp = self.hp.saturating_sub(dmg).max(0);
        self.hp
    }

    /// Heal by `amount`, capped at `max_hp`.
    pub fn heal(&mut self, amount: i32) -> i32 {
        self.hp = (self.hp + amount).min(self.max_hp);
        self.hp
    }

    /// True when HP has reached zero.
    pub fn is_dead(&self) -> bool {
        self.hp <= 0
    }

    /// Force HP to zero.
    pub fn kill(&mut self) {
        self.hp = 0;
    }

    /// Convert into the synced `TraitState` representation used in
    /// the actor's trait list (carries only `hp`; max_hp is metadata).
    pub fn to_state(self) -> TraitState {
        TraitState::Health { hp: self.hp }
    }

    /// Read the `hp` field from a `TraitState::Health`. Returns `None`
    /// for any other variant.
    pub fn read_hp(state: &TraitState) -> Option<i32> {
        if let TraitState::Health { hp } = state {
            Some(*hp)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_damage_saturates_at_zero() {
        let mut h = Health::full(100);
        assert_eq!(h.take_damage(40), 60);
        assert_eq!(h.take_damage(200), 0);
        assert!(h.is_dead());
    }

    #[test]
    fn heal_caps_at_max() {
        let mut h = Health::full(100);
        h.take_damage(50);
        assert_eq!(h.heal(30), 80);
        assert_eq!(h.heal(100), 100); // capped
    }

    #[test]
    fn kill_sets_hp_to_zero() {
        let mut h = Health::full(50);
        h.kill();
        assert!(h.is_dead());
        assert_eq!(h.hp, 0);
    }

    #[test]
    fn to_state_round_trip() {
        let h = Health::full(123);
        let state = h.to_state();
        assert_eq!(Health::read_hp(&state), Some(123));
    }

    #[test]
    fn read_hp_rejects_other_variants() {
        let s = TraitState::DebugPauseState { paused: false };
        assert_eq!(Health::read_hp(&s), None);
    }
}
