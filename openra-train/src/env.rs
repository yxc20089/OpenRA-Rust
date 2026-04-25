//! `OpenRAEnv` — the gym-like step/reset wrapper.
//!
//! The env wraps a single deterministic `World` running the rush-hour
//! scenario. It exists in two flavours:
//!   * `Env` — a plain Rust struct used by integration tests.
//!   * `OpenRAEnv` — a PyO3 wrapper around `Env` that owns the
//!     marshalling to/from Python.
//!
//! Both flavours share the same core logic; the PyO3 layer is a thin
//! shim that translates `Vec<PyCommand>` → `Vec<Command>` and
//! `Observation` → `PyDict`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use openra_data::oramap::{self, MapActor, MapDef, OraMap, PlayerDef, ScenarioActor};
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::math::{CPos, WAngle};
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, build_world, center_of_cell, set_test_unpaused, ActorSnapshot, GameOrder, LobbyInfo,
    SlotInfo, World,
};

use crate::command::Command;
use crate::observation::{EnemyPos, Observation, UnitPos};

/// Default ticks-per-step (~ one game-second at NetFrameInterval=3
/// and the engine's 25 ticks / second cadence; we use 30 for a round
/// number and to align with the C# rush-hour reference).
pub const DEFAULT_TICKS_PER_STEP: u32 = 30;

/// Hard cap on episode length — matches the prod openra-rl 6000-tick
/// timeout (~4 game-minutes).
pub const DEFAULT_MAX_TICKS: u32 = 6000;

/// Errors surfaced from `Env::new` / `Env::reset`.
#[derive(Debug)]
pub enum EnvError {
    BadScenario(String),
    MissingScenario(PathBuf),
}

impl std::fmt::Display for EnvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvError::BadScenario(msg) => write!(f, "scenario load error: {msg}"),
            EnvError::MissingScenario(p) => {
                write!(f, "scenario yaml not found: {}", p.display())
            }
        }
    }
}

impl std::error::Error for EnvError {}

/// Pure-Rust env. PyO3 wraps this.
pub struct Env {
    /// Path to the scenario YAML (e.g.
    /// `~/Projects/OpenRA-RL-Training/scenarios/discovery/rush-hour.yaml`).
    /// Resolved through the same fallback chain `MapDef` uses.
    #[allow(dead_code)]
    scenario_path: PathBuf,
    /// Random seed (forwarded to MersenneTwister).
    seed: u64,
    /// Cached map for fast resets.
    map_def: MapDef,
    /// Current world. None until first `reset()`.
    world: Option<World>,
    /// Player-actor id of the agent (always allocated as the first
    /// playable slot).
    agent_player_id: u32,
    /// Player-actor id of the enemy.
    enemy_player_id: u32,
    /// Cumulative own-team kills observed so far (delta-tracked across
    /// ticks).
    units_killed: i32,
    /// Per-step union of "ever revealed" cells for the agent. Used to
    /// compute `explored_percent`.
    explored_cells: HashSet<(i32, i32)>,
    /// Total revealable cells on the map (for the percentage denominator).
    map_total_cells: i32,
    /// Configuration: how many ticks each `step()` advances.
    ticks_per_step: u32,
    /// Configuration: hard episode cap.
    max_ticks: u32,
    /// Last-step warnings (e.g. ignored unit ids). Surfaced through `info`.
    last_warnings: Vec<String>,
}

impl Env {
    /// Construct a new env. The scenario YAML is loaded once; `reset()`
    /// rebuilds the world from the cached `MapDef`.
    pub fn new(scenario_path_or_alias: &str, seed: u64) -> Result<Self, EnvError> {
        let scenario_path = resolve_scenario(scenario_path_or_alias)?;
        let map_def = oramap::load_rush_hour_map(&scenario_path)
            .map_err(|e| EnvError::BadScenario(e.to_string()))?;

        // Total cells inside the playable bounds: bounds = (x, y, w, h).
        // Falling back to `map_size` if bounds are missing.
        let (_, _, bw, bh) = map_def.bounds;
        let map_total_cells = if bw > 0 && bh > 0 {
            bw * bh
        } else {
            map_def.map_size.0 * map_def.map_size.1
        };

        Ok(Env {
            scenario_path,
            seed,
            map_def,
            world: None,
            agent_player_id: 0,
            enemy_player_id: 0,
            units_killed: 0,
            explored_cells: HashSet::new(),
            map_total_cells,
            ticks_per_step: DEFAULT_TICKS_PER_STEP,
            max_ticks: DEFAULT_MAX_TICKS,
            last_warnings: Vec::new(),
        })
    }

    /// Override ticks-per-step (default `DEFAULT_TICKS_PER_STEP`).
    pub fn with_ticks_per_step(mut self, n: u32) -> Self {
        self.ticks_per_step = n.max(1);
        self
    }

    /// Override max-ticks (default `DEFAULT_MAX_TICKS`).
    pub fn with_max_ticks(mut self, n: u32) -> Self {
        self.max_ticks = n.max(1);
        self
    }

    /// Reset the world to its initial state.
    pub fn reset(&mut self) -> Observation {
        self.world = Some(self.build_world_for_episode());
        self.units_killed = 0;
        self.explored_cells.clear();
        self.last_warnings.clear();
        self.refresh_explored_cells();
        self.observation()
    }

    /// One env step. Returns (obs, reward, done, info) — for v1 reward
    /// is always 0 (Python computes shaped rewards externally).
    pub fn step(&mut self, commands: &[Command]) -> StepResult {
        self.last_warnings.clear();

        // Apply all commands, building up a flat order list.
        let orders = self.build_orders(commands);

        // Issue orders, then run N ticks. We issue all orders on the
        // *first* frame so subsequent ticks just advance state.
        if self.world.is_some() {
            self.tick_world_with_orders(&orders);
            for _ in 1..self.ticks_per_step {
                self.tick_world_with_orders(&[]);
            }
        }

        self.refresh_explored_cells();
        self.update_kill_counter();

        let obs = self.observation();
        let done = self.is_terminal();
        StepResult {
            obs,
            reward: 0.0,
            done,
            warnings: self.last_warnings.clone(),
        }
    }

    /// Render an ASCII map for debugging (rows top-to-bottom).
    pub fn render(&self) -> String {
        let world = match &self.world {
            Some(w) => w,
            None => return "<env not reset>".into(),
        };
        let snap = world.snapshot();
        let (mw, mh) = self.map_def.map_size;
        let mut grid: Vec<Vec<char>> = vec![vec!['.'; mw as usize]; mh as usize];
        for actor in &snap.actors {
            if actor.x < 0 || actor.y < 0 || actor.x >= mw || actor.y >= mh {
                continue;
            }
            let mark = match actor.kind {
                ActorKind::Infantry => {
                    if actor.owner == self.agent_player_id {
                        'a'
                    } else if actor.owner == self.enemy_player_id {
                        'e'
                    } else {
                        '?'
                    }
                }
                ActorKind::Vehicle | ActorKind::Mcv => {
                    if actor.owner == self.agent_player_id {
                        'A'
                    } else if actor.owner == self.enemy_player_id {
                        'E'
                    } else {
                        '?'
                    }
                }
                ActorKind::Building => '#',
                ActorKind::Tree => 'T',
                ActorKind::Mine => 'M',
                _ => continue,
            };
            grid[actor.y as usize][actor.x as usize] = mark;
        }
        let mut out = String::with_capacity(((mw + 1) * mh) as usize);
        for row in &grid {
            for &c in row {
                out.push(c);
            }
            out.push('\n');
        }
        out
    }

    // ---- Inspection helpers used by tests ------------------------------------

    pub fn agent_player_id(&self) -> u32 {
        self.agent_player_id
    }

    pub fn enemy_player_id(&self) -> u32 {
        self.enemy_player_id
    }

    pub fn world(&self) -> Option<&World> {
        self.world.as_ref()
    }

    pub fn ticks_per_step(&self) -> u32 {
        self.ticks_per_step
    }

    pub fn max_ticks(&self) -> u32 {
        self.max_ticks
    }

    pub fn last_warnings(&self) -> &[String] {
        &self.last_warnings
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    // ---- Internals -----------------------------------------------------------

    /// Build a `World` seeded with the rush-hour map and scenario actors.
    fn build_world_for_episode(&mut self) -> World {
        // Construct an OraMap shim that satisfies `build_world` — the
        // sim crate isn't yet aware of `MapDef` (planned for a later
        // phase). We synthesize a Neutral non-playable player + two
        // playable slots (agent, enemy).
        let players = vec![
            PlayerDef {
                name: "Neutral".into(),
                playable: false,
                owns_world: true,
                non_combatant: true,
                faction: "allies".into(),
                enemies: Vec::new(),
            },
            PlayerDef {
                name: "Multi0".into(),
                playable: true,
                owns_world: false,
                non_combatant: false,
                faction: self.map_def.agent_faction.clone(),
                enemies: vec!["Multi1".into()],
            },
            PlayerDef {
                name: "Multi1".into(),
                playable: true,
                owns_world: false,
                non_combatant: false,
                faction: self.map_def.enemy_faction.clone(),
                enemies: vec!["Multi0".into()],
            },
        ];

        // `build_world` requires at least one `mpspawn` per occupied
        // slot, even though the rush-hour scenario doesn't itself use
        // them (units are placed directly). Inject two synthetic
        // spawns at corners of the playable bounds so spawn assignment
        // doesn't panic on `available_spawns.remove(0)`.
        let (bx, by, bw, bh) = self.map_def.bounds;
        let spawn_a = (bx + 1, by + 1);
        let spawn_b = (bx + bw - 2, by + bh - 2);
        let spawn_actors = vec![
            MapActor {
                id: "mpspawn1".into(),
                actor_type: "mpspawn".into(),
                owner: "Neutral".into(),
                location: spawn_a,
            },
            MapActor {
                id: "mpspawn2".into(),
                actor_type: "mpspawn".into(),
                owner: "Neutral".into(),
                location: spawn_b,
            },
        ];

        let ora = OraMap {
            title: self.map_def.title.clone(),
            tileset: self.map_def.tileset.clone(),
            map_size: self.map_def.map_size,
            bounds: self.map_def.bounds,
            players,
            actors: spawn_actors,
            tiles: self.map_def.tiles.clone(),
        };

        let lobby = LobbyInfo {
            starting_cash: 0,
            allow_spectators: true,
            occupied_slots: vec![
                SlotInfo {
                    player_reference: "Multi0".into(),
                    faction: self.map_def.agent_faction.clone(),
                    is_bot: false,
                },
                SlotInfo {
                    player_reference: "Multi1".into(),
                    faction: self.map_def.enemy_faction.clone(),
                    is_bot: false,
                },
            ],
        };

        // Use the per-episode seed verbatim. `build_world` takes i32
        // for the seed — clamp the high bits.
        let seed_i32 = self.seed as u32 as i32;
        let mut world = build_world(&ora, seed_i32, &lobby, None, 0);

        // Resolve player ids: `build_world` allocates the World actor
        // (id 0), then non-playable players, then playable players, then
        // the "Everyone" spectator. With one Neutral + two playable,
        // `player_ids()` returns [Neutral, Multi0, Multi1, Everyone].
        let player_ids = world.player_ids().to_vec();
        let agent_pid = player_ids
            .get(1)
            .copied()
            .expect("expected at least 2 player ids (Neutral + agent)");
        let enemy_pid = player_ids
            .get(2)
            .copied()
            .expect("expected at least 3 player ids (Neutral + agent + enemy)");
        self.agent_player_id = agent_pid;
        self.enemy_player_id = enemy_pid;

        // `build_world` auto-spawns one MCV per occupied slot at the
        // mpspawn locations we synthesised. Those are stand-ins for
        // the spawn-assignment plumbing — the rush-hour scenario
        // doesn't want them. Strip MCVs (and mpspawn beacons) before
        // injecting the scenario's own actors.
        let strip_ids: Vec<u32> = world::all_actor_ids(&world)
            .into_iter()
            .filter(|&id| {
                let kind = world.actor_kind(id);
                let actor_type = world.actor_type_name(id).map(str::to_string);
                matches!(kind, Some(ActorKind::Mcv) | Some(ActorKind::Spawn))
                    || actor_type.as_deref() == Some("mpspawn")
            })
            .collect();
        for id in strip_ids {
            world::remove_test_actor(&mut world, id);
        }

        // Inject scenario actors. We allocate ids well above any id
        // assigned by `build_world` so nothing collides.
        let mut next_id = scenario_id_seed(&world);
        for sa in self.map_def.actors.iter() {
            let owner = match sa.owner.as_str() {
                "agent" => agent_pid,
                "enemy" => enemy_pid,
                _ => continue,
            };
            let actor = build_scenario_actor(next_id, sa, owner, &world);
            world::insert_test_actor(&mut world, actor);
            next_id += 1;
        }

        // Lift the order-latency pause so the very first `step()`
        // advances the tick counter.
        set_test_unpaused(&mut world);

        // Run one no-op frame so shroud reveals are recomputed with
        // the injected actors.
        world.process_frame(&[]);
        world
    }

    /// Translate `[Command]` into raw `GameOrder`s.
    fn build_orders(&mut self, commands: &[Command]) -> Vec<GameOrder> {
        let mut orders = Vec::new();
        let world = match &self.world {
            Some(w) => w,
            None => return orders,
        };

        let agent_owned: HashSet<u32> = world
            .actor_ids_for_player(self.agent_player_id)
            .into_iter()
            .collect();

        for cmd in commands {
            match cmd {
                Command::Observe => {}
                Command::MoveUnits {
                    unit_ids,
                    target_x,
                    target_y,
                } => {
                    for id_str in unit_ids {
                        let aid = match parse_actor_id(id_str) {
                            Some(v) => v,
                            None => {
                                self.last_warnings
                                    .push(format!("invalid unit_id {id_str:?}"));
                                continue;
                            }
                        };
                        if !agent_owned.contains(&aid) {
                            self.last_warnings
                                .push(format!("unit {aid} not owned by player_0"));
                            continue;
                        }
                        orders.push(GameOrder {
                            order_string: "Move".into(),
                            subject_id: Some(aid),
                            target_string: Some(format!("{target_x},{target_y}")),
                            extra_data: None,
                        });
                    }
                }
                Command::AttackUnit {
                    unit_ids,
                    target_id,
                } => {
                    let target_aid = match parse_actor_id(target_id) {
                        Some(v) => v,
                        None => {
                            self.last_warnings
                                .push(format!("invalid target_id {target_id:?}"));
                            continue;
                        }
                    };
                    for id_str in unit_ids {
                        let aid = match parse_actor_id(id_str) {
                            Some(v) => v,
                            None => {
                                self.last_warnings
                                    .push(format!("invalid unit_id {id_str:?}"));
                                continue;
                            }
                        };
                        if !agent_owned.contains(&aid) {
                            self.last_warnings
                                .push(format!("unit {aid} not owned by player_0"));
                            continue;
                        }
                        orders.push(GameOrder {
                            order_string: "Attack".into(),
                            subject_id: Some(aid),
                            target_string: None,
                            extra_data: Some(target_aid),
                        });
                    }
                }
            }
        }
        orders
    }

    fn tick_world_with_orders(&mut self, orders: &[GameOrder]) {
        if let Some(world) = self.world.as_mut() {
            world.process_frame(orders);
            // Refresh the typed shroud so observation/visibility reads
            // use the post-tick state.
            world.update_typed_shroud_all_players();
        }
    }

    /// Return the most recent observation snapshot. Useful for tests
    /// that want to peek at visible enemies between step calls
    /// without re-stepping.
    pub fn last_observation(&self) -> Observation {
        self.observation()
    }

    /// World-level sync hash (sum of trait sync hashes + RNG state +
    /// effects). Identical to `World::sync_hash`. Used by determinism
    /// tests to detect divergence on RNG-dependent state that doesn't
    /// surface through the public observation (e.g. the seeds-resource
    /// timer, untouched MCV facing, internal pathfinder counters).
    pub fn world_sync_hash(&self) -> i32 {
        match &self.world {
            Some(w) => w.sync_hash(),
            None => 0,
        }
    }

    /// Build the observation snapshot, fog-filtering enemies through
    /// the agent's shroud.
    fn observation(&self) -> Observation {
        let world = match &self.world {
            Some(w) => w,
            None => {
                return Observation {
                    unit_positions: Vec::new(),
                    unit_hp: Vec::new(),
                    enemy_positions: Vec::new(),
                    enemy_hp: Vec::new(),
                    units_killed: 0,
                    game_tick: 0,
                    explored_percent: 0.0,
                };
            }
        };

        let snap = world.snapshot();

        let mut unit_positions: Vec<(String, UnitPos)> = Vec::new();
        let mut unit_hp: Vec<(String, f32)> = Vec::new();
        let mut enemy_positions: Vec<EnemyPos> = Vec::new();
        let mut enemy_hp: Vec<(String, f32)> = Vec::new();

        // ActorSnapshot list is ordered by actor.id (BTreeMap iteration
        // in `World::snapshot`), so output ordering is deterministic.
        let mut sorted: Vec<&ActorSnapshot> = snap.actors.iter().collect();
        sorted.sort_by_key(|a| a.id);

        for a in sorted {
            if matches!(a.kind, ActorKind::World | ActorKind::Player | ActorKind::Spawn) {
                continue;
            }
            // Skip non-combat decorations.
            if matches!(a.kind, ActorKind::Tree | ActorKind::Mine) {
                continue;
            }
            let id_str = a.id.to_string();
            let pct = if a.max_hp > 0 {
                (a.hp as f32 / a.max_hp as f32).clamp(0.0, 1.0)
            } else {
                1.0
            };
            if a.owner == self.agent_player_id {
                unit_positions.push((
                    id_str.clone(),
                    UnitPos {
                        cell_x: a.x,
                        cell_y: a.y,
                    },
                ));
                unit_hp.push((id_str, pct));
            } else if a.owner == self.enemy_player_id {
                if !self.is_visible_to_agent(world, a.x, a.y) {
                    continue;
                }
                enemy_positions.push(EnemyPos {
                    cell_x: a.x,
                    cell_y: a.y,
                    id: id_str.clone(),
                });
                enemy_hp.push((id_str, pct));
            }
        }

        let explored_percent = if self.map_total_cells > 0 {
            (self.explored_cells.len() as f32 / self.map_total_cells as f32) * 100.0
        } else {
            0.0
        };

        Observation {
            unit_positions,
            unit_hp,
            enemy_positions,
            enemy_hp,
            units_killed: self.units_killed,
            game_tick: world.world_tick as i32,
            explored_percent,
        }
    }

    /// Refresh `explored_cells` from the agent's typed shroud
    /// (`World::typed_shroud(player)`). The shroud's `is_explored`
    /// flag is sticky — once a cell has been seen by any agent unit
    /// it stays explored, matching OpenRA's `Shroud.IsExplored`.
    fn refresh_explored_cells(&mut self) {
        let world = match &self.world {
            Some(w) => w,
            None => return,
        };
        let shroud = match world.typed_shroud(self.agent_player_id) {
            Some(s) => s,
            None => return,
        };
        let (mw, mh) = self.map_def.map_size;
        for y in 0..mh {
            for x in 0..mw {
                if shroud.is_explored(x, y) {
                    self.explored_cells.insert((x, y));
                }
            }
        }
    }

    /// Cell visibility via the typed shroud's `is_visible` flag —
    /// only counts cells currently in sight of any agent unit.
    fn is_visible_to_agent(&self, world: &World, cx: i32, cy: i32) -> bool {
        match world.typed_shroud(self.agent_player_id) {
            Some(s) => s.is_visible(cx, cy),
            None => false,
        }
    }

    /// Read the agent's kill tally directly from the World combat
    /// counter. `kills_for_player` is incremented by both the
    /// data-driven `tick_actors` attack loop and the typed
    /// `AttackActivity` path whenever an attack reduces a target's
    /// HP to zero. Monotonically non-decreasing.
    fn update_kill_counter(&mut self) {
        let world = match &self.world {
            Some(w) => w,
            None => return,
        };
        let total = world.kills_for_player(self.agent_player_id) as i32;
        self.units_killed = self.units_killed.max(total);
    }

    fn is_terminal(&self) -> bool {
        let world = match &self.world {
            Some(w) => w,
            None => return true,
        };
        if world.world_tick >= self.max_ticks {
            return true;
        }
        // Either side at zero combat units → done.
        let agent_alive = has_combat_units(world, self.agent_player_id);
        let enemy_alive = has_combat_units(world, self.enemy_player_id);
        !agent_alive || !enemy_alive
    }
}

/// Result of `Env::step`. Mirrors what PyO3 returns (`obs`, `reward`,
/// `done`, `info`).
#[derive(Debug, Clone)]
pub struct StepResult {
    pub obs: Observation,
    pub reward: f32,
    pub done: bool,
    /// Warnings strung up under `info["warnings"]` on the Python side.
    pub warnings: Vec<String>,
}

// ---- Helpers (private) -------------------------------------------------------

fn parse_actor_id(s: &str) -> Option<u32> {
    s.parse::<u32>().ok()
}

/// Pick a starting actor id high enough to avoid collision with
/// anything `build_world` allocated. We can't read `World::next_actor_id`
/// directly, so we walk the public APIs.
fn scenario_id_seed(world: &World) -> u32 {
    let mut max_id = 0u32;
    for &pid in world.player_ids() {
        max_id = max_id.max(pid);
        for aid in world.actor_ids_for_player(pid) {
            max_id = max_id.max(aid);
        }
    }
    // Add a generous margin so any unowned actors `build_world`
    // allocated (none in our scenario, but be robust) are clear.
    max_id.max(1000) + 1
}

fn has_combat_units(world: &World, player_id: u32) -> bool {
    for aid in world.actor_ids_for_player(player_id) {
        if let Some(kind) = world.actor_kind(aid) {
            if matches!(
                kind,
                ActorKind::Infantry
                    | ActorKind::Vehicle
                    | ActorKind::Aircraft
                    | ActorKind::Ship
                    | ActorKind::Mcv
            ) {
                return true;
            }
        }
    }
    false
}

/// Build a freshly-spawned `Actor` from a `ScenarioActor`.
fn build_scenario_actor(id: u32, sa: &ScenarioActor, owner: u32, world: &World) -> Actor {
    let kind = if let Some(stats) = world.rules.actor(&sa.actor_type) {
        stats.kind
    } else {
        kind_for_unit_type(&sa.actor_type)
    };
    let hp = world
        .rules
        .actor(&sa.actor_type)
        .map(|s| s.hp)
        .unwrap_or(50000);
    let cell = CPos::new(sa.position.0, sa.position.1);
    let center = center_of_cell(sa.position.0, sa.position.1);
    let facing = WAngle::new(512).angle;
    Actor {
        id,
        kind,
        owner_id: Some(owner),
        location: Some(sa.position),
        traits: vec![
            TraitState::BodyOrientation { quantized_facings: 32 },
            TraitState::Mobile {
                facing,
                from_cell: cell,
                to_cell: cell,
                center_position: center,
            },
            TraitState::Health { hp },
        ],
        activity: None,
        actor_type: Some(sa.actor_type.clone()),
        kills: 0,
        rank: 0,
    }
}

fn kind_for_unit_type(t: &str) -> ActorKind {
    match t {
        "1tnk" | "2tnk" | "3tnk" | "4tnk" | "harv" | "jeep" | "apc" | "arty" | "ftrk" => {
            ActorKind::Vehicle
        }
        "yak" | "mig" | "heli" | "hind" => ActorKind::Aircraft,
        "lst" | "ss" | "msub" | "ca" | "dd" | "pt" => ActorKind::Ship,
        // Default to Infantry — covers e1/e3/e6/dog/medi etc.
        _ => ActorKind::Infantry,
    }
}

/// Resolve a scenario alias or path. Aliases:
///   * `rush-hour`, `rush_hour`, `rush-hour.yaml` →
///     `~/Projects/OpenRA-RL-Training/scenarios/discovery/rush-hour.yaml`
///
/// Falls back to a literal path lookup.
fn resolve_scenario(alias_or_path: &str) -> Result<PathBuf, EnvError> {
    let candidate = Path::new(alias_or_path);
    if candidate.exists() {
        return Ok(candidate.to_path_buf());
    }

    let lower = alias_or_path.to_ascii_lowercase();
    let normalised = lower.trim_end_matches(".yaml").replace('_', "-");
    if normalised == "rush-hour" {
        let mut tried: Vec<PathBuf> = Vec::new();
        if let Ok(home) = std::env::var("HOME") {
            for p in [
                "Projects/OpenRA-RL-Training/scenarios/discovery/rush-hour.yaml",
                "Projects/openra-rl/scenarios/discovery/rush-hour.yaml",
            ] {
                let full = PathBuf::from(&home).join(p);
                if full.exists() {
                    return Ok(full);
                }
                tried.push(full);
            }
        }
        return Err(EnvError::MissingScenario(
            tried
                .last()
                .cloned()
                .unwrap_or_else(|| PathBuf::from(alias_or_path)),
        ));
    }

    Err(EnvError::MissingScenario(PathBuf::from(alias_or_path)))
}

/// Test-only stub world (no scenario load required). Used by
/// `tests/env_terminal.rs` to rig "0 enemy units at start".
#[doc(hidden)]
pub fn build_test_env_with_no_enemies(map_size: (i32, i32), seed: u64) -> Env {
    let map_def = MapDef {
        title: "test_no_enemies".into(),
        tileset: "TEMPERAT".into(),
        map_size,
        bounds: (0, 0, map_size.0, map_size.1),
        tiles: Vec::new(),
        agent_faction: "allies".into(),
        enemy_faction: "soviet".into(),
        actors: vec![ScenarioActor {
            actor_type: "e1".into(),
            owner: "agent".into(),
            position: (5, 5),
        }],
    };
    let mut env = Env {
        scenario_path: PathBuf::from("<test>"),
        seed,
        map_def,
        world: None,
        agent_player_id: 0,
        enemy_player_id: 0,
        units_killed: 0,
        explored_cells: HashSet::new(),
        map_total_cells: map_size.0 * map_size.1,
        ticks_per_step: DEFAULT_TICKS_PER_STEP,
        max_ticks: DEFAULT_MAX_TICKS,
        last_warnings: Vec::new(),
    };
    env.reset();
    env
}

// ---- PyO3 wrapper ------------------------------------------------------------

#[cfg(feature = "python")]
mod py {
    use super::*;
    use crate::command::PyCommand;
    use pyo3::exceptions::PyValueError;
    use pyo3::prelude::*;
    use pyo3::types::{PyDict, PyList};

    #[pyclass(name = "OpenRAEnv", module = "openra_train")]
    pub struct OpenRAEnv {
        inner: Env,
    }

    #[pymethods]
    impl OpenRAEnv {
        #[new]
        #[pyo3(signature = (scenario_path, seed, ticks_per_step=None, max_ticks=None))]
        fn new(
            scenario_path: String,
            seed: u64,
            ticks_per_step: Option<u32>,
            max_ticks: Option<u32>,
        ) -> PyResult<Self> {
            let mut env = Env::new(&scenario_path, seed)
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
            if let Some(n) = ticks_per_step {
                env = env.with_ticks_per_step(n);
            }
            if let Some(n) = max_ticks {
                env = env.with_max_ticks(n);
            }
            Ok(OpenRAEnv { inner: env })
        }

        fn reset<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
            let obs = self.inner.reset();
            obs.to_pydict(py)
        }

        fn step<'py>(
            &mut self,
            py: Python<'py>,
            commands: Vec<PyRef<PyCommand>>,
        ) -> PyResult<(Bound<'py, PyDict>, f32, bool, Bound<'py, PyDict>)> {
            let cmds: Vec<Command> = commands.into_iter().map(|c| c.inner.clone()).collect();
            let result = self.inner.step(&cmds);

            let obs = result.obs.to_pydict(py)?;
            let info = PyDict::new_bound(py);
            let warnings = PyList::empty_bound(py);
            for w in &result.warnings {
                warnings.append(w)?;
            }
            info.set_item("warnings", warnings)?;
            info.set_item("game_tick", result.obs.game_tick)?;
            Ok((obs, result.reward, result.done, info))
        }

        fn render(&self) -> String {
            self.inner.render()
        }

        #[getter]
        fn agent_player_id(&self) -> u32 {
            self.inner.agent_player_id()
        }

        #[getter]
        fn enemy_player_id(&self) -> u32 {
            self.inner.enemy_player_id()
        }

        #[getter]
        fn ticks_per_step(&self) -> u32 {
            self.inner.ticks_per_step()
        }

        #[getter]
        fn max_ticks(&self) -> u32 {
            self.inner.max_ticks()
        }

        fn __repr__(&self) -> String {
            format!(
                "<OpenRAEnv tick={} agent={} enemy={}>",
                self.inner.world().map(|w| w.world_tick).unwrap_or(0),
                self.inner.agent_player_id(),
                self.inner.enemy_player_id(),
            )
        }
    }
}

#[cfg(feature = "python")]
pub use py::OpenRAEnv;
