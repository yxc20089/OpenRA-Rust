//! Phase-2 activity-stack API.
//!
//! Each actor owns a stack of `Box<dyn Activity>` objects; the top of the
//! stack ticks first and may push child activities (which run to
//! completion before the parent resumes) or finish/cancel itself.
//!
//! The existing `world::tick_actors` operates on the data-driven
//! `actor::Activity` enum for Move/Turn/Attack/Harvest. The trait-based
//! interface here is a parallel architecture for new high-level
//! activities; concrete implementations live under `activities/`.
//!
//! Determinism note: implementors must use only fixed-point math
//! (`WPos`, `WAngle`, `CPos`) and `MersenneTwister` RNG — never `f32`/`f64`.

use crate::actor::Actor;
use crate::world::World;

/// Result of a single `Activity::tick` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityState {
    /// Activity has more work to do — keep it on the stack.
    Continue,
    /// Activity has completed successfully — pop it.
    Done,
    /// Activity was cancelled (e.g. target died, blocked) — pop it
    /// and propagate cancellation to anything that cares.
    Cancel,
}

/// Activity component — runs once per game tick while at the top
/// of an actor's activity stack.
pub trait Activity: std::fmt::Debug + Send {
    /// Advance the activity by one tick. Returning `Done` or `Cancel`
    /// pops the activity from the stack.
    fn tick(&mut self, actor: &mut Actor, world: &mut World) -> ActivityState;

    /// Activities may push a child during `tick`. The default returns
    /// `None`; concrete implementations such as `MoveActivity` may
    /// return a queued child via interior mutation if they need to
    /// nest sub-activities (e.g. Move pushing Wait).
    fn take_child(&mut self) -> Option<Box<dyn Activity>> {
        None
    }

    /// Human-readable activity name (for debug snapshots).
    fn name(&self) -> &'static str;
}

/// LIFO stack of activities owned by an actor.
///
/// `run_top` ticks the top activity, applies any child it queued,
/// and pops finished/cancelled activities until the next ready one.
#[derive(Debug, Default)]
pub struct ActivityStack {
    stack: Vec<Box<dyn Activity>>,
}

impl ActivityStack {
    pub const fn new() -> Self {
        ActivityStack { stack: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    /// Push an activity onto the top of the stack.
    pub fn push(&mut self, activity: Box<dyn Activity>) {
        self.stack.push(activity);
    }

    /// Tick the topmost activity once and process its result.
    /// Returns the post-tick state (or `Done` if the stack was empty).
    pub fn run_top(&mut self, actor: &mut Actor, world: &mut World) -> ActivityState {
        let Some(top) = self.stack.last_mut() else {
            return ActivityState::Done;
        };
        let state = top.tick(actor, world);
        if let Some(child) = top.take_child() {
            // Push child *after* the tick result so the child runs next.
            self.stack.push(child);
            return ActivityState::Continue;
        }
        match state {
            ActivityState::Continue => ActivityState::Continue,
            ActivityState::Done | ActivityState::Cancel => {
                self.stack.pop();
                state
            }
        }
    }

    /// Cancel everything (used when an `Order::Stop` arrives).
    pub fn cancel_all(&mut self) {
        self.stack.clear();
    }

    /// Read-only top of the stack (for debug / observation).
    pub fn top_name(&self) -> Option<&'static str> {
        self.stack.last().map(|a| a.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::{ActorKind};
    use crate::math::CPos;
    use crate::world::{self, LobbyInfo};
    use openra_data::oramap::{OraMap, PlayerDef};

    #[derive(Debug)]
    struct CountingActivity {
        ticks_remaining: i32,
        ticked: i32,
    }

    impl Activity for CountingActivity {
        fn tick(&mut self, _actor: &mut Actor, _world: &mut World) -> ActivityState {
            self.ticked += 1;
            self.ticks_remaining -= 1;
            if self.ticks_remaining > 0 {
                ActivityState::Continue
            } else {
                ActivityState::Done
            }
        }
        fn name(&self) -> &'static str { "Counting" }
    }

    #[derive(Debug, Default)]
    struct ChildPusher {
        pushed: bool,
        queued: Option<Box<dyn Activity>>,
    }

    impl Activity for ChildPusher {
        fn tick(&mut self, _actor: &mut Actor, _world: &mut World) -> ActivityState {
            if !self.pushed {
                self.queued = Some(Box::new(CountingActivity {
                    ticks_remaining: 2, ticked: 0,
                }));
                self.pushed = true;
                ActivityState::Continue
            } else {
                ActivityState::Done
            }
        }
        fn take_child(&mut self) -> Option<Box<dyn Activity>> {
            self.queued.take()
        }
        fn name(&self) -> &'static str { "ChildPusher" }
    }

    /// Build a tiny 5x5 empty world (no replay needed) for stack tests.
    fn tiny_world() -> (Actor, World) {
        let map = OraMap {
            title: "tiny".into(),
            tileset: "TEMPERAT".into(),
            map_size: (5, 5),
            bounds: (0, 0, 5, 5),
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
        let dummy = Actor {
            id: 999,
            kind: ActorKind::Infantry,
            owner_id: None,
            location: Some((0, 0)),
            traits: Vec::new(),
            activity: None,
            actor_type: Some("e1".into()),
            kills: 0,
            rank: 0,
        };
        (dummy, world)
    }

    #[test]
    fn empty_stack_returns_done() {
        let mut stack = ActivityStack::new();
        let (mut a, mut w) = tiny_world();
        assert_eq!(stack.run_top(&mut a, &mut w), ActivityState::Done);
        assert!(stack.is_empty());
    }

    #[test]
    fn done_pops_activity() {
        let mut stack = ActivityStack::new();
        stack.push(Box::new(CountingActivity { ticks_remaining: 1, ticked: 0 }));
        let (mut a, mut w) = tiny_world();
        assert_eq!(stack.run_top(&mut a, &mut w), ActivityState::Done);
        assert!(stack.is_empty());
    }

    #[test]
    fn child_runs_before_parent_resumes() {
        let mut stack = ActivityStack::new();
        stack.push(Box::new(ChildPusher::default()));
        let (mut a, mut w) = tiny_world();
        // Tick 1: parent pushes child, returns Continue
        assert_eq!(stack.run_top(&mut a, &mut w), ActivityState::Continue);
        assert_eq!(stack.len(), 2);
        assert_eq!(stack.top_name(), Some("Counting"));
        // Tick 2: child Continues
        assert_eq!(stack.run_top(&mut a, &mut w), ActivityState::Continue);
        // Tick 3: child Done — popped
        assert_eq!(stack.run_top(&mut a, &mut w), ActivityState::Done);
        assert_eq!(stack.len(), 1);
        // Tick 4: parent now Done
        assert_eq!(stack.run_top(&mut a, &mut w), ActivityState::Done);
        assert!(stack.is_empty());
    }

    #[test]
    fn cancel_all_clears_stack() {
        let mut stack = ActivityStack::new();
        stack.push(Box::new(CountingActivity { ticks_remaining: 100, ticked: 0 }));
        stack.push(Box::new(CountingActivity { ticks_remaining: 100, ticked: 0 }));
        assert_eq!(stack.len(), 2);
        stack.cancel_all();
        assert!(stack.is_empty());
        // Suppress unused-variable warnings from CPos import
        let _ = CPos::new(0, 0);
    }
}
