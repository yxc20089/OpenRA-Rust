//! Wait activity — idle for N ticks.

use crate::actor::Actor;
use crate::activity::{Activity, ActivityState};
use crate::world::World;

#[derive(Debug)]
pub struct WaitActivity {
    pub ticks_remaining: i32,
}

impl WaitActivity {
    pub fn new(ticks: i32) -> Self {
        WaitActivity { ticks_remaining: ticks.max(0) }
    }
}

impl Activity for WaitActivity {
    fn tick(&mut self, _actor: &mut Actor, _world: &mut World) -> ActivityState {
        if self.ticks_remaining <= 0 {
            return ActivityState::Done;
        }
        self.ticks_remaining -= 1;
        if self.ticks_remaining <= 0 {
            ActivityState::Done
        } else {
            ActivityState::Continue
        }
    }

    fn name(&self) -> &'static str { "Wait" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::ActorKind;
    use crate::world::{self, LobbyInfo};
    use openra_data::oramap::{OraMap, PlayerDef};

    fn fixture() -> (Actor, World) {
        let map = OraMap {
            title: "tiny".into(),
            tileset: "TEMPERAT".into(),
            map_size: (3, 3),
            bounds: (0, 0, 3, 3),
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
        let world = world::build_world(&map, 0, &LobbyInfo::default(), None, 0);
        let actor = Actor {
            id: 7,
            kind: ActorKind::Infantry,
            owner_id: None,
            location: Some((0, 0)),
            traits: Vec::new(),
            activity: None,
            actor_type: Some("e1".into()),
            kills: 0,
            rank: 0,
        };
        (actor, world)
    }

    #[test]
    fn counts_down_then_done() {
        let (mut a, mut w) = fixture();
        let mut wait = WaitActivity::new(3);
        assert_eq!(wait.tick(&mut a, &mut w), ActivityState::Continue);
        assert_eq!(wait.tick(&mut a, &mut w), ActivityState::Continue);
        assert_eq!(wait.tick(&mut a, &mut w), ActivityState::Done);
    }

    #[test]
    fn zero_ticks_done_immediately() {
        let (mut a, mut w) = fixture();
        let mut wait = WaitActivity::new(0);
        assert_eq!(wait.tick(&mut a, &mut w), ActivityState::Done);
    }

    #[test]
    fn negative_ticks_clamped_to_zero() {
        let wait = WaitActivity::new(-5);
        assert_eq!(wait.ticks_remaining, 0);
    }
}
