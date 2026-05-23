//! Actor component — the core entity in the game world.
//!
//! Each actor has an ID, a kind, optional owner and location,
//! a list of traits (in construction order), and an optional activity.

use serde::Serialize;
use crate::traits::TraitState;

/// Actor kind for rendering and classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ActorKind {
    World,
    Player,
    Tree,
    Mine,
    Spawn,
    Mcv,
    Building,
    Infantry,
    Vehicle,
    Aircraft,
    Ship,
}

/// An activity queued on an actor (simplified C# Activity system).
#[derive(Debug, Clone)]
pub enum Activity {
    /// Turn toward a target facing at the given speed (WAngle units/tick).
    /// Optional `then` activity to execute after turn completes.
    Turn { target: i32, speed: i32, then: Option<Box<Activity>> },
    /// Move along a path of cells at a given speed (world units/tick).
    Move {
        path: Vec<(i32, i32)>,
        path_index: usize,
        speed: i32,
    },
    /// Attack a target actor. Move in range, then fire.
    Attack {
        target_id: u32,
        weapon_range: i32,     // cells
        weapon_damage: i32,    // base damage per shot
        reload_delay: i32,     // ticks between shots
        reload_remaining: i32, // ticks until next shot
        burst: i32,            // shots per reload cycle
        burst_remaining: i32,  // shots left in current burst
        /// `true` when this Attack was acquired by auto-target /
        /// opportunistic engagement (idle auto-engage, defensive
        /// building scan). `false` when it came from an explicit
        /// agent/player "Attack" order. HoldFire stance abandons
        /// auto-acquired attacks every tick but NEVER cancels an
        /// order-issued one (player intent overrides stance) —
        /// faithful to C# AttackBase / AutoTarget.
        auto_acquired: bool,
    },
    /// Guard a friendly actor: stay within `leash` cells of it,
    /// repathing toward it whenever it drifts out of `leash`. C#
    /// `GuardActivity` / `Guard` trait (follow subset). `speed` is the
    /// guard's movement speed (world units/tick) reused for the
    /// internal chase Move.
    Guard {
        target_id: u32,
        leash: i32,
        speed: i32,
    },
    /// Move to a transport actor and board it (C# `EnterTransport` /
    /// `Cargo`). When the passenger reaches a cell adjacent to the
    /// transport it is removed from the world and stored as cargo.
    EnterTransport {
        transport_id: u32,
        speed: i32,
    },
    /// Walk to an enemy building and capture it (C# `Captures` /
    /// `CapturesTransform`). When the engineer reaches a cell adjacent
    /// to the target the target's `owner_id` is reassigned to the
    /// capturer's owner and the engineer is consumed (removed from the
    /// world). Pragmatic subset: no "external capture" weights,
    /// instant on-arrival transfer.
    Capture {
        target_id: u32,
        speed: i32,
    },
    /// Harvest resources: find ore → move → harvest → deliver → repeat.
    Harvest {
        state: HarvestState,
        /// Refinery actor ID to deliver to.
        refinery_id: u32,
        /// Resources carried (ore units).
        carried_ore: i32,
        /// Resources carried (gem units).
        carried_gems: i32,
        /// Capacity (total units).
        capacity: i32,
        /// Movement path (reused for both to-ore and to-refinery).
        path: Vec<(i32, i32)>,
        path_index: usize,
        speed: i32,
        /// Ticks remaining for current harvest action.
        harvest_ticks: i32,
        /// Last harvested cell (for searching nearby).
        last_harvest_cell: Option<(i32, i32)>,
    },
}

/// Sub-state for the Harvest activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarvestState {
    /// Searching for a resource cell.
    FindingOre,
    /// Moving to a resource cell.
    MovingToOre,
    /// Harvesting at current cell (waiting BaleLoadDelay ticks).
    Harvesting,
    /// Moving to refinery to deliver.
    MovingToRefinery,
    /// Unloading at refinery.
    Unloading,
}

/// A game actor with its traits and current activity.
#[derive(Debug, Clone)]
pub struct Actor {
    pub id: u32,
    pub kind: ActorKind,
    /// Owning player's actor ID (None for World/Player actors).
    pub owner_id: Option<u32>,
    /// Cell location (for positioned actors).
    pub location: Option<(i32, i32)>,
    /// Traits in construction order (determines sync hash order).
    pub traits: Vec<TraitState>,
    /// Current activity (Turn, Move, Attack, etc.).
    pub activity: Option<Activity>,
    /// Actor type name from rules (e.g., "fact", "2tnk", "e1").
    /// Not synced — used for game logic lookups.
    pub actor_type: Option<String>,
    /// Kill count for veterancy (not synced).
    pub kills: u16,
    /// Veterancy rank: 0=none, 1=veteran, 2=elite, 3=heroic (not synced).
    pub rank: u8,
}

impl Activity {
    /// Get a cell from a Move path at the given index. Returns None for non-Move activities.
    pub fn path_cell(&self, index: usize) -> Option<(i32, i32)> {
        if let Activity::Move { path, .. } = self {
            path.get(index).copied()
        } else {
            None
        }
    }
}

impl Actor {
    /// Compute sync hashes for all traits.
    pub fn sync_hashes(&self) -> Vec<i32> {
        self.traits.iter().map(|t| t.sync_hash()).collect()
    }

    /// Get player's cash from PlayerResources trait.
    pub fn cash(&self) -> i32 {
        for t in &self.traits {
            if let TraitState::PlayerResources { cash, .. } = t {
                return *cash;
            }
        }
        0
    }

    /// Set player's cash in PlayerResources trait.
    pub fn set_cash(&mut self, new_cash: i32) {
        for t in &mut self.traits {
            if let TraitState::PlayerResources { cash, .. } = t {
                *cash = new_cash;
                return;
            }
        }
    }

    /// Stored (harvested-but-not-yet-cashed) resources.
    pub fn resources(&self) -> i32 {
        for t in &self.traits {
            if let TraitState::PlayerResources { resources, .. } = t {
                return *resources;
            }
        }
        0
    }

    pub fn set_resources(&mut self, n: i32) {
        for t in &mut self.traits {
            if let TraitState::PlayerResources { resources, .. } = t {
                *resources = n;
                return;
            }
        }
    }

    /// Storage capacity (from refineries/silos) — the cap on stored
    /// resources; harvested ore beyond this is lost.
    pub fn resource_capacity(&self) -> i32 {
        for t in &self.traits {
            if let TraitState::PlayerResources { resource_capacity, .. } = t {
                return *resource_capacity;
            }
        }
        0
    }

    pub fn set_resource_capacity(&mut self, n: i32) {
        for t in &mut self.traits {
            if let TraitState::PlayerResources { resource_capacity, .. } = t {
                *resource_capacity = n;
                return;
            }
        }
    }
}
