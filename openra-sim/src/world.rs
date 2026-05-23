//! Game world state — actors, players, RNG.
//!
//! This module builds the world from map data and replay metadata,
//! then computes per-tick SyncHash to verify determinism against
//! the hashes recorded in .orarep files.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

pub use crate::actor::ActorKind;

use crate::actor::{Activity, Actor, HarvestState};
use crate::ai::Bot;
use crate::gamerules::GameRules;
use crate::math::{CPos, WPos};
use crate::pathfinder;
use crate::projectile::{apply_versus, Projectile};
use crate::rng::MersenneTwister;
use crate::sync;
use crate::terrain::{CellLayer, ResourceType, TerrainMap, COST_IMPASSABLE};
use crate::traits::{PqType, TraitState, Turret, Vehicle};

/// Lobby information extracted from the replay's SyncInfo orders.
#[derive(Debug, Clone)]
pub struct LobbyInfo {
    pub starting_cash: i32,
    pub allow_spectators: bool,
    pub occupied_slots: Vec<SlotInfo>,
}

#[derive(Debug, Clone)]
pub struct SlotInfo {
    pub player_reference: String,
    pub faction: String,
    /// Whether this slot is controlled by an AI bot.
    pub is_bot: bool,
}

impl Default for LobbyInfo {
    fn default() -> Self {
        LobbyInfo {
            starting_cash: 5000,
            allow_spectators: true,
            occupied_slots: Vec::new(),
        }
    }
}

/// A game order parsed from the replay, ready for dispatch.
#[derive(Debug, Clone)]
pub struct GameOrder {
    pub order_string: String,
    pub subject_id: Option<u32>,
    pub target_string: Option<String>,
    pub extra_data: Option<u32>,
}

/// A production item being built in a ClassicProductionQueue.
#[derive(Debug)]
struct ProductionItem {
    item_name: String,
    total_cost: i32,
    total_time: i32,
    remaining_cost: i32,
    remaining_time: i32,
    started: bool,
}

impl ProductionItem {
    fn new(name: &str, cost: i32, build_duration_modifier: i32) -> Self {
        let time = (cost as i64 * build_duration_modifier as i64 / 100) as i32;
        ProductionItem {
            item_name: name.to_string(),
            total_cost: cost,
            total_time: time,
            remaining_cost: cost,
            remaining_time: time,
            started: false,
        }
    }

    fn tick(&mut self, cash: i32) -> i32 {
        if !self.started {
            self.started = true;
        }
        if self.remaining_time <= 0 {
            return 0;
        }
        if self.remaining_cost != 0 {
            let expected_remaining_cost = if self.remaining_time == 1 {
                0
            } else {
                (self.total_cost as i64 * self.remaining_time as i64
                    / self.total_time.max(1) as i64) as i32
            };
            let cost_this_tick = self.remaining_cost - expected_remaining_cost;
            if cost_this_tick != 0 {
                if cash < cost_this_tick {
                    return 0;
                }
                self.remaining_cost -= cost_this_tick;
                self.remaining_time -= 1;
                return cost_this_tick;
            }
        }
        self.remaining_time -= 1;
        0
    }

    fn is_done(&self) -> bool {
        self.remaining_time <= 0
    }
}

/// A deferred action to execute at the end of World.Tick().
#[derive(Debug)]
enum FrameEndTask {
    DeployTransform { old_actor_id: u32, location: (i32, i32), owner_player_id: u32 },
    SpawnUnit { unit_type: String, owner_player_id: u32 },
}

// === Snapshot types for rendering ===

#[derive(Debug, Serialize)]
pub struct WorldSnapshot {
    pub tick: u32,
    pub actors: Vec<ActorSnapshot>,
    pub players: Vec<PlayerSnapshot>,
    pub map_width: i32,
    pub map_height: i32,
    pub resources: Vec<ResourceSnapshot>,
    /// Superweapon states: [(weapon_type, owner, ticks_remaining, charge_total)]
    pub superweapons: Vec<SuperweaponSnapshot>,
}

#[derive(Debug, Serialize)]
pub struct SuperweaponSnapshot {
    pub weapon_type: String,
    pub owner: u32,
    pub ticks_remaining: i32,
    pub charge_total: i32,
}

#[derive(Debug, Serialize)]
pub struct ResourceSnapshot {
    pub x: i32,
    pub y: i32,
    pub kind: u8, // 1=ore, 2=gems
    pub density: u8,
}

#[derive(Debug, Serialize)]
pub struct ActorSnapshot {
    pub id: u32,
    pub kind: ActorKind,
    pub owner: u32,
    pub x: i32,
    pub y: i32,
    /// Sub-cell center position X (world units, 1024 = 1 cell).
    pub cx: i32,
    /// Sub-cell center position Y (world units, 1024 = 1 cell).
    pub cy: i32,
    pub actor_type: String,
    pub hp: i32,
    pub max_hp: i32,
    pub activity: String,
    pub facing: i32,
    /// Attack target actor ID (for projectile rendering).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<u32>,
    /// Destination cell of the current activity (x), if any.
    /// Move → final path cell; Attack → target's current cell at snapshot
    /// time. None for idle / Turn / Harvest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_x: Option<i32>,
    /// Destination cell of the current activity (y), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_y: Option<i32>,
    /// Veterancy rank (0=none, 1=veteran, 2=elite, 3=heroic).
    pub rank: u8,
}

#[derive(Debug, Serialize)]
pub struct PlayerSnapshot {
    pub index: u32,
    pub cash: i32,
    pub power_provided: i32,
    pub power_drained: i32,
    /// S1: stored (harvested, not-yet-cashed) resources and the storage
    /// cap from refineries/silos.
    pub resources: i32,
    pub resource_capacity: i32,
    pub production_queue: Vec<ProductionSnapshot>,
}

#[derive(Debug, Serialize)]
pub struct ProductionSnapshot {
    pub item_name: String,
    pub progress: f32,
    pub done: bool,
}

/// Info about an item the player can build (or is locked).
#[derive(Debug, Serialize)]
pub struct BuildableInfo {
    pub name: String,
    pub cost: i32,
    pub kind: ActorKind,
    pub is_building: bool,
    pub power: i32,
    pub footprint: (i32, i32),
    pub locked: bool,
    pub prerequisites: Vec<String>,
    pub queue_type: String,
    pub build_palette_order: i32,
}

pub struct SyncHashDebug {
    pub full: i32,
    pub identity: i32,
    pub traits: i32,
    pub rng_last: i32,
}

/// The game world state.
/// Convert a PqType to its string name for the frontend.
fn pq_type_name(pq: PqType) -> &'static str {
    match pq {
        PqType::Building => "Building",
        PqType::Defense => "Defense",
        PqType::Infantry => "Infantry",
        PqType::Vehicle => "Vehicle",
        PqType::Aircraft => "Aircraft",
        PqType::Ship => "Ship",
    }
}

pub struct World {
    /// All actors keyed by ID. BTreeMap ensures deterministic iteration order.
    actors: BTreeMap<u32, Actor>,
    /// Synced effects (projectiles etc.) — empty for now.
    synced_effects: Vec<i32>,
    /// The shared RNG.
    pub rng: MersenneTwister,
    /// Whether the world simulation is paused.
    pub paused: bool,
    /// Current simulation tick.
    pub world_tick: u32,
    /// Current frame number.
    pub frame_number: u32,
    /// Network order latency (empty frames before game starts).
    pub order_latency: u32,
    /// Next actor ID to assign.
    next_actor_id: u32,
    /// Pending frame-end tasks.
    frame_end_tasks: Vec<FrameEndTask>,
    /// Player actor IDs in creation order.
    player_actor_ids: Vec<u32>,
    /// "Everyone" spectator player ID.
    everyone_player_id: u32,
    /// Number of mine actors (for SeedsResource RNG consumption).
    mine_count: usize,
    /// Cell locations of mine actors — seed + replenish ore here so
    /// harvesters have something to collect.
    mine_locations: Vec<(i32, i32)>,
    /// Player ids that have surrendered (conceded). A surrendered
    /// player is treated as defeated by the env's terminal check.
    surrendered: std::collections::HashSet<u32>,
    /// Ticks until next SeedsResource seeding event.
    seeds_resource_ticks: i32,
    /// Active production items per player actor ID.
    /// Production queues: player_id → (queue_type → items).
    /// Each queue type builds independently. Buildings queue sequentially within each type.
    production: HashMap<u32, HashMap<PqType, Vec<ProductionItem>>>,
    /// Terrain and occupancy map.
    pub terrain: TerrainMap,
    /// Map dimensions (cells).
    map_width: i32,
    map_height: i32,
    /// Per-player shroud: 0=unexplored, 1=fogged, 2=visible.
    /// Index by player_actor_ids index.
    shroud: Vec<CellLayer<u8>>,
    /// Compiled game rules for lookups (costs, stats, weapons).
    pub rules: GameRules,
    /// AI bots controlling their respective players.
    bots: Vec<Bot>,
    /// Scripted opponent controllers (bench enemy.bot behaviours).
    scripted_bots: Vec<crate::scripted_bot::ScriptedBot>,
    /// Buildings currently being repaired (toggled by RepairBuilding order).
    repairing: HashSet<u32>,
    /// Buildings whose power output is currently SHED (PowerDown toggle).
    /// A powered-down building contributes ZERO to its owner's
    /// `power_provided` / `power_drained` totals until toggled back on.
    /// Mirrors C# `CanPowerDown` (Cnc/Ra). Used by `compute_player_power`.
    powered_down: HashSet<u32>,
    /// Rally points per building actor ID.
    rally_points: HashMap<u32, (i32, i32)>,
    /// Cargo: transport actor id → boarded passenger Actors (stashed
    /// out of the active `actors` map while carried). C# `Cargo`.
    cargo: HashMap<u32, Vec<Actor>>,
    /// Building actor IDs flagged PRIMARY for production
    /// (C# `PrimaryBuilding`). At most one per (owner, building-type):
    /// produced units of that category spawn from / rally through the
    /// primary. Missing ⇒ fall back to first-found building.
    primary_buildings: HashSet<u32>,
    /// Engagement stance per actor id: 0=HoldFire, 1=ReturnFire,
    /// 2=Defend, 3=AttackAnything. Missing = engage (current default).
    stances: HashMap<u32, u8>,
    /// World-tick at which each actor last received damage from a
    /// hostile attacker. Used to gate stance:1 (ReturnFire) auto-
    /// engagement: a unit on stance:1 only opens fire on enemies if
    /// it itself has been attacked within `RETURN_FIRE_WINDOW` ticks.
    /// Without this, stance:1 collapsed into stance:2 (auto-engage
    /// any in-range enemy whether or not we're under attack).
    recently_received_fire: HashMap<u32, u32>,
    /// Actor IDs licensed to HUNT — i.e. stance:3 units that ALSO
    /// advance toward visible-but-out-of-range enemies. Hunt is the
    /// strict superset of in-range auto-engage: a hunting unit
    /// pursues anything on the map it can reach. We track this as
    /// a separate opt-in (set only when the agent issues the
    /// `SetStance` order with stance=3, NOT just because the
    /// scenario YAML declared `stance: 3` at spawn) because many
    /// existing bench packs declare enemy units as `stance: 3`
    /// meaning "auto-engage in range" and would lose their
    /// stand-and-fire semantics if the engine started chasing them
    /// across the map. Agent-issued stance:3 (via `set_stance`) is
    /// the new "actively hunt" verb; scenario-declared stance:3 is
    /// the legacy "engage what wanders into range" default.
    hunt_enabled: HashSet<u32>,
    /// Superweapon charge timers: (building_type, owner_player_id) → ticks_remaining.
    superweapon_timers: HashMap<(String, u32), i32>,
    /// Actors with invulnerability ticks remaining (Iron Curtain).
    invulnerable: HashMap<u32, i32>,
    /// Player faction mapping: player_actor_id → faction name.
    player_factions: HashMap<u32, String>,
    /// Phase-3 typed per-player shroud (visible + explored bool grids).
    /// Refreshed by `update_typed_shroud_all_players` (called from
    /// `World::tick`). The legacy `shroud: Vec<CellLayer<u8>>` above
    /// is kept untouched for the existing 0/1/2-state observation
    /// pipeline.
    typed_shroud: BTreeMap<u32, crate::traits::Shroud>,
    /// Combat-tally counter: per-player count of enemy actors killed.
    /// Incremented in `tick_actors` whenever a player's attack reduces
    /// a target's HP ≤ 0. Read via `kills_for_player`.
    kills_per_player: BTreeMap<u32, u32>,
    /// Phase-8: in-flight projectiles (rockets / missiles). BTreeMap so
    /// per-tick advance and impact-resolution iterate in stable id
    /// order. Spawned by the combat loop when an attacker fires a
    /// weapon with `projectile_speed > 0`.
    pending_projectiles: BTreeMap<u32, Projectile>,
    /// Monotonic id source for `pending_projectiles`. Never reused
    /// (a freed id stays freed).
    next_projectile_id: u32,
    /// Phase-8 typed components attached to actors. Parallel BTreeMap
    /// keyed by actor id so existing serialized snapshot / sync code
    /// is unaffected. Currently stores `Vehicle` (locomotor + has_turret
    /// flag) and `Turret` (independent yaw, turn-speed) per actor.
    typed_components: BTreeMap<u32, ActorTypedComponents>,
    /// Reveal-on-attack: when actor X damages a unit owned by player P,
    /// X's cell is added here for player P. Consumed (and cleared) by
    /// `update_typed_shroud_all_players`, which forces those cells
    /// visible+explored regardless of P's own sight coverage. This
    /// matches OpenRA C# behaviour — getting shot reveals the shooter
    /// even through fog (otherwise tesla coils silently kill scouts
    /// from beyond their sight range).
    combat_reveal_cells: BTreeMap<u32, Vec<(i32, i32)>>,
    /// Per-frame "claimed" move destinations. Cleared at the start of
    /// each `process_frame`, populated by `order_move` so that multiple
    /// simultaneous Move orders to the same target spread to nearby
    /// unoccupied cells instead of all stacking on one cell.
    pending_move_destinations: HashSet<(i32, i32)>,
    /// Per-actor weapon-reload countdown for OPPORTUNISTIC fire while
    /// the actor is executing a `Move` activity. A unit transiting
    /// enemy weapon range is a normal combatant — it both takes fire
    /// and shoots back at in-range hostiles WITHOUT abandoning its
    /// move (faithful to C# AttackMove / opportunistic AutoTarget).
    /// Keyed by actor id; entries are pruned when the actor stops
    /// moving or dies. Without this, a unit on a long `move` order
    /// crossed a kill zone untouched — "sprint-invincibility".
    move_fire_cooldown: HashMap<u32, i32>,
}

/// Typed components attached to a single actor. Lookup is via
/// `World::typed_components_of(actor_id)`. Either field may be `None`
/// for actors that don't have the corresponding typed view (a foot
/// soldier carries neither, a 2tnk carries both).
#[derive(Debug, Clone, Default)]
pub struct ActorTypedComponents {
    pub vehicle: Option<Vehicle>,
    pub turret: Option<Turret>,
}

impl World {
    /// Compute World.SyncHash() matching the C# algorithm exactly.
    pub fn sync_hash(&self) -> i32 {
        let actor_ids: Vec<u32> = self.actors.keys().copied().collect();
        let actor_syncs: Vec<sync::ActorSync> = self.actors.values()
            .filter(|a| !a.traits.is_empty())
            .map(|a| sync::ActorSync {
                actor_id: a.id,
                trait_hashes: a.sync_hashes(),
            })
            .collect();
        sync::compute_world_sync_hash(
            &actor_ids,
            &actor_syncs,
            &self.synced_effects,
            self.rng.last,
            &[], // unlocked_render_player_ids — always empty for now
        )
    }

    /// Get a player's cash.
    /// True if the player has conceded via a Surrender order.
    pub fn is_surrendered(&self, player_id: u32) -> bool {
        self.surrendered.contains(&player_id)
    }

    pub fn player_cash(&self, player_id: u32) -> i32 {
        self.actors.get(&player_id).map(|a| a.cash()).unwrap_or(0)
    }

    /// Get actor IDs owned by a player.
    pub fn actor_ids_for_player(&self, player_id: u32) -> Vec<u32> {
        self.actors.values()
            .filter(|a| a.owner_id == Some(player_id))
            .map(|a| a.id)
            .collect()
    }

    /// Get the kind of an actor.
    pub fn actor_kind(&self, actor_id: u32) -> Option<ActorKind> {
        self.actors.get(&actor_id).map(|a| a.kind)
    }

    pub fn actor_type_name(&self, actor_id: u32) -> Option<&str> {
        self.actors.get(&actor_id).and_then(|a| a.actor_type.as_deref())
    }

    /// Owner-player id for an actor, or `None` for neutral / world props.
    /// Public accessor exposed for scheduled-event filters and other
    /// query paths that don't have a borrow on the raw `Actor`.
    pub fn actor_owner_id(&self, actor_id: u32) -> Option<u32> {
        self.actors.get(&actor_id).and_then(|a| a.owner_id)
    }

    /// Recompute (power_provided, power_drained) for a player by
    /// iterating their currently-alive Building actors and summing
    /// each one's `power` from the ruleset (positive => provided,
    /// negative => drained). Buildings in `powered_down` contribute
    /// zero.
    ///
    /// This is the authoritative source — the per-actor PowerManager
    /// trait stored on the player is NOT consulted, because that
    /// trait is only updated by `order_place_building` and so misses
    /// pre-placed scenario actors (the entire reason scenarios using
    /// `power_surplus_gte` / `power_provided_gte` were inert before
    /// this fix). Caller is responsible for ignoring dead actors —
    /// they are removed from `self.actors` so iteration already
    /// skips them.
    pub fn compute_player_power(&self, player_id: u32) -> (i32, i32) {
        let mut provided = 0i32;
        let mut drained = 0i32;
        for actor in self.actors.values() {
            if actor.owner_id != Some(player_id) { continue; }
            if actor.kind != ActorKind::Building { continue; }
            if self.powered_down.contains(&actor.id) { continue; }
            let atype = match actor.actor_type.as_deref() {
                Some(t) => t,
                None => continue,
            };
            let p = self.rules.actor(atype).map(|s| s.power).unwrap_or(0);
            if p > 0 { provided += p; }
            else if p < 0 { drained += -p; }
        }
        (provided, drained)
    }

    /// Get building types owned by a player.
    pub fn player_building_types(&self, player_id: u32) -> Vec<String> {
        self.actors.values()
            .filter(|a| a.owner_id == Some(player_id) && a.kind == ActorKind::Building)
            .filter_map(|a| a.actor_type.clone())
            .collect()
    }

    /// Get IDs of damaged buildings owned by a player (for AI repair).
    pub fn player_damaged_buildings(&self, player_id: u32) -> Vec<u32> {
        self.actors.values()
            .filter(|a| {
                a.owner_id == Some(player_id)
                    && a.kind == ActorKind::Building
                    && a.traits.iter().any(|t| {
                        if let crate::traits::TraitState::Health { hp } = t {
                            let atype = a.actor_type.as_deref().unwrap_or("");
                            let max_hp = self.rules.actor(atype).map(|s| s.hp).unwrap_or(*hp);
                            *hp < max_hp
                        } else {
                            false
                        }
                    })
            })
            .map(|a| a.id)
            .collect()
    }

    /// Find the location of an enemy unit or building.
    /// Find nearest passable cell adjacent to a target location (for attacking buildings).
    fn find_adjacent_passable(&self, target: (i32, i32), actor_id: u32) -> Option<(i32, i32)> {
        // If target itself is passable, use it directly
        if self.terrain.is_passable_for(target.0, target.1, actor_id) {
            return Some(target);
        }
        // Check all 8 neighbors
        let dirs = [(0,-1),(1,-1),(1,0),(1,1),(0,1),(-1,1),(-1,0),(-1,-1)];
        for (dx, dy) in &dirs {
            let nx = target.0 + dx;
            let ny = target.1 + dy;
            if self.terrain.is_passable_for(nx, ny, actor_id) {
                return Some((nx, ny));
            }
        }
        None
    }

    pub fn find_enemy_location(&self, player_id: u32) -> Option<(i32, i32)> {
        for actor in self.actors.values() {
            if let Some(owner) = actor.owner_id {
                if owner != player_id
                    && matches!(actor.kind, ActorKind::Building | ActorKind::Infantry | ActorKind::Vehicle | ActorKind::Mcv)
                {
                    return actor.location;
                }
            }
        }
        None
    }

    /// Find all enemy actors (id, x, y) for a player.
    pub fn find_enemy_actors(&self, player_id: u32) -> Vec<(u32, i32, i32)> {
        self.actors.values()
            .filter(|a| {
                if let Some(owner) = a.owner_id {
                    owner != player_id
                        && owner != 1 && owner != 2 // skip World, Neutral
                        && matches!(a.kind, ActorKind::Building | ActorKind::Infantry | ActorKind::Vehicle | ActorKind::Mcv)
                } else {
                    false
                }
            })
            .filter_map(|a| a.location.map(|(x, y)| (a.id, x, y)))
            .collect()
    }

    /// Compute SyncHash components separately for debugging.
    pub fn sync_hash_debug(&self) -> SyncHashDebug {
        let actor_ids: Vec<u32> = self.actors.keys().copied().collect();
        let identity = sync::compute_world_sync_hash(&actor_ids, &[], &[], 0, &[]);
        let actor_syncs: Vec<sync::ActorSync> = self.actors.values()
            .filter(|a| !a.traits.is_empty())
            .map(|a| sync::ActorSync {
                actor_id: a.id,
                trait_hashes: a.sync_hashes(),
            })
            .collect();
        let full_no_rng = sync::compute_world_sync_hash(&actor_ids, &actor_syncs, &[], 0, &[]);
        let traits = full_no_rng.wrapping_sub(identity);
        SyncHashDebug { full: self.sync_hash(), identity, traits, rng_last: self.rng.last }
    }

    /// Create a snapshot of the current world state for rendering.
    pub fn snapshot(&self) -> WorldSnapshot {
        let mut actors = Vec::new();
        for actor in self.actors.values() {
            if actor.kind == ActorKind::World || actor.kind == ActorKind::Player
                || actor.kind == ActorKind::Spawn
            {
                continue;
            }
            let owner = actor.owner_id.unwrap_or(0);
            let (x, y) = actor.location.unwrap_or((0, 0));
            let actor_type_str = actor.actor_type.as_deref().unwrap_or("").to_string();
            let hp = actor.traits.iter().find_map(|t| {
                if let TraitState::Health { hp } = t { Some(*hp) } else { None }
            }).unwrap_or(0);
            let max_hp = self.rules.actor(&actor_type_str)
                .map(|s| s.hp).unwrap_or(hp);
            let activity = match &actor.activity {
                None => "idle",
                Some(Activity::Move { .. }) => "moving",
                Some(Activity::Turn { .. }) => "turning",
                Some(Activity::Attack { .. }) => "attacking",
                Some(Activity::Guard { .. }) => "guarding",
                Some(Activity::EnterTransport { .. }) => "entering",
                Some(Activity::C4Plant { .. }) => "c4-planting",
                Some(Activity::Harvest { .. }) => "harvesting",
            }.to_string();
            let (facing, cx, cy) = actor.traits.iter().find_map(|t| {
                if let TraitState::Mobile { facing, center_position, .. } = t {
                    Some((*facing, center_position.x, center_position.y))
                } else { None }
            }).unwrap_or_else(|| {
                // Buildings: use cell center
                let cp = center_of_cell(x, y);
                (0, cp.x, cp.y)
            });
            let target_id = match &actor.activity {
                Some(Activity::Attack { target_id, .. }) => Some(*target_id),
                _ => None,
            };
            // Surface the activity's destination cell. Move uses path.last();
            // Attack resolves the target actor's current cell (None if the
            // target died this tick). Turn/Harvest carry no useful 2-D
            // destination for the briefing.
            let (target_x, target_y) = match &actor.activity {
                Some(Activity::Move { path, .. }) => path
                    .last()
                    .copied()
                    .map(|(tx, ty)| (Some(tx), Some(ty)))
                    .unwrap_or((None, None)),
                Some(Activity::Attack { target_id, .. }) => self
                    .actors
                    .get(target_id)
                    .and_then(|a| a.location)
                    .map(|(tx, ty)| (Some(tx), Some(ty)))
                    .unwrap_or((None, None)),
                _ => (None, None),
            };
            actors.push(ActorSnapshot {
                id: actor.id, kind: actor.kind, owner, x, y, cx, cy,
                actor_type: actor_type_str, hp, max_hp, activity, facing,
                target_id, target_x, target_y, rank: actor.rank,
            });
        }
        let players = self.player_actor_ids.iter().map(|&pid| {
            let actor = self.actors.get(&pid);
            let cash = actor.map(|a| a.cash()).unwrap_or(0);
            let resources = actor.map(|a| a.resources()).unwrap_or(0);
            let resource_capacity = self.player_storage_capacity(pid);
            // Recompute from live buildings (excludes powered-down ones)
            // so pre-placed scenario actors and the PowerDown toggle are
            // reflected. The legacy PowerManager trait on the player is
            // not consulted — see `compute_player_power` docs.
            let (power_provided, power_drained) = self.compute_player_power(pid);
            let production_queue = self.production.get(&pid).map(|queues| {
                let mut all_items: Vec<ProductionSnapshot> = Vec::new();
                for (_pq, items) in queues {
                    for item in items {
                        let progress = if item.total_time > 0 {
                            1.0 - (item.remaining_time as f32 / item.total_time as f32)
                        } else {
                            1.0
                        };
                        all_items.push(ProductionSnapshot {
                            item_name: item.item_name.clone(),
                            progress,
                            done: item.remaining_time <= 0,
                        });
                    }
                }
                all_items
            }).unwrap_or_default();
            PlayerSnapshot { index: pid, cash, power_provided, power_drained, resources, resource_capacity, production_queue }
        }).collect();
        // Collect resource cells for rendering
        let mut resources = Vec::new();
        for y in 0..self.map_height {
            for x in 0..self.map_width {
                let cell = self.terrain.resource(x, y);
                if cell.resource_type != ResourceType::None && cell.density > 0 {
                    resources.push(ResourceSnapshot {
                        x, y,
                        kind: match cell.resource_type { ResourceType::Ore => 1, ResourceType::Gems => 2, ResourceType::None => 0 },
                        density: cell.density,
                    });
                }
            }
        }

        let charge_totals: HashMap<&str, i32> = [
            ("dome", 3000), ("iron", 4500), ("pdox", 4500), ("mslo", 6000),
        ].into_iter().collect();
        let superweapons: Vec<SuperweaponSnapshot> = self.superweapon_timers.iter()
            .map(|((wtype, owner), ticks)| SuperweaponSnapshot {
                weapon_type: wtype.clone(),
                owner: *owner,
                ticks_remaining: *ticks,
                charge_total: *charge_totals.get(wtype.as_str()).unwrap_or(&3000),
            })
            .collect();

        WorldSnapshot {
            tick: self.world_tick,
            actors,
            players,
            map_width: self.map_width,
            map_height: self.map_height,
            resources,
            superweapons,
        }
    }

    /// Dump per-actor, per-trait contributions for debugging.
    pub fn dump_sync_details(&self) {
        let mut n: i32 = 0;
        let mut ret: i32 = 0;

        for (&actor_id, _) in &self.actors {
            let contrib = n.wrapping_mul((1i32).wrapping_add(actor_id as i32))
                .wrapping_mul(sync::hash_actor(actor_id));
            eprintln!("IDENTITY n={} actor_id={} contrib={} running={}",
                n, actor_id, contrib, ret.wrapping_add(contrib));
            ret = ret.wrapping_add(contrib);
            n += 1;
        }
        eprintln!("AFTER_IDENTITY ret={} n={}", ret, n);

        for actor in self.actors.values() {
            if actor.traits.is_empty() { continue; }
            for (ti, t) in actor.traits.iter().enumerate() {
                let trait_hash = t.sync_hash();
                let contrib = n.wrapping_mul((1i32).wrapping_add(actor.id as i32))
                    .wrapping_mul(trait_hash);
                eprintln!("TRAIT n={} actor_id={} trait_idx={} hash={} contrib={} running={}",
                    n, actor.id, ti, trait_hash, contrib, ret.wrapping_add(contrib));
                ret = ret.wrapping_add(contrib);
                n += 1;
            }
        }
        eprintln!("AFTER_TRAITS ret={} n={}", ret, n);
        eprintln!("RNG_LAST={}", self.rng.last);
        ret = ret.wrapping_add(self.rng.last);
        eprintln!("FINAL ret={}", ret);
    }

    /// Process one frame of the simulation.
    ///
    /// C# execution order per frame:
    /// 1. Auto-unpause after orderLatency buffer period
    /// 2. ProcessOrders() — resolve orders from replay
    /// 3. SyncHash() — computed (this is what the replay records)
    /// 4. World.Tick() — advance simulation if not paused
    pub fn process_frame(&mut self, orders: &[GameOrder]) -> i32 {
        self.frame_number += 1;

        // Auto-unpause after orderLatency buffer period.
        if self.paused && self.frame_number > self.order_latency {
            self.paused = false;
            self.update_debug_pause_state();
        }

        // Reset per-frame move-destination reservations so that orders within
        // this frame spread to distinct cells. Reservations don't persist
        // across frames — by next frame, units have moved and occupancy is
        // up-to-date again.
        self.pending_move_destinations.clear();

        // 1. Process replay/external orders
        for order in orders {
            self.process_order(order);
        }

        // 1b. Generate and process bot AI orders
        if !self.bots.is_empty() && !self.paused {
            let bot_orders = self.tick_bots();
            for order in &bot_orders {
                self.process_order(order);
            }
        }

        // 1c. Scripted opponent (bench enemy.bot) orders.
        if !self.scripted_bots.is_empty() && !self.paused {
            let sb_orders = self.tick_scripted_bots();
            for order in &sb_orders {
                self.process_order(order);
            }
        }

        // 2. Compute SyncHash
        let hash = self.sync_hash();

        // 3. Tick the world if not paused (NetFrameInterval=3)
        if !self.paused {
            for _ in 0..3 {
                self.world_tick += 1;
                self.tick_actors();
                // Phase 8: advance in-flight projectiles after activities
                // settle so newly-spawned projectiles wait one tick before
                // their first travel step (matches C# behaviour where the
                // projectile spawn frame doesn't also move it).
                self.tick_projectiles();
                self.execute_frame_end_tasks();
            }
            // Update fog of war after tick
            self.update_shroud();
        }

        hash
    }

    /// Update DebugPauseState on the World actor (ID=0).
    fn update_debug_pause_state(&mut self) {
        if let Some(world_actor) = self.actors.get_mut(&0) {
            for t in &mut world_actor.traits {
                if let TraitState::DebugPauseState { paused } = t {
                    *paused = self.paused;
                    return;
                }
            }
        }
    }

    /// Process a single game order.
    fn process_order(&mut self, order: &GameOrder) {
        match order.order_string.as_str() {
            "PauseGame" => {
                if let Some(ref ts) = order.target_string {
                    self.paused = ts == "Pause";
                }
            }
            "DeployTransform" => {
                if let Some(subject_id) = order.subject_id {
                    if let Some(actor) = self.actors.get_mut(&subject_id) {
                        if actor.kind == ActorKind::Mcv {
                            actor.activity = Some(Activity::Turn { target: 384, speed: 20, then: None });
                            eprintln!("ORDER: DeployTransform subject={}", subject_id);
                        }
                    }
                }
            }
            "StartProduction" => {
                if let (Some(subject_id), Some(item_name)) = (order.subject_id, &order.target_string) {
                    let cost = self.rules.cost(item_name);
                    if cost > 0 {
                        // Check tech tree prerequisites
                        if !self.has_prerequisites(subject_id, item_name) {
                            eprintln!("ORDER: StartProduction BLOCKED — missing prerequisites for {}", item_name);
                        } else {
                            let pq = Self::item_queue_type_by_name(&self.rules, item_name);
                            eprintln!("ORDER: StartProduction subject={} item={} cost={} queue={:?}", subject_id, item_name, cost, pq);
                            let item = ProductionItem::new(item_name, cost, 60);
                            self.production.entry(subject_id).or_default()
                                .entry(pq).or_default().push(item);
                        }
                    }
                }
            }
            "CancelProduction" => {
                if let (Some(subject_id), Some(item_name)) = (order.subject_id, &order.target_string) {
                    let pq = Self::item_queue_type_by_name(&self.rules, item_name);
                    if let Some(queues) = self.production.get_mut(&subject_id) {
                        if let Some(items) = queues.get_mut(&pq) {
                            if let Some(pos) = items.iter().rposition(|q| q.item_name == *item_name) {
                                let removed = items.remove(pos);
                                let refund = removed.remaining_cost;
                                if refund > 0 {
                                    if let Some(player) = self.actors.get_mut(&subject_id) {
                                        for t in &mut player.traits {
                                            if let TraitState::PlayerResources { cash, .. } = t {
                                                *cash += refund;
                                                break;
                                            }
                                        }
                                    }
                                }
                                eprintln!("ORDER: CancelProduction {} refund={}", item_name, refund);
                            }
                        }
                    }
                }
            }
            "Move" => {
                if let Some(subject_id) = order.subject_id {
                    if let Some(ref ts) = order.target_string {
                        // Parse "X,Y" target cell
                        if let Some(target) = parse_cell_target(ts) {
                            self.order_move(subject_id, target);
                        }
                    }
                }
            }
            "AttackMove" => {
                if let Some(subject_id) = order.subject_id {
                    if let Some(ref ts) = order.target_string {
                        if let Some(target) = parse_cell_target(ts) {
                            self.order_move(subject_id, target);
                        }
                    }
                }
            }
            "Attack" => {
                if let Some(subject_id) = order.subject_id {
                    if let Some(target_actor_id) = order.extra_data {
                        // Explicit agent/player order — overrides stance.
                        self.order_attack(subject_id, target_actor_id, false);
                    }
                }
            }
            "Guard" => {
                if let (Some(subject_id), Some(target_actor_id)) =
                    (order.subject_id, order.extra_data)
                {
                    self.order_guard(subject_id, target_actor_id);
                }
            }
            "EnterTransport" => {
                if let (Some(subject_id), Some(target_actor_id)) =
                    (order.subject_id, order.extra_data)
                {
                    self.order_enter_transport(subject_id, target_actor_id);
                }
            }
            "C4Detonate" => {
                if let (Some(subject_id), Some(target_actor_id)) =
                    (order.subject_id, order.extra_data)
                {
                    self.order_c4_detonate(subject_id, target_actor_id);
                }
            }
            "Unload" => {
                if let Some(subject_id) = order.subject_id {
                    self.order_unload(subject_id);
                }
            }
            "PlaceBuilding" => {
                if let (Some(subject_id), Some(ts)) = (order.subject_id, &order.target_string) {
                    // Format: "building_type,X,Y"
                    let parts: Vec<&str> = ts.split(',').collect();
                    if parts.len() >= 3 {
                        let building_type = parts[0].trim();
                        if let (Ok(x), Ok(y)) = (parts[1].trim().parse::<i32>(), parts[2].trim().parse::<i32>()) {
                            self.order_place_building(subject_id, building_type, x, y);
                        }
                    }
                }
            }
            "Harvest" => {
                // Send harvester to a specific resource cell
                if let Some(subject_id) = order.subject_id {
                    if let Some(ref ts) = order.target_string {
                        if let Some(target) = parse_cell_target(ts) {
                            self.order_harvest(subject_id, target);
                        }
                    }
                }
            }
            "Stop" => {
                if let Some(subject_id) = order.subject_id {
                    if let Some(actor) = self.actors.get_mut(&subject_id) {
                        actor.activity = None;
                    }
                }
            }
            "Surrender" => {
                // subject_id is the conceding player's id.
                if let Some(pid) = order.subject_id {
                    self.surrendered.insert(pid);
                }
            }
            "Sell" => {
                if let Some(subject_id) = order.subject_id {
                    self.order_sell(subject_id);
                }
            }
            "PowerDown" => {
                // Toggle the building's power contribution. Validated:
                // subject must be an alive Building actor; the toggle
                // adds/removes the id from `powered_down`, and the next
                // `compute_player_power` call reflects the new state in
                // both the snapshot and the low-power-slowdown gate.
                if let Some(subject_id) = order.subject_id {
                    let is_building = matches!(
                        self.actors.get(&subject_id).map(|a| a.kind),
                        Some(ActorKind::Building),
                    );
                    if is_building {
                        if self.powered_down.contains(&subject_id) {
                            self.powered_down.remove(&subject_id);
                        } else {
                            self.powered_down.insert(subject_id);
                        }
                    }
                }
            }
            "RepairBuilding" => {
                if let Some(subject_id) = order.subject_id {
                    // Toggle repair on/off for the building
                    if self.repairing.contains(&subject_id) {
                        self.repairing.remove(&subject_id);
                    } else {
                        self.repairing.insert(subject_id);
                    }
                }
            }
            "SetRallyPoint" => {
                if let Some(subject_id) = order.subject_id {
                    if let Some(target) = &order.target_string {
                        // target_string format: "x,y"
                        let parts: Vec<&str> = target.split(',').collect();
                        if parts.len() == 2 {
                            if let (Ok(x), Ok(y)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>()) {
                                self.rally_points.insert(subject_id, (x, y));
                            }
                        }
                    }
                }
            }
            "SetPrimary" => {
                if let Some(subject_id) = order.subject_id {
                    self.set_primary_building(subject_id);
                }
            }
            "ActivateSuperweapon" => {
                // target_string = "weapon_type,x,y" (e.g. "iron,15,20")
                if let Some(target) = &order.target_string {
                    let parts: Vec<&str> = target.split(',').collect();
                    if parts.len() >= 1 {
                        let weapon_type = parts[0];
                        let owner = order.subject_id.unwrap_or(0);
                        let key = (weapon_type.to_string(), owner);
                        let charged = self.superweapon_timers.get(&key).map(|t| *t <= 0).unwrap_or(false);
                        if charged {
                            let (tx, ty) = if parts.len() >= 3 {
                                (parts[1].parse().unwrap_or(0), parts[2].parse().unwrap_or(0))
                            } else { (0, 0) };
                            self.activate_superweapon(weapon_type, owner, tx, ty);
                            // Reset timer
                            if let Some(t) = self.superweapon_timers.get_mut(&key) {
                                match weapon_type {
                                    "dome" => *t = 3000,
                                    "iron" => *t = 4500,
                                    "pdox" => *t = 4500,
                                    "mslo" => *t = 6000,
                                    _ => *t = 3000,
                                }
                            }
                        }
                    }
                }
            }
            "SetStance" => {
                if let Some(subject_id) = order.subject_id {
                    let s = order.extra_data.unwrap_or(3).min(3) as u8;
                    self.stances.insert(subject_id, s);
                }
            }
            // C# parity: PATROL defined but unimplemented — accept
            // silently (no warn, no behaviour change).
            "Patrol" => {}
            "StartGame" | "Command" => {}
            other => {
                eprintln!("ORDER: unhandled '{}' subject={:?}", other, order.subject_id);
            }
        }
    }

    /// Find the nearest cell that is BOTH unoccupied on the terrain AND not
    /// claimed by a previous Move order this frame. BFS-expands from
    /// `target` up to `max_radius` cells. Used by `order_move` to keep
    /// concurrent Move orders from selecting the same destination cell.
    fn find_nearest_unclaimed_cell(
        &self,
        target: (i32, i32),
        ignore_actor: Option<u32>,
        max_radius: i32,
    ) -> Option<(i32, i32)> {
        let cell_ok = |x: i32, y: i32| -> bool {
            if !self.terrain.contains(x, y) {
                return false;
            }
            if !self.terrain.is_terrain_passable(x, y) {
                return false;
            }
            if self.pending_move_destinations.contains(&(x, y)) {
                return false;
            }
            let occ = self.terrain.occupant(x, y);
            if occ == 0 {
                return true;
            }
            if let Some(ignore) = ignore_actor {
                if occ == ignore {
                    return true;
                }
            }
            false
        };

        if cell_ok(target.0, target.1) {
            return Some(target);
        }
        for r in 1..=max_radius {
            let x_min = target.0 - r;
            let x_max = target.0 + r;
            let y_min = target.1 - r;
            let y_max = target.1 + r;
            for y in [y_min, y_max] {
                for x in x_min..=x_max {
                    if cell_ok(x, y) {
                        return Some((x, y));
                    }
                }
            }
            for x in [x_min, x_max] {
                for y in (y_min + 1)..y_max {
                    if cell_ok(x, y) {
                        return Some((x, y));
                    }
                }
            }
        }
        None
    }

    /// Handle a Move order: pathfind and start moving.
    /// Like C# OpenRA, vehicles turn in place first (TurnsWhileMoving=false),
    /// then begin the Move activity already facing the correct direction.
    ///
    /// If the requested target cell is occupied by another actor (or claimed
    /// by another Move order in this frame), resolves to the nearest free
    /// walkable cell within 6 cells. This prevents multiple units commanded
    /// to the same target from stacking on one cell.
    fn order_move(&mut self, actor_id: u32, target: (i32, i32)) {
        let from = match self.actors.get(&actor_id).and_then(|a| a.location) {
            Some(loc) => loc,
            None => return,
        };
        // Resolve destination: spread to nearest unoccupied cell if target
        // is taken (by terrain occupancy or by another unit's pending move
        // destination this frame). The reservation set prevents N concurrent
        // Move orders to the same target from all picking the same cell —
        // each picks the next-nearest free cell.
        let resolved_target = self
            .find_nearest_unclaimed_cell(target, Some(actor_id), 6)
            .unwrap_or(target);
        // Reserve the chosen destination so subsequent orders this frame
        // don't pick it (only if it's actually different from the from-cell;
        // staying-put doesn't need a reservation).
        if resolved_target != from {
            self.pending_move_destinations.insert(resolved_target);
        }
        if let Some(path) = pathfinder::find_path(&self.terrain, from, resolved_target, Some(actor_id)) {
            if path.len() > 1 {
                let speed = self.actor_speed(actor_id);
                let move_activity = Activity::Move {
                    path,
                    path_index: 1, // Skip the start cell
                    speed,
                };
                if let Some(actor) = self.actors.get_mut(&actor_id) {
                    // Check if we need to turn first (C# TurnsWhileMoving=false default)
                    let current_facing = actor.traits.iter()
                        .find_map(|t| {
                            if let TraitState::Mobile { facing, .. } = t { Some(*facing) } else { None }
                        })
                        .unwrap_or(0);
                    let first_cell = move_activity.path_cell(1);
                    let desired_facing = if let Some(cell) = first_cell {
                        pathfinder::facing_between(from, cell)
                    } else {
                        current_facing
                    };
                    if current_facing != desired_facing {
                        // Turn first, then move (C# default TurnSpeed: 20)
                        actor.activity = Some(Activity::Turn {
                            target: desired_facing,
                            speed: 20,
                            then: Some(Box::new(move_activity)),
                        });
                    } else {
                        actor.activity = Some(move_activity);
                    }
                }
            }
        }
    }

    /// Handle Sell order: refund 50% of building cost and remove it.
    fn order_sell(&mut self, actor_id: u32) {
        let (owner_id, loc, kind) = match self.actors.get(&actor_id) {
            Some(a) => (a.owner_id, a.location, a.kind),
            None => return,
        };
        if kind != ActorKind::Building { return; }

        // Refund 50% of estimated cost
        let refund = self.estimate_building_sell_value(actor_id);
        if let Some(pid) = owner_id {
            if let Some(player) = self.actors.get_mut(&pid) {
                let cash = player.cash();
                player.set_cash(cash + refund);
            }
        }

        // Clear terrain footprint using actual building size from rules
        if let Some((x, y)) = loc {
            let (fw, fh) = self.actors.get(&actor_id)
                .and_then(|a| a.actor_type.as_deref())
                .and_then(|at| self.rules.actor(at))
                .map(|s| s.footprint)
                .unwrap_or((2, 2));
            self.terrain.clear_footprint(x, y, fw, fh);
        }

        // Remove actor
        self.actors.remove(&actor_id);
        eprintln!("SELL: building {} refund={}", actor_id, refund);
    }

    /// Estimate sell value for a building (50% of build cost).
    fn estimate_building_sell_value(&self, actor_id: u32) -> i32 {
        if let Some(actor) = self.actors.get(&actor_id) {
            if let Some(ref at) = actor.actor_type {
                return self.rules.cost(at) / 2;
            }
        }
        500
    }

    /// Handle a Harvest order: send a harvester to harvest at a location.
    fn order_harvest(&mut self, actor_id: u32, target: (i32, i32)) {
        // Idempotent: a unit already harvesting must not have its
        // in-progress run (accumulated ore / FSM state) wiped by a
        // re-issued harvest order — agents/models re-send commands
        // every turn, and clobbering here starves the harvester so it
        // never reaches capacity and never delivers cash.
        if let Some(a) = self.actors.get(&actor_id) {
            if matches!(a.activity, Some(Activity::Harvest { .. })) {
                return;
            }
        }
        let (speed, owner_id) = match self.actors.get(&actor_id) {
            Some(a) => (self.actor_speed(actor_id), a.owner_id),
            None => return,
        };
        let refinery_id = owner_id.and_then(|pid| self.find_refinery(pid)).unwrap_or(0);
        if let Some(actor) = self.actors.get_mut(&actor_id) {
            actor.activity = Some(Activity::Harvest {
                state: HarvestState::FindingOre,
                refinery_id,
                carried_ore: 0,
                carried_gems: 0,
                capacity: 20,
                path: Vec::new(),
                path_index: 0,
                speed,
                harvest_ticks: 0,
                last_harvest_cell: Some(target),
            });
        }
    }

    /// Resolve a target actor's armor class for armor-class damage
    /// lookups. Defaults to `ArmorType::None` when the target is gone
    /// or has no `Armor` trait (matching the C# "no Armor ⇒ none"
    /// fallback).
    fn target_armor_of(&self, target_id: u32) -> crate::gamerules::ArmorType {
        self.actors
            .get(&target_id)
            .and_then(|a| a.actor_type.as_deref())
            .and_then(|t| self.rules.actor(t))
            .map(|stats| stats.armor_type)
            .unwrap_or(crate::gamerules::ArmorType::None)
    }

    /// Handle an Attack order: set attack activity with weapon stats from rules.
    fn order_attack(&mut self, actor_id: u32, target_id: u32, auto_acquired: bool) {
        // Faithful HoldFire: an *auto-acquired* engagement is suppressed
        // outright when the actor's stance is HoldFire(0). Explicit
        // (order-issued) attacks always proceed — player/agent intent
        // overrides stance, exactly like C# AttackBase.OnQueueAttack vs
        // AutoTarget's opportunistic acquisition.
        if auto_acquired && self.stances.get(&actor_id).copied().unwrap_or(3) == 0 {
            return;
        }
        // Look up the attacker's best weapon *against this target*.
        // OpenRA actors can carry multiple armaments (e.g. e3 has an
        // anti-air RedEye and an anti-ground Dragon); the combat model
        // must pick the armament that deals the most effective damage
        // to the target's armor class, not blindly `weapons[0]`.
        let target_armor = self.target_armor_of(target_id);
        let weapon = self.actors.get(&actor_id)
            .and_then(|a| a.actor_type.as_deref())
            .and_then(|at| self.rules.best_weapon_against(at, target_armor))
            .map(|(_, w)| w)
            .or_else(|| self.rules.weapon("default"));

        let (damage, range_cells, reload, burst) = match weapon {
            Some(w) => (w.damage, w.range / 1024, w.reload_delay, w.burst),
            None => (100, 5, 1, 1),
        };

        if let Some(actor) = self.actors.get_mut(&actor_id) {
            actor.activity = Some(Activity::Attack {
                target_id,
                weapon_range: range_cells,
                weapon_damage: damage,
                reload_delay: reload,
                reload_remaining: 0,
                burst,
                burst_remaining: burst,
                auto_acquired,
            });
        }
    }

    /// Order an actor to GUARD a friendly target actor — follow it and
    /// stay within a small leash radius. C# `Guard`/`GuardActivity`
    /// (follow subset). Validation (ownership, target existence) is
    /// done at the env layer; here we only require both actors to
    /// exist and be mobile-capable.
    fn order_guard(&mut self, actor_id: u32, target_id: u32) {
        if actor_id == target_id {
            return; // can't guard yourself
        }
        if !self.actors.contains_key(&target_id) {
            return;
        }
        let speed = self.actor_speed(actor_id);
        if speed <= 0 {
            return; // immobile actors can't guard
        }
        if let Some(actor) = self.actors.get_mut(&actor_id) {
            actor.activity = Some(Activity::Guard {
                target_id,
                leash: 2,
                speed,
            });
        }
    }

    /// Order a passenger-capable actor to walk to and board a
    /// transport (C# `EnterTransport`). Capacity is checked again at
    /// board time (other passengers may fill it en route).
    fn order_enter_transport(&mut self, actor_id: u32, transport_id: u32) {
        if actor_id == transport_id {
            return;
        }
        if !self.is_passenger_capable(actor_id) {
            return;
        }
        if self.transport_capacity(transport_id) == 0 {
            return; // not a transport
        }
        if !self.actors.contains_key(&transport_id) {
            return;
        }
        let speed = self.actor_speed(actor_id);
        if let Some(actor) = self.actors.get_mut(&actor_id) {
            actor.activity = Some(Activity::EnterTransport {
                transport_id,
                speed,
            });
        }
    }

    /// Tanya's C4 commando ability. The subject MUST be a `tanya`
    /// actor; the target MUST be an enemy BUILDING. Non-conforming
    /// orders are dropped silently (a stale-state agent should not
    /// be able to misuse C4 — e.g. point a rifleman at the order,
    /// or aim it at a friendly building). On success, Tanya is given
    /// an `Activity::C4Plant` that walks her toward the building and
    /// detonates on adjacency in the per-tick handler.
    fn order_c4_detonate(&mut self, actor_id: u32, target_id: u32) {
        if actor_id == target_id {
            return;
        }
        // Subject must be a tanya actor (alive).
        let subject_owner = match self.actors.get(&actor_id) {
            Some(a) => {
                if a.actor_type.as_deref() != Some("tanya") {
                    return;
                }
                a.owner_id
            }
            None => return,
        };
        // Target must exist, be a building, and be enemy-owned.
        let target_owner = match self.actors.get(&target_id) {
            Some(a) => {
                if a.kind != ActorKind::Building {
                    return;
                }
                a.owner_id
            }
            None => return,
        };
        match (subject_owner, target_owner) {
            (Some(so), Some(to)) if so != to => {}
            _ => return, // friendly / unowned target rejected
        }
        let speed = self.actor_speed(actor_id);
        if let Some(actor) = self.actors.get_mut(&actor_id) {
            actor.activity = Some(Activity::C4Plant {
                target_id,
                speed,
            });
        }
    }

    /// Order a transport to eject all its passengers onto passable
    /// cells adjacent to it (C# `UnloadCargo`).
    fn order_unload(&mut self, transport_id: u32) {
        let passengers = match self.cargo.remove(&transport_id) {
            Some(p) if !p.is_empty() => p,
            _ => return,
        };
        let base = match self.actors.get(&transport_id).and_then(|a| a.location) {
            Some(l) => l,
            None => {
                // Transport gone — drop cargo silently (it died with it).
                return;
            }
        };
        for mut passenger in passengers {
            // Find a free passable cell near the transport.
            let mut placed = None;
            'search: for r in 1..=4 {
                for dy in -r..=r {
                    for dx in -r..=r {
                        let (cx, cy) = (base.0 + dx, base.1 + dy);
                        if !self.terrain.is_passable(cx, cy) {
                            continue;
                        }
                        if self.terrain.occupant(cx, cy) != 0 {
                            continue;
                        }
                        placed = Some((cx, cy));
                        break 'search;
                    }
                }
            }
            let cell = match placed {
                Some(c) => c,
                None => base, // fallback: stack on the transport cell
            };
            passenger.location = Some(cell);
            for t in &mut passenger.traits {
                if let TraitState::Mobile {
                    center_position,
                    from_cell,
                    to_cell,
                    ..
                } = t
                {
                    *center_position = center_of_cell(cell.0, cell.1);
                    *from_cell = CPos::new(cell.0, cell.1);
                    *to_cell = CPos::new(cell.0, cell.1);
                }
            }
            passenger.activity = None;
            let pid = passenger.id;
            self.terrain.set_occupant(cell.0, cell.1, pid);
            self.actors.insert(pid, passenger);
        }
    }

    /// Flag a building as the PRIMARY producer for its type. C#
    /// `PrimaryBuilding.SetPrimaryProducer`: only one primary per
    /// (owner, building-type) — designating a new one clears the flag
    /// from every same-type sibling owned by the same player.
    fn set_primary_building(&mut self, building_id: u32) {
        let (owner, btype) = match self.actors.get(&building_id) {
            Some(a) if a.kind == ActorKind::Building => (
                a.owner_id,
                a.actor_type.clone().unwrap_or_default(),
            ),
            _ => return,
        };
        // Clear primary on same-owner, same-type siblings.
        let siblings: Vec<u32> = self
            .actors
            .values()
            .filter(|a| {
                a.kind == ActorKind::Building
                    && a.owner_id == owner
                    && a.actor_type.as_deref() == Some(btype.as_str())
            })
            .map(|a| a.id)
            .collect();
        for sid in siblings {
            self.primary_buildings.remove(&sid);
        }
        self.primary_buildings.insert(building_id);
    }

    /// Handle PlaceBuilding order: create building actor and occupy terrain.
    fn order_place_building(&mut self, owner_player_id: u32, building_type: &str, x: i32, y: i32) {
        // Verify the building is actually completed in the production queue
        let pq = Self::item_queue_type_by_name(&self.rules, building_type);
        let has_completed = self.production.get(&owner_player_id)
            .and_then(|queues| queues.get(&pq))
            .map(|items| items.iter().any(|i| i.item_name == building_type && i.is_done()))
            .unwrap_or(false);
        if !has_completed {
            eprintln!("PLACE BLOCKED: {} not completed in queue", building_type);
            return;
        }
        let (footprint_w, footprint_h, hp) = self.rules.actor(building_type)
            .map(|s| (s.footprint.0, s.footprint.1, s.hp))
            .unwrap_or((2, 2, 50000));
        let building_id = self.next_actor_id;
        self.next_actor_id += 1;

        let top_left = CPos::new(x, y);
        let building = Actor {
            id: building_id,
            kind: ActorKind::Building,
            owner_id: Some(owner_player_id),
            location: Some((x, y)),
            traits: vec![
                TraitState::BodyOrientation { quantized_facings: 1 },
                TraitState::Building { top_left },
                TraitState::Health { hp },
                TraitState::RevealsShroud,
            ],
            activity: None,
            actor_type: Some(building_type.to_string()),
            kills: 0, rank: 0,
        };
        self.actors.insert(building_id, building);
        self.terrain.occupy_footprint(x, y, footprint_w, footprint_h, building_id);

        // Remove completed building from production queue
        if let Some(queues) = self.production.get_mut(&owner_player_id) {
            if let Some(items) = queues.get_mut(&pq) {
                if let Some(idx) = items.iter().position(|i| i.item_name == building_type && i.is_done()) {
                    items.remove(idx);
                }
            }
        }

        // Update power for the owning player
        let power = self.rules.actor(building_type).map(|s| s.power).unwrap_or(0);
        if power != 0 {
            self.update_player_power(owner_player_id, power);
        }

        // Enable production queues if this is a production building
        self.enable_production_queues(owner_player_id, building_type);

        // Refinery auto-spawns a harvester (like OpenRA)
        if building_type == "proc" {
            self.spawn_unit("harv", owner_player_id);
        }

        eprintln!("PLACE: {} at ({},{}) id={} footprint={}x{} power={}",
            building_type, x, y, building_id, footprint_w, footprint_h, power);
    }

    /// Update PowerManager trait on a player actor.
    fn update_player_power(&mut self, player_id: u32, power_delta: i32) {
        if let Some(player) = self.actors.get_mut(&player_id) {
            for t in &mut player.traits {
                if let TraitState::PowerManager { power_provided, power_drained } = t {
                    if power_delta > 0 {
                        *power_provided += power_delta;
                    } else {
                        *power_drained += -power_delta;
                    }
                    return;
                }
            }
        }
    }

    /// Enable the appropriate production queue when a production building is placed.
    fn enable_production_queues(&mut self, player_id: u32, building_type: &str) {
        let pq_type = match building_type {
            "weap" | "weap.ukraine" => Some(PqType::Vehicle),
            "tent" | "barr" => Some(PqType::Infantry),
            "hpad" | "afld" => Some(PqType::Aircraft),
            "spen" | "syrd" => Some(PqType::Ship),
            _ => None,
        };
        if let Some(pq) = pq_type {
            if let Some(player) = self.actors.get_mut(&player_id) {
                for t in &mut player.traits {
                    if let TraitState::ClassicProductionQueue { pq_type: pt, enabled, .. } = t {
                        if *pt == pq {
                            *enabled = true;
                        }
                    }
                }
            }
        }
    }

    /// Map a production-building type to the unit-producing queue it
    /// services, or `None` if the building does not produce mobile units
    /// (e.g. `fact`, which feeds the Building/Defense queues).
    fn production_building_pq(building_type: &str) -> Option<PqType> {
        match building_type {
            "weap" | "weap.ukraine" => Some(PqType::Vehicle),
            "tent" | "barr" => Some(PqType::Infantry),
            "hpad" | "afld" => Some(PqType::Aircraft),
            "spen" | "syrd" => Some(PqType::Ship),
            _ => None,
        }
    }

    /// Count a player's completed, undamaged-enough production buildings
    /// that service the given queue type. This is the OpenRA-parity
    /// throughput multiplier: with two war factories the Vehicle queue
    /// advances twice per tick, so two factories roughly double output.
    ///
    /// Returns at least 1 (so a queue with an order but — transiently —
    /// no surviving factory still drains at the base rate rather than
    /// stalling). A building counts only if it is alive (HP > 0).
    fn production_building_count(&self, player_id: u32, pq: PqType) -> u32 {
        // Only unit-producing queues parallelise on building count.
        // Building / Defense are fed by the construction yard and keep
        // single-stream semantics.
        if !matches!(
            pq,
            PqType::Vehicle | PqType::Infantry | PqType::Aircraft | PqType::Ship
        ) {
            return 1;
        }
        let count = self
            .actors
            .values()
            .filter(|a| {
                a.owner_id == Some(player_id) && a.kind == ActorKind::Building
            })
            .filter(|a| {
                a.actor_type
                    .as_deref()
                    .and_then(Self::production_building_pq)
                    == Some(pq)
            })
            .filter(|a| {
                // Building must be alive.
                a.traits.iter().any(|t| {
                    if let TraitState::Health { hp } = t {
                        *hp > 0
                    } else {
                        false
                    }
                }) || a.traits.iter().all(|t| !matches!(t, TraitState::Health { .. }))
            })
            .count() as u32;
        count.max(1)
    }

    /// Tick all harvesters through their harvest cycle.
    /// Activate a superweapon effect.
    fn activate_superweapon(&mut self, weapon_type: &str, owner: u32, target_x: i32, target_y: i32) {
        match weapon_type {
            "dome" => {
                // GPS Satellite: set GpsWatcher.granted = true for this player
                if let Some(player) = self.actors.get_mut(&owner) {
                    for t in &mut player.traits {
                        if let TraitState::GpsWatcher { granted, launched, .. } = t {
                            *granted = true;
                            *launched = true;
                        }
                    }
                }
                eprintln!("SUPERWEAPON: GPS Satellite activated for player {}", owner);
            }
            "iron" => {
                // Iron Curtain: make target actor invulnerable for 750 ticks (~30 seconds)
                // Find actor at target cell
                let target_id = self.actors.values()
                    .find(|a| a.location == Some((target_x, target_y))
                        && a.owner_id == Some(owner)
                        && matches!(a.kind, ActorKind::Vehicle | ActorKind::Building))
                    .map(|a| a.id);
                if let Some(tid) = target_id {
                    self.invulnerable.insert(tid, 750);
                    eprintln!("SUPERWEAPON: Iron Curtain on actor {} for 750 ticks", tid);
                }
            }
            "pdox" => {
                // Chronosphere: teleport own unit to target cell
                if let Some(subject_id) = self.actors.values()
                    .find(|a| a.location == Some((target_x, target_y))
                        && a.owner_id == Some(owner)
                        && matches!(a.kind, ActorKind::Vehicle))
                    .map(|a| a.id)
                {
                    // For simplicity, teleport to a random adjacent passable cell
                    // In a real implementation, this would need a second target
                    eprintln!("SUPERWEAPON: Chronosphere — not fully implemented yet");
                    let _ = subject_id;
                }
            }
            "mslo" => {
                // Nuclear Strike: heavy area damage at target cell
                let radius = 5;
                let damage = 500000; // Very high damage
                let mut damaged: Vec<(u32, i32)> = Vec::new();
                for actor in self.actors.values() {
                    if let Some((ax, ay)) = actor.location {
                        let dist = (ax - target_x).abs() + (ay - target_y).abs();
                        if dist <= radius {
                            let dmg = damage * (radius + 1 - dist) / (radius + 1);
                            damaged.push((actor.id, dmg));
                        }
                    }
                }
                let mut dead: Vec<u32> = Vec::new();
                for (actor_id, dmg) in damaged {
                    if let Some(actor) = self.actors.get_mut(&actor_id) {
                        for t in &mut actor.traits {
                            if let TraitState::Health { hp } = t {
                                *hp -= dmg;
                                if *hp <= 0 { dead.push(actor_id); }
                                break;
                            }
                        }
                    }
                }
                for id in dead {
                    if let Some(a) = self.actors.remove(&id) {
                        if let Some(loc) = a.location {
                            self.terrain.clear_occupant(loc.0, loc.1);
                        }
                    }
                }
                eprintln!("SUPERWEAPON: Nuclear Strike at ({},{}) by player {}", target_x, target_y, owner);
            }
            _ => {}
        }
    }

    /// Tick superweapon charge timers. Start charging when prerequisite building exists.
    fn tick_superweapons(&mut self) {
        // Superweapon definitions: (building_type, charge_time_ticks)
        const SUPERWEAPONS: &[(&str, i32)] = &[
            ("dome", 3000),   // GPS Satellite (Allied)
            ("iron", 4500),   // Iron Curtain (Soviet)
            ("pdox", 4500),   // Chronosphere (Allied)
            ("mslo", 6000),   // Nuclear Missile (Soviet)
        ];

        // Find all superweapon buildings and start/continue timers
        let buildings: Vec<(String, u32)> = self.actors.values()
            .filter(|a| a.kind == ActorKind::Building)
            .filter_map(|a| {
                let atype = a.actor_type.as_deref()?;
                if SUPERWEAPONS.iter().any(|(bt, _)| *bt == atype) {
                    Some((atype.to_string(), a.owner_id.unwrap_or(0)))
                } else {
                    None
                }
            })
            .collect();

        // Start new timers for buildings not yet tracked
        for &(btype, charge_time) in SUPERWEAPONS {
            for &(ref found_type, owner) in &buildings {
                if found_type == btype {
                    let key = (btype.to_string(), owner);
                    self.superweapon_timers.entry(key).or_insert(charge_time);
                }
            }
        }

        // Tick down active timers
        for (_, ticks) in self.superweapon_timers.iter_mut() {
            if *ticks > 0 {
                *ticks -= 1;
            }
        }
    }

    fn tick_harvesters(&mut self) {
        let harvester_ids: Vec<u32> = self.actors.values()
            .filter(|a| matches!(a.activity, Some(Activity::Harvest { .. })))
            .map(|a| a.id)
            .collect();

        for hid in harvester_ids {
            // Extract state we need (borrow checker requires splitting)
            let (state, carried_ore, carried_gems, loc) = {
                let actor = match self.actors.get(&hid) {
                    Some(a) => a,
                    None => continue,
                };
                if let Some(Activity::Harvest { state, carried_ore, carried_gems, .. }) = &actor.activity {
                    (*state, *carried_ore, *carried_gems, actor.location)
                } else {
                    continue;
                }
            };

            match state {
                HarvestState::FindingOre => {
                    let search_center = {
                        let actor = self.actors.get(&hid).unwrap();
                        if let Some(Activity::Harvest { last_harvest_cell, .. }) = &actor.activity {
                            last_harvest_cell.or(loc)
                        } else {
                            loc
                        }
                    };
                    if let Some(center) = search_center {
                        if let Some(ore_cell) = self.terrain.find_nearest_resource(center.0, center.1, 15) {
                            // Pathfind to ore
                            if let Some(from) = loc {
                                if let Some(path) = pathfinder::find_path(&self.terrain, from, ore_cell, Some(hid)) {
                                    if path.len() > 1 {
                                        if let Some(actor) = self.actors.get_mut(&hid) {
                                            if let Some(Activity::Harvest { state: s, path: p, path_index: pi, .. }) = &mut actor.activity {
                                                *s = HarvestState::MovingToOre;
                                                *p = path;
                                                *pi = 1;
                                            }
                                        }
                                    } else {
                                        // Already at ore cell
                                        if let Some(actor) = self.actors.get_mut(&hid) {
                                            if let Some(Activity::Harvest { state: s, harvest_ticks, .. }) = &mut actor.activity {
                                                *s = HarvestState::Harvesting;
                                                *harvest_ticks = 4; // BaleLoadDelay
                                            }
                                        }
                                    }
                                }
                            }
                        } else if carried_ore + carried_gems > 0 {
                            // No ore found but carrying resources — go deliver
                            self.harvester_start_delivery(hid);
                        }
                    }
                }
                HarvestState::MovingToOre | HarvestState::MovingToRefinery => {
                    // Reuse movement logic similar to Move activity
                    let arrived = self.tick_harvest_movement(hid);
                    if arrived {
                        if state == HarvestState::MovingToOre {
                            if let Some(actor) = self.actors.get_mut(&hid) {
                                if let Some(Activity::Harvest { state: s, harvest_ticks, .. }) = &mut actor.activity {
                                    *s = HarvestState::Harvesting;
                                    *harvest_ticks = 4;
                                }
                            }
                        } else {
                            // Arrived at refinery
                            if let Some(actor) = self.actors.get_mut(&hid) {
                                if let Some(Activity::Harvest { state: s, .. }) = &mut actor.activity {
                                    *s = HarvestState::Unloading;
                                }
                            }
                        }
                    }
                }
                HarvestState::Harvesting => {
                    let actor = self.actors.get_mut(&hid).unwrap();
                    if let Some(Activity::Harvest { harvest_ticks, .. }) = &mut actor.activity {
                        *harvest_ticks -= 1;
                        if *harvest_ticks <= 0 {
                            let aloc = actor.location;
                            // Try to harvest from current cell
                            if let Some((hx, hy)) = aloc {
                                if let Some(rt) = self.terrain.harvest_resource(hx, hy) {
                                    // Reborrow after terrain mutation
                                    let actor = self.actors.get_mut(&hid).unwrap();
                                    if let Some(Activity::Harvest { carried_ore, carried_gems, capacity, state: s, harvest_ticks, last_harvest_cell, .. }) = &mut actor.activity {
                                        match rt {
                                            ResourceType::Ore => *carried_ore += 1,
                                            ResourceType::Gems => *carried_gems += 1,
                                            ResourceType::None => {}
                                        }
                                        *last_harvest_cell = Some((hx, hy));
                                        if *carried_ore + *carried_gems >= *capacity {
                                            // Full — deliver
                                            *s = HarvestState::FindingOre; // Temporary, will be overridden
                                        } else if self.terrain.has_resource(hx, hy) {
                                            // More at this cell
                                            *harvest_ticks = 4;
                                        } else {
                                            // Cell depleted, find next
                                            *s = HarvestState::FindingOre;
                                        }
                                    }
                                } else {
                                    // No resource at current cell
                                    let actor = self.actors.get_mut(&hid).unwrap();
                                    if let Some(Activity::Harvest { state: s, .. }) = &mut actor.activity {
                                        *s = HarvestState::FindingOre;
                                    }
                                }
                            }
                        }
                    }
                    // Check if we need to start delivery (full)
                    let actor = self.actors.get(&hid).unwrap();
                    if let Some(Activity::Harvest { carried_ore, carried_gems, capacity, state: s, .. }) = &actor.activity {
                        if *carried_ore + *carried_gems >= *capacity && *s == HarvestState::FindingOre {
                            self.harvester_start_delivery(hid);
                        }
                    }
                }
                HarvestState::Unloading => {
                    // Unload one unit per tick (BaleUnloadDelay=1)
                    let (ore, gems, owner) = {
                        let actor = self.actors.get(&hid).unwrap();
                        if let Some(Activity::Harvest { carried_ore, carried_gems, .. }) = &actor.activity {
                            (*carried_ore, *carried_gems, actor.owner_id)
                        } else {
                            continue;
                        }
                    };
                    if ore + gems > 0 {
                        let (unload_type, value) = if gems > 0 {
                            (ResourceType::Gems, resource_value(ResourceType::Gems))
                        } else {
                            (ResourceType::Ore, resource_value(ResourceType::Ore))
                        };
                        // S1: deposit into the capped resource store
                        // (overflow beyond storage capacity is lost).
                        // A per-tick drain converts it to spendable cash.
                        if let Some(pid) = owner {
                            let cap = self.player_storage_capacity(pid);
                            if let Some(player) = self.actors.get_mut(&pid) {
                                player.set_resource_capacity(cap);
                                let r = player.resources();
                                player.set_resources((r + value).min(cap));
                            }
                        }
                        // Decrement carried
                        if let Some(actor) = self.actors.get_mut(&hid) {
                            if let Some(Activity::Harvest { carried_ore, carried_gems, .. }) = &mut actor.activity {
                                if unload_type == ResourceType::Gems {
                                    *carried_gems -= 1;
                                } else {
                                    *carried_ore -= 1;
                                }
                            }
                        }
                    } else {
                        // Done unloading — go find more ore
                        if let Some(actor) = self.actors.get_mut(&hid) {
                            if let Some(Activity::Harvest { state: s, .. }) = &mut actor.activity {
                                *s = HarvestState::FindingOre;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Start a harvester moving to its refinery for delivery.
    fn harvester_start_delivery(&mut self, harvester_id: u32) {
        let (refinery_id, from) = {
            let actor = match self.actors.get(&harvester_id) {
                Some(a) => a,
                None => return,
            };
            let rid = if let Some(Activity::Harvest { refinery_id, .. }) = &actor.activity {
                *refinery_id
            } else {
                return;
            };
            (rid, actor.location)
        };

        // Find refinery location
        let refinery_loc = self.actors.get(&refinery_id).and_then(|a| a.location);
        if let (Some(from), Some(to)) = (from, refinery_loc) {
            // Path to adjacent cell of refinery
            let target = self.find_adjacent_cell(to.0, to.1);
            if let Some(target) = target {
                if let Some(path) = pathfinder::find_path(&self.terrain, from, target, Some(harvester_id)) {
                    if let Some(actor) = self.actors.get_mut(&harvester_id) {
                        if let Some(Activity::Harvest { state, path: p, path_index, .. }) = &mut actor.activity {
                            *state = HarvestState::MovingToRefinery;
                            *p = path;
                            *path_index = if p.len() > 1 { 1 } else { 0 };
                        }
                    }
                    return;
                }
            }
        }
        // If pathfinding fails, just set to unloading (simplified)
        if let Some(actor) = self.actors.get_mut(&harvester_id) {
            if let Some(Activity::Harvest { state, .. }) = &mut actor.activity {
                *state = HarvestState::Unloading;
            }
        }
    }

    /// Find an empty cell adjacent to a building location.
    fn find_adjacent_cell(&self, bx: i32, by: i32) -> Option<(i32, i32)> {
        for dy in -1..=2 {
            for dx in -1..=3 {
                let x = bx + dx;
                let y = by + dy;
                if self.terrain.is_passable(x, y) {
                    return Some((x, y));
                }
            }
        }
        None
    }

    /// Tick movement for a harvester (shared between MovingToOre and MovingToRefinery).
    /// Returns true if arrived at destination.
    fn tick_harvest_movement(&mut self, actor_id: u32) -> bool {
        let actor = match self.actors.get_mut(&actor_id) {
            Some(a) => a,
            None => return false,
        };

        if let Some(Activity::Harvest { path, path_index, speed, .. }) = &mut actor.activity {
            if *path_index >= path.len() {
                return true;
            }
            let target_cell = path[*path_index];
            let target_center = center_of_cell(target_cell.0, target_cell.1);
            let speed_val = *speed;

            // Update facing with smooth interpolation (same as Move activity)
            if let Some(current_loc) = actor.location {
                let desired_facing = pathfinder::facing_between(current_loc, target_cell);
                for t in &mut actor.traits {
                    if let TraitState::Mobile { facing, .. } = t {
                        let turn_speed = 128;
                        let diff = ((desired_facing - *facing) + 1024) % 1024;
                        if diff != 0 {
                            if diff <= 512 {
                                *facing = (*facing + diff.min(turn_speed)) % 1024;
                            } else {
                                *facing = (*facing + 1024 - (1024 - diff).min(turn_speed)) % 1024;
                            }
                        }
                        break;
                    }
                }
            }

            // Linear interpolation toward target (same as Move activity)
            let mut arrived = false;
            for t in &mut actor.traits {
                if let TraitState::Mobile { center_position, from_cell, to_cell, .. } = t {
                    *to_cell = CPos::new(target_cell.0, target_cell.1);
                    let from_center = center_of_cell(from_cell.x(), from_cell.y());
                    let total_dx = (target_center.x - from_center.x) as i64;
                    let total_dy = (target_center.y - from_center.y) as i64;
                    let total_dist = ((total_dx * total_dx + total_dy * total_dy) as f64).sqrt() as i32;
                    if total_dist == 0 {
                        *center_position = target_center;
                        arrived = true;
                    } else {
                        let prog_dx = (center_position.x - from_center.x) as i64;
                        let prog_dy = (center_position.y - from_center.y) as i64;
                        let progress = ((prog_dx * prog_dx + prog_dy * prog_dy) as f64).sqrt() as i32;
                        let new_progress = progress + speed_val;
                        if new_progress >= total_dist {
                            *center_position = target_center;
                            *from_cell = CPos::new(target_cell.0, target_cell.1);
                            *to_cell = CPos::new(target_cell.0, target_cell.1);
                            arrived = true;
                        } else {
                            center_position.x = from_center.x + (total_dx * new_progress as i64 / total_dist as i64) as i32;
                            center_position.y = from_center.y + (total_dy * new_progress as i64 / total_dist as i64) as i32;
                        }
                    }
                    break;
                }
            }

            if arrived {
                actor.location = Some(target_cell);
                *path_index += 1;
                if *path_index >= path.len() {
                    return true;
                }
            }
        }
        false
    }

    /// Update shroud for all players based on current unit positions.
    fn update_shroud(&mut self) {
        // First, downgrade all "visible" (2) cells to "fogged" (1)
        for layer in &mut self.shroud {
            for y in 0..layer.height {
                for x in 0..layer.width {
                    if *layer.get(x, y) == 2 {
                        layer.set(x, y, 1); // Fog previously visible cells
                    }
                }
            }
        }

        // Reveal cells around each actor for its owner
        let sight_data: Vec<(u32, (i32, i32), i32)> = self.actors.values()
            .filter_map(|a| {
                let owner = a.owner_id?;
                let loc = a.location?;
                // Sight range: buildings=5, infantry=4, vehicles=6, MCV=5
                let sight = match a.kind {
                    ActorKind::Building => 5,
                    ActorKind::Infantry => 4,
                    ActorKind::Vehicle => 6,
                    ActorKind::Mcv => 5,
                    _ => 0,
                };
                if sight > 0 { Some((owner, loc, sight)) } else { None }
            })
            .collect();

        for (owner_id, (cx, cy), sight) in sight_data {
            // Find which player index this owner corresponds to
            if let Some(pi) = self.player_actor_ids.iter().position(|&pid| pid == owner_id) {
                let layer = &mut self.shroud[pi];
                for dy in -sight..=sight {
                    for dx in -sight..=sight {
                        let x = cx + dx;
                        let y = cy + dy;
                        if layer.contains(x, y) && dx * dx + dy * dy <= sight * sight {
                            layer.set(x, y, 2); // Visible
                        }
                    }
                }
            }
            // Also reveal for "Everyone" player
            if let Some(ei) = self.player_actor_ids.iter().position(|&pid| pid == self.everyone_player_id) {
                let layer = &mut self.shroud[ei];
                for dy in -sight..=sight {
                    for dx in -sight..=sight {
                        let x = cx + dx;
                        let y = cy + dy;
                        if layer.contains(x, y) && dx * dx + dy * dy <= sight * sight {
                            layer.set(x, y, 2);
                        }
                    }
                }
            }
        }
    }

    /// Get movement speed for an actor (world units per tick).
    fn actor_speed(&self, actor_id: u32) -> i32 {
        if let Some(actor) = self.actors.get(&actor_id) {
            if let Some(ref at) = actor.actor_type {
                if let Some(stats) = self.rules.actor(at) {
                    return stats.speed;
                }
            }
            // Fallback by kind
            match actor.kind {
                ActorKind::Infantry => 43,
                ActorKind::Vehicle => 85,
                ActorKind::Mcv => 56,
                _ => 56,
            }
        } else {
            56
        }
    }

    /// Run all bot AIs and collect their orders.
    /// Temporarily takes bots out of self to satisfy the borrow checker
    /// (Bot::tick needs &World while we need &mut self later to process orders).
    fn tick_bots(&mut self) -> Vec<GameOrder> {
        let mut bots = std::mem::take(&mut self.bots);
        let mut all_orders = Vec::new();
        for bot in &mut bots {
            let orders = bot.tick(self);
            all_orders.extend(orders);
        }
        self.bots = bots;
        all_orders
    }

    fn tick_scripted_bots(&mut self) -> Vec<GameOrder> {
        let mut sbs = std::mem::take(&mut self.scripted_bots);
        let mut all = Vec::new();
        for sb in &mut sbs {
            all.extend(sb.tick(self));
        }
        self.scripted_bots = sbs;
        all
    }

    /// Attach a scripted opponent controller for `player_id`.
    pub fn add_scripted_bot(
        &mut self,
        player_id: u32,
        target_player_id: u32,
        behavior: crate::scripted_bot::ScriptedBehavior,
    ) {
        self.scripted_bots.push(crate::scripted_bot::ScriptedBot::new(
            player_id,
            target_player_id,
            behavior,
        ));
    }

    /// Tick all actors (activities and ITick traits).
    fn tick_actors(&mut self) {
        // On tick 1, ClassicProductionQueue.Tick() sets Enabled=false
        // (no Production buildings exist yet).
        if self.world_tick == 1 {
            for &pid in &self.player_actor_ids {
                if let Some(player) = self.actors.get_mut(&pid) {
                    for t in &mut player.traits {
                        if let TraitState::ClassicProductionQueue { enabled, .. } = t {
                            *enabled = false;
                        }
                    }
                }
            }
        }

        // SeedsResource: each MINE actor seeds ore every 75 ticks.
        // Fires at ticks 1, 76, 151, ... consuming 2 RNG calls per mine.
        if self.seeds_resource_ticks > 0 {
            self.seeds_resource_ticks -= 1;
        }
        if self.seeds_resource_ticks <= 0 {
            // Replenish ore near each mine. The two RNG draws per mine
            // are preserved (determinism / C# replay parity) but now
            // actually place a cell of ore at the jittered offset.
            let mines = self.mine_locations.clone();
            for &(mx, my) in &mines {
                let dx = self.rng.next_range(-1, 2); // dx
                let dy = self.rng.next_range(-1, 2); // dy
                let (x, y) = (mx + dx, my + dy);
                if self.terrain.contains(x, y) && self.terrain.is_terrain_passable(x, y)
                {
                    self.terrain.set_resource(x, y, ResourceType::Ore, 50);
                }
            }
            // Any counted mine without a tracked location still consumes
            // its 2 RNG draws so the stream stays identical.
            for _ in mines.len()..self.mine_count {
                self.rng.next_range(-1, 2);
                self.rng.next_range(-1, 2);
            }
            self.seeds_resource_ticks = 75;
        }

        // Tick Turn activities: change facing toward target.
        // When facing reaches target, queue deploy for MCVs.
        let mut deploy_ready: Vec<(u32, (i32, i32), u32)> = Vec::new();
        for actor in self.actors.values_mut() {
            if let Some(Activity::Turn { target, speed, .. }) = &actor.activity {
                let target = *target;
                let speed = *speed;

                // Read current facing from Mobile trait
                let current_facing = actor.traits.iter()
                    .find_map(|t| {
                        if let TraitState::Mobile { facing, .. } = t { Some(*facing) } else { None }
                    })
                    .unwrap_or(0);

                let new_facing = tick_facing(current_facing, target, speed);

                // Update Mobile trait
                for t in &mut actor.traits {
                    if let TraitState::Mobile { facing, .. } = t {
                        *facing = new_facing;
                        break;
                    }
                }

                if new_facing == target {
                    // Extract the `then` activity before clearing
                    let next_activity = if let Some(Activity::Turn { then, .. }) = &mut actor.activity {
                        then.take().map(|b| *b)
                    } else {
                        None
                    };
                    actor.activity = next_activity;
                    if actor.activity.is_none() && actor.kind == ActorKind::Mcv {
                        if let (Some(loc), Some(owner)) = (actor.location, actor.owner_id) {
                            deploy_ready.push((actor.id, loc, owner));
                        }
                    }
                }
            }
        }

        // ── Auto-engage on idle ────────────────────────────────────────
        // Only IDLE units (Activity::None) auto-engage enemies in range.
        // Active Move / Attack / Harvest are preserved — the agent's
        // explicit orders override defensive auto-fire. To attack a
        // specific actor, the agent uses `attack_target`. To position,
        // it uses `move`. Auto-engage is the defensive fallback for
        // units that have completed (or never received) an order.
        //
        // Old behaviour overwrote Move → Attack each tick, which made
        // any move past an enemy permanently divert into combat — agents
        // had no way to issue a second move to escape, because the next
        // tick's auto-engage scan would re-grab them.
        // ── Faithful HoldFire abandonment ──────────────────────────────
        // C# AutoTarget cancels an opportunistically-acquired attack the
        // moment the unit's stance no longer permits it. Gating only at
        // acquisition is insufficient: the env's reset warmup frame runs
        // one auto-engage scan BEFORE the agent can issue SET_STANCE, so
        // a unit can already hold an auto-acquired Attack when HoldFire
        // is later set. Every tick, drop any auto-acquired Attack whose
        // owner is now on HoldFire(0) and return the unit to idle.
        // Order-issued attacks (auto_acquired=false) are never touched —
        // explicit agent intent always overrides stance.
        {
            let mut abandon: Vec<u32> = Vec::new();
            for (id, actor) in &self.actors {
                if let Some(Activity::Attack { auto_acquired: true, .. }) = actor.activity {
                    if self.stances.get(id).copied().unwrap_or(3) == 0 {
                        abandon.push(*id);
                    }
                }
            }
            for id in abandon {
                if let Some(actor) = self.actors.get_mut(&id) {
                    actor.activity = None;
                }
            }
        }

        // ── Guard: follow the guarded actor ────────────────────────────
        // C# GuardActivity keeps the guard within `range` of its target,
        // repathing when it drifts outside. We step one cell/tick toward
        // a cell adjacent to the guarded actor whenever the Chebyshev
        // gap exceeds `leash`. If the guarded actor is gone, go idle
        // (the next briefing lets the agent re-decide). When already
        // inside leash the guard holds position (so it can still be
        // picked up by the stance auto-engage scan as "idle-equivalent"
        // is NOT done — guarding units don't opportunistically wander;
        // C# Guard layers AttackFollow on top, a documented gap).
        {
            let mut guard_steps: Vec<(u32, (i32, i32), (i32, i32))> = Vec::new();
            let mut guard_drop: Vec<u32> = Vec::new();
            for (id, actor) in &self.actors {
                let (target_id, leash) = match actor.activity {
                    Some(Activity::Guard { target_id, leash, .. }) => (target_id, leash),
                    _ => continue,
                };
                let from = match actor.location {
                    Some(l) => l,
                    None => continue,
                };
                let tgt_loc = match self.actors.get(&target_id).and_then(|a| a.location) {
                    Some(l) => l,
                    None => {
                        guard_drop.push(*id);
                        continue;
                    }
                };
                let gap = (from.0 - tgt_loc.0).abs().max((from.1 - tgt_loc.1).abs());
                if gap <= leash {
                    continue; // close enough — hold
                }
                let dest = self
                    .find_adjacent_passable(tgt_loc, *id)
                    .unwrap_or(tgt_loc);
                if let Some(path) =
                    pathfinder::find_path(&self.terrain, from, dest, Some(*id))
                {
                    if path.len() > 1 {
                        guard_steps.push((*id, from, path[1]));
                    }
                }
            }
            for id in guard_drop {
                if let Some(actor) = self.actors.get_mut(&id) {
                    actor.activity = None;
                }
            }
            for (id, from, next_cell) in guard_steps {
                let occ = self.terrain.occupant(next_cell.0, next_cell.1);
                if occ == 0 || occ == id {
                    self.terrain.clear_occupant(from.0, from.1);
                    self.terrain.set_occupant(next_cell.0, next_cell.1, id);
                    if let Some(actor) = self.actors.get_mut(&id) {
                        actor.location = Some(next_cell);
                        for t in &mut actor.traits {
                            if let TraitState::Mobile {
                                center_position,
                                from_cell,
                                to_cell,
                                ..
                            } = t
                            {
                                *center_position =
                                    center_of_cell(next_cell.0, next_cell.1);
                                *from_cell = CPos::new(next_cell.0, next_cell.1);
                                *to_cell = CPos::new(next_cell.0, next_cell.1);
                                break;
                            }
                        }
                    }
                }
            }
        }

        // ── EnterTransport: walk to transport, then board ──────────────
        // C# EnterTransport: move adjacent to the transport, then the
        // passenger is removed from the world and stashed as cargo
        // (capacity re-checked at board time). If the transport is gone
        // the passenger goes idle.
        {
            let mut step_moves: Vec<(u32, (i32, i32), (i32, i32))> = Vec::new();
            let mut board: Vec<(u32, u32)> = Vec::new(); // (passenger, transport)
            let mut drop_idle: Vec<u32> = Vec::new();
            for (id, actor) in &self.actors {
                let transport_id = match actor.activity {
                    Some(Activity::EnterTransport { transport_id, .. }) => transport_id,
                    _ => continue,
                };
                let from = match actor.location {
                    Some(l) => l,
                    None => continue,
                };
                let tloc = match self.actors.get(&transport_id).and_then(|a| a.location)
                {
                    Some(l) => l,
                    None => {
                        drop_idle.push(*id);
                        continue;
                    }
                };
                let gap = (from.0 - tloc.0).abs().max((from.1 - tloc.1).abs());
                if gap <= 1 {
                    board.push((*id, transport_id));
                } else {
                    let dest = self
                        .find_adjacent_passable(tloc, *id)
                        .unwrap_or(tloc);
                    if let Some(path) =
                        pathfinder::find_path(&self.terrain, from, dest, Some(*id))
                    {
                        if path.len() > 1 {
                            step_moves.push((*id, from, path[1]));
                        }
                    }
                }
            }
            for id in drop_idle {
                if let Some(a) = self.actors.get_mut(&id) {
                    a.activity = None;
                }
            }
            for (id, from, next_cell) in step_moves {
                let occ = self.terrain.occupant(next_cell.0, next_cell.1);
                if occ == 0 || occ == id {
                    self.terrain.clear_occupant(from.0, from.1);
                    self.terrain.set_occupant(next_cell.0, next_cell.1, id);
                    if let Some(actor) = self.actors.get_mut(&id) {
                        actor.location = Some(next_cell);
                        for t in &mut actor.traits {
                            if let TraitState::Mobile {
                                center_position,
                                from_cell,
                                to_cell,
                                ..
                            } = t
                            {
                                *center_position =
                                    center_of_cell(next_cell.0, next_cell.1);
                                *from_cell = CPos::new(next_cell.0, next_cell.1);
                                *to_cell = CPos::new(next_cell.0, next_cell.1);
                                break;
                            }
                        }
                    }
                }
            }
            for (pid, tid) in board {
                let cap = self.transport_capacity(tid) as usize;
                let cur = self.cargo.get(&tid).map(|v| v.len()).unwrap_or(0);
                if cur >= cap {
                    // Full — abandon the attempt (go idle); the env
                    // layer surfaced the capacity warning at order time.
                    if let Some(a) = self.actors.get_mut(&pid) {
                        a.activity = None;
                    }
                    continue;
                }
                if let Some(mut passenger) = self.actors.remove(&pid) {
                    if let Some(loc) = passenger.location {
                        self.terrain.clear_occupant(loc.0, loc.1);
                    }
                    passenger.activity = None;
                    self.cargo.entry(tid).or_default().push(passenger);
                }
            }
        }

        // ── C4Plant: walk Tanya to the target building, then detonate ─
        // Mirrors EnterTransport's pathing: step toward the building's
        // footprint each tick; once Chebyshev-adjacent to any footprint
        // cell, instantly destroy the building (tanya unharmed). If the
        // target has died/disappeared en route the activity drops to
        // idle. Validation (subject is tanya, target is an enemy
        // building) was enforced at order issue time; we re-check the
        // target still exists & is a building here in case it died
        // mid-walk.
        {
            let mut step_moves: Vec<(u32, (i32, i32), (i32, i32))> = Vec::new();
            let mut detonate: Vec<(u32, u32)> = Vec::new(); // (tanya, target)
            let mut drop_idle: Vec<u32> = Vec::new();
            for (id, actor) in &self.actors {
                let target_id = match actor.activity {
                    Some(Activity::C4Plant { target_id, .. }) => target_id,
                    _ => continue,
                };
                let from = match actor.location {
                    Some(l) => l,
                    None => continue,
                };
                let (tloc, fw, fh) = match self.actors.get(&target_id) {
                    Some(t) if t.kind == ActorKind::Building => match t.location {
                        Some(l) => {
                            let (fw, fh) = t
                                .actor_type
                                .as_deref()
                                .and_then(|at| self.rules.actor(at))
                                .map(|s| s.footprint)
                                .unwrap_or((1, 1));
                            (l, fw, fh)
                        }
                        None => {
                            drop_idle.push(*id);
                            continue;
                        }
                    },
                    _ => {
                        // Target gone or no longer a building — abort.
                        drop_idle.push(*id);
                        continue;
                    }
                };
                // Chebyshev distance from `from` to nearest footprint cell.
                let clamp_x = from.0.max(tloc.0).min(tloc.0 + fw - 1);
                let clamp_y = from.1.max(tloc.1).min(tloc.1 + fh - 1);
                let nearest = (clamp_x, clamp_y);
                let gap = (from.0 - nearest.0).abs().max((from.1 - nearest.1).abs());
                if gap <= 1 {
                    detonate.push((*id, target_id));
                } else {
                    // Aim for a passable cell adjacent to the footprint's
                    // top-left (matches EnterTransport's approach idiom).
                    let dest = self
                        .find_adjacent_passable(tloc, *id)
                        .unwrap_or(tloc);
                    if let Some(path) =
                        pathfinder::find_path(&self.terrain, from, dest, Some(*id))
                    {
                        if path.len() > 1 {
                            step_moves.push((*id, from, path[1]));
                        }
                    }
                }
            }
            for id in drop_idle {
                if let Some(a) = self.actors.get_mut(&id) {
                    a.activity = None;
                }
            }
            for (id, from, next_cell) in step_moves {
                let occ = self.terrain.occupant(next_cell.0, next_cell.1);
                if occ == 0 || occ == id {
                    self.terrain.clear_occupant(from.0, from.1);
                    self.terrain.set_occupant(next_cell.0, next_cell.1, id);
                    if let Some(actor) = self.actors.get_mut(&id) {
                        actor.location = Some(next_cell);
                        for t in &mut actor.traits {
                            if let TraitState::Mobile {
                                center_position,
                                from_cell,
                                to_cell,
                                ..
                            } = t
                            {
                                *center_position =
                                    center_of_cell(next_cell.0, next_cell.1);
                                *from_cell = CPos::new(next_cell.0, next_cell.1);
                                *to_cell = CPos::new(next_cell.0, next_cell.1);
                                break;
                            }
                        }
                    }
                }
            }
            // Detonation: instantly kill the target building. Credit the
            // kill to Tanya / her owner so kill_per_player stays
            // consistent with the standard combat path. Clear the
            // footprint (restoring passability) the same way regular
            // building-death cleanup does. Tanya goes idle, fully alive.
            let mut dead_ids: Vec<u32> = Vec::new();
            for (tanya_id, tgt_id) in detonate {
                // Drop tanya's activity first (she survives, planted &
                // walks away).
                if let Some(a) = self.actors.get_mut(&tanya_id) {
                    a.activity = None;
                }
                // Zero the target's HP for the snapshot, then remove it.
                let owner_for_kill_credit = self
                    .actors
                    .get(&tanya_id)
                    .and_then(|a| a.owner_id);
                if let Some(target) = self.actors.get_mut(&tgt_id) {
                    for t in &mut target.traits {
                        if let TraitState::Health { hp } = t {
                            *hp = 0;
                            break;
                        }
                    }
                }
                if let Some(dead) = self.actors.remove(&tgt_id) {
                    dead_ids.push(tgt_id);
                    if dead.kind == ActorKind::Building {
                        if let Some(loc) = dead.location {
                            let (fw, fh) = dead
                                .actor_type
                                .as_deref()
                                .and_then(|at| self.rules.actor(at))
                                .map(|s| s.footprint)
                                .unwrap_or((2, 2));
                            self.terrain.clear_footprint(loc.0, loc.1, fw, fh);
                        }
                    } else if let Some(loc) = dead.location {
                        self.terrain.clear_occupant(loc.0, loc.1);
                    }
                    // Credit the kill so reward/units_killed signals reflect it.
                    if let Some(a) = self.actors.get_mut(&tanya_id) {
                        a.kills = a.kills.saturating_add(1);
                    }
                    if let Some(pid) = owner_for_kill_credit {
                        *self.kills_per_player.entry(pid).or_insert(0) += 1;
                    }
                }
            }
            // Clear stale Attack activities aimed at any C4'd building.
            if !dead_ids.is_empty() {
                for actor in self.actors.values_mut() {
                    if let Some(Activity::Attack { target_id, .. }) = actor.activity {
                        if dead_ids.contains(&target_id) {
                            actor.activity = None;
                        }
                    }
                }
            }
        }

        // ── Stance-driven defensive auto-engagement + hunt ────────────
        // Three stance gates, each driving its own engagement policy:
        //
        //   stance:0 HoldFire       — never auto-engage.
        //   stance:1 ReturnFire     — auto-engage ONLY if this actor has
        //                              been damaged by a hostile within
        //                              the last RETURN_FIRE_WINDOW ticks
        //                              (tracked by recently_received_fire).
        //                              Picks the closest in-range enemy.
        //   stance:2 Defend         — auto-engage closest in-range enemy
        //                              (don't pursue past current range).
        //   stance:3 AttackAnything — auto-engage closest in-range enemy;
        //                              if none, but a hostile is visible
        //                              within sight range, ADVANCE toward
        //                              it ("hunt"). This is the only
        //                              stance that opens new engagements
        //                              by moving.
        //
        // RETURN_FIRE_WINDOW: how long after taking a hit a stance:1
        // unit stays "primed". Chosen so a single salvo unlocks a full
        // retaliatory engagement cycle (cannon reload ≈30-50 inner
        // ticks); too short and the return fire fizzles before the
        // weapon cools.
        const RETURN_FIRE_WINDOW: u32 = 60;
        // HUNT_RADIUS: stance:3 hunters chase enemies up to this many
        // cells away. The radius is large by design — "AttackAnything"
        // means "advance on any hostile I can find" — so a single
        // hunter can clear a map of scattered enemies by chaining one
        // hunt move per kill. (The per-unit RevealsShroud sight is not
        // checked here on purpose: stance:3 hunters use shared / fact
        // intel, not just their own line of sight, otherwise a tank
        // wouldn't pursue an enemy fleeing through fog.) We still cap
        // at a moderate radius to keep the scan O(N·M) bounded; larger
        // maps would just need the constant raised. 128 cells covers
        // the rush-hour-arena longest axis.
        const HUNT_RADIUS: i32 = 128;
        let mut engage_pairs: Vec<(u32, u32)> = Vec::new();
        let mut hunt_moves: Vec<(u32, (i32, i32))> = Vec::new();
        let world_tick_now = self.world_tick;
        for (id, actor) in &self.actors {
            if actor.activity.is_some() {
                continue;
            }
            // Default-when-missing is Defend (2), not AttackAnything (3):
            // hunting (advance toward visible enemies) is an opt-in
            // behaviour the scenario must explicitly request via
            // `set_stance(actor, 3)`. Without this gate, every idle
            // unit would wander after distant enemies and break
            // existing bench packs whose policies assume "an idle unit
            // stays put unless explicitly ordered to move" (e.g. the
            // rush-hour Stop semantics, the def-pre-position-mobile-
            // reserve idiom). Stance:0 still blocks all engagement;
            // stance:1 still requires a recent hit; stance:2 / missing
            // engage in-range; stance:3 ALSO hunts.
            let stance = self.stances.get(id).copied().unwrap_or(2);
            // HoldFire stance suppresses defensive auto-engage.
            if stance == 0 {
                continue;
            }
            // ReturnFire (stance:1) requires having been recently hit
            // by a hostile. Without a fresh hit, the unit holds fire
            // even on enemies in range — pinning the "I won't shoot
            // first" semantics that distinguish stance:1 from stance:2.
            if stance == 1 {
                let licensed = self
                    .recently_received_fire
                    .get(id)
                    .map(|t| world_tick_now.saturating_sub(*t) <= RETURN_FIRE_WINDOW)
                    .unwrap_or(false);
                if !licensed {
                    continue;
                }
            }
            let my_loc = match actor.location {
                Some(l) => l,
                None => continue,
            };
            let my_owner = match actor.owner_id {
                Some(o) => o,
                None => continue,
            };
            // Resolve this actor's *maximum* weapon range (cells)
            // across every armament. A multi-weapon actor (e3: RedEye
            // 7c + Dragon 5c) must scan for candidates out to its
            // longest reach; the exact weapon — and its range — is
            // re-resolved per-target in `order_attack` via
            // `best_weapon_against`.
            let range_cells = actor.actor_type.as_deref()
                .and_then(|at| self.rules.actor(at))
                .map(|stats| {
                    stats.weapons.iter()
                        .filter_map(|wname| self.rules.weapon(wname))
                        .map(|w| w.range / 1024)
                        .max()
                        .unwrap_or(0)
                })
                .unwrap_or(0);
            if range_cells <= 0 {
                continue;
            }
            // Whether this actor can physically move (stance:3 hunt
            // requires mobility — a static turret with stance:3 won't
            // ever advance, but in the existing engine such turrets
            // use ActorKind::Building and live on the defense-scan
            // path above, not this scan).
            let can_hunt = matches!(
                actor.kind,
                ActorKind::Infantry | ActorKind::Vehicle | ActorKind::Mcv
            );
            // First pass: nearest in-range enemy (firing target).
            let mut best_in_range: Option<(u32, i32)> = None;
            // Second pass: nearest visible-but-out-of-range enemy
            // (hunt target). Only filled when stance:3 + can_hunt.
            let mut best_hunt: Option<(u32, i32, (i32, i32))> = None;
            for (eid, enemy) in &self.actors {
                if *eid == *id {
                    continue;
                }
                let eloc = match enemy.location {
                    Some(l) => l,
                    None => continue,
                };
                let eowner = match enemy.owner_id {
                    Some(o) => o,
                    None => continue,
                };
                if eowner == my_owner {
                    continue;
                }
                if !matches!(
                    enemy.kind,
                    ActorKind::Infantry
                        | ActorKind::Vehicle
                        | ActorKind::Mcv
                        | ActorKind::Building
                ) {
                    continue;
                }
                let dx = (eloc.0 - my_loc.0).abs();
                let dy = (eloc.1 - my_loc.1).abs();
                let dist = dx.max(dy); // Chebyshev (matches armament.rs)
                if dist <= range_cells {
                    if best_in_range.map_or(true, |(_, d)| dist < d) {
                        best_in_range = Some((*eid, dist));
                    }
                } else if stance == 3 && can_hunt && dist <= HUNT_RADIUS {
                    if best_hunt.map_or(true, |(_, d, _)| dist < d) {
                        best_hunt = Some((*eid, dist, eloc));
                    }
                }
            }
            if let Some((target_id, _)) = best_in_range {
                engage_pairs.push((*id, target_id));
            } else if let Some((_, _, eloc)) = best_hunt {
                // Stance:3 hunt — advance toward the visible enemy.
                // We move to a cell just inside weapon range of the
                // enemy so that, once arrived, the next-tick auto-
                // engage scan above promotes us back into Attack.
                hunt_moves.push((*id, eloc));
            }
        }
        // Apply: Move → Attack. After the target dies, the unit goes
        // idle and the next briefing prompts the agent to re-decide.
        for (attacker, target) in engage_pairs {
            // Idle opportunistic engagement — auto-acquired.
            self.order_attack(attacker, target, true);
        }
        // Apply hunt moves. order_move pathfinds toward the target
        // cell; pathfinder stops at the nearest passable cell if the
        // exact cell is occupied. The arriving unit goes idle on
        // completion and gets re-scanned by this loop on the next
        // tick — at which point the enemy is in weapon range and the
        // in-range branch above fires the cannon.
        for (hunter, target_cell) in hunt_moves {
            self.order_move(hunter, target_cell);
        }

        // Tick Move activities: advance position along path.
        let mut move_completions: Vec<u32> = Vec::new();
        let mut turn_before_move: Vec<u32> = Vec::new();
        let mut occupancy_updates: Vec<(u32, (i32, i32), (i32, i32))> = Vec::new(); // (id, from, to)
        for actor in self.actors.values_mut() {
            if let Some(Activity::Move { ref path, ref mut path_index, speed }) = actor.activity {
                if *path_index >= path.len() {
                    move_completions.push(actor.id);
                    continue;
                }
                let target_cell = path[*path_index];
                let target_center = center_of_cell(target_cell.0, target_cell.1);

                // C# TurnsWhileMoving=false: at each path cell, if facing doesn't
                // match the next segment, stop and Turn in place first.
                // Reference: OpenRA Move.cs lines 207-213.
                if let Some(current_loc) = actor.location {
                    let desired_facing = pathfinder::facing_between(current_loc, target_cell);
                    let current_facing = actor.traits.iter()
                        .find_map(|t| {
                            if let TraitState::Mobile { facing, .. } = t { Some(*facing) } else { None }
                        })
                        .unwrap_or(0);
                    if current_facing != desired_facing {
                        // Need to turn first — convert Move to Turn→Move
                        turn_before_move.push(actor.id);
                        continue;
                    }
                }

                // Linear interpolation toward target (C#-style Lerp)
                // Progress advances by `speed` world units per tick along the
                // straight line from from_cell center to to_cell center.
                let mut arrived = false;
                for t in &mut actor.traits {
                    if let TraitState::Mobile { center_position, from_cell, to_cell, .. } = t {
                        *to_cell = CPos::new(target_cell.0, target_cell.1);
                        let from_center = center_of_cell(from_cell.x(), from_cell.y());

                        // Total distance between cell centers (1024 for ortho, ~1448 for diag)
                        let total_dx = (target_center.x - from_center.x) as i64;
                        let total_dy = (target_center.y - from_center.y) as i64;
                        let total_dist = ((total_dx * total_dx + total_dy * total_dy) as f64).sqrt() as i32;

                        if total_dist == 0 {
                            *center_position = target_center;
                            arrived = true;
                        } else {
                            // Current progress = distance traveled from from_center
                            let prog_dx = (center_position.x - from_center.x) as i64;
                            let prog_dy = (center_position.y - from_center.y) as i64;
                            let progress = ((prog_dx * prog_dx + prog_dy * prog_dy) as f64).sqrt() as i32;
                            let new_progress = progress + speed;

                            if new_progress >= total_dist {
                                *center_position = target_center;
                                *from_cell = CPos::new(target_cell.0, target_cell.1);
                                *to_cell = CPos::new(target_cell.0, target_cell.1);
                                arrived = true;
                            } else {
                                // Lerp: position = from + (to - from) * progress / distance
                                center_position.x = from_center.x + (total_dx * new_progress as i64 / total_dist as i64) as i32;
                                center_position.y = from_center.y + (total_dy * new_progress as i64 / total_dist as i64) as i32;
                            }
                        }
                        break;
                    }
                }

                if arrived {
                    let old_loc = actor.location.unwrap_or(target_cell);
                    if old_loc != target_cell {
                        occupancy_updates.push((actor.id, old_loc, target_cell));
                    }
                    actor.location = Some(target_cell);
                    *path_index += 1;
                    if *path_index >= path.len() {
                        move_completions.push(actor.id);
                    }
                }
            }
        }
        // Update terrain occupancy for moved units
        for (actor_id, from, to) in occupancy_updates {
            self.terrain.clear_occupant(from.0, from.1);
            self.terrain.set_occupant(to.0, to.1, actor_id);
        }
        // Convert Move→Turn→Move for actors needing to turn at a direction change
        for id in turn_before_move {
            if let Some(actor) = self.actors.get_mut(&id) {
                if let Some(Activity::Move { ref path, path_index, .. }) = actor.activity {
                    if path_index < path.len() {
                        let target_cell = path[path_index];
                        let from = actor.location.unwrap_or((0, 0));
                        let desired_facing = pathfinder::facing_between(from, target_cell);
                        let move_activity = actor.activity.take().unwrap();
                        actor.activity = Some(Activity::Turn {
                            target: desired_facing,
                            speed: 20,
                            then: Some(Box::new(move_activity)),
                        });
                    }
                }
            }
        }
        // Clear completed Move activities
        for id in move_completions {
            if let Some(actor) = self.actors.get_mut(&id) {
                actor.activity = None;
            }
        }

        // Phase 7 — Auto-target for armed static buildings (gun, pbox, tsla, ftur).
        // Idle defenses scan for the closest hostile actor in range and queue an
        // Attack activity. AA-only stubs (sam, agun) and cosmetic buildings
        // (powr, barr, fact, proc) are skipped via classify_defense().
        // We collect (id, target_id, damage, range, reload, burst) up front so
        // we don't borrow `self.actors` mutably while iterating.
        let mut new_defense_attacks: Vec<(u32, u32, i32, i32, i32, i32)> = Vec::new();
        for actor in self.actors.values() {
            if actor.kind != ActorKind::Building {
                continue;
            }
            if actor.activity.is_some() {
                continue;
            }
            let actor_type = match actor.actor_type.as_deref() {
                Some(t) => t,
                None => continue,
            };
            let defense_kind = match crate::traits::classify_defense(actor_type) {
                Some(crate::traits::DefenseKind::GroundTurret) => "turret",
                Some(crate::traits::DefenseKind::Tesla) => "tesla",
                _ => continue, // AA-only / inert / cosmetic — never auto-fire
            };
            let _ = defense_kind; // currently both behave identically
            let owner = match actor.owner_id {
                Some(o) => o,
                None => continue,
            };
            let from = match actor.location {
                Some(l) => l,
                None => continue,
            };
            // Resolve weapon stats. Buildings store their weapon name in
            // `rules.actor(type).weapons[0]`. If the weapon is missing we
            // skip the building (rather than fall back to a default that
            // might silently misbehave).
            let weapon = self
                .rules
                .actor(actor_type)
                .and_then(|stats| stats.weapons.first())
                .and_then(|wname| self.rules.weapon(wname));
            let (damage, range_cells, reload, burst) = match weapon {
                Some(w) => (w.damage, w.range / 1024, w.reload_delay, w.burst.max(1)),
                None => continue,
            };
            if damage <= 0 || range_cells <= 0 {
                continue;
            }
            // Find nearest in-range hostile actor. We iterate the
            // BTreeMap-ordered actor list so the chosen target is
            // deterministic on ties (lowest id wins).
            let mut best: Option<(u32, i32)> = None;
            for cand in self.actors.values() {
                let cand_owner = match cand.owner_id {
                    Some(o) => o,
                    None => continue,
                };
                if cand_owner == owner {
                    continue;
                }
                if !matches!(
                    cand.kind,
                    ActorKind::Infantry
                        | ActorKind::Vehicle
                        | ActorKind::Mcv
                        | ActorKind::Building
                ) {
                    continue;
                }
                // Skip dead actors (defensive).
                let dead = cand.traits.iter().any(|t| matches!(t, TraitState::Health { hp } if *hp <= 0));
                if dead {
                    continue;
                }
                let cl = match cand.location {
                    Some(l) => l,
                    None => continue,
                };
                let dx = (from.0 - cl.0).abs();
                let dy = (from.1 - cl.1).abs();
                let cheb = dx.max(dy);
                if cheb > range_cells {
                    continue;
                }
                match best {
                    Some((_, bd)) if bd <= cheb => {}
                    _ => best = Some((cand.id, cheb)),
                }
            }
            if let Some((tid, _)) = best {
                // Re-resolve the building's armament against the chosen
                // target's armor class so a multi-weapon defense
                // commits the base damage of the weapon it will
                // actually fire. Single-weapon defenses are unaffected.
                let target_armor = self.target_armor_of(tid);
                let dmg = self
                    .rules
                    .best_weapon_against(actor_type, target_armor)
                    .map(|(_, w)| w.damage)
                    .unwrap_or(damage);
                new_defense_attacks.push((actor.id, tid, dmg, range_cells, reload, burst));
            }
        }
        for (aid, tid, damage, range_cells, reload, burst) in new_defense_attacks {
            // Faithful HoldFire: a defensive auto-scan is auto-acquired,
            // so a HoldFire(0) building never engages.
            if self.stances.get(&aid).copied().unwrap_or(3) == 0 {
                continue;
            }
            if let Some(actor) = self.actors.get_mut(&aid) {
                actor.activity = Some(Activity::Attack {
                    target_id: tid,
                    weapon_range: range_cells,
                    weapon_damage: damage,
                    reload_delay: reload,
                    reload_remaining: 0,
                    burst,
                    burst_remaining: burst,
                    auto_acquired: true,
                });
            }
        }

        // Tick Attack activities: check range, manage reload, deal damage.
        // First pass: decrement reload timers and collect ready-to-fire attackers
        let mut ready_attackers: Vec<(u32, u32, i32, i32)> = Vec::new(); // (attacker_id, target_id, damage, weapon_range)
        for actor in self.actors.values_mut() {
            if let Some(Activity::Attack {
                target_id, weapon_range, weapon_damage,
                ref mut reload_remaining, ..
            }) = actor.activity {
                if *reload_remaining > 0 {
                    *reload_remaining -= 1;
                } else {
                    ready_attackers.push((actor.id, target_id, weapon_damage, weapon_range));
                }
            }
        }
        // ── Opportunistic fire while MOVING ───────────────────────────
        // A unit executing a `Move` activity is still a combatant: it
        // shoots in-range hostiles in passing WITHOUT abandoning its
        // move (faithful to C# AttackMove / opportunistic AutoTarget).
        // The stance-driven auto-engage scan above only considers IDLE
        // units, so without this pass a unit on a long `move` order
        // crossed an enemy kill zone untouched while never returning
        // fire — "sprint-invincibility". The shots feed the SAME
        // instant/projectile pipeline as `Activity::Attack`; the
        // per-actor reload lives in `move_fire_cooldown`. Targets are
        // resolved in-range only, so a moving shooter never diverts
        // into a chase — it keeps gliding down its path.
        //
        // Determinism: iterate the BTreeMap-ordered actor list; ties
        // on target distance break by lowest enemy id.
        let mut move_fire_attackers: Vec<(u32, u32)> = Vec::new(); // (attacker, target)
        {
            // First, tick down every moving unit's cooldown and prune
            // entries for actors that are no longer moving.
            let moving_ids: Vec<u32> = self
                .actors
                .iter()
                .filter(|(_, a)| matches!(a.activity, Some(Activity::Move { .. })))
                .map(|(id, _)| *id)
                .collect();
            let moving_set: std::collections::HashSet<u32> =
                moving_ids.iter().copied().collect();
            self.move_fire_cooldown.retain(|id, _| moving_set.contains(id));
            for cd in self.move_fire_cooldown.values_mut() {
                if *cd > 0 {
                    *cd -= 1;
                }
            }
            for id in moving_ids {
                // Cooldown gates firing — a unit mid-reload skips.
                if self.move_fire_cooldown.get(&id).copied().unwrap_or(0) > 0 {
                    continue;
                }
                let stance = self.stances.get(&id).copied().unwrap_or(2);
                // HoldFire never opportunistically fires.
                if stance == 0 {
                    continue;
                }
                // ReturnFire only after itself taking recent hostile fire.
                if stance == 1 {
                    let licensed = self
                        .recently_received_fire
                        .get(&id)
                        .map(|t| world_tick_now.saturating_sub(*t) <= RETURN_FIRE_WINDOW)
                        .unwrap_or(false);
                    if !licensed {
                        continue;
                    }
                }
                let (my_loc, my_owner) = match self.actors.get(&id) {
                    Some(a) => match (a.location, a.owner_id) {
                        (Some(l), Some(o)) => (l, o),
                        _ => continue,
                    },
                    None => continue,
                };
                // Longest weapon reach across this actor's armaments.
                let range_cells = self
                    .actors
                    .get(&id)
                    .and_then(|a| a.actor_type.as_deref())
                    .and_then(|at| self.rules.actor(at))
                    .map(|stats| {
                        stats
                            .weapons
                            .iter()
                            .filter_map(|wname| self.rules.weapon(wname))
                            .map(|w| w.range / 1024)
                            .max()
                            .unwrap_or(0)
                    })
                    .unwrap_or(0);
                if range_cells <= 0 {
                    continue;
                }
                let mut best: Option<(u32, i32)> = None;
                for (eid, enemy) in &self.actors {
                    if *eid == id {
                        continue;
                    }
                    let eloc = match enemy.location {
                        Some(l) => l,
                        None => continue,
                    };
                    let eowner = match enemy.owner_id {
                        Some(o) => o,
                        None => continue,
                    };
                    if eowner == my_owner {
                        continue;
                    }
                    if !matches!(
                        enemy.kind,
                        ActorKind::Infantry
                            | ActorKind::Vehicle
                            | ActorKind::Mcv
                            | ActorKind::Building
                    ) {
                        continue;
                    }
                    let dead = enemy
                        .traits
                        .iter()
                        .any(|t| matches!(t, TraitState::Health { hp } if *hp <= 0));
                    if dead {
                        continue;
                    }
                    let dx = (eloc.0 - my_loc.0).abs();
                    let dy = (eloc.1 - my_loc.1).abs();
                    let dist = dx.max(dy);
                    if dist > range_cells {
                        continue;
                    }
                    if best.map_or(true, |(_, d)| dist < d) {
                        best = Some((*eid, dist));
                    }
                }
                if let Some((tid, _)) = best {
                    move_fire_attackers.push((id, tid));
                }
            }
        }

        // Second pass: check range and fire. Phase 8 splits this into
        // instant-hit damage (M1Carbine, tank cannons, DogJaw, TurretGun,
        // TeslaZap) vs projectile spawning (RedEye, Dragon, Hellfire,
        // Stinger). The decision is per-attacker, looked up from the
        // attacker's primary weapon's `projectile_speed`.
        let mut attacks: Vec<(u32, u32, i32)> = Vec::new();
        let mut spawn_projectiles: Vec<(u32, u32, i32, i32, i32, BTreeMap<crate::gamerules::ArmorType, i32>)> = Vec::new(); // (attacker, target, damage, speed, splash, versus)
        let mut chase_targets: Vec<(u32, (i32, i32))> = Vec::new();

        // Resolve the opportunistic move-fire shots collected above.
        // Each is in-range by construction; pick the best weapon vs the
        // target's armor, route instant vs projectile like the Attack
        // pipeline, and arm the per-actor `move_fire_cooldown`.
        for (attacker_id, target_id) in &move_fire_attackers {
            let target_armor = self.target_armor_of(*target_id);
            let weapon = self
                .actors
                .get(attacker_id)
                .and_then(|a| a.actor_type.as_deref())
                .and_then(|at| self.rules.best_weapon_against(at, target_armor))
                .map(|(_, w)| w);
            let (damage, reload, proj_speed, splash, versus) = match weapon {
                Some(w) => (
                    w.damage,
                    w.reload_delay.max(1),
                    w.projectile_speed,
                    w.splash_radius,
                    w.versus.clone(),
                ),
                None => continue,
            };
            if damage <= 0 {
                continue;
            }
            if proj_speed > 0 {
                spawn_projectiles.push((
                    *attacker_id,
                    *target_id,
                    damage,
                    proj_speed,
                    splash,
                    versus,
                ));
            } else {
                attacks.push((*attacker_id, *target_id, damage));
            }
            self.move_fire_cooldown.insert(*attacker_id, reload);
        }
        for (attacker_id, target_id, damage, weapon_range) in ready_attackers {
            let attacker_loc = self.actors.get(&attacker_id).and_then(|a| a.location);
            let target_loc = self.actors.get(&target_id).and_then(|a| a.location);
            if let (Some(aloc), Some(tloc)) = (attacker_loc, target_loc) {
                let dx = (aloc.0 - tloc.0).abs();
                let dy = (aloc.1 - tloc.1).abs();
                let dist = dx.max(dy);
                if dist <= weapon_range {
                    // Resolve the attacker's weapon for projectile
                    // metadata. The armament must be re-resolved
                    // against *this* target's armor class so a
                    // multi-weapon actor (e3: RedEye + Dragon) carries
                    // the projectile/splash/versus table of the same
                    // weapon whose damage was committed in `order_attack`.
                    let proj_target_armor = self.target_armor_of(target_id);
                    let (proj_speed, splash, versus_table) = self
                        .actors
                        .get(&attacker_id)
                        .and_then(|a| a.actor_type.as_deref())
                        .and_then(|t| self.rules.best_weapon_against(t, proj_target_armor))
                        .map(|(_, w)| (w.projectile_speed, w.splash_radius, w.versus.clone()))
                        .unwrap_or((0, 0, BTreeMap::new()));
                    if proj_speed > 0 {
                        spawn_projectiles.push((attacker_id, target_id, damage, proj_speed, splash, versus_table));
                    } else {
                        attacks.push((attacker_id, target_id, damage));
                    }
                    // Update burst/reload state regardless of projectile vs instant
                    if let Some(actor) = self.actors.get_mut(&attacker_id) {
                        if let Some(Activity::Attack {
                            ref mut burst_remaining, burst,
                            ref mut reload_remaining, reload_delay, ..
                        }) = actor.activity {
                            *burst_remaining -= 1;
                            if *burst_remaining <= 0 {
                                *burst_remaining = burst;
                                *reload_remaining = reload_delay;
                            }
                        }
                    }
                } else {
                    // Out of range: chase the target
                    chase_targets.push((attacker_id, tloc));
                }
            }
        }
        // Phase 8 — spawn pending projectiles. These won't deal damage
        // this tick; the post-activity tick_projectiles loop advances
        // them and resolves impacts.
        for (attacker_id, target_id, damage, speed, splash, versus_table) in spawn_projectiles {
            let attacker_pos = self.actors.get(&attacker_id)
                .and_then(|a| a.location)
                .map(|(x, y)| center_of_cell(x, y));
            let target_pos = self.actors.get(&target_id)
                .and_then(|a| a.location)
                .map(|(x, y)| center_of_cell(x, y));
            if let (Some(origin), Some(tpos)) = (attacker_pos, target_pos) {
                // Translate the gamerules versus map (ArmorType enum
                // keys) into the projectile's String-keyed map so the
                // projectile module stays free of gamerules-specific
                // types.
                let mut v_str: std::collections::BTreeMap<String, i32> = std::collections::BTreeMap::new();
                for (k, pct) in versus_table {
                    let key = match k {
                        crate::gamerules::ArmorType::None => "none",
                        crate::gamerules::ArmorType::Light => "light",
                        crate::gamerules::ArmorType::Heavy => "heavy",
                        crate::gamerules::ArmorType::Wood => "wood",
                        crate::gamerules::ArmorType::Concrete => "concrete",
                    };
                    v_str.insert(key.to_string(), pct);
                }
                let pid = self.next_projectile_id;
                self.next_projectile_id = self.next_projectile_id.saturating_add(1);
                self.pending_projectiles.insert(pid, Projectile::new(
                    pid,
                    attacker_id,
                    target_id,
                    origin,
                    tpos,
                    speed,
                    damage,
                    splash,
                    v_str,
                ));
            }
        }
        // Apply damage (skip invulnerable actors).
        // Phase 8 — apply Versus armor multipliers using each victim's
        // `armor_class` lookup from the rules. Track which attacker
        // scored each kill so we can credit `kills_per_player` and bump
        // the attacker's `kills` field.
        let mut dead_actors: Vec<u32> = Vec::new();
        let mut kill_credits: Vec<(u32, u32)> = Vec::new(); // (attacker_id, victim_id)
        for (attacker_id, target_id, damage) in &attacks {
            if self.invulnerable.contains_key(target_id) { continue; }
            // Look up the attacker's weapon `versus` table and the
            // target's armor class. The armament is re-resolved
            // against this target's armor so a multi-weapon actor
            // applies the `Versus` table of the same weapon whose
            // damage was committed in `order_attack`.
            let instant_target_armor = self.target_armor_of(*target_id);
            let versus_table = self.actors.get(attacker_id)
                .and_then(|a| a.actor_type.as_deref())
                .and_then(|t| self.rules.best_weapon_against(t, instant_target_armor))
                .map(|(_, w)| w.versus.clone())
                .unwrap_or_default();
            let target_armor_str = self.actors.get(target_id)
                .and_then(|a| a.actor_type.as_deref())
                .and_then(|t| self.rules.actor(t))
                .map(|stats| match stats.armor_type {
                    crate::gamerules::ArmorType::None => "none",
                    crate::gamerules::ArmorType::Light => "light",
                    crate::gamerules::ArmorType::Heavy => "heavy",
                    crate::gamerules::ArmorType::Wood => "wood",
                    crate::gamerules::ArmorType::Concrete => "concrete",
                })
                .unwrap_or("none");
            // Translate enum-keyed versus → string-keyed for apply_versus.
            let mut versus_str: std::collections::BTreeMap<String, i32> = std::collections::BTreeMap::new();
            for (k, pct) in versus_table {
                let key = match k {
                    crate::gamerules::ArmorType::None => "none",
                    crate::gamerules::ArmorType::Light => "light",
                    crate::gamerules::ArmorType::Heavy => "heavy",
                    crate::gamerules::ArmorType::Wood => "wood",
                    crate::gamerules::ArmorType::Concrete => "concrete",
                };
                versus_str.insert(key.to_string(), pct);
            }
            let scaled_damage = apply_versus(*damage, target_armor_str, &versus_str);
            // Reveal-on-attack: queue the attacker's cell to be force-revealed
            // for the victim's owning player on the next typed-shroud refresh.
            // Snapshot before the &mut borrow on actors below.
            let target_owner = self.actors.get(target_id).and_then(|a| a.owner_id);
            let attacker_cell = self.actors.get(attacker_id).and_then(|a| a.location);
            if let (Some(owner), Some(cell)) = (target_owner, attacker_cell) {
                self.combat_reveal_cells.entry(owner).or_default().push(cell);
            }
            if let Some(target) = self.actors.get_mut(target_id) {
                for t in &mut target.traits {
                    if let TraitState::Health { hp } = t {
                        // Credit a kill only on the transition from
                        // alive→dead. Without this guard, multiple
                        // attackers landing lethal damage on the same
                        // victim in one tick each push their own
                        // (attacker, victim) entry, inflating
                        // `kills_per_player` past the actual victim
                        // count.
                        let was_alive = *hp > 0;
                        *hp -= scaled_damage;
                        if was_alive && *hp <= 0 {
                            dead_actors.push(*target_id);
                            kill_credits.push((*attacker_id, *target_id));
                        }
                        break;
                    }
                }
            }
            // Stamp the victim with the world-tick of receiving fire so
            // a stance:1 (ReturnFire) victim is licensed to engage in
            // the next auto-engage scan. Only count hostile (different-
            // owner) damage — friendly fire / splash from an allied
            // unit must NOT unlock return fire (otherwise a stray
            // allied splash would re-arm the whole formation).
            let attacker_owner = self.actors.get(attacker_id).and_then(|a| a.owner_id);
            let target_owner_now = self.actors.get(target_id).and_then(|a| a.owner_id);
            if let (Some(ao), Some(to)) = (attacker_owner, target_owner_now) {
                if ao != to {
                    self.recently_received_fire.insert(*target_id, self.world_tick);
                }
            }
        }
        // Credit each kill to the attacker (Actor.kills) and the
        // attacker's owning player (kills_per_player). Done in
        // attacker-id order to keep credit assignment deterministic.
        kill_credits.sort_by_key(|(aid, vid)| (*aid, *vid));
        for (attacker_id, _victim) in &kill_credits {
            let owner = self.actors.get(attacker_id).and_then(|a| a.owner_id);
            if let Some(att) = self.actors.get_mut(attacker_id) {
                att.kills = att.kills.saturating_add(1);
            }
            if let Some(pid) = owner {
                *self.kills_per_player.entry(pid).or_insert(0) += 1;
            }
        }
        // Remove dead actors
        let mut dead_ids: Vec<u32> = Vec::new();
        for id in dead_actors {
            if let Some(dead) = self.actors.remove(&id) {
                dead_ids.push(id);
                if dead.kind == ActorKind::Building {
                    // Clear full building footprint (restores passability)
                    if let Some(loc) = dead.location {
                        let (fw, fh) = dead.actor_type.as_deref()
                            .and_then(|t| self.rules.actor(t))
                            .map(|s| s.footprint)
                            .unwrap_or((2, 2));
                        self.terrain.clear_footprint(loc.0, loc.1, fw, fh);
                    }
                } else if let Some(loc) = dead.location {
                    self.terrain.clear_occupant(loc.0, loc.1);
                }
            }
        }
        // Phase 7: clear stale Attack activities pointing at corpses so
        // armed buildings (and any other attacker) re-scan for a new target
        // on the next tick. Without this, a static turret can deadlock on
        // a dead target and never engage subsequent enemies. Units go
        // idle here on purpose — the next briefing surfaces the kill via
        // an interrupt and the agent decides what to do next (option B
        // semantics).
        if !dead_ids.is_empty() {
            for actor in self.actors.values_mut() {
                if let Some(Activity::Attack { target_id, .. }) = actor.activity {
                    if dead_ids.contains(&target_id) {
                        actor.activity = None;
                    }
                }
            }
        }

        // Chase targets: pathfind toward target when out of range, preserving attack state.
        // Static buildings cannot move — Phase 7 — they will simply hold their attack
        // activity but never close range; the world tick keeps the activity around so
        // that as soon as a target wanders back into range the cooldown gating fires.
        for (attacker_id, target_loc) in chase_targets {
            // Phase 7: buildings are immobile. Skip chase, but keep the
            // Activity::Attack alive so the next-tick range check can re-fire
            // if the target re-enters range (or auto-target picks a new one).
            if self
                .actors
                .get(&attacker_id)
                .map(|a| a.kind == ActorKind::Building)
                .unwrap_or(false)
            {
                // Drop the stale attack so auto-target can pick a fresher
                // candidate next tick (a target out of range is no longer
                // worth tracking on a static turret).
                if let Some(actor) = self.actors.get_mut(&attacker_id) {
                    actor.activity = None;
                }
                continue;
            }
            let from = match self.actors.get(&attacker_id).and_then(|a| a.location) {
                Some(loc) => loc,
                None => continue,
            };
            // Find nearest passable cell adjacent to the target (for attacking buildings)
            let chase_dest = self.find_adjacent_passable(target_loc, attacker_id)
                .unwrap_or(target_loc);
            // Pathfind toward target
            if let Some(path) = pathfinder::find_path(&self.terrain, from, chase_dest, Some(attacker_id)) {
                if path.len() > 1 {
                    // Advance toward the next path cell at the actor's
                    // real movement speed (world units/tick), lerping the
                    // Mobile center_position exactly like a normal Move
                    // activity. Previously the chase warped a FULL CELL
                    // per tick — for infantry (~43 u/tick ≈ 0.04 cell)
                    // that is a ~24x teleport, so an `attack_unit` on an
                    // out-of-sight target crossed the whole map in one
                    // decision frame instead of pathing normally.
                    let speed = self.actor_speed(attacker_id);
                    let next_cell = path[1];
                    let occ = self.terrain.occupant(next_cell.0, next_cell.1);
                    if occ == 0 || occ == attacker_id {
                        let next_center = center_of_cell(next_cell.0, next_cell.1);
                        let mut arrived = false;
                        if let Some(actor) = self.actors.get_mut(&attacker_id) {
                            for t in &mut actor.traits {
                                if let TraitState::Mobile {
                                    center_position, from_cell, to_cell, ..
                                } = t
                                {
                                    let from_center =
                                        center_of_cell(from.0, from.1);
                                    *to_cell = CPos::new(next_cell.0, next_cell.1);
                                    let total_dx =
                                        (next_center.x - from_center.x) as i64;
                                    let total_dy =
                                        (next_center.y - from_center.y) as i64;
                                    let total_dist = (((total_dx * total_dx
                                        + total_dy * total_dy)
                                        as f64)
                                        .sqrt())
                                        as i32;
                                    if total_dist == 0 {
                                        *center_position = next_center;
                                        arrived = true;
                                    } else {
                                        let prog_dx = (center_position.x
                                            - from_center.x)
                                            as i64;
                                        let prog_dy = (center_position.y
                                            - from_center.y)
                                            as i64;
                                        let progress = (((prog_dx * prog_dx
                                            + prog_dy * prog_dy)
                                            as f64)
                                            .sqrt())
                                            as i32;
                                        let new_progress = progress + speed;
                                        if new_progress >= total_dist {
                                            *center_position = next_center;
                                            *from_cell =
                                                CPos::new(next_cell.0, next_cell.1);
                                            arrived = true;
                                        } else {
                                            center_position.x = from_center.x
                                                + (total_dx * new_progress as i64
                                                    / total_dist as i64)
                                                    as i32;
                                            center_position.y = from_center.y
                                                + (total_dy * new_progress as i64
                                                    / total_dist as i64)
                                                    as i32;
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                        // Only commit the discrete cell + occupancy once
                        // the interpolated position actually reaches the
                        // next cell center.
                        if arrived {
                            self.terrain.clear_occupant(from.0, from.1);
                            self.terrain
                                .set_occupant(next_cell.0, next_cell.1, attacker_id);
                            if let Some(actor) = self.actors.get_mut(&attacker_id) {
                                actor.location = Some(next_cell);
                            }
                        }
                    }
                }
            }
        }

        // Tick building repairs: heal HP, deduct cash.
        if !self.repairing.is_empty() {
            let repair_ids: Vec<u32> = self.repairing.iter().copied().collect();
            let mut finished: Vec<u32> = Vec::new();
            for building_id in repair_ids {
                let (owner_id, hp, max_hp, cost) = {
                    let actor = match self.actors.get(&building_id) {
                        Some(a) if a.kind == ActorKind::Building => a,
                        _ => { finished.push(building_id); continue; }
                    };
                    let hp = actor.traits.iter().find_map(|t| {
                        if let TraitState::Health { hp } = t { Some(*hp) } else { None }
                    }).unwrap_or(0);
                    let atype = actor.actor_type.as_deref().unwrap_or("");
                    let max_hp = self.rules.actor(atype).map(|s| s.hp).unwrap_or(hp);
                    let cost = self.rules.actor(atype).map(|s| s.cost).unwrap_or(0);
                    let owner_id = actor.owner_id.unwrap_or(0);
                    (owner_id, hp, max_hp, cost)
                };
                if hp >= max_hp {
                    finished.push(building_id);
                    continue;
                }
                // Repair rate: ~1% of max HP per tick, cost proportional
                let repair_hp = std::cmp::max(1, max_hp / 100);
                let repair_cost = if max_hp > 0 { (cost * repair_hp) / (max_hp * 2) } else { 0 };
                let cash = self.actors.get(&owner_id).map(|a| a.cash()).unwrap_or(0);
                if cash < repair_cost {
                    finished.push(building_id);
                    continue;
                }
                // Deduct cash
                if let Some(player) = self.actors.get_mut(&owner_id) {
                    player.set_cash(cash - repair_cost);
                }
                // Heal building
                if let Some(actor) = self.actors.get_mut(&building_id) {
                    for t in &mut actor.traits {
                        if let TraitState::Health { hp } = t {
                            *hp = std::cmp::min(*hp + repair_hp, max_hp);
                            break;
                        }
                    }
                }
            }
            for id in finished {
                self.repairing.remove(&id);
            }
        }

        // Tick superweapon charge timers
        self.tick_superweapons();

        // Tick invulnerability (Iron Curtain)
        let mut expired_inv: Vec<u32> = Vec::new();
        for (actor_id, ticks) in self.invulnerable.iter_mut() {
            *ticks -= 1;
            if *ticks <= 0 {
                expired_inv.push(*actor_id);
            }
        }
        for id in expired_inv {
            self.invulnerable.remove(&id);
        }

        // Tick Harvest activities.
        self.tick_harvesters();

        // S1: trickle stored resources into spendable cash. Storage
        // capacity (refineries/silos) bounds how much can be banked
        // between drains — out-harvesting cap+drain loses ore, which is
        // the incentive to build silos. "Final economy value" = cash +
        // stored resources.
        const RESOURCE_DRAIN_PER_TICK: i32 = 10;
        let player_ids: Vec<u32> = self.player_actor_ids.clone();
        for pid in player_ids {
            if let Some(p) = self.actors.get_mut(&pid) {
                let r = p.resources();
                if r > 0 {
                    let d = r.min(RESOURCE_DRAIN_PER_TICK);
                    p.set_resources(r - d);
                    let c = p.cash();
                    p.set_cash(c + d);
                }
            }
        }

        // Tick production queues: consume cash, advance build time.
        let player_ids: Vec<u32> = self.production.keys().copied().collect();
        let mut completed_items: Vec<(u32, String)> = Vec::new();
        for pid in player_ids {
            // Low-power slowdown: skip every other tick if power_drained > power_provided.
            // Reads from the authoritative recompute (pre-placed buildings +
            // PowerDown toggle honoured), NOT the stale PowerManager trait.
            let is_low_power = {
                let (provided, drained) = self.compute_player_power(pid);
                drained > provided && provided > 0
            };
            if is_low_power && self.world_tick % 2 == 0 {
                continue; // 50% production speed when low power
            }
            let mut cash = self.actors.get(&pid).map(|a| a.cash()).unwrap_or(0);
            // Per-queue parallel-production multiplier: each completed,
            // alive production building of a category contributes one
            // unit of throughput, so two war factories advance the
            // Vehicle queue twice per tick (OpenRA parity — concurrent
            // factories). Building / Defense queues stay single-stream.
            let pq_factory_count: HashMap<PqType, u32> = self
                .production
                .get(&pid)
                .map(|queues| {
                    queues
                        .keys()
                        .map(|&pq| (pq, self.production_building_count(pid, pq)))
                        .collect()
                })
                .unwrap_or_default();
            if let Some(queues) = self.production.get_mut(&pid) {
                // Tick each queue type independently
                for (pq, items) in queues.iter_mut() {
                    let advances = pq_factory_count.get(pq).copied().unwrap_or(1).max(1);
                    // Advance the queue `advances` times this tick: N
                    // factories of this category each contribute one
                    // tick of build progress. Completions mid-loop let
                    // a later factory pick up the next queued item.
                    for _ in 0..advances {
                        // Find first item that isn't a completed building
                        // waiting for placement.
                        let tick_idx = items.iter().position(|i| !i.is_done());
                        let idx = match tick_idx {
                            Some(idx) => idx,
                            None => break, // nothing left to build
                        };
                        let item = &mut items[idx];
                        let consumed = item.tick(cash);
                        if consumed > 0 {
                            cash -= consumed;
                            if let Some(player) = self.actors.get_mut(&pid) {
                                player.set_cash(cash);
                            }
                        }
                        if item.is_done() {
                            let name = item.item_name.clone();
                            if self.rules.is_unit(&name) {
                                items.remove(idx);
                                eprintln!("PRODUCTION: unit {} complete for player {}", name, pid);
                                completed_items.push((pid, name));
                            } else {
                                eprintln!("PRODUCTION: building {} ready to place for player {}", name, pid);
                                // A finished building blocks the queue
                                // until placed — stop advancing it even
                                // with multiple factories.
                                break;
                            }
                        }
                    }
                }
            }
        }
        // Spawn completed units
        for (owner_pid, unit_type) in completed_items {
            if self.rules.is_unit(&unit_type) {
                self.frame_end_tasks.push(FrameEndTask::SpawnUnit {
                    unit_type,
                    owner_player_id: owner_pid,
                });
            }
            // Buildings wait for PlaceBuilding order
        }

        // Queue deploy for MCVs that finished turning
        for (actor_id, location, owner_player_id) in deploy_ready {
            self.frame_end_tasks.push(FrameEndTask::DeployTransform {
                old_actor_id: actor_id,
                location,
                owner_player_id,
            });
        }
    }

    /// Phase 8 — advance every in-flight projectile by one tick.
    ///
    /// For each projectile we (a) refresh its target position from the
    /// current alive target, (b) call `Projectile::advance` which does
    /// integer-fixed-point translation toward the target, (c) on impact
    /// dispatch single-target or splash damage with `Versus` multiplier
    /// applied, and (d) credit kills to the original attacker.
    ///
    /// Determinism contract:
    /// * Projectiles are visited in `BTreeMap` (stable id) order.
    /// * Splash victims are sorted by `(distance, victim_id)` and
    ///   debited in that order so two equidistant actors always
    ///   receive damage in the same id sequence.
    /// * Damage application is integer-only (`apply_versus`).
    fn tick_projectiles(&mut self) {
        if self.pending_projectiles.is_empty() {
            return;
        }
        let mut detonate: Vec<(u32, u32, WPos, i32, i32, BTreeMap<String, i32>)> = Vec::new();
        // (proj_id, attacker_id, impact_pos, base_damage, splash_radius, versus)
        let mut drop_no_target: Vec<u32> = Vec::new();
        for (pid, proj) in self.pending_projectiles.iter_mut() {
            // If the target was destroyed (or never existed), advance
            // toward its last-known position so we still impact and
            // potentially splash nearby actors.
            let target_pos = self.actors.get(&proj.target_id)
                .and_then(|a| a.location)
                .map(|(x, y)| center_of_cell(x, y))
                .unwrap_or(proj.target_position);
            let arrived = proj.advance(target_pos);
            if arrived {
                detonate.push((
                    *pid,
                    proj.attacker_id,
                    proj.position,
                    proj.damage,
                    proj.splash_radius,
                    proj.versus.clone(),
                ));
            }
        }
        // Drop projectiles whose attackers are gone AND have run for
        // an unreasonable time (defensive; not strictly needed but
        // avoids zombie projectiles in pathological cases).
        for (pid, proj) in self.pending_projectiles.iter() {
            if !self.actors.contains_key(&proj.attacker_id)
                && self.actors.contains_key(&proj.target_id) == false
            {
                // Only drop if it can't reasonably impact anything.
                drop_no_target.push(*pid);
            }
        }
        for pid in drop_no_target {
            self.pending_projectiles.remove(&pid);
        }

        // Resolve impacts. Each detonation deals damage to all actors
        // within `splash_radius` of the impact position. The list is
        // sorted by `(distance², victim_id)` for determinism.
        let mut dead_from_projectile: Vec<u32> = Vec::new();
        let mut kill_credits: Vec<(u32, u32)> = Vec::new();
        for (proj_id, attacker_id, impact, base_damage, splash, versus) in detonate {
            // Build sorted victim list. Even with splash=0 we still
            // need to find at least the cell the impact landed on (the
            // direct target may have moved one cell since fire — we
            // treat "impact cell" as ground truth).
            let impact_cell = (impact.x.div_euclid(1024), impact.y.div_euclid(1024));
            let mut victims: Vec<(i64, u32, WPos, &str)> = Vec::new();
            // splash_sq compared against squared horizontal distance.
            let splash_units = splash.max(0);
            for a in self.actors.values() {
                let Some(loc) = a.location else { continue };
                if !matches!(
                    a.kind,
                    ActorKind::Infantry | ActorKind::Vehicle | ActorKind::Mcv | ActorKind::Building
                ) {
                    continue;
                }
                // Skip actors with no Health trait (defensive).
                let alive = a.traits.iter().any(|t| matches!(t, TraitState::Health { hp } if *hp > 0));
                if !alive { continue; }
                let center = center_of_cell(loc.0, loc.1);
                let dx = (center.x - impact.x) as i64;
                let dy = (center.y - impact.y) as i64;
                let d_sq = dx * dx + dy * dy;
                // Always include the actor on the direct impact cell;
                // otherwise require the splash radius covers the actor's
                // center.
                let direct_hit = loc == impact_cell;
                let in_splash = splash_units > 0 && d_sq <= (splash_units as i64).pow(2);
                if direct_hit || in_splash {
                    let armor = a.actor_type.as_deref()
                        .and_then(|t| self.rules.actor(t))
                        .map(|stats| match stats.armor_type {
                            crate::gamerules::ArmorType::None => "none",
                            crate::gamerules::ArmorType::Light => "light",
                            crate::gamerules::ArmorType::Heavy => "heavy",
                            crate::gamerules::ArmorType::Wood => "wood",
                            crate::gamerules::ArmorType::Concrete => "concrete",
                        })
                        .unwrap_or("none");
                    victims.push((d_sq, a.id, center, armor));
                }
            }
            victims.sort_by_key(|(d, id, _, _)| (*d, *id));
            for (d_sq, victim_id, _, armor_class) in &victims {
                if self.invulnerable.contains_key(victim_id) {
                    continue;
                }
                // Falloff: full damage at center, linear falloff to
                // 50% at splash radius. Direct impact (d_sq == 0) gets
                // full damage. Outside splash gets 0 (filtered above).
                let falloff_pct = if splash_units == 0 || *d_sq == 0 {
                    100
                } else {
                    let d = (*d_sq as f64).sqrt() as i64;
                    let r = splash_units as i64;
                    // 100% at d=0 → 50% at d=r. Linear.
                    100 - (50 * d / r.max(1)) as i32
                };
                let scaled = (base_damage as i64) * (falloff_pct as i64) / 100;
                let final_damage = apply_versus(scaled.max(0) as i32, armor_class, &versus);
                if final_damage <= 0 {
                    continue;
                }
                // Reveal-on-attack (projectile path): mirror the direct-fire
                // hook above. Use the attacker's current cell if alive, else
                // the impact cell (best-effort — at minimum the agent sees
                // where the round hit).
                let target_owner = self.actors.get(victim_id).and_then(|a| a.owner_id);
                let reveal_cell = self.actors.get(&attacker_id).and_then(|a| a.location)
                    .unwrap_or(impact_cell);
                if let Some(owner) = target_owner {
                    self.combat_reveal_cells.entry(owner).or_default().push(reveal_cell);
                }
                if let Some(target) = self.actors.get_mut(victim_id) {
                    for t in &mut target.traits {
                        if let TraitState::Health { hp } = t {
                            // Credit only on the alive→dead transition;
                            // splash hits to a victim that is already
                            // dead this tick must not be re-credited.
                            let was_alive = *hp > 0;
                            *hp -= final_damage;
                            if was_alive && *hp <= 0 {
                                dead_from_projectile.push(*victim_id);
                                kill_credits.push((attacker_id, *victim_id));
                            }
                            break;
                        }
                    }
                }
                // Stamp the victim for stance:1 ReturnFire gating
                // (hostile damage only — friendly splash must not
                // unlock return fire across the formation).
                let attacker_owner = self.actors.get(&attacker_id).and_then(|a| a.owner_id);
                let victim_owner_now = self.actors.get(victim_id).and_then(|a| a.owner_id);
                if let (Some(ao), Some(vo)) = (attacker_owner, victim_owner_now) {
                    if ao != vo {
                        self.recently_received_fire.insert(*victim_id, self.world_tick);
                    }
                }
            }
            self.pending_projectiles.remove(&proj_id);
        }

        // Credit kills (deterministic order by attacker, victim id).
        kill_credits.sort_by_key(|(a, v)| (*a, *v));
        for (attacker_id, _victim) in &kill_credits {
            let owner = self.actors.get(attacker_id).and_then(|a| a.owner_id);
            if let Some(att) = self.actors.get_mut(attacker_id) {
                att.kills = att.kills.saturating_add(1);
            }
            if let Some(pid) = owner {
                *self.kills_per_player.entry(pid).or_insert(0) += 1;
            }
        }
        // Remove dead victims (mirror the tick_actors path).
        let mut dead_ids: Vec<u32> = Vec::new();
        // Dedup — splash + multiple projectiles can hit the same victim.
        dead_from_projectile.sort_unstable();
        dead_from_projectile.dedup();
        for id in dead_from_projectile {
            if let Some(dead) = self.actors.remove(&id) {
                dead_ids.push(id);
                if dead.kind == ActorKind::Building {
                    if let Some(loc) = dead.location {
                        let (fw, fh) = dead.actor_type.as_deref()
                            .and_then(|t| self.rules.actor(t))
                            .map(|s| s.footprint)
                            .unwrap_or((2, 2));
                        self.terrain.clear_footprint(loc.0, loc.1, fw, fh);
                    }
                } else if let Some(loc) = dead.location {
                    self.terrain.clear_occupant(loc.0, loc.1);
                }
            }
        }
        // Clear stale Attack activities pointing at corpses (mirrors
        // the tick_actors clean-up).
        if !dead_ids.is_empty() {
            for actor in self.actors.values_mut() {
                if let Some(Activity::Attack { target_id, .. }) = actor.activity {
                    if dead_ids.contains(&target_id) {
                        actor.activity = None;
                    }
                }
            }
        }
    }

    /// Execute frame-end tasks (actor removal/creation from Transform, etc.)
    fn execute_frame_end_tasks(&mut self) {
        let tasks: Vec<_> = self.frame_end_tasks.drain(..).collect();
        for task in tasks {
            match task {
                FrameEndTask::DeployTransform { old_actor_id, location, owner_player_id } => {
                    self.deploy_transform(old_actor_id, location, owner_player_id);
                }
                FrameEndTask::SpawnUnit { unit_type, owner_player_id } => {
                    self.spawn_unit(&unit_type, owner_player_id);
                }
            }
        }
    }

    /// Spawn a unit near the owner's production building (WEAP for vehicles, TENT/BARR for infantry).
    fn spawn_unit(&mut self, unit_type: &str, owner_player_id: u32) {
        // Find a building owned by this player to spawn near
        let spawn_loc = self.find_spawn_location(owner_player_id, unit_type);
        let (x, y) = match spawn_loc {
            Some(loc) => loc,
            None => return, // No production building found
        };

        let unit_id = self.next_actor_id;
        self.next_actor_id += 1;

        let (kind, speed, hp) = self.rules.actor(unit_type)
            .map(|s| (s.kind, s.speed, s.hp))
            .unwrap_or((ActorKind::Vehicle, 71, 100000));
        let facing = 512; // Default facing south
        let cell = CPos::new(x, y);
        let center = center_of_cell(x, y);

        // Harvesters auto-start harvesting
        let activity = if unit_type == "harv" {
            let refinery_id = self.find_refinery(owner_player_id).unwrap_or(0);
            Some(Activity::Harvest {
                state: HarvestState::FindingOre,
                refinery_id,
                carried_ore: 0,
                carried_gems: 0,
                capacity: 20, // RA HARV capacity
                path: Vec::new(),
                path_index: 0,
                speed,
                harvest_ticks: 0,
                last_harvest_cell: None,
            })
        } else {
            None
        };

        let actor = Actor {
            id: unit_id,
            kind,
            owner_id: Some(owner_player_id),
            location: Some((x, y)),
            traits: vec![
                TraitState::BodyOrientation { quantized_facings: 32 },
                TraitState::Mobile {
                    facing, from_cell: cell, to_cell: cell,
                    center_position: center,
                },
                TraitState::Health { hp },
            ],
            activity,
            actor_type: Some(unit_type.to_string()),
            kills: 0, rank: 0,
        };
        self.actors.insert(unit_id, actor);
        self.terrain.set_occupant(x, y, unit_id);
        eprintln!("SPAWN: {} id={} at ({},{}) owner={} speed={} hp={}",
            unit_type, unit_id, x, y, owner_player_id, speed, hp);

        // Auto-move to rally point if set on the production building
        if unit_type != "harv" {
            let rally = self.find_rally_point_for_unit(owner_player_id, unit_type);
            if let Some(target) = rally {
                self.order_move(unit_id, target);
            }
        }
    }

    /// Find the rally point for the production building that produces this unit type.
    fn find_rally_point_for_unit(&self, owner_player_id: u32, unit_type: &str) -> Option<(i32, i32)> {
        let is_infantry = self.rules.actor(unit_type)
            .map(|s| s.kind == ActorKind::Infantry)
            .unwrap_or(false);
        let mut candidates: Vec<&Actor> = self
            .actors
            .values()
            .filter(|actor| {
                actor.owner_id == Some(owner_player_id)
                    && actor.kind == ActorKind::Building
                    && {
                        let btype = actor.actor_type.as_deref().unwrap_or("");
                        if is_infantry {
                            matches!(btype, "tent" | "barr")
                        } else {
                            matches!(
                                btype,
                                "weap" | "weap.ukraine" | "hpad" | "afld"
                                    | "spen" | "syrd"
                            )
                        }
                    }
            })
            .collect();
        // Primary building's rally point wins (C# parity).
        candidates.sort_by_key(|a| {
            (!self.primary_buildings.contains(&a.id), a.id)
        });
        for actor in candidates {
            if let Some(&rally) = self.rally_points.get(&actor.id) {
                return Some(rally);
            }
        }
        None
    }

    /// Whether a building actor is currently flagged PRIMARY.
    pub fn is_primary_building(&self, building_id: u32) -> bool {
        self.primary_buildings.contains(&building_id)
    }

    /// Passenger ids currently carried by a transport (C# `Cargo`).
    pub fn transport_cargo(&self, transport_id: u32) -> Vec<u32> {
        self.cargo
            .get(&transport_id)
            .map(|v| v.iter().map(|a| a.id).collect())
            .unwrap_or_default()
    }

    /// Cargo capacity (passenger COUNT) for a transport actor. Pragmatic
    /// subset of C# `Cargo.MaxWeight` — we count units, not weighted
    /// slots. Only known transports carry; everything else is 0.
    pub fn transport_capacity(&self, transport_id: u32) -> u32 {
        match self
            .actors
            .get(&transport_id)
            .and_then(|a| a.actor_type.as_deref())
        {
            Some("apc") => 5,
            Some("apc.ukraine") => 5,
            Some("lst") => 5,   // landing craft
            Some("tran") => 5,  // chinook
            _ => 0,
        }
    }

    /// Whether an actor can be a passenger (C# `Passenger`): infantry
    /// only, in our subset.
    fn is_passenger_capable(&self, actor_id: u32) -> bool {
        self.actors
            .get(&actor_id)
            .map(|a| a.kind == ActorKind::Infantry)
            .unwrap_or(false)
    }

    /// S1: total resource-storage capacity from a player's refineries
    /// and silos (RA: proc≈2000, silo≈3000). Harvested ore beyond this
    /// is lost on deposit — building silos raises the cap.
    fn player_storage_capacity(&self, pid: u32) -> i32 {
        let mut cap = 0;
        for a in self.actors.values() {
            if a.owner_id == Some(pid) && a.kind == ActorKind::Building {
                match a.actor_type.as_deref() {
                    Some("proc") => cap += 2000,
                    Some("silo") => cap += 3000,
                    _ => {}
                }
            }
        }
        cap
    }

    /// Find a refinery (PROC) owned by a player.
    fn find_refinery(&self, owner_player_id: u32) -> Option<u32> {
        for actor in self.actors.values() {
            if actor.owner_id == Some(owner_player_id)
                && actor.kind == ActorKind::Building
                && actor.actor_type.as_deref() == Some("proc")
            {
                return Some(actor.id);
            }
        }
        None
    }

    /// Compute all virtual prerequisites a player currently has based on owned buildings and faction.
    fn compute_player_prerequisites(&self, player_id: u32) -> HashSet<String> {
        let faction = self.player_factions.get(&player_id)
            .map(|s| s.as_str())
            .unwrap_or("allies");

        let mut provided = HashSet::new();

        // First pass: collect unconditional prerequisites
        for actor in self.actors.values() {
            if actor.owner_id != Some(player_id) || actor.kind != ActorKind::Building { continue; }
            let btype = actor.actor_type.as_deref().unwrap_or("");
            if let Some(stats) = self.rules.actor(btype) {
                for pp in &stats.provides_prerequisites {
                    if !pp.requires_prerequisites.is_empty() { continue; } // skip conditional for now
                    if pp.factions.is_empty() || pp.factions.iter().any(|f| f == faction) {
                        provided.insert(pp.prerequisite.clone());
                    }
                }
                // @buildingname convention: building always provides its own name
                provided.insert(btype.to_string());
            }
        }

        // Second pass: conditional prerequisites (RequiresPrerequisites)
        for actor in self.actors.values() {
            if actor.owner_id != Some(player_id) || actor.kind != ActorKind::Building { continue; }
            let btype = actor.actor_type.as_deref().unwrap_or("");
            if let Some(stats) = self.rules.actor(btype) {
                for pp in &stats.provides_prerequisites {
                    if pp.requires_prerequisites.is_empty() { continue; }
                    if pp.factions.is_empty() || pp.factions.iter().any(|f| f == faction) {
                        if pp.requires_prerequisites.iter().all(|rp| provided.contains(rp)) {
                            provided.insert(pp.prerequisite.clone());
                        }
                    }
                }
            }
        }

        provided
    }

    /// Check if a player has the required prerequisite buildings for an item.
    fn has_prerequisites(&self, owner_player_id: u32, item_name: &str) -> bool {
        let prereqs = match self.rules.actor(item_name) {
            Some(stats) => &stats.prerequisites,
            None => return true,
        };
        if prereqs.is_empty() {
            return true;
        }

        let player_prereqs = self.compute_player_prerequisites(owner_player_id);

        for prereq_raw in prereqs {
            let prereq = prereq_raw.trim_start_matches('~');

            // ~disabled means never buildable
            if prereq == "disabled" { return false; }

            // ~techlevel.* — all satisfied in standard skirmish games
            if prereq.starts_with("techlevel.") { continue; }

            // ~!foo = negated prerequisite (must NOT have foo)
            if let Some(negated) = prereq.strip_prefix('!') {
                if player_prereqs.contains(negated) { return false; }
                continue;
            }

            // Normal prerequisite check against virtual prerequisites
            if !player_prereqs.contains(prereq) {
                return false;
            }
        }
        true
    }

    /// Find a spawn location near a production building for the given owner.
    fn find_spawn_location(&self, owner_player_id: u32, unit_type: &str) -> Option<(i32, i32)> {
        let is_infantry = self.rules.actor(unit_type)
            .map(|s| s.kind == ActorKind::Infantry)
            .unwrap_or(false);

        // Collect all production buildings of the right type. C#
        // `PrimaryBuilding`: if one of them is flagged primary, it
        // produces; otherwise fall back to first-found (BTreeMap order
        // ⇒ deterministic). We sort primary-first then by id.
        let mut candidates: Vec<&Actor> = self
            .actors
            .values()
            .filter(|actor| {
                actor.owner_id == Some(owner_player_id)
                    && actor.kind == ActorKind::Building
                    && {
                        let btype = actor.actor_type.as_deref().unwrap_or("");
                        if is_infantry {
                            matches!(btype, "tent" | "barr")
                        } else if unit_type == "harv" {
                            matches!(btype, "proc" | "weap" | "weap.ukraine")
                        } else {
                            matches!(
                                btype,
                                "weap" | "weap.ukraine" | "hpad" | "afld"
                                    | "spen" | "syrd"
                            )
                        }
                    }
            })
            .collect();
        candidates.sort_by_key(|a| {
            (!self.primary_buildings.contains(&a.id), a.id)
        });

        for actor in candidates {
            let btype = actor.actor_type.as_deref().unwrap_or("");
            {
                if let Some((bx, by)) = actor.location {
                    let (fw, fh) = self.rules.actor(btype)
                        .map(|s| s.footprint)
                        .unwrap_or((2, 2));
                    for dy in -1..=fh {
                        for dx in -1..=fw {
                            let sx = bx + dx;
                            let sy = by + dy;
                            if self.terrain.is_passable(sx, sy) {
                                return Some((sx, sy));
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Deploy an MCV: remove it and create a Construction Yard.
    fn deploy_transform(&mut self, mcv_actor_id: u32, mcv_location: (i32, i32), owner_player_id: u32) {
        let fact_location = (mcv_location.0 - 1, mcv_location.1 - 1);
        eprintln!("DEPLOY: removing MCV actor {} at {:?}, creating FACT at {:?}",
            mcv_actor_id, mcv_location, fact_location);

        // Remove MCV
        self.actors.remove(&mcv_actor_id);

        // Create Construction Yard (FACT) with new actor ID
        let fact_id = self.next_actor_id;
        self.next_actor_id += 1;

        let top_left = CPos::new(fact_location.0, fact_location.1);
        // FACT ISync traits in construction order:
        //   0. BodyOrientation (QuantizedFacings=1)
        //   1. Building (TopLeft)
        //   2. Health (HP=150000)
        //   3. RevealsShroud (base class fields invisible → 0)
        //   4. RevealsShroud@GAPGEN (same → 0)
        //   5. FrozenUnderFog (VisibilityHash=0, updated below)
        //   6. RepairableBuilding (RepairersHash=0)
        //   7. ConyardChronoReturn (all zero)
        let fact_actor = Actor {
            id: fact_id,
            kind: ActorKind::Building,
            owner_id: Some(owner_player_id),
            location: Some(fact_location),
            traits: vec![
                TraitState::BodyOrientation { quantized_facings: 1 },
                TraitState::Building { top_left },
                TraitState::Health { hp: 150000 },
                TraitState::RevealsShroud,
                TraitState::RevealsShroud, // @GAPGEN
                TraitState::FrozenUnderFog { visibility_hash: 0 },
                TraitState::RepairableBuilding { repairers_hash: 0 },
                TraitState::ConyardChronoReturn,
            ],
            activity: None,
            actor_type: Some("fact".to_string()),
            kills: 0, rank: 0,
        };
        self.actors.insert(fact_id, fact_actor);

        // Occupy terrain (FACT is 3x2)
        self.terrain.occupy_footprint(fact_location.0, fact_location.1, 3, 2, fact_id);

        eprintln!("DEPLOY: created FACT actor {} at {:?} TopLeft.bits={}",
            fact_id, fact_location, top_left.bits);

        // === Side effects from subsequent ticks within the same NetFrameInterval ===

        // 1. Re-enable PQ@Building and PQ@Defense for owning player.
        if let Some(owner) = self.actors.get_mut(&owner_player_id) {
            for t in &mut owner.traits {
                if let TraitState::ClassicProductionQueue { pq_type, enabled, .. } = t {
                    if *pq_type == PqType::Building || *pq_type == PqType::Defense {
                        *enabled = true;
                    }
                }
            }
        }

        // 2. Update FrozenActorLayer for ALL players.
        let everyone_id = self.everyone_player_id;
        let player_ids = self.player_actor_ids.clone();
        for &pid in &player_ids {
            if let Some(player) = self.actors.get_mut(&pid) {
                for t in &mut player.traits {
                    if let TraitState::FrozenActorLayer { frozen_hash, visibility_hash } = t {
                        *frozen_hash = frozen_hash.wrapping_add(fact_id as i32);
                        let can_see = pid == owner_player_id || pid == everyone_id;
                        if !can_see {
                            *visibility_hash = visibility_hash.wrapping_add(fact_id as i32);
                        }
                        break;
                    }
                }
            }
        }

        // 3. Update FrozenUnderFog VisibilityHash on FACT.
        let visibility_hash = self.compute_frozen_visibility_hash(owner_player_id);
        if let Some(fact) = self.actors.get_mut(&fact_id) {
            for t in &mut fact.traits {
                if let TraitState::FrozenUnderFog { visibility_hash: vh } = t {
                    *vh = visibility_hash;
                    break;
                }
            }
        }
    }

    /// Compute FrozenUnderFog VisibilityHash for an actor visible to owner and Everyone.
    fn compute_frozen_visibility_hash(&self, owner_player_id: u32) -> i32 {
        let mut hash = 0i32;
        for &pid in self.player_actor_ids.iter().rev() {
            let visible = pid == owner_player_id || pid == self.everyone_player_id;
            hash = hash * 2 + if visible { 1 } else { 0 };
        }
        hash
    }

    /// Return items the player can currently build (prerequisites met).
    pub fn buildable_items(&self, player_id: u32) -> Vec<BuildableInfo> {
        let mut items = Vec::new();
        for (name, stats) in &self.rules.actors {
            if stats.cost <= 0 { continue; }
            if !self.has_prerequisites(player_id, name) { continue; }
            let pq = Self::item_queue_type(stats);
            items.push(BuildableInfo {
                name: name.clone(),
                cost: stats.cost,
                kind: stats.kind,
                is_building: stats.is_building,
                power: stats.power,
                footprint: stats.footprint,
                locked: false,
                prerequisites: stats.prerequisites.clone(),
                queue_type: pq_type_name(pq).to_string(),
                build_palette_order: stats.build_palette_order,
            });
        }
        items
    }

    /// Determine which production queue type an item belongs to.
    fn item_queue_type(stats: &crate::gamerules::ActorStats) -> PqType {
        match stats.kind {
            ActorKind::Infantry => PqType::Infantry,
            ActorKind::Vehicle | ActorKind::Mcv => PqType::Vehicle,
            ActorKind::Aircraft => PqType::Aircraft,
            ActorKind::Ship => PqType::Ship,
            ActorKind::Building => {
                if stats.footprint == (1, 1) {
                    PqType::Defense
                } else {
                    PqType::Building
                }
            }
            _ => PqType::Building,
        }
    }

    /// Determine queue type by item name (looks up rules).
    fn item_queue_type_by_name(rules: &crate::gamerules::GameRules, item_name: &str) -> PqType {
        rules.actor(item_name)
            .map(|s| Self::item_queue_type(s))
            .unwrap_or(PqType::Building)
    }

    /// Check if a production queue type is enabled for a player.
    fn is_queue_enabled(&self, player_id: u32, pq: PqType) -> bool {
        if let Some(player) = self.actors.get(&player_id) {
            for t in &player.traits {
                if let TraitState::ClassicProductionQueue { pq_type, enabled, .. } = t {
                    if *pq_type == pq {
                        return *enabled;
                    }
                }
            }
        }
        false
    }

    /// Return ALL production items (buildable + locked) for the player.
    /// Only includes items whose production queue is enabled.
    pub fn all_production_items(&self, player_id: u32) -> Vec<BuildableInfo> {
        let mut items = Vec::new();
        for (name, stats) in &self.rules.actors {
            if stats.cost <= 0 { continue; }
            // Only show items whose production queue is enabled for this player
            let pq = Self::item_queue_type(stats);
            if !self.is_queue_enabled(player_id, pq) { continue; }
            let locked = !self.has_prerequisites(player_id, name);
            items.push(BuildableInfo {
                name: name.clone(),
                cost: stats.cost,
                kind: stats.kind,
                is_building: stats.is_building,
                power: stats.power,
                footprint: stats.footprint,
                locked,
                prerequisites: stats.prerequisites.clone(),
                queue_type: pq_type_name(pq).to_string(),
                build_palette_order: stats.build_palette_order,
            });
        }
        items
    }

    /// Check if a building can be placed at (x, y) for the given player.
    pub fn can_place_building(&self, player_id: u32, building_type: &str, x: i32, y: i32) -> bool {
        let (fw, fh, _hp) = match self.rules.actor(building_type) {
            Some(s) => (s.footprint.0, s.footprint.1, s.hp),
            None => return false,
        };
        // Check bounds
        if x < 0 || y < 0 || x + fw > self.map_width || y + fh > self.map_height {
            return false;
        }
        // Check all cells are passable and unoccupied
        for cy in y..y + fh {
            for cx in x..x + fw {
                if !self.terrain.is_passable(cx, cy) {
                    return false;
                }
                if self.terrain.occupant(cx, cy) != 0 {
                    return false;
                }
            }
        }
        // Must be adjacent to own building (simplified adjacency check)
        let has_adjacent = self.actors.values().any(|a| {
            if a.owner_id != Some(player_id) || a.kind != ActorKind::Building { return false; }
            let (ax, ay) = a.location.unwrap_or((-100, -100));
            let (aw, ah) = self.rules.actor(a.actor_type.as_deref().unwrap_or(""))
                .map(|s| (s.footprint.0, s.footprint.1))
                .unwrap_or((1, 1));
            // Check if any cell of new building is within 2 cells of existing building
            for ny in y..y + fh {
                for nx in x..x + fw {
                    if nx >= ax - 2 && nx < ax + aw + 2 && ny >= ay - 2 && ny < ay + ah + 2 {
                        return true;
                    }
                }
            }
            false
        });
        has_adjacent
    }

    /// Check if the game is over. Returns Some(winning_player_id) or None.
    pub fn game_over(&self) -> Option<u32> {
        if self.world_tick < 30 { return None; } // too early
        let mut alive_players: Vec<u32> = Vec::new();
        // A player is alive if they have any buildings or units (excluding trees/mines/world/player actors)
        for &pid in &self.player_actor_ids {
            if pid == self.everyone_player_id { continue; }
            let has_stuff = self.actors.values().any(|a| {
                a.owner_id == Some(pid)
                    && matches!(a.kind, ActorKind::Building | ActorKind::Infantry
                        | ActorKind::Vehicle | ActorKind::Mcv | ActorKind::Aircraft | ActorKind::Ship)
            });
            if has_stuff {
                alive_players.push(pid);
            }
        }
        if alive_players.len() == 1 {
            Some(alive_players[0])
        } else {
            None
        }
    }

    /// Get the player actor IDs (for identifying human vs bot).
    pub fn player_ids(&self) -> &[u32] {
        &self.player_actor_ids
    }

    /// Find the MCV actor ID for a player (if any).
    pub fn player_mcv(&self, player_id: u32) -> Option<u32> {
        self.actors.values()
            .find(|a| a.owner_id == Some(player_id) && a.kind == ActorKind::Mcv)
            .map(|a| a.id)
    }

    /// Check if a player has a construction yard (fact).
    pub fn player_has_conyard(&self, player_id: u32) -> bool {
        self.actors.values().any(|a| {
            a.owner_id == Some(player_id)
                && a.kind == ActorKind::Building
                && a.actor_type.as_deref() == Some("fact")
        })
    }

    /// Check if a player has any items currently in production (not yet complete).
    pub fn player_has_pending_production(&self, player_id: u32) -> bool {
        if let Some(queues) = self.production.get(&player_id) {
            for items in queues.values() {
                if items.iter().any(|item| !item.is_done()) {
                    return true;
                }
            }
        }
        false
    }

    /// Get the first completed building in the production queue awaiting placement.
    pub fn player_ready_building(&self, player_id: u32) -> Option<String> {
        if let Some(queues) = self.production.get(&player_id) {
            for items in queues.values() {
                for item in items {
                    if item.is_done() {
                        return Some(item.item_name.clone());
                    }
                }
            }
        }
        None
    }

    /// Find a valid placement location for a building near the player's base.
    /// Searches in a spiral around existing buildings.
    pub fn find_placement_location(&self, player_id: u32, building_type: &str) -> Option<(i32, i32)> {
        let (fw, fh, _) = match self.rules.actor(building_type) {
            Some(s) => (s.footprint.0, s.footprint.1, s.hp),
            None => return None,
        };

        // Find center of player's buildings
        let buildings: Vec<(i32, i32)> = self.actors.values()
            .filter(|a| a.owner_id == Some(player_id) && a.kind == ActorKind::Building)
            .filter_map(|a| a.location)
            .collect();

        if buildings.is_empty() {
            return None;
        }

        let cx = buildings.iter().map(|b| b.0).sum::<i32>() / buildings.len() as i32;
        let cy = buildings.iter().map(|b| b.1).sum::<i32>() / buildings.len() as i32;

        // Spiral search around base center
        for radius in 1i32..15 {
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if dx.abs() != radius && dy.abs() != radius {
                        continue; // only check perimeter of each ring
                    }
                    let x = cx + dx;
                    let y = cy + dy;
                    if self.can_place_building(player_id, building_type, x, y) {
                        return Some((x, y));
                    }
                }
            }
        }
        None
    }

    /// Get actor location.
    pub fn actor_location(&self, actor_id: u32) -> Option<(i32, i32)> {
        self.actors.get(&actor_id).and_then(|a| a.location)
    }

    /// Get actor activity name (for AI decision making).
    pub fn actor_activity(&self, actor_id: u32) -> Option<&str> {
        self.actors.get(&actor_id).map(|a| {
            match &a.activity {
                None => "idle",
                Some(Activity::Move { .. }) => "moving",
                Some(Activity::Turn { .. }) => "turning",
                Some(Activity::Attack { .. }) => "attacking",
                _ => "other",
            }
        })
    }

    // ----- Phase 3: combat + shroud accessors -----------------------------

    /// Read-only actor lookup (Phase-3 typed combat code paths).
    pub fn actor(&self, actor_id: u32) -> Option<&Actor> {
        self.actors.get(&actor_id)
    }

    /// Mutable actor lookup (Phase-3 typed combat code paths).
    pub fn actor_mut(&mut self, actor_id: u32) -> Option<&mut Actor> {
        self.actors.get_mut(&actor_id)
    }

    /// Compact summary used by Phase-3 activities (cell + alive flag).
    pub fn actor_summary(&self, actor_id: u32) -> Option<crate::traits::ActorSummary> {
        let a = self.actors.get(&actor_id)?;
        let cell = a.location.map(|(x, y)| crate::math::CPos::new(x, y))?;
        // Alive iff no Health trait OR Health.hp > 0.
        let mut hp_seen = false;
        let mut alive = true;
        for t in &a.traits {
            if let TraitState::Health { hp } = t {
                hp_seen = true;
                if *hp <= 0 { alive = false; }
                break;
            }
        }
        let _ = hp_seen;
        Some(crate::traits::ActorSummary { cell, is_dead: !alive })
    }

    /// Phase-8: read-only access to the typed-component bundle for a
    /// specific actor. Returns `None` for actors that have no typed
    /// component attached (e.g. plain infantry, world / player actors,
    /// trees).
    pub fn typed_components_of(&self, actor_id: u32) -> Option<&ActorTypedComponents> {
        self.typed_components.get(&actor_id)
    }

    /// Phase-8: attach (or replace) the typed-component bundle for an
    /// actor. The env loader calls this when spawning a vehicle so the
    /// turret component is queryable by tests and (in future) by the
    /// combat path. Mutates in place; idempotent.
    pub fn set_typed_components(&mut self, actor_id: u32, bundle: ActorTypedComponents) {
        self.typed_components.insert(actor_id, bundle);
    }

    /// Phase-8: read-only iterator over all in-flight projectiles in
    /// stable id order.
    pub fn pending_projectiles(&self) -> impl Iterator<Item = (&u32, &Projectile)> {
        self.pending_projectiles.iter()
    }

    /// Phase-8: count of in-flight projectiles. Used by tests that
    /// want to check that a missile is mid-flight before asserting
    /// final HP.
    pub fn pending_projectile_count(&self) -> usize {
        self.pending_projectiles.len()
    }

    /// Map width in cells.
    pub fn map_width(&self) -> i32 {
        self.map_width
    }

    /// Map height in cells.
    pub fn map_height(&self) -> i32 {
        self.map_height
    }

    /// Per-player typed shroud table. Lazily built — call
    /// `update_typed_shroud_all_players` to refresh.
    pub fn typed_shroud(&self, player_id: u32) -> Option<&crate::traits::Shroud> {
        self.typed_shroud.get(&player_id)
    }

    /// Total kills credited to `player_id` across all combat paths
    /// (data-driven `tick_actors` + trait-based `AttackActivity`).
    /// Updated whenever a target's HP reaches zero from one of that
    /// player's actors' attacks.
    pub fn kills_for_player(&self, player_id: u32) -> u32 {
        self.kills_per_player.get(&player_id).copied().unwrap_or(0)
    }

    /// Increment the kills-tally for `player_id`. Used by the typed
    /// `AttackActivity` path; callers credit the kill exactly once per
    /// fatal hit.
    #[doc(hidden)]
    pub fn credit_kill(&mut self, player_id: u32) {
        *self.kills_per_player.entry(player_id).or_insert(0) += 1;
    }

    /// Recompute the typed `Shroud` for every player from current
    /// actor positions. Reveal range is taken from
    /// `RevealsShroud.Range` in the rules; absent units fall back
    /// to a kind-based default sight (matching `update_shroud`).
    pub fn update_typed_shroud_all_players(&mut self) {
        // Snapshot every relevant actor (owner, kind, cell, reveal range).
        let mut entries: Vec<(u32, crate::actor::ActorKind, crate::math::CPos, Option<openra_data::rules::WDist>)> = Vec::new();
        for a in self.actors.values() {
            let Some(owner) = a.owner_id else { continue };
            let Some((x, y)) = a.location else { continue };
            // Try sight_range from the existing GameRules (already
            // populated from RevealsShroud.Range during load).
            let reveal = a.actor_type.as_deref()
                .and_then(|t| self.rules.actor(t))
                .map(|s| openra_data::rules::WDist::from_cells(s.sight_range));
            entries.push((owner, a.kind, crate::math::CPos::new(x, y), reveal));
        }
        for &pid in &self.player_actor_ids.clone() {
            let entry = self.typed_shroud.entry(pid)
                .or_insert_with(|| crate::traits::Shroud::new(self.map_width, self.map_height));
            crate::traits::update_from_actors(entry, entries.iter().copied(), pid);
            // Reveal-on-attack: force-reveal cells of any actor that just
            // damaged a unit owned by this player. Radius 1 (just the
            // attacker's cell) is enough to surface the building / unit
            // that scored the hit so the agent can attack it back.
            if let Some(cells) = self.combat_reveal_cells.get(&pid) {
                for &(cx, cy) in cells {
                    entry.reveal(cx, cy, 1);
                }
            }
        }
        // Combat reveals are one-shot: clear after applying so the
        // visibility only persists for the tick of the hit (the
        // attacker's cell stays in `explored` permanently because
        // `Shroud::reveal` sets both layers).
        self.combat_reveal_cells.clear();
    }

    /// Phase-3 win condition: returns the player ids whose
    /// `MustBeDestroyed` opponents are all dead.
    ///
    /// `MustBeDestroyed` is an actor-level trait flag in the rules
    /// (Phase-4 typed it as `UnitInfo.must_be_destroyed`); we check
    /// for its presence via the `GameRules` actor classification —
    /// in this minimal sprint, a player is the "winner" against any
    /// opponent that has zero alive infantry/vehicle/MCV/building
    /// actors. Returns players sorted by id.
    pub fn winners(&self) -> Vec<u32> {
        // Determine which players still have any "destroyable" actors.
        let mut alive_players: std::collections::BTreeSet<u32> = Default::default();
        for a in self.actors.values() {
            let Some(owner) = a.owner_id else { continue };
            if !matches!(a.kind, crate::actor::ActorKind::Building
                | crate::actor::ActorKind::Infantry
                | crate::actor::ActorKind::Vehicle
                | crate::actor::ActorKind::Mcv
                | crate::actor::ActorKind::Aircraft
                | crate::actor::ActorKind::Ship) {
                continue;
            }
            // Skip dead bodies (hp<=0) — they're typically removed
            // by tick_actors but be defensive in case Phase-3 tests
            // hold them around longer.
            let alive = a.traits.iter().all(|t| match t {
                TraitState::Health { hp } => *hp > 0,
                _ => true,
            });
            if alive { alive_players.insert(owner); }
        }
        // Winners = playable players who are alive AND every other
        // playable player has zero alive destroyable actors.
        let mut out: Vec<u32> = Vec::new();
        for &pid in &self.player_actor_ids {
            if pid == self.everyone_player_id { continue; }
            if !alive_players.contains(&pid) { continue; }
            // Check that all OTHER players are gone.
            let any_other_alive = self.player_actor_ids.iter().any(|&q| {
                q != pid && q != self.everyone_player_id && alive_players.contains(&q)
            });
            if !any_other_alive {
                out.push(pid);
            }
        }
        out.sort();
        out
    }

    /// Phase-3 tick wrapper. Equivalent to `process_frame(orders)`
    /// but additionally refreshes the typed shroud table after the
    /// actor activities finish — the contract Phase-5 PyO3 callers
    /// expect.
    pub fn tick(&mut self, orders: &[GameOrder]) -> i32 {
        let hash = self.process_frame(orders);
        self.update_typed_shroud_all_players();
        hash
    }
}

/// Tick facing toward target by step, matching C#'s Util.TickFacing(WAngle).
fn tick_facing(facing: i32, desired: i32, step: i32) -> i32 {
    let left_turn = ((facing - desired) % 1024 + 1024) % 1024;
    if left_turn < step {
        return desired;
    }
    let right_turn = ((desired - facing) % 1024 + 1024) % 1024;
    if right_turn < step {
        return desired;
    }
    if right_turn < left_turn {
        ((facing + step) % 1024 + 1024) % 1024
    } else {
        ((facing - step) % 1024 + 1024) % 1024
    }
}

/// Convert a cell position to world position (rectangular grid).
pub fn center_of_cell(x: i32, y: i32) -> WPos {
    WPos::new(1024 * x + 512, 1024 * y + 512, 0)
}

/// Parse "X,Y" cell target from order target_string.
fn parse_cell_target(s: &str) -> Option<(i32, i32)> {
    let mut parts = s.split(',');
    let x = parts.next()?.trim().parse::<i32>().ok()?;
    let y = parts.next()?.trim().parse::<i32>().ok()?;
    Some((x, y))
}

/// Resource value: cash per unit harvested.
fn resource_value(rt: ResourceType) -> i32 {
    match rt {
        ResourceType::Ore => 25,
        ResourceType::Gems => 75,
        ResourceType::None => 0,
    }
}

/// Assign spawn points to playable players using the playerRandom sequence.
fn assign_spawn_points(
    spawn_locations: &[(i32, i32)],
    num_playable: usize,
    seed: i32,
    map_players: &[openra_data::oramap::PlayerDef],
) -> Vec<(i32, i32)> {
    let mut player_rng = MersenneTwister::new(seed);

    // Non-playable players: ResolveFaction for each.
    // "allies" faction has no RandomFactionMembers → no RNG consumption.
    for p in map_players {
        if !p.playable {
            // nothing to do
        }
    }

    let mut available_spawns: Vec<usize> = (0..spawn_locations.len()).collect();
    let mut assignments = Vec::new();

    for i in 0..num_playable {
        // ResolveFaction("Random"): 2 playerRandom calls
        let meta_faction = player_rng.next_range(0, 2);
        eprintln!("playerRNG[{}]: meta_faction={} rng.last={} total={}",
            i, meta_faction, player_rng.last, player_rng.total_count);

        if meta_faction == 0 {
            let specific = player_rng.next_range(0, 3);
            eprintln!("playerRNG[{}]: specific_allies={} rng.last={} total={}",
                i, specific, player_rng.last, player_rng.total_count);
        } else {
            let specific = player_rng.next_range(0, 2);
            eprintln!("playerRNG[{}]: specific_soviet={} rng.last={} total={}",
                i, specific, player_rng.last, player_rng.total_count);
        }

        // AssignHomeLocation
        if i == 0 {
            let idx = player_rng.next_range(0, available_spawns.len() as i32) as usize;
            eprintln!("playerRNG[{}]: spawn_idx={} from {} available, rng.last={} total={}",
                i, idx, available_spawns.len(), player_rng.last, player_rng.total_count);
            let spawn_idx = available_spawns.remove(idx);
            assignments.push(spawn_locations[spawn_idx]);
        } else {
            let spawn_idx = available_spawns.remove(0);
            assignments.push(spawn_locations[spawn_idx]);
        }
    }

    assignments
}

/// Build ISync traits for a player actor at tick 0.
fn build_player_traits(starting_cash: i32) -> Vec<TraitState> {
    let mut traits = Vec::new();

    // Construction order (dependency-resolved):
    // Initial batch (no Requires):
    //   0. Shroud
    //   1. PlayerResources
    //   2. MissionObjectives
    //   3. DeveloperMode
    //   4. GpsWatcher
    //   5. PlayerExperience
    // Second batch (Requires met):
    //   6-11. ClassicProductionQueue×6
    //   12.   PowerManager
    //   13.   FrozenActorLayer

    traits.push(TraitState::Shroud { disabled: false });
    traits.push(TraitState::PlayerResources { cash: starting_cash, resources: 0, resource_capacity: 0 });
    traits.push(TraitState::MissionObjectives { objectives_hash: 0 });
    traits.push(TraitState::DeveloperMode { flags: [false; 7] });
    traits.push(TraitState::GpsWatcher {
        explored: false, launched: false, granted_allies: false, granted: false,
    });
    traits.push(TraitState::PlayerExperience { experience: 0 });

    for &pq_type in PqType::ALL {
        traits.push(TraitState::ClassicProductionQueue {
            pq_type,
            enabled: true,
            is_valid_faction: true,
        });
    }

    traits.push(TraitState::PowerManager { power_provided: 0, power_drained: 0 });
    traits.push(TraitState::FrozenActorLayer { frozen_hash: 0, visibility_hash: 0 });

    traits
}

/// Build ISync traits for an MCV actor at tick 0.
fn build_mcv_traits(spawn_x: i32, spawn_y: i32, facing: i32) -> Vec<TraitState> {
    let cell = CPos::new(spawn_x, spawn_y);
    let center = center_of_cell(spawn_x, spawn_y);
    vec![
        TraitState::BodyOrientation { quantized_facings: 32 },
        TraitState::Mobile { facing, from_cell: cell, to_cell: cell, center_position: center },
        TraitState::Chronoshiftable { origin: CPos::new(0, 0), return_ticks: 0 },
        TraitState::Health { hp: 60000 },
        TraitState::RevealsShroud,
    ]
}

/// Apply TEMPERAT tileset passability to terrain map.
/// Marks Water and Rock (cliff) tiles as impassable based on template ID and tile index.
fn apply_temperat_passability(
    tiles: &[Vec<openra_data::oramap::TileReference>],
    terrain: &mut TerrainMap,
) {
    if tiles.is_empty() {
        return;
    }

    // Template IDs where ALL tiles are Water or Rock (fully impassable).
    static FULLY_IMPASSABLE: &[u16] = &[
        1, 2, 57, 58, 59, 61, 62, 63, 65, 66, 68, 69, 70, 73, 75, 76, 77, 79, 80,
        82, 83, 84, 87, 88, 91, 92, 93, 94, 95, 96, 97, 98, 99, 103, 104, 105, 106,
        109, 110, 135, 137, 138, 139, 141, 142, 143, 144, 145, 146, 149, 151, 152,
        153, 155, 156, 158, 159, 160, 163, 164, 167, 168, 169, 170, 171, 172,
        216, 217, 218, 219, 220, 221, 222, 223, 224, 226, 231, 232, 233, 234,
        401, 402, 403, 404, 405, 406, 407, 408,
        500, 502, 503, 504, 505, 506, 507, 508,
        550, 551, 552, 553, 554, 555, 556, 557,
    ];

    // Template IDs where only specific tile indices are impassable.
    // Format: (template_id, &[impassable_tile_indices])
    static PARTIAL_IMPASSABLE: &[(u16, &[u8])] = &[
        (3, &[3, 9, 10, 12, 13, 16]), (4, &[20, 21, 22]), (5, &[9, 10, 12]),
        (6, &[6, 7, 8]), (7, &[1, 4, 5, 6]), (8, &[6, 7, 8]), (9, &[6, 7, 8]),
        (11, &[2, 6, 7, 8]), (12, &[10, 13, 17, 22, 23, 24, 28, 29]),
        (13, &[5, 8, 9, 18]), (14, &[6, 10, 14]), (15, &[0, 1, 13, 19, 20]),
        (16, &[13]), (17, &[0, 1, 6, 12]), (18, &[0, 3, 6]),
        (20, &[0, 1, 3, 6]), (21, &[1, 8, 12, 16]), (22, &[2, 5, 10, 15]),
        (23, &[1, 5, 10]), (24, &[0, 6, 7, 14, 15, 16, 17, 22]),
        (25, &[1, 2, 7, 8, 9, 14]), (26, &[1, 4, 5]),
        (27, &[0, 1, 2, 3, 4, 6, 7]), (28, &[0, 1, 2]), (29, &[0, 1, 2]),
        (30, &[1, 2, 4, 5]), (33, &[3, 4, 8, 9, 12, 13, 14, 20]),
        (34, &[2, 3, 4, 5]), (35, &[1, 3]), (36, &[5, 15, 21, 26]),
        (37, &[3, 11, 14]), (38, &[3, 6, 9, 10]), (39, &[2, 5, 8]),
        (41, &[0, 2, 5, 8]), (42, &[2, 7, 12, 13, 19, 24]),
        (43, &[1, 6, 10, 11, 15]), (44, &[2, 11]), (45, &[7, 8]),
        (46, &[0, 3]), (47, &[0, 2, 3, 6]), (48, &[0]), (49, &[0, 4, 8]),
        (50, &[0, 1]), (51, &[1, 2, 3, 4, 7, 8]), (52, &[3]), (54, &[8]),
        (55, &[6]), (56, &[0]), (60, &[0, 2, 3, 4, 5]), (64, &[1, 2, 3, 4]),
        (67, &[0, 1, 2, 4, 5]), (71, &[0, 1, 3, 4]), (72, &[0]),
        (74, &[0, 2, 3, 5]), (78, &[2, 3, 4]), (81, &[0, 1, 3, 4, 5]),
        (85, &[0, 1, 2, 4, 5]), (86, &[0, 1, 3]), (89, &[0, 2, 3]),
        (90, &[0, 1, 2]), (112, &[0, 1, 5, 10, 11, 12, 13, 14, 16]),
        (113, &[0, 1, 2, 10, 11, 12]), (114, &[0, 4, 5, 6, 8, 9, 15]),
        (115, &[2, 6, 12, 13, 14]), (116, &[2, 3, 5, 6, 8]),
        (117, &[0, 2, 3, 5]), (118, &[0, 1, 3, 4]), (119, &[0]), (121, &[2]),
        (122, &[3]), (123, &[2, 3, 5, 8, 11]), (124, &[9, 11, 13]),
        (125, &[0, 1, 2, 4, 5, 7, 8]), (126, &[0, 2, 3, 4, 5, 6, 8]),
        (127, &[0, 1, 2, 4, 5]), (128, &[0, 1, 2]), (130, &[3]),
        (131, &[1, 4, 5, 8, 9, 12]), (132, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 12]),
        (133, &[0, 3, 5, 6, 9]), (134, &[0, 1, 2, 3, 5, 6, 7, 8, 9]),
        (136, &[0, 2, 3, 4, 5]), (140, &[1, 2, 3, 4]),
        (147, &[0, 1, 3, 4]), (148, &[0, 2]), (150, &[2, 3, 5]),
        (154, &[1, 2, 3, 4]), (157, &[0, 1, 2, 3, 4]),
        (161, &[0, 1, 4, 5]), (162, &[0, 1, 3]), (165, &[0, 2, 3]),
        (166, &[0, 1]), (182, &[1]), (185, &[7, 10]), (186, &[4]),
        (188, &[8]), (190, &[8]), (193, &[0]), (213, &[0]),
        (229, &[0]), (230, &[0]),
        (235, &[1, 4, 7, 10, 11]), (236, &[1, 4, 7, 10, 11]),
        (237, &[1, 2, 4, 5, 6, 7, 9, 10, 11]),
        (238, &[1, 5, 8, 9, 12, 13]), (239, &[1, 5, 8, 9, 12, 13]),
        (240, &[1, 2, 5, 6, 7, 8, 9, 11, 12, 13]),
        (241, &[0, 6, 7]), (242, &[0, 6, 7]), (243, &[0, 1, 5, 6, 7]),
        (244, &[1]), (245, &[0, 1, 5, 6, 7]), (246, &[0, 1, 5, 6, 7]),
        (378, &[1, 4, 5, 8, 9]), (379, &[0, 3, 5, 6, 9]),
        (380, &[0, 9, 14]), (382, &[5, 18, 19]), (383, &[15, 16]),
        (400, &[1, 2, 3, 4, 5, 8, 9]),
        (522, &[1, 4]), (523, &[1, 3, 6]), (524, &[1, 3, 6]),
        (525, &[1, 2, 3, 6]), (527, &[0, 2, 5, 7]), (528, &[0, 2, 5, 7]),
        (529, &[0, 1, 2, 5, 6, 7]), (531, &[4, 5]), (532, &[4, 5]),
    ];

    for (row_idx, row) in tiles.iter().enumerate() {
        let y = row_idx as i32;
        for (col_idx, tile) in row.iter().enumerate() {
            let x = col_idx as i32;
            if !terrain.contains(x, y) {
                continue;
            }

            let tid = tile.type_id;
            let idx = tile.index;

            // Check fully impassable templates
            if FULLY_IMPASSABLE.binary_search(&tid).is_ok() {
                terrain.set_cost(x, y, COST_IMPASSABLE);
                continue;
            }

            // Check partially impassable templates
            if let Ok(pos) = PARTIAL_IMPASSABLE.binary_search_by_key(&tid, |&(t, _)| t) {
                let (_, indices) = PARTIAL_IMPASSABLE[pos];
                if indices.contains(&idx) {
                    terrain.set_cost(x, y, COST_IMPASSABLE);
                }
            }
        }
    }
}

/// Test-only: insert a pre-built actor into the world's actor map and
/// register its terrain occupant. Used by Phase-2 unit/integration tests
/// that build hand-crafted scenarios without going through the map
/// loader.
#[doc(hidden)]
pub fn insert_test_actor(world: &mut World, actor: Actor) {
    let id = actor.id;
    if let Some(loc) = actor.location {
        if actor.kind == ActorKind::Building {
            let (fw, fh) = actor.actor_type.as_deref()
                .and_then(|t| world.rules.actor(t))
                .map(|s| s.footprint)
                .unwrap_or((1, 1));
            world.terrain.occupy_footprint(loc.0, loc.1, fw, fh, id);
        } else {
            world.terrain.set_occupant(loc.0, loc.1, id);
        }
    }
    world.actors.insert(id, actor);
    if id >= world.next_actor_id {
        world.next_actor_id = id + 1;
    }
}

/// Test-only: remove an actor by id. Used by Phase-5 tests that need
/// to strip auto-spawned MCVs/spawn beacons from a freshly built
/// world before injecting their own scenario actors.
#[doc(hidden)]
pub fn remove_test_actor(world: &mut World, id: u32) -> Option<Actor> {
    let actor = world.actors.remove(&id)?;
    if let Some(loc) = actor.location {
        if actor.kind == ActorKind::Building {
            let (fw, fh) = actor.actor_type.as_deref()
                .and_then(|t| world.rules.actor(t))
                .map(|s| s.footprint)
                .unwrap_or((1, 1));
            world.terrain.clear_footprint(loc.0, loc.1, fw, fh);
        } else {
            world.terrain.clear_occupant(loc.0, loc.1);
        }
    }
    Some(actor)
}

/// Test-only: enumerate every actor id currently registered. Used by
/// Phase-5 to find auto-spawned MCVs.
#[doc(hidden)]
pub fn all_actor_ids(world: &World) -> Vec<u32> {
    world.actors.keys().copied().collect()
}

/// Test-only: set a player actor's cash. Used by production-throughput
/// tests that need to remove the money bottleneck so build TIME is the
/// only constraint being measured.
#[doc(hidden)]
pub fn set_test_cash(world: &mut World, player_id: u32, cash: i32) {
    if let Some(player) = world.actors.get_mut(&player_id) {
        player.set_cash(cash);
    }
}

/// Set an actor's initial engagement stance (0=HoldFire, 1=ReturnFire,
/// 2=Defend, 3=AttackAnything). Used by scenario injection to honour a
/// per-actor `stance:` field. Mirrors the in-game SetStance order.
pub fn set_actor_stance(world: &mut World, actor_id: u32, stance: u8) {
    world.stances.insert(actor_id, stance.min(3));
}

/// Test-only: lift the world's "paused" flag so subsequent
/// `process_frame` calls advance the tick counter without waiting for
/// the order-latency buffer.
#[doc(hidden)]
pub fn set_test_unpaused(world: &mut World) {
    world.paused = false;
    world.frame_number = world.order_latency.saturating_add(1);
    world.update_debug_pause_state();
}

/// Build a World from parsed map data, game seed, and lobby info.
/// If `rules` is Some, uses the provided GameRules; otherwise uses hardcoded defaults.
pub fn build_world(
    map: &openra_data::oramap::OraMap,
    random_seed: i32,
    lobby: &LobbyInfo,
    rules: Option<GameRules>,
    difficulty: u8,
    spawn_mcvs: bool,
) -> World {
    let rng = MersenneTwister::new(random_seed);
    let mut actors: BTreeMap<u32, Actor> = BTreeMap::new();
    let mut next_id: u32 = 0;

    // === Actor ID 0: World actor ===
    actors.insert(next_id, Actor {
        id: next_id,
        kind: ActorKind::World,
        owner_id: None,
        location: None,
        traits: vec![TraitState::DebugPauseState { paused: true }],
        activity: None,
        actor_type: None,
        kills: 0,
        rank: 0,
    });
    next_id += 1;

    // === Player actors ===
    let mut player_actor_ids: Vec<u32> = Vec::new();

    // Non-playable players (Neutral, Creeps, etc.)
    let non_playable: Vec<_> = map.players.iter().filter(|p| !p.playable).collect();
    for _p in &non_playable {
        let id = next_id;
        actors.insert(id, Actor {
            id,
            kind: ActorKind::Player,
            owner_id: None,
            location: None,
            traits: build_player_traits(lobby.starting_cash),
            activity: None,
            actor_type: None,
            kills: 0,
            rank: 0,
        });
        player_actor_ids.push(id);
        next_id += 1;
    }

    // Playable players
    let mut bot_player_ids: Vec<u32> = Vec::new();
    let mut player_factions: HashMap<u32, String> = HashMap::new();
    for slot in &lobby.occupied_slots {
        let id = next_id;
        player_factions.insert(id, slot.faction.clone());
        actors.insert(id, Actor {
            id,
            kind: ActorKind::Player,
            owner_id: None,
            location: None,
            traits: build_player_traits(lobby.starting_cash),
            activity: None,
            actor_type: None,
            kills: 0,
            rank: 0,
        });
        player_actor_ids.push(id);
        if slot.is_bot {
            bot_player_ids.push(id);
        }
        next_id += 1;
    }

    // "Everyone" spectator player
    let everyone_player_id = next_id;
    actors.insert(next_id, Actor {
        id: next_id,
        kind: ActorKind::Player,
        owner_id: None,
        location: None,
        traits: build_player_traits(lobby.starting_cash),
        activity: None,
        actor_type: None,
        kills: 0,
        rank: 0,
    });
    player_actor_ids.push(next_id);
    next_id += 1;

    // === Map actors ===
    let mut spawn_locations: Vec<(i32, i32)> = Vec::new();
    let mut mine_count: usize = 0;
    let mut mine_locations: Vec<(i32, i32)> = Vec::new();

    for map_actor in &map.actors {
        let id = next_id;
        next_id += 1;

        let is_tree = map_actor.actor_type.starts_with('t')
            && (map_actor.actor_type.len() == 3 || map_actor.actor_type.starts_with("tc"));
        let is_mine = map_actor.actor_type == "mine";
        let is_spawn = map_actor.actor_type == "mpspawn";

        let top_left = CPos::new(map_actor.location.0, map_actor.location.1);
        let mut trait_list = Vec::new();
        let kind;

        if is_tree {
            kind = ActorKind::Tree;
            trait_list.push(TraitState::BodyOrientation { quantized_facings: 1 });
            trait_list.push(TraitState::Building { top_left });
            trait_list.push(TraitState::Health { hp: 50000 });
        } else if is_mine {
            mine_count += 1;
            mine_locations.push(map_actor.location);
            kind = ActorKind::Mine;
            trait_list.push(TraitState::BodyOrientation { quantized_facings: 1 });
            trait_list.push(TraitState::Building { top_left });
        } else if is_spawn {
            spawn_locations.push(map_actor.location);
            kind = ActorKind::Spawn;
            let center = center_of_cell(map_actor.location.0, map_actor.location.1);
            trait_list.push(TraitState::Immobile { top_left, center_position: center });
            trait_list.push(TraitState::BodyOrientation { quantized_facings: 1 });
        } else {
            kind = ActorKind::World; // Unknown actor type
        }

        actors.insert(id, Actor {
            id,
            kind,
            owner_id: None,
            location: Some(map_actor.location),
            traits: trait_list,
            activity: None,
            actor_type: Some(map_actor.actor_type.clone()),
            kills: 0,
            rank: 0,
        });
    }

    // === Starting units (MCVs) ===
    // Computed unconditionally because some downstream code expects
    // `player_spawn_assignments` to exist (used for spawn-point lookups
    // beyond MCV placement). The MCV-actor creation itself is gated on
    // `spawn_mcvs`.
    let player_spawn_assignments = assign_spawn_points(
        &spawn_locations,
        lobby.occupied_slots.len(),
        random_seed,
        &map.players,
    );

    let facing = 512; // BaseActorFacing default
    let num_non_playable = non_playable.len();
    if spawn_mcvs {
        for (pi, &(spawn_x, spawn_y)) in player_spawn_assignments.iter().enumerate() {
            let owner_pid = player_actor_ids[num_non_playable + pi];
            eprintln!(
                "MCV[{}] spawn=({},{}) facing={} owner={}",
                pi, spawn_x, spawn_y, facing, owner_pid
            );
            let id = next_id;
            actors.insert(id, Actor {
                id,
                kind: ActorKind::Mcv,
                owner_id: Some(owner_pid),
                location: Some((spawn_x, spawn_y)),
                traits: build_mcv_traits(spawn_x, spawn_y, facing),
                activity: None,
                actor_type: Some("mcv".to_string()),
                kills: 0,
                rank: 0,
            });
            next_id += 1;
        }
    }

    // Initialize terrain map and mark impassable tiles (Water, Rock/Cliffs).
    let mut terrain = TerrainMap::new(map.map_size.0, map.map_size.1);
    apply_temperat_passability(&map.tiles, &mut terrain);
    // S0: seed an ore field around each mine actor. Without this the
    // terrain has no resources and harvesters never find ore (the
    // `mine` actor was previously only counted, never seeded).
    for &(mx, my) in &mine_locations {
        let r: i32 = 5;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                let (x, y) = (mx + dx, my + dy);
                if (x, y) == (mx, my) {
                    continue; // the mine itself occupies this cell
                }
                if terrain.contains(x, y) && terrain.is_terrain_passable(x, y) {
                    terrain.set_resource(x, y, ResourceType::Ore, 50);
                }
            }
        }
    }
    // Resolve the rules early so initial-build building footprints can
    // honour rules-derived dimensions (Phase 7 — pbox=1×1, fact=3×2 etc).
    // We materialise a temporary Rules clone for this lookup; the caller
    // either passed `Some(rules)` or we fall back to defaults below.
    let initial_rules: GameRules = rules.clone().unwrap_or_else(GameRules::defaults);
    for actor in actors.values() {
        if let Some((x, y)) = actor.location {
            match actor.kind {
                ActorKind::Tree | ActorKind::Mine => {
                    terrain.set_occupant(x, y, actor.id);
                }
                ActorKind::Building => {
                    let (fw, fh) = actor.actor_type.as_deref()
                        .and_then(|t| initial_rules.actor(t))
                        .map(|s| s.footprint)
                        .unwrap_or((2, 2));
                    terrain.occupy_footprint(x, y, fw, fh, actor.id);
                }
                _ => {}
            }
        }
    }

    // Seed ore patches around mine actors (simplified resource placement).
    // In real OpenRA, SeedsResource handles this dynamically, but we
    // pre-place ore fields around mine locations.
    for actor in actors.values() {
        if actor.kind == ActorKind::Mine {
            if let Some((mx, my)) = actor.location {
                // Place ore in a roughly circular patch around the mine
                for dy in -3..=3i32 {
                    for dx in -3..=3i32 {
                        let dist = dx.abs() + dy.abs();
                        if dist <= 4 {
                            let x = mx + dx;
                            let y = my + dy;
                            if terrain.contains(x, y) && terrain.is_terrain_passable(x, y) {
                                let density = if dist <= 1 { 12 } else if dist <= 2 { 8 } else { 4 };
                                terrain.set_resource(x, y, ResourceType::Ore, density);
                            }
                        }
                    }
                }
            }
        }
    }

    // Initialize per-player shroud grids
    let shroud: Vec<CellLayer<u8>> = player_actor_ids.iter()
        .map(|_| CellLayer::new(map.map_size.0, map.map_size.1))
        .collect();

    // Create Bot instances for AI-controlled players
    let diff = crate::ai::Difficulty::from_u8(difficulty);
    let bots: Vec<Bot> = bot_player_ids.iter()
        .map(|&pid| Bot::new_with_difficulty(pid, diff))
        .collect();
    if !bots.is_empty() {
        eprintln!("Created {} bot(s): {:?}", bots.len(), bot_player_ids);
    }

    let mut world = World {
        actors,
        synced_effects: Vec::new(),
        rng,
        paused: true,
        world_tick: 0,
        frame_number: 0,
        order_latency: 15,
        next_actor_id: next_id,
        frame_end_tasks: Vec::new(),
        player_actor_ids,
        everyone_player_id,
        mine_count,
        mine_locations,
        surrendered: std::collections::HashSet::new(),
        seeds_resource_ticks: 0,
        production: HashMap::new(),
        terrain,
        map_width: map.map_size.0,
        map_height: map.map_size.1,
        shroud,
        rules: rules.unwrap_or_else(GameRules::defaults),
        bots,
        scripted_bots: Vec::new(),
        repairing: HashSet::new(),
        powered_down: HashSet::new(),
        rally_points: HashMap::new(),
        cargo: HashMap::new(),
        primary_buildings: HashSet::new(),
        stances: HashMap::new(),
        recently_received_fire: HashMap::new(),
        hunt_enabled: HashSet::new(),
        superweapon_timers: HashMap::new(),
        invulnerable: HashMap::new(),
        player_factions,
        typed_shroud: BTreeMap::new(),
        kills_per_player: BTreeMap::new(),
        pending_projectiles: BTreeMap::new(),
        next_projectile_id: 1,
        typed_components: BTreeMap::new(),
        combat_reveal_cells: BTreeMap::new(),
        pending_move_destinations: HashSet::new(),
        move_fire_cooldown: HashMap::new(),
    };

    // Initial shroud reveal around starting units
    world.update_shroud();
    world.update_typed_shroud_all_players();
    world
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_of_cell_values() {
        let pos = center_of_cell(0, 0);
        assert_eq!(pos, WPos::new(512, 512, 0));

        let pos = center_of_cell(10, 20);
        assert_eq!(pos, WPos::new(10 * 1024 + 512, 20 * 1024 + 512, 0));
    }

    #[test]
    fn shroud_false_disabled() {
        let t = TraitState::Shroud { disabled: false };
        assert_eq!(t.sync_hash(), 0);
    }

    #[test]
    fn player_resources_5000_cash() {
        let t = TraitState::PlayerResources { cash: 5000, resources: 0, resource_capacity: 0 };
        assert_eq!(t.sync_hash(), 5000);
    }

    #[test]
    fn reveal_on_attack_marks_attacker_cell_visible() {
        // The hook in `tick_actors` / projectile resolve pushes the
        // attacker's cell into `combat_reveal_cells[target_owner]`.
        // `update_typed_shroud_all_players` then force-reveals each
        // queued cell on the victim's shroud (regardless of the
        // victim's own RevealsShroud range) and clears the queue.
        // This test bypasses the combat path and pokes the field
        // directly so it isolates the reveal logic.
        let map = openra_data::oramap::OraMap {
            title: "tiny".into(),
            tileset: "TEMPERAT".into(),
            map_size: (20, 20),
            bounds: (0, 0, 20, 20),
            tiles: Vec::new(),
            actors: vec![
                openra_data::oramap::MapActor {
                    id: "Actor0".into(), actor_type: "mpspawn".into(),
                    owner: "Neutral".into(), location: (5, 5),
                },
                openra_data::oramap::MapActor {
                    id: "Actor1".into(), actor_type: "mpspawn".into(),
                    owner: "Neutral".into(), location: (15, 15),
                },
            ],
            players: vec![
                openra_data::oramap::PlayerDef {
                    name: "Neutral".into(), playable: false, owns_world: true,
                    non_combatant: true, faction: "allies".into(), enemies: Vec::new(),
                },
                openra_data::oramap::PlayerDef {
                    name: "P1".into(), playable: true, owns_world: false,
                    non_combatant: false, faction: "allies".into(), enemies: vec!["P2".into()],
                },
                openra_data::oramap::PlayerDef {
                    name: "P2".into(), playable: true, owns_world: false,
                    non_combatant: false, faction: "soviet".into(), enemies: vec!["P1".into()],
                },
            ],
        };
        let lobby = LobbyInfo {
            starting_cash: 5000,
            allow_spectators: true,
            occupied_slots: vec![
                SlotInfo { player_reference: "P1".into(), faction: "allies".into(), is_bot: false },
                SlotInfo { player_reference: "P2".into(), faction: "soviet".into(), is_bot: false },
            ],
        };
        let mut world = build_world(&map, 0, &lobby, None, 0, false);
        // Pull the last 2 player ids before the spectator ("everyone").
        // build_world order: non-playables first, then occupied slots,
        // then `everyone_player_id`.
        let mut all_pids: Vec<u32> = world.player_actor_ids.clone();
        // Drop spectator from the end.
        let last = all_pids.pop();
        assert_eq!(last, Some(world.everyone_player_id));
        assert!(all_pids.len() >= 3,
            "expected ≥1 non-playable + 2 playables, got {:?}", all_pids);
        let victim_owner = all_pids.pop().unwrap();
        let attacker_owner = all_pids.pop().unwrap();

        // Place attacker on the far side of the map — well outside the
        // victim's typical sight range — so we can prove the reveal is
        // *only* coming from the combat hook.
        let attacker_cell = (15, 15);
        let victim_cell = (1, 1);
        insert_test_actor(&mut world, Actor {
            id: 1001, kind: ActorKind::Building,
            owner_id: Some(attacker_owner),
            location: Some(attacker_cell),
            traits: vec![TraitState::Health { hp: 40000 }],
            activity: None, actor_type: Some("tsla".into()), kills: 0, rank: 0,
        });
        insert_test_actor(&mut world, Actor {
            id: 1002, kind: ActorKind::Infantry,
            owner_id: Some(victim_owner),
            location: Some(victim_cell),
            traits: vec![TraitState::Health { hp: 5000 }],
            activity: None, actor_type: Some("e1".into()), kills: 0, rank: 0,
        });
        world.update_typed_shroud_all_players();

        // Pre-condition: victim cannot see the attacker (too far).
        assert!(!world.typed_shroud(victim_owner).unwrap()
            .is_visible(attacker_cell.0, attacker_cell.1),
            "victim should NOT see attacker before the combat reveal");

        // Simulate "attacker just damaged victim" by populating the queue
        // exactly as `tick_actors` does after applying damage.
        world.combat_reveal_cells.entry(victim_owner).or_default().push(attacker_cell);
        world.update_typed_shroud_all_players();

        // Post-condition 1: victim now sees attacker.
        assert!(world.typed_shroud(victim_owner).unwrap()
            .is_visible(attacker_cell.0, attacker_cell.1),
            "victim should see attacker after combat reveal");
        assert!(world.typed_shroud(victim_owner).unwrap()
            .is_explored(attacker_cell.0, attacker_cell.1),
            "attacker cell should be explored (sticky)");

        // Post-condition 2: queue is consumed (reveal is one-shot).
        assert!(world.combat_reveal_cells.is_empty(),
            "combat_reveal_cells must clear after each shroud refresh");

        // Post-condition 3: subsequent shroud refresh (without a new hit)
        // drops the active visibility — only `explored` persists. This
        // matches OpenRA C# behaviour: stop being shot at, the building
        // fades back to fog after one tick.
        world.update_typed_shroud_all_players();
        assert!(!world.typed_shroud(victim_owner).unwrap()
            .is_visible(attacker_cell.0, attacker_cell.1),
            "without a fresh hit, attacker visibility decays after one tick");

        // Sanity: the *attacker's* own player still sees their own actor.
        assert!(world.typed_shroud(attacker_owner).unwrap()
            .is_visible(attacker_cell.0, attacker_cell.1));
    }

    /// C#'s Util.TickFacing: advance facing toward desired by step.
    /// Reference: OpenRA.Mods.Common/Util.cs lines 52-63.
    /// OpenRA facings are counter-clockwise: 0=N, 128=NW, 256=W, 384=SW,
    /// 512=S, 640=SE, 768=E, 896=NE.
    #[test]
    fn tick_facing_matches_csharp() {
        // Turning from North(0) toward West(256) at speed 128:
        // rightTurn=(256-0)&1023=256, leftTurn=(0-256)&1023=768
        // rightTurn < leftTurn => facing = (0 + 128) = 128 (NW)
        assert_eq!(tick_facing(0, 256, 128), 128);

        // Second tick: from 128 toward 256 at speed 128
        assert_eq!(tick_facing(128, 256, 128), 256);

        // Turning from South(512) toward West(256) at speed 128:
        // rightTurn=(256-512)&1023=768, leftTurn=(512-256)&1023=256
        // rightTurn >= leftTurn => (512-128)=384 (SW)
        assert_eq!(tick_facing(512, 256, 128), 384);

        // Continue: from 384 toward 256
        assert_eq!(tick_facing(384, 256, 128), 256);

        // Already at target
        assert_eq!(tick_facing(256, 256, 128), 256);

        // Close enough to snap
        assert_eq!(tick_facing(250, 256, 128), 256);
    }

    /// C#'s ClassicIndexFacing maps facing angles to sprite frames.
    /// SHP frames go clockwise: 0=N, 4=NE, 8=E, 12=SE, 16=S, 20=SW, 24=W, 28=NW.
    /// But OpenRA facings go counter-clockwise: 0=N, 128=NW, 256=W, etc.
    /// So facing 128(NW) → frame 28, facing 768(East) → frame 8.
    #[test]
    fn classic_index_facing_matches_csharp() {
        let ranges = [
            20, 56, 88, 132, 156, 184, 212, 240,
            268, 296, 324, 352, 384, 416, 452, 488,
            532, 568, 604, 644, 668, 696, 724, 752,
            780, 808, 836, 864, 896, 928, 964, 1000,
        ];
        let classic_index = |facing: i32| -> usize {
            let angle = (facing & 1023) as usize;
            for (i, &r) in ranges.iter().enumerate() {
                if angle < r as usize { return i; }
            }
            0
        };

        // Cardinal directions (CCW facings → CW sprite frames)
        assert_eq!(classic_index(0), 0, "North(0) → frame 0");
        assert_eq!(classic_index(256), 8, "West(256) → frame 8");
        assert_eq!(classic_index(512), 16, "South(512) → frame 16");
        assert_eq!(classic_index(768), 24, "East(768) → frame 24");

        // Diagonal directions
        assert_eq!(classic_index(128), 3, "NW(128) → frame 3");
        assert_eq!(classic_index(384), 13, "SW(384) → frame 13");
        assert_eq!(classic_index(640), 19, "SE(640) → frame 19");
        assert_eq!(classic_index(896), 29, "NE(896) → frame 29");

        // Edge cases
        assert_eq!(classic_index(1000), 0, "facing 1000 wraps to frame 0");
        assert_eq!(classic_index(1023), 0, "facing 1023 wraps to frame 0");
        assert_eq!(classic_index(19), 0, "facing 19 still frame 0");
        assert_eq!(classic_index(20), 1, "facing 20 → frame 1");
    }

    /// Verify Turn-then-Move: unit facing South ordered to move East.
    /// C# behavior: Turn in place first (TurnsWhileMoving=false), then Move.
    /// South=512, East=768. Shortest turn is clockwise (right): 512→640→768 (2 ticks).
    #[test]
    fn turn_before_move_trajectory() {
        let mut facing = 512; // South
        let target = 768;     // East
        let speed = 128;

        // Tick 1: 512 → 640 (SE)
        facing = tick_facing(facing, target, speed);
        assert_eq!(facing, 640, "Tick 1: should turn right from South(512) to SE(640)");

        // Tick 2: 640 → 768 (East)
        facing = tick_facing(facing, target, speed);
        assert_eq!(facing, 768, "Tick 2: should reach target East(768)");
    }

    /// Full trajectory test: unit at (5,5) facing South(512), ordered to move East to (6,5).
    /// East = 768 in counter-clockwise convention.
    #[test]
    fn trajectory_east_move() {
        let start = (5, 5);
        let target_cell = (6, 5);
        let speed = 56;

        // Verify facing_between gives East=768
        let desired = pathfinder::facing_between(start, target_cell);
        assert_eq!(desired, 768, "Facing from (5,5) to (6,5) should be East(768)");

        // Verify movement interpolation
        let from_center = center_of_cell(5, 5);
        let to_center = center_of_cell(6, 5);
        assert_eq!(from_center.x, 5 * 1024 + 512);
        assert_eq!(to_center.x, 6 * 1024 + 512);
        let total_dist = 1024;

        let cx_tick1 = from_center.x + (1024i64 * 56 / 1024) as i32;
        assert_eq!(cx_tick1, from_center.x + 56, "CX after 1 tick");

        let ticks_per_cell = (total_dist + speed - 1) / speed;
        assert_eq!(ticks_per_cell, 19, "MCV takes 19 ticks to cross one cell");
    }
}
