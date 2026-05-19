//! Scripted opponent behaviours for bench scenarios.
//!
//! Unlike `ai::Bot` (a full HackyAI economy/base builder), a
//! `ScriptedBot` only commands the enemy's PRE-PLACED combat actors
//! with a fixed, deterministic, **map-agnostic** behaviour selected
//! from the scenario YAML (`enemy: {bot: hunt|rusher|patrol|turtle}`).
//! All targets are derived from live world state (own/foe actor
//! positions, each unit's own spawn cell) — never hard-coded — so the
//! same behaviour works on any map.
//!
//! It runs ground-truth inside the sim (full actor visibility), which
//! is why this lives engine-side: the Python boundary can neither see
//! fogged enemy ids nor issue orders for the enemy player.

use std::collections::HashMap;

use crate::actor::ActorKind;
use crate::world::{GameOrder, World};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptedBehavior {
    /// Each unit attacks the nearest agent actor (ground truth).
    Hunt,
    /// Relentless concentrated charge at the agent's mass (all units
    /// target the foe nearest the agent centroid); fast cadence.
    Rusher,
    /// Each unit oscillates around its own spawn cell; engages
    /// intruders only via its stance (set to Defend).
    Patrol,
    /// Hold the spawn position; Defend stance (return fire), no moves.
    Turtle,
}

impl ScriptedBehavior {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "hunt" => Some(Self::Hunt),
            "rusher" | "rush" => Some(Self::Rusher),
            "patrol" => Some(Self::Patrol),
            "turtle" => Some(Self::Turtle),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct ScriptedBot {
    player_id: u32,
    /// The player this bot is hostile to (the agent). Targeted
    /// explicitly rather than via `find_enemy_actors`, whose
    /// World/Neutral skip hard-codes ids 1,2 and would filter the
    /// agent out for an enemy whose own id is 3.
    target_player_id: u32,
    behavior: ScriptedBehavior,
    /// Ticks between re-issuing orders (cadence).
    interval: u32,
    ticks: u32,
    /// Each controlled unit's initial cell (for patrol/turtle anchor).
    spawn_cell: HashMap<u32, (i32, i32)>,
    /// Patrol leg toggle per unit (out vs back).
    patrol_out: HashMap<u32, bool>,
    /// One-shot setup done (stance applied).
    initialized: bool,
}

const PATROL_RADIUS: i32 = 8;

impl ScriptedBot {
    pub fn new(
        player_id: u32,
        target_player_id: u32,
        behavior: ScriptedBehavior,
    ) -> Self {
        let interval = match behavior {
            ScriptedBehavior::Rusher => 8,
            ScriptedBehavior::Hunt => 16,
            ScriptedBehavior::Patrol => 24,
            ScriptedBehavior::Turtle => 64,
        };
        ScriptedBot {
            player_id,
            target_player_id,
            behavior,
            interval,
            ticks: 0,
            spawn_cell: HashMap::new(),
            patrol_out: HashMap::new(),
            initialized: false,
        }
    }

    fn own_mobile(&self, world: &World) -> Vec<(u32, i32, i32)> {
        world
            .actor_ids_for_player(self.player_id)
            .into_iter()
            .filter(|id| {
                matches!(
                    world.actor_kind(*id),
                    Some(ActorKind::Infantry)
                        | Some(ActorKind::Vehicle)
                        | Some(ActorKind::Mcv)
                )
            })
            .filter_map(|id| world.actor_location(id).map(|(x, y)| (id, x, y)))
            .collect()
    }

    /// The agent's combat actors with position — ground truth,
    /// fog-independent (targeted explicitly by player id).
    fn foes(&self, world: &World) -> Vec<(u32, i32, i32)> {
        world
            .actor_ids_for_player(self.target_player_id)
            .into_iter()
            .filter(|id| {
                matches!(
                    world.actor_kind(*id),
                    Some(ActorKind::Infantry)
                        | Some(ActorKind::Vehicle)
                        | Some(ActorKind::Mcv)
                        | Some(ActorKind::Building)
                )
            })
            .filter_map(|id| world.actor_location(id).map(|(x, y)| (id, x, y)))
            .collect()
    }

    fn stance_all(&self, units: &[(u32, i32, i32)], stance: u32) -> Vec<GameOrder> {
        units
            .iter()
            .map(|&(id, _, _)| GameOrder {
                order_string: "SetStance".to_string(),
                subject_id: Some(id),
                target_string: None,
                extra_data: Some(stance),
            })
            .collect()
    }

    pub fn tick(&mut self, world: &World) -> Vec<GameOrder> {
        let mut orders = Vec::new();
        let units = self.own_mobile(world);
        if units.is_empty() {
            return orders;
        }

        // One-shot: cache spawn cells; set stance per behaviour.
        if !self.initialized {
            for &(id, x, y) in &units {
                self.spawn_cell.insert(id, (x, y));
            }
            // Patrol/Turtle defend their ground (auto-fire on
            // intruders); Hunt/Rusher attack anything.
            let st = match self.behavior {
                ScriptedBehavior::Patrol | ScriptedBehavior::Turtle => 2,
                ScriptedBehavior::Hunt | ScriptedBehavior::Rusher => 3,
            };
            orders.extend(self.stance_all(&units, st));
            self.initialized = true;
        }

        self.ticks += 1;
        if self.ticks < self.interval {
            return orders;
        }
        self.ticks = 0;

        match self.behavior {
            ScriptedBehavior::Turtle => {
                // Hold ground: cancel any movement, rely on stance.
                for &(id, _, _) in &units {
                    orders.push(GameOrder {
                        order_string: "Stop".to_string(),
                        subject_id: Some(id),
                        target_string: None,
                        extra_data: None,
                    });
                }
            }
            ScriptedBehavior::Patrol => {
                for &(id, _, _) in &units {
                    let (sx, sy) = *self.spawn_cell.get(&id).unwrap_or(&(0, 0));
                    let out = self.patrol_out.entry(id).or_insert(true);
                    let (tx, ty) = if *out {
                        (sx + PATROL_RADIUS, sy)
                    } else {
                        (sx - PATROL_RADIUS, sy)
                    };
                    *out = !*out;
                    orders.push(GameOrder {
                        order_string: "Move".to_string(),
                        subject_id: Some(id),
                        target_string: Some(format!("{},{}", tx.max(0), ty.max(0))),
                        extra_data: None,
                    });
                }
            }
            ScriptedBehavior::Hunt => {
                let foes = self.foes(world);
                for &(id, ux, uy) in &units {
                    if let Some((tid, _, _)) = nearest(&foes, ux, uy) {
                        orders.push(GameOrder {
                            order_string: "Attack".to_string(),
                            subject_id: Some(id),
                            target_string: None,
                            extra_data: Some(tid),
                        });
                    }
                }
            }
            ScriptedBehavior::Rusher => {
                let foes = self.foes(world);
                if !foes.is_empty() {
                    // Concentrate: every unit attacks the foe nearest
                    // the agent's centroid (charge the mass together).
                    let (cx, cy) = centroid(&foes);
                    if let Some((tid, _, _)) = nearest(&foes, cx, cy) {
                        for &(id, _, _) in &units {
                            orders.push(GameOrder {
                                order_string: "Attack".to_string(),
                                subject_id: Some(id),
                                target_string: None,
                                extra_data: Some(tid),
                            });
                        }
                    }
                }
            }
        }
        orders
    }
}

fn nearest(foes: &[(u32, i32, i32)], x: i32, y: i32) -> Option<(u32, i32, i32)> {
    foes
        .iter()
        .min_by_key(|&&(_, fx, fy)| {
            let dx = (fx - x) as i64;
            let dy = (fy - y) as i64;
            dx * dx + dy * dy
        })
        .copied()
}

fn centroid(foes: &[(u32, i32, i32)]) -> (i32, i32) {
    let n = foes.len() as i64;
    let sx: i64 = foes.iter().map(|&(_, x, _)| x as i64).sum();
    let sy: i64 = foes.iter().map(|&(_, _, y)| y as i64).sum();
    ((sx / n) as i32, (sy / n) as i32)
}
