//! Move activity — pathfind to a target cell, walk one step at a time.
//!
//! On the first tick the activity calls `pathfinder::find_path` and
//! installs the resulting path on the actor's data-driven `Activity::Move`
//! enum, which the existing `World::tick_actors` advances each tick.
//! When the data-driven activity completes (or no path exists), the
//! trait-based wrapper reports `Done`/`Cancel`.

use crate::actor::{Activity as DataActivity, Actor};
use crate::activity::{Activity, ActivityState};
use crate::math::CPos;
use crate::pathfinder;
use crate::traits::TraitState;
use crate::world::World;

/// Move the actor to `target_cell`. Speed is taken from the actor's
/// rules entry (or its `Mobile` trait fallback if no rules entry).
#[derive(Debug)]
pub struct MoveActivity {
    pub target_cell: CPos,
    /// Set on first tick; once true, the activity simply observes the
    /// data-driven `Activity::Move` enum until it finishes.
    started: bool,
    /// Cached actor speed used to install `Activity::Move`.
    speed: i32,
}

impl MoveActivity {
    pub fn new(target_cell: CPos) -> Self {
        MoveActivity { target_cell, started: false, speed: 0 }
    }

    /// Pathfind from `actor.location` to `self.target_cell` and, if a
    /// path exists, install `Activity::Move` on the actor. Returns true
    /// if a path was installed (or already at target), false on failure.
    fn install(&mut self, actor: &mut Actor, world: &World) -> bool {
        let from = match actor.location {
            Some(loc) => loc,
            None => return false,
        };
        let to = (self.target_cell.x(), self.target_cell.y());
        if from == to {
            return true;
        }
        let path = match pathfinder::find_path(&world.terrain, from, to, Some(actor.id)) {
            Some(p) if p.len() > 1 => p,
            _ => return false,
        };
        // Speed from rules; fall back to a reasonable infantry default.
        let speed = actor
            .actor_type
            .as_deref()
            .and_then(|t| world.rules.actor(t))
            .map(|s| s.speed)
            .unwrap_or(43);
        self.speed = speed;
        actor.activity = Some(DataActivity::Move { path, path_index: 1, speed });
        true
    }
}

impl Activity for MoveActivity {
    fn tick(&mut self, actor: &mut Actor, world: &mut World) -> ActivityState {
        if !self.started {
            self.started = true;
            if !self.install(actor, world) {
                return ActivityState::Cancel;
            }
            return ActivityState::Continue;
        }
        // Already running — observe the data-driven activity.
        match &actor.activity {
            Some(DataActivity::Move { .. }) | Some(DataActivity::Turn { .. }) => {
                ActivityState::Continue
            }
            _ => {
                // Data-driven Move finished (cleared) or replaced. Check
                // if we actually arrived.
                if actor.location == Some((self.target_cell.x(), self.target_cell.y())) {
                    ActivityState::Done
                } else {
                    ActivityState::Cancel
                }
            }
        }
    }

    fn name(&self) -> &'static str { "Move" }
}

/// Best-effort lookup of the unit's current cell from its `Mobile` trait.
/// Useful for tests that build actors without a full world.
pub fn current_cell(actor: &Actor) -> Option<CPos> {
    actor.location.map(|(x, y)| CPos::new(x, y)).or_else(|| {
        actor.traits.iter().find_map(|t| match t {
            TraitState::Mobile { from_cell, .. } => Some(*from_cell),
            _ => None,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::ActorKind;
    use crate::world::{self, LobbyInfo};
    use openra_data::oramap::{OraMap, PlayerDef};

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
        world::build_world(&map, 0, &LobbyInfo::default(), None, 0)
    }

    fn fresh_actor(at: (i32, i32)) -> Actor {
        Actor {
            id: 100,
            kind: ActorKind::Infantry,
            owner_id: None,
            location: Some(at),
            traits: Vec::new(),
            activity: None,
            actor_type: Some("e1".into()),
            kills: 0,
            rank: 0,
        }
    }

    #[test]
    fn install_sets_data_activity_with_path() {
        let mut world = empty_world(10, 10);
        let mut actor = fresh_actor((0, 0));
        let mut mv = MoveActivity::new(CPos::new(3, 0));
        let state = mv.tick(&mut actor, &mut world);
        assert_eq!(state, ActivityState::Continue);
        match &actor.activity {
            Some(DataActivity::Move { path, .. }) => {
                assert!(path.len() >= 4, "expected path of >=4 cells, got {:?}", path);
                assert_eq!(path.first(), Some(&(0, 0)));
                assert_eq!(path.last(), Some(&(3, 0)));
            }
            other => panic!("expected Activity::Move, got {:?}", other),
        }
    }

    #[test]
    fn already_at_target_starts_done() {
        let mut world = empty_world(5, 5);
        let mut actor = fresh_actor((2, 2));
        let mut mv = MoveActivity::new(CPos::new(2, 2));
        // First tick: install (no-op, already there); status Continue.
        let s1 = mv.tick(&mut actor, &mut world);
        assert_eq!(s1, ActivityState::Continue);
        // Second tick: actor.activity is None and location matches → Done
        let s2 = mv.tick(&mut actor, &mut world);
        assert_eq!(s2, ActivityState::Done);
    }

    #[test]
    fn cancel_when_no_path() {
        // Actor with no location at all — install fails.
        let mut world = empty_world(5, 5);
        let mut actor = fresh_actor((0, 0));
        actor.location = None;
        let mut mv = MoveActivity::new(CPos::new(4, 4));
        assert_eq!(mv.tick(&mut actor, &mut world), ActivityState::Cancel);
    }

    #[test]
    fn current_cell_prefers_location() {
        let actor = fresh_actor((2, 3));
        assert_eq!(current_cell(&actor), Some(CPos::new(2, 3)));
    }
}
