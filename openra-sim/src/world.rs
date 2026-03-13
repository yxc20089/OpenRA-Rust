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
use crate::rng::MersenneTwister;
use crate::sync;
use crate::terrain::{CellLayer, ResourceType, TerrainMap, COST_IMPASSABLE};
use crate::traits::{PqType, TraitState};

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
    /// Veterancy rank (0=none, 1=veteran, 2=elite, 3=heroic).
    pub rank: u8,
}

#[derive(Debug, Serialize)]
pub struct PlayerSnapshot {
    pub index: u32,
    pub cash: i32,
    pub power_provided: i32,
    pub power_drained: i32,
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
}

pub struct SyncHashDebug {
    pub full: i32,
    pub identity: i32,
    pub traits: i32,
    pub rng_last: i32,
}

/// The game world state.
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
    /// Ticks until next SeedsResource seeding event.
    seeds_resource_ticks: i32,
    /// Active production items per player actor ID.
    production: HashMap<u32, Vec<ProductionItem>>,
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
    /// Buildings currently being repaired (toggled by RepairBuilding order).
    repairing: HashSet<u32>,
    /// Rally points per building actor ID.
    rally_points: HashMap<u32, (i32, i32)>,
    /// Superweapon charge timers: (building_type, owner_player_id) → ticks_remaining.
    superweapon_timers: HashMap<(String, u32), i32>,
    /// Actors with invulnerability ticks remaining (Iron Curtain).
    invulnerable: HashMap<u32, i32>,
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
            actors.push(ActorSnapshot {
                id: actor.id, kind: actor.kind, owner, x, y, cx, cy,
                actor_type: actor_type_str, hp, max_hp, activity, facing,
                target_id, rank: actor.rank,
            });
        }
        let players = self.player_actor_ids.iter().map(|&pid| {
            let actor = self.actors.get(&pid);
            let cash = actor.map(|a| a.cash()).unwrap_or(0);
            let (power_provided, power_drained) = actor
                .map(|a| {
                    for t in &a.traits {
                        if let TraitState::PowerManager { power_provided, power_drained } = t {
                            return (*power_provided, *power_drained);
                        }
                    }
                    (0, 0)
                })
                .unwrap_or((0, 0));
            let production_queue = self.production.get(&pid).map(|items| {
                items.iter().map(|item| {
                    let progress = if item.total_time > 0 {
                        1.0 - (item.remaining_time as f32 / item.total_time as f32)
                    } else {
                        1.0
                    };
                    ProductionSnapshot {
                        item_name: item.item_name.clone(),
                        progress,
                        done: item.remaining_time <= 0,
                    }
                }).collect()
            }).unwrap_or_default();
            PlayerSnapshot { index: pid, cash, power_provided, power_drained, production_queue }
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

        // 2. Compute SyncHash
        let hash = self.sync_hash();

        // 3. Tick the world if not paused (NetFrameInterval=3)
        if !self.paused {
            for _ in 0..3 {
                self.world_tick += 1;
                self.tick_actors();
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
                            eprintln!("ORDER: StartProduction subject={} item={} cost={}", subject_id, item_name, cost);
                            let item = ProductionItem::new(item_name, cost, 60);
                            self.production.entry(subject_id).or_default().push(item);
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
                        self.order_attack(subject_id, target_actor_id);
                    }
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
            "Sell" => {
                if let Some(subject_id) = order.subject_id {
                    self.order_sell(subject_id);
                }
            }
            "PowerDown" => {
                // Toggle power on a building (not yet tracked per-building)
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
            "StartGame" | "Command" => {}
            other => {
                eprintln!("ORDER: unhandled '{}' subject={:?}", other, order.subject_id);
            }
        }
    }

    /// Handle a Move order: pathfind and start moving.
    /// Like C# OpenRA, vehicles turn in place first (TurnsWhileMoving=false),
    /// then begin the Move activity already facing the correct direction.
    fn order_move(&mut self, actor_id: u32, target: (i32, i32)) {
        let from = match self.actors.get(&actor_id).and_then(|a| a.location) {
            Some(loc) => loc,
            None => return,
        };
        if let Some(path) = pathfinder::find_path(&self.terrain, from, target, Some(actor_id)) {
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

    /// Handle an Attack order: set attack activity with weapon stats from rules.
    fn order_attack(&mut self, actor_id: u32, target_id: u32) {
        // Look up attacker's primary weapon from rules
        let weapon = self.actors.get(&actor_id)
            .and_then(|a| a.actor_type.as_deref())
            .and_then(|at| self.rules.actor(at))
            .and_then(|stats| stats.weapons.first())
            .and_then(|wname| self.rules.weapon(wname))
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
            });
        }
    }

    /// Handle PlaceBuilding order: create building actor and occupy terrain.
    fn order_place_building(&mut self, owner_player_id: u32, building_type: &str, x: i32, y: i32) {
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
        if let Some(items) = self.production.get_mut(&owner_player_id) {
            if let Some(idx) = items.iter().position(|i| i.item_name == building_type && i.is_done()) {
                items.remove(idx);
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
                        // Add cash to player
                        if let Some(pid) = owner {
                            if let Some(player) = self.actors.get_mut(&pid) {
                                let current = player.cash();
                                player.set_cash(current + value);
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
            for _ in 0..self.mine_count {
                self.rng.next_range(-1, 2); // dx
                self.rng.next_range(-1, 2); // dy
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
        // Second pass: check range and fire
        let mut attacks: Vec<(u32, u32, i32)> = Vec::new();
        let mut chase_targets: Vec<(u32, (i32, i32))> = Vec::new();
        for (attacker_id, target_id, damage, weapon_range) in ready_attackers {
            let attacker_loc = self.actors.get(&attacker_id).and_then(|a| a.location);
            let target_loc = self.actors.get(&target_id).and_then(|a| a.location);
            if let (Some(aloc), Some(tloc)) = (attacker_loc, target_loc) {
                let dx = (aloc.0 - tloc.0).abs();
                let dy = (aloc.1 - tloc.1).abs();
                let dist = dx.max(dy);
                if dist <= weapon_range {
                    attacks.push((attacker_id, target_id, damage));
                    // Update burst/reload state
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
        // Apply damage (skip invulnerable actors)
        let mut dead_actors: Vec<u32> = Vec::new();
        for (_attacker, target_id, damage) in &attacks {
            if self.invulnerable.contains_key(target_id) { continue; }
            if let Some(target) = self.actors.get_mut(target_id) {
                for t in &mut target.traits {
                    if let TraitState::Health { hp } = t {
                        *hp -= damage;
                        if *hp <= 0 {
                            dead_actors.push(*target_id);
                        }
                        break;
                    }
                }
            }
        }
        // Remove dead actors
        for id in dead_actors {
            if let Some(dead) = self.actors.remove(&id) {
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

        // Chase targets: pathfind toward target when out of range, preserving attack state
        for (attacker_id, target_loc) in chase_targets {
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
                    let _speed = self.actor_speed(attacker_id);
                    // Save attack params, switch to Move, then restore Attack after move completes
                    // For simplicity: just move one cell closer each tick by updating location directly
                    let next_cell = path[1];
                    if self.terrain.occupant(next_cell.0, next_cell.1) == 0 || self.terrain.occupant(next_cell.0, next_cell.1) == attacker_id {
                        self.terrain.clear_occupant(from.0, from.1);
                        self.terrain.set_occupant(next_cell.0, next_cell.1, attacker_id);
                        if let Some(actor) = self.actors.get_mut(&attacker_id) {
                            actor.location = Some(next_cell);
                            // Update Mobile trait center_position
                            for t in &mut actor.traits {
                                if let TraitState::Mobile { center_position, from_cell, to_cell, .. } = t {
                                    *center_position = center_of_cell(next_cell.0, next_cell.1);
                                    *from_cell = CPos::new(next_cell.0, next_cell.1);
                                    *to_cell = CPos::new(next_cell.0, next_cell.1);
                                    break;
                                }
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

        // Tick production queues: consume cash, advance build time.
        let player_ids: Vec<u32> = self.production.keys().copied().collect();
        let mut completed_items: Vec<(u32, String)> = Vec::new();
        for pid in player_ids {
            // Low-power slowdown: skip every other tick if power_drained > power_provided
            let is_low_power = self.actors.get(&pid).map(|a| {
                for t in &a.traits {
                    if let TraitState::PowerManager { power_provided, power_drained } = t {
                        return *power_drained > *power_provided && *power_provided > 0;
                    }
                }
                false
            }).unwrap_or(false);
            if is_low_power && self.world_tick % 2 == 0 {
                continue; // 50% production speed when low power
            }
            let cash = self.actors.get(&pid).map(|a| a.cash()).unwrap_or(0);
            if let Some(items) = self.production.get_mut(&pid) {
                // Find first item that isn't a completed building waiting for placement
                let tick_idx = items.iter().position(|i| !i.is_done());
                if let Some(idx) = tick_idx {
                    let item = &mut items[idx];
                    let consumed = item.tick(cash);
                    if consumed > 0 {
                        if let Some(player) = self.actors.get_mut(&pid) {
                            player.set_cash(cash - consumed);
                        }
                    }
                    if item.is_done() {
                        let name = item.item_name.clone();
                        if self.rules.is_unit(&name) {
                            // Units: remove from queue and spawn immediately
                            items.remove(idx);
                            eprintln!("PRODUCTION: unit {} complete for player {}", name, pid);
                            completed_items.push((pid, name));
                        } else {
                            eprintln!("PRODUCTION: building {} ready to place for player {}", name, pid);
                        }
                        // Buildings stay in queue as "ready to place" until PlaceBuilding order
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
        for actor in self.actors.values() {
            if actor.owner_id != Some(owner_player_id) { continue; }
            if actor.kind != ActorKind::Building { continue; }
            let btype = actor.actor_type.as_deref().unwrap_or("");
            let is_right = if is_infantry {
                matches!(btype, "tent" | "barr")
            } else {
                matches!(btype, "weap" | "weap.ukraine" | "hpad" | "afld" | "spen" | "syrd")
            };
            if is_right {
                if let Some(&rally) = self.rally_points.get(&actor.id) {
                    return Some(rally);
                }
            }
        }
        None
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

    /// Check if a player has the required prerequisite buildings for an item.
    fn has_prerequisites(&self, owner_player_id: u32, item_name: &str) -> bool {
        let prereqs = match self.rules.actor(item_name) {
            Some(stats) => &stats.prerequisites,
            None => return true,
        };
        if prereqs.is_empty() {
            return true;
        }
        // Check that the player owns at least one of each prerequisite building type
        for prereq in prereqs {
            let has_it = self.actors.values().any(|a| {
                a.owner_id == Some(owner_player_id)
                    && a.kind == ActorKind::Building
                    && a.actor_type.as_deref() == Some(prereq.as_str())
            });
            if !has_it {
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

        // Find production building for this unit type
        for actor in self.actors.values() {
            if actor.owner_id != Some(owner_player_id) { continue; }
            if actor.kind != ActorKind::Building { continue; }

            let btype = actor.actor_type.as_deref().unwrap_or("");
            let is_right_building = if is_infantry {
                matches!(btype, "tent" | "barr")
            } else if unit_type == "harv" {
                matches!(btype, "proc" | "weap" | "weap.ukraine")
            } else {
                matches!(btype, "weap" | "weap.ukraine" | "hpad" | "afld" | "spen" | "syrd")
            };

            if is_right_building {
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
            items.push(BuildableInfo {
                name: name.clone(),
                cost: stats.cost,
                kind: stats.kind,
                is_building: stats.is_building,
                power: stats.power,
                footprint: stats.footprint,
                locked: false,
                prerequisites: stats.prerequisites.clone(),
            });
        }
        items
    }

    /// Return ALL production items (buildable + locked) for the player.
    pub fn all_production_items(&self, player_id: u32) -> Vec<BuildableInfo> {
        let mut items = Vec::new();
        for (name, stats) in &self.rules.actors {
            if stats.cost <= 0 { continue; }
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
        if let Some(items) = self.production.get(&player_id) {
            items.iter().any(|item| !item.is_done())
        } else {
            false
        }
    }

    /// Get the first completed building in the production queue awaiting placement.
    pub fn player_ready_building(&self, player_id: u32) -> Option<String> {
        if let Some(items) = self.production.get(&player_id) {
            for item in items {
                if item.is_done() {
                    return Some(item.item_name.clone());
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

/// Build a World from parsed map data, game seed, and lobby info.
/// If `rules` is Some, uses the provided GameRules; otherwise uses hardcoded defaults.
pub fn build_world(
    map: &openra_data::oramap::OraMap,
    random_seed: i32,
    lobby: &LobbyInfo,
    rules: Option<GameRules>,
    difficulty: u8,
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
    for slot in &lobby.occupied_slots {
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
    let player_spawn_assignments = assign_spawn_points(
        &spawn_locations,
        lobby.occupied_slots.len(),
        random_seed,
        &map.players,
    );

    let facing = 512; // BaseActorFacing default
    let num_non_playable = non_playable.len();
    for (pi, &(spawn_x, spawn_y)) in player_spawn_assignments.iter().enumerate() {
        let owner_pid = player_actor_ids[num_non_playable + pi];
        eprintln!("MCV[{}] spawn=({},{}) facing={} owner={}", pi, spawn_x, spawn_y, facing, owner_pid);
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

    // Initialize terrain map and mark impassable tiles (Water, Rock/Cliffs).
    let mut terrain = TerrainMap::new(map.map_size.0, map.map_size.1);
    apply_temperat_passability(&map.tiles, &mut terrain);
    for actor in actors.values() {
        if let Some((x, y)) = actor.location {
            match actor.kind {
                ActorKind::Tree | ActorKind::Mine => {
                    terrain.set_occupant(x, y, actor.id);
                }
                ActorKind::Building => {
                    // Default 2x2 footprint for initial buildings
                    terrain.occupy_footprint(x, y, 2, 2, actor.id);
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
        seeds_resource_ticks: 0,
        production: HashMap::new(),
        terrain,
        map_width: map.map_size.0,
        map_height: map.map_size.1,
        shroud,
        rules: rules.unwrap_or_else(GameRules::defaults),
        bots,
        repairing: HashSet::new(),
        rally_points: HashMap::new(),
        superweapon_timers: HashMap::new(),
        invulnerable: HashMap::new(),
    };

    // Initial shroud reveal around starting units
    world.update_shroud();
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
