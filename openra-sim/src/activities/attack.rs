//! Attack activity — Phase-3 typed component.
//!
//! Mirrors C# `AttackBase` + `AttackFrontal`: every tick, locate the
//! target, decide whether to chase (push a `MoveActivity` child),
//! cool down, or fire. The armament owns the per-actor cooldown
//! state; on `mark_fired` it resets to `weapon.reload_delay`.
//!
//! Out of scope (deferred to v2):
//! * Projectile flight time — damage is applied instantly.
//! * Splash damage — single-target only.
//! * `Versus` armor multipliers — flat `weapon.damage` applied.
//! * Multi-weapon armaments / turrets.
//! * Burst patterns (Burst != 1) and BurstDelay.
//!
//! Determinism: damage application is collected by the world's tick
//! loop into a `BTreeMap<u32, i32>` keyed by victim id (see
//! `World::tick`), then applied in sorted order so HashMap iteration
//! never enters the credit assignment.

use crate::activities::move_::MoveActivity;
use crate::activity::{Activity, ActivityState};
use crate::actor::Actor;
use crate::math::CPos;
use crate::traits::{armament::Armament, Health, TraitState};
use crate::world::World;

/// Attack `target_actor_id` until they're dead, gone, or the
/// activity is cancelled. Pushes child `MoveActivity` instances when
/// the target moves out of range. The cooldown lives on the
/// activity's `Armament`; the world loop is *not* required to tick
/// it separately — `AttackActivity::tick` does so itself.
#[derive(Debug)]
pub struct AttackActivity {
    pub target_actor_id: u32,
    pub armament: Armament,
    /// Set on the tick we fired (reset every tick after). Useful for
    /// tests that want to assert the firing schedule without scraping
    /// the target's HP timeline.
    fired_this_tick: bool,
    /// Pending child activity (e.g. a chase Move).
    queued_child: Option<Box<dyn Activity>>,
}

impl AttackActivity {
    pub fn new(target_actor_id: u32, armament: Armament) -> Self {
        AttackActivity {
            target_actor_id,
            armament,
            fired_this_tick: false,
            queued_child: None,
        }
    }

    /// Did this activity fire on its last tick? Cleared at the start
    /// of every tick.
    pub fn fired_this_tick(&self) -> bool {
        self.fired_this_tick
    }

    /// Read-only view of the underlying armament (cooldown state).
    pub fn armament(&self) -> &Armament {
        &self.armament
    }
}

impl Activity for AttackActivity {
    fn tick(&mut self, actor: &mut Actor, world: &mut World) -> ActivityState {
        self.fired_this_tick = false;

        // Always tick the armament cooldown first, regardless of range.
        // (This matches AttackBase.Tick which decrements timers each
        // frame the trait runs.)
        self.armament.tick();

        // 1. Resolve attacker position.
        let attacker_pos = match actor.location {
            Some((x, y)) => CPos::new(x, y),
            None => return ActivityState::Cancel,
        };

        // 2. Resolve target — if missing or dead, we're done.
        let (target_pos, target_dead) = match world.actor_summary(self.target_actor_id) {
            Some(s) => (s.cell, s.is_dead),
            None => return ActivityState::Done,
        };
        if target_dead {
            return ActivityState::Done;
        }

        // 3. Range check (Chebyshev cells, matching the existing
        //    world combat loop and the OpenRA grid melee semantics).
        let dx = (attacker_pos.x() - target_pos.x()).abs();
        let dy = (attacker_pos.y() - target_pos.y()).abs();
        let cheb = dx.max(dy);
        let weapon_range_cells = (self.armament.weapon.range.length / 1024).max(0);

        if cheb > weapon_range_cells {
            // Out of range — clear any pending data-driven activity
            // (the Move child will install its own) and chase. We
            // still want the cooldown to keep ticking while moving.
            self.queued_child = Some(Box::new(MoveActivity::new(target_pos)));
            return ActivityState::Continue;
        }

        // 4. In range — cooldown gates firing.
        if !self.armament.is_ready() {
            return ActivityState::Continue;
        }

        // 5. Fire. Apply damage immediately (no projectile flight).
        let damage = self.armament.weapon.damage;
        if let Some(target) = world.actor_mut(self.target_actor_id) {
            // Update both the typed Health (if anyone is reading it)
            // and the existing TraitState::Health (which the rest of
            // the engine treats as the source of truth). To keep the
            // two in sync we mutate TraitState::Health directly here
            // — there is exactly one such trait per actor in the
            // production world.
            for t in target.traits.iter_mut() {
                if let TraitState::Health { hp } = t {
                    let mut h = Health { hp: *hp, max_hp: *hp };
                    h.take_damage(damage);
                    *hp = h.hp;
                    break;
                }
            }
        }
        self.armament.mark_fired();
        self.fired_this_tick = true;

        // If the shot killed them, finish next tick after the world
        // gets a chance to clean up the corpse — otherwise keep
        // attacking.
        ActivityState::Continue
    }

    fn take_child(&mut self) -> Option<Box<dyn Activity>> {
        self.queued_child.take()
    }

    fn name(&self) -> &'static str {
        "Attack"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::ActorKind;
    use crate::math::CPos;
    use crate::traits::TraitState;
    use crate::world::{self, insert_test_actor, set_test_unpaused, LobbyInfo};
    use openra_data::oramap::{OraMap, PlayerDef};
    use openra_data::rules::{WDist, WeaponStats};

    fn empty_world(w: i32, h: i32) -> World {
        let map = OraMap {
            title: "tiny".into(),
            tileset: "TEMPERAT".into(),
            map_size: (w, h),
            bounds: (0, 0, w, h),
            tiles: Vec::new(),
            actors: Vec::new(),
            players: vec![PlayerDef {
                name: "Neutral".into(),
                playable: false,
                owns_world: true,
                non_combatant: true,
                faction: "allies".into(),
                enemies: Vec::new(),
            }],
        };
        let mut world = world::build_world(&map, 0, &LobbyInfo::default(), None, 0);
        set_test_unpaused(&mut world);
        world
    }

    fn make_actor(id: u32, at: (i32, i32), hp: i32) -> Actor {
        Actor {
            id,
            kind: ActorKind::Infantry,
            owner_id: None,
            location: Some(at),
            traits: vec![TraitState::Health { hp }],
            activity: None,
            actor_type: Some("e1".into()),
            kills: 0,
            rank: 0,
        }
    }

    fn m1carbine() -> WeaponStats {
        WeaponStats {
            name: "M1Carbine".into(),
            range: WDist::from_cells(5),
            reload_delay: 20,
            damage: 1000,
        }
    }

    #[test]
    fn done_when_target_already_dead() {
        let mut world = empty_world(10, 10);
        insert_test_actor(&mut world, make_actor(101, (0, 0), 5000));
        insert_test_actor(&mut world, make_actor(102, (1, 0), 0));
        let mut atk = AttackActivity::new(102, Armament::new(m1carbine()));
        let mut a = world.actor(101).unwrap().clone();
        let s = atk.tick(&mut a, &mut world);
        assert_eq!(s, ActivityState::Done);
    }

    #[test]
    fn fires_when_in_range_and_cooldown_zero() {
        let mut world = empty_world(10, 10);
        insert_test_actor(&mut world, make_actor(101, (0, 0), 5000));
        insert_test_actor(&mut world, make_actor(102, (3, 0), 5000));
        let mut atk = AttackActivity::new(102, Armament::new(m1carbine()));
        let mut a = world.actor(101).unwrap().clone();
        let s = atk.tick(&mut a, &mut world);
        assert_eq!(s, ActivityState::Continue);
        assert!(atk.fired_this_tick());
        assert_eq!(atk.armament().current_cooldown_ticks, 20);
        // Target HP dropped by weapon damage (1000)
        assert_eq!(world.actor(102).unwrap().traits.iter().find_map(|t| {
            if let TraitState::Health { hp } = t { Some(*hp) } else { None }
        }), Some(4000));
    }

    #[test]
    fn pushes_move_child_when_out_of_range() {
        let mut world = empty_world(40, 40);
        insert_test_actor(&mut world, make_actor(101, (0, 0), 5000));
        insert_test_actor(&mut world, make_actor(102, (20, 0), 5000));
        let mut atk = AttackActivity::new(102, Armament::new(m1carbine()));
        let mut a = world.actor(101).unwrap().clone();
        let s = atk.tick(&mut a, &mut world);
        assert_eq!(s, ActivityState::Continue);
        assert!(!atk.fired_this_tick());
        let child = atk.take_child().expect("expected chase Move child");
        assert_eq!(child.name(), "Move");
    }
}
