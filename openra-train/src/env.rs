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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use openra_data::oramap::{self, MapActor, MapDef, OraMap, PlayerDef, ScenarioActor};
use openra_data::rules as data_rules;
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WAngle};
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, build_world, center_of_cell, set_test_unpaused, ActorSnapshot, GameOrder, LobbyInfo,
    SlotInfo, World,
};

use crate::command::Command;
use crate::observation::{EnemyBuilding, EnemyPos, Observation, UnitPos};

/// Default ticks-per-step (~ one game-second at NetFrameInterval=3
/// and the engine's 25 ticks / second cadence; we use 30 for a round
/// number and to align with the C# rush-hour reference).
pub const DEFAULT_TICKS_PER_STEP: u32 = 30;

/// Hard cap on episode length. 10000 ticks = 400 game-seconds at the
/// engine's 25 ticks/second cadence, matching the "of 400s" budget the
/// briefing displays to the model.
pub const DEFAULT_MAX_TICKS: u32 = 10000;

/// Default per-signal cooldown — minimum ticks between consecutive fires
/// for the same dedup key. Prevents flicker spam at fog boundaries.
/// 30 ticks ≈ 1 game-second.
pub const DEFAULT_INTERRUPT_COOLDOWN_TICKS: i32 = 30;

/// All interrupt signal names the engine knows about. Mirrors the
/// production agent_rollout.py:_DEDUPED_INTERRUPTS list. Names are
/// stable strings so the Python side can opt in/out by name.
pub const INTERRUPT_SIGNAL_NAMES: &[&str] = &[
    "enemy_unit_spotted",
    "enemy_building_spotted",
    "engage_start",
    "own_unit_destroyed",
    // Fires when a newly-produced agent unit or completed building
    // appears (queue item finished / construction complete) — lets the
    // agent re-plan the moment capacity arrives.
    "production_complete",
];

/// Per-episode tracking state for interrupt detection. Resets on
/// every `reset()` call.
#[derive(Debug, Default)]
pub struct InterruptState {
    /// Visible-to-agent enemy unit IDs at the *previous check*
    /// (single-frame snapshot, NOT cumulative). Diffed against the
    /// current frame to find newly-visible IDs.
    prev_visible_enemy_unit_ids: HashSet<u32>,
    /// Same for enemy buildings.
    prev_visible_enemy_building_ids: HashSet<u32>,
    /// Own (agent-owned) unit IDs alive at the previous check. Diffed
    /// against the current frame to detect lost units (used by the
    /// `own_unit_destroyed` signal so the agent gets re-prompted while
    /// it's losing forces, not after the wipeout).
    prev_own_unit_ids: HashSet<u32>,
    /// Own unit IDs that were attacking last check, keyed to the
    /// target they were attacking. The dedup key is
    /// `(own_actor_id, target_actor_id)` — different target = new event.
    prev_attacking_pairs: HashSet<(u32, u32)>,
    /// Agent-owned actor IDs (units + buildings) alive at the previous
    /// check. A newly-appeared ID = a finished production / completed
    /// construction → `production_complete`.
    prev_own_actor_ids: HashSet<u32>,
    /// False until the first check populates the baseline (so starting
    /// actors don't all false-fire `production_complete`).
    production_baseline_set: bool,
    /// Per-(signal, dedup_key) last-fire tick. Suppresses re-fire if
    /// `current_tick - last_fire_tick < cooldown_ticks`.
    last_fire_tick: HashMap<(String, u64), i32>,
}

impl InterruptState {
    fn cooldown_ok(&self, signal: &str, key: u64, now: i32, cooldown: i32) -> bool {
        match self.last_fire_tick.get(&(signal.to_string(), key)) {
            Some(&t) => now - t >= cooldown,
            None => true,
        }
    }

    fn mark_fired(&mut self, signal: &str, key: u64, now: i32) {
        self.last_fire_tick
            .insert((signal.to_string(), key), now);
    }

    fn clear(&mut self) {
        self.prev_visible_enemy_unit_ids.clear();
        self.prev_visible_enemy_building_ids.clear();
        self.prev_own_unit_ids.clear();
        self.prev_attacking_pairs.clear();
        self.last_fire_tick.clear();
    }
}

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
    /// Interrupt-detection state. Reset on every `reset()`.
    interrupt_state: InterruptState,
    /// Which interrupt signals are enabled. Empty = none (back-compat,
    /// `step()` keeps working as before).
    enabled_signals: HashSet<String>,
    /// Cooldown ticks between fires for the same dedup key.
    cooldown_ticks: i32,
    /// Whether the enemy spawned with at least one MustBeDestroyed
    /// building (i.e. fact / proc / weap / barr / tent etc.). Decides
    /// the win-condition policy in `is_terminal`:
    ///   - true (e.g. maginot): destroying the buildings ends the game;
    ///     remaining combat units don't matter.
    ///   - false (e.g. rush-hour, where the enemy is units-only):
    ///     fall back to "all combat units dead" semantic.
    /// Set once at the end of `reset()`.
    enemy_started_with_buildings: bool,
    /// True iff the scenario placed any enemy actor (see is_terminal).
    enemy_started_present: bool,
    /// Lazily-cached typed ruleset used by `attach_typed_components` for
    /// scheduled-event SpawnActors injections. `None` until the first
    /// scheduled-spawn fires (most scenarios declare no scheduled events,
    /// so we avoid the I/O cost on the hot reset path).
    typed_rules_cache: Option<openra_data::rules::Rules>,
    /// Per-event "already fired" flags, parallel to
    /// `map_def.scheduled_events`. Cleared and resized on `reset()`.
    /// Each event fires exactly once when `world.world_tick` first
    /// reaches its declared trigger.
    fired_scheduled_events: Vec<bool>,
}

impl Env {
    /// Construct a new env. The scenario YAML is loaded once; `reset()`
    /// rebuilds the world from the cached `MapDef`.
    ///
    /// Auto-selects the agent's `spawn_point` deterministically from the
    /// seed. Use [`Env::new_with_spawn_point`] to force a specific one.
    pub fn new(scenario_path_or_alias: &str, seed: u64) -> Result<Self, EnvError> {
        Self::new_with_spawn_point(scenario_path_or_alias, seed, None)
    }

    /// Construct a new env with an explicit agent `spawn_point`.
    /// `spawn_point=None` means "round-robin across the spawn_points
    /// declared in the scenario YAML, picked by `seed % n`". A scenario
    /// without any `spawn_point:` fields collapses to `0` (no filter).
    /// `Some(n)` forces that spawn_point regardless of what the
    /// scenario declares — caller's responsibility to pass a valid one.
    ///
    /// Wave-9: the seed-axis source falls back to the enemy side when
    /// the agent declares no spawn_points (e.g.
    /// `adv-rps-counter-pick`, where the agent base is identical
    /// every seed but the enemy composition rotates across
    /// `spawn_point` groups). When the agent declares spawn_points,
    /// those drive the round-robin (back-compat for every existing
    /// pack) and the enemy filter activates independently on the
    /// SAME chosen `spawn_point` (so a pack can co-vary agent corner
    /// and enemy composition if it wants).
    pub fn new_with_spawn_point(
        scenario_path_or_alias: &str,
        seed: u64,
        spawn_point: Option<i32>,
    ) -> Result<Self, EnvError> {
        let scenario_path = resolve_scenario(scenario_path_or_alias)?;
        let chosen_sp = match spawn_point {
            Some(n) => n,
            None => {
                let agent_sps = oramap::distinct_agent_spawn_points(&scenario_path)
                    .map_err(|e| EnvError::BadScenario(e.to_string()))?;
                if !agent_sps.is_empty() {
                    agent_sps[(seed as usize) % agent_sps.len()]
                } else {
                    let enemy_sps = oramap::distinct_enemy_spawn_points(&scenario_path)
                        .map_err(|e| EnvError::BadScenario(e.to_string()))?;
                    if enemy_sps.is_empty() {
                        0
                    } else {
                        enemy_sps[(seed as usize) % enemy_sps.len()]
                    }
                }
            }
        };
        let map_def = oramap::load_rush_hour_map_with_spawn(&scenario_path, chosen_sp)
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
            interrupt_state: InterruptState::default(),
            enabled_signals: HashSet::new(),
            cooldown_ticks: DEFAULT_INTERRUPT_COOLDOWN_TICKS,
            enemy_started_with_buildings: false,
            enemy_started_present: false,
            typed_rules_cache: None,
            fired_scheduled_events: Vec::new(),
        })
    }

    /// Configure which interrupt signals are emitted by `step_until_event`.
    /// Pass an empty set to disable all (back-compat). Names must come
    /// from `INTERRUPT_SIGNAL_NAMES`.
    pub fn with_enabled_signals<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.enabled_signals = names.into_iter().map(Into::into).collect();
        self
    }

    /// Override cooldown ticks (default `DEFAULT_INTERRUPT_COOLDOWN_TICKS`).
    pub fn with_cooldown_ticks(mut self, n: i32) -> Self {
        self.cooldown_ticks = n.max(0);
        self
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
        self.interrupt_state.clear();
        // Reset the per-episode scheduled-event fired flags. Each event
        // fires exactly once per episode, so resizing here is sufficient
        // to re-arm them for the new run.
        self.fired_scheduled_events =
            vec![false; self.map_def.scheduled_events.len()];
        // Snapshot whether the enemy starts with any MustBeDestroyed
        // building. Decides the terminal-condition policy for the rest
        // of the episode (see `is_terminal`). Done after build_world
        // so the actor table is populated.
        self.enemy_started_with_buildings = match &self.world {
            Some(w) => has_must_be_destroyed_buildings(w, self.enemy_player_id),
            None => false,
        };
        // Whether the scenario placed *any* enemy actor. A no-enemy
        // scenario (agent-objective only, everything from YAML) must NOT
        // be auto-terminated by the enemy-elimination check — it would
        // otherwise be `done` at tick 0.
        self.enemy_started_present = match &self.world {
            Some(w) => !w.actor_ids_for_player(self.enemy_player_id).is_empty(),
            None => false,
        };
        // Reveal the shroud around starting units *before* the first
        // observation — OpenRA grants sight at game start (units have
        // RevealsShroud), so explored_percent must be > 0 at reset, not
        // only after the first step (which is the only other caller of
        // update_typed_shroud_all_players).
        if let Some(w) = self.world.as_mut() {
            w.update_typed_shroud_all_players();
        }
        self.refresh_explored_cells();
        self.observation()
    }

    /// One env step. Returns (obs, reward, done, info) — for v1 reward
    /// is always 0 (Python computes shaped rewards externally).
    pub fn step(&mut self, commands: &[Command]) -> StepResult {
        self.last_warnings.clear();

        // Apply all commands, building up a flat order list.
        let orders = self.build_orders(commands, self.agent_player_id);

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

    /// Advance the shared world one decision step with commands from
    /// BOTH players — the engine entry point for agent-vs-agent 1v1.
    ///
    /// Orders for each side are built independently (each scoped to its
    /// own unit ownership via `build_orders`) and applied into the SAME
    /// first frame, then `ticks_per_step` frames run — so neither side
    /// moves "first". Returns the post-step observation for EACH
    /// player, each from its own fog-of-war view. `done` is true once
    /// either base falls (or the deadline hits); the caller inspects
    /// each side's surviving buildings to decide the winner.
    pub fn step_1v1(
        &mut self,
        agent_commands: &[Command],
        enemy_commands: &[Command],
    ) -> Step1v1Result {
        self.last_warnings.clear();

        let mut orders = self.build_orders(agent_commands, self.agent_player_id);
        let enemy_orders =
            self.build_orders(enemy_commands, self.enemy_player_id);
        orders.extend(enemy_orders);

        if self.world.is_some() {
            self.tick_world_with_orders(&orders);
            for _ in 1..self.ticks_per_step {
                self.tick_world_with_orders(&[]);
            }
        }

        self.refresh_explored_cells();
        self.update_kill_counter();

        // Agent side reuses the cached single-controller state.
        let agent_obs = self.observation();
        // Enemy side: scan its own shroud + kill tally fresh.
        let enemy_kills = match &self.world {
            Some(w) => w.kills_for_player(self.enemy_player_id) as i32,
            None => 0,
        };
        let enemy_explored = self.explored_cells_for(self.enemy_player_id);
        let enemy_obs = self.observation_for(
            self.enemy_player_id,
            self.agent_player_id,
            &enemy_explored,
            enemy_kills,
        );

        let done = self.is_terminal();
        Step1v1Result {
            agent_obs,
            enemy_obs,
            done,
            warnings: self.last_warnings.clone(),
        }
    }

    /// Advance up to `max_ticks` frames, checking for interrupt signals
    /// every `check_every` frames. Returns early as soon as a signal
    /// fires (or the world reaches terminal). Mirrors the C# bridge's
    /// `advance` + `CheckInterrupts()` pattern in
    /// `ExternalBotBridge.cs:614`.
    ///
    /// Commands are issued on tick 0 only (same as `step()`).
    /// `enabled_signals_override` lets a single call narrow the
    /// configured `enabled_signals` (used for fine-grained per-call
    /// gating, e.g. silencing engage_start during pure-scout phases).
    /// Pass `None` to use the env-level `enabled_signals` set.
    pub fn step_until_event(
        &mut self,
        commands: &[Command],
        max_ticks: u32,
        check_every: u32,
        enabled_signals_override: Option<HashSet<String>>,
    ) -> StepUntilEventResult {
        self.last_warnings.clear();
        let orders = self.build_orders(commands, self.agent_player_id);

        // Clone into an owned set so we can call `&mut self` methods
        // (tick_world_with_orders, check_interrupts) without holding an
        // immutable borrow on `self.enabled_signals`. The set is tiny.
        let signals: HashSet<String> = enabled_signals_override
            .unwrap_or_else(|| self.enabled_signals.clone());
        let any_enabled = !signals.is_empty();
        let check_every = check_every.max(1);
        let max_ticks = max_ticks.max(1);

        let mut ticks_done: u32 = 0;
        let mut interrupt_reason: Option<String> = None;
        let mut applied_orders = false;

        while ticks_done < max_ticks {
            // Issue orders on the very first frame; subsequent ticks
            // just advance state.
            if !applied_orders {
                self.tick_world_with_orders(&orders);
                applied_orders = true;
            } else {
                self.tick_world_with_orders(&[]);
            }
            ticks_done += 1;

            if self.is_terminal() {
                break;
            }

            // Check signals every `check_every` ticks (and on the last
            // tick) to keep per-frame overhead bounded.
            let on_check_boundary = ticks_done % check_every == 0;
            if any_enabled && on_check_boundary {
                if let Some(reason) = self.check_interrupts(&signals) {
                    interrupt_reason = Some(reason);
                    break;
                }
            }
        }

        self.refresh_explored_cells();
        self.update_kill_counter();

        let obs = self.observation();
        let done = self.is_terminal();
        StepUntilEventResult {
            obs,
            reward: 0.0,
            done,
            warnings: self.last_warnings.clone(),
            interrupted: interrupt_reason.is_some(),
            interrupt_reason,
            ticks_advanced: ticks_done,
        }
    }

    /// Walk the current world and check each enabled signal against the
    /// previous-frame state. Updates `prev_*` snapshots and the
    /// last-fire ledger as side-effects. Returns the first signal name
    /// that fires (priority order matches `INTERRUPT_SIGNAL_NAMES`),
    /// or `None`.
    fn check_interrupts(&mut self, signals: &HashSet<String>) -> Option<String> {
        let world = self.world.as_ref()?;
        let now = world.world_tick as i32;
        let cooldown = self.cooldown_ticks;

        // Snapshot the current frame: which enemy unit / building IDs
        // are agent-visible right now, and which own-unit→target attack
        // pairs are active.
        let snap = world.snapshot();
        let mut cur_visible_enemy_units: HashSet<u32> = HashSet::new();
        let mut cur_visible_enemy_buildings: HashSet<u32> = HashSet::new();
        let mut cur_attacking_pairs: HashSet<(u32, u32)> = HashSet::new();
        let mut cur_own_unit_ids: HashSet<u32> = HashSet::new();
        let mut cur_own_actor_ids: HashSet<u32> = HashSet::new();

        for a in &snap.actors {
            if matches!(
                a.kind,
                ActorKind::World | ActorKind::Player | ActorKind::Spawn
            ) {
                continue;
            }
            if matches!(a.kind, ActorKind::Tree | ActorKind::Mine) {
                continue;
            }
            if a.owner == self.enemy_player_id {
                let cell_visible = self.is_visible_to_agent(world, a.x, a.y);
                let is_building = matches!(a.kind, ActorKind::Building);
                // Spy infiltration permanently reveals enemy buildings
                // to the agent even through fog (one-shot reveal scan).
                let infiltrated = is_building
                    && world.was_infiltration_revealed(self.agent_player_id, a.id);
                if !cell_visible && !infiltrated {
                    continue;
                }
                if is_building {
                    cur_visible_enemy_buildings.insert(a.id);
                } else {
                    cur_visible_enemy_units.insert(a.id);
                }
            } else if a.owner == self.agent_player_id {
                // Track agent-owned combat units (incl. MCV) for the
                // own_unit_destroyed interrupt. Buildings are excluded —
                // building loss is a coarser endgame condition handled
                // by `is_terminal`.
                if matches!(
                    a.kind,
                    ActorKind::Infantry
                        | ActorKind::Vehicle
                        | ActorKind::Aircraft
                        | ActorKind::Ship
                        | ActorKind::Mcv
                ) {
                    cur_own_unit_ids.insert(a.id);
                }
                // Units + buildings count toward production_complete
                // (a finished unit OR a completed structure).
                if matches!(
                    a.kind,
                    ActorKind::Infantry
                        | ActorKind::Vehicle
                        | ActorKind::Aircraft
                        | ActorKind::Ship
                        | ActorKind::Mcv
                        | ActorKind::Building
                ) {
                    cur_own_actor_ids.insert(a.id);
                }
                if let Some(tid) = a.target_id {
                    cur_attacking_pairs.insert((a.id, tid));
                }
            }
        }

        let mut fired: Option<String> = None;

        // Priority order: own_unit_destroyed > engage_start > enemy_unit_spotted
        // > enemy_building_spotted. Losing one of your own units is the most
        // urgent signal — re-prompt the agent immediately so it can react
        // before the rest of the force is wiped.
        if signals.contains("own_unit_destroyed") {
            // A previously-tracked own unit ID that no longer appears
            // in the world snapshot has been destroyed. Report up to
            // one such loss per check (cooldown-throttled per id).
            let lost_ids: Vec<u32> = self
                .interrupt_state
                .prev_own_unit_ids
                .iter()
                .filter(|id| !cur_own_unit_ids.contains(id))
                .copied()
                .collect();
            for id in lost_ids {
                let key = id as u64;
                if self
                    .interrupt_state
                    .cooldown_ok("own_unit_destroyed", key, now, cooldown)
                {
                    self.interrupt_state
                        .mark_fired("own_unit_destroyed", key, now);
                    fired = Some(format!("own_unit_destroyed: id {}", id));
                    break;
                }
            }
        }

        if fired.is_none() && signals.contains("engage_start") {
            for &(uid, tid) in &cur_attacking_pairs {
                if !self.interrupt_state.prev_attacking_pairs.contains(&(uid, tid)) {
                    let key = ((uid as u64) << 32) | (tid as u64);
                    if self
                        .interrupt_state
                        .cooldown_ok("engage_start", key, now, cooldown)
                    {
                        self.interrupt_state.mark_fired("engage_start", key, now);
                        fired = Some(format!("engage_start: own {} → target {}", uid, tid));
                        break;
                    }
                }
            }
        }

        if fired.is_none()
            && signals.contains("production_complete")
            && self.interrupt_state.production_baseline_set
        {
            let new_ids: Vec<u32> = cur_own_actor_ids
                .iter()
                .filter(|id| !self.interrupt_state.prev_own_actor_ids.contains(id))
                .copied()
                .collect();
            for id in new_ids {
                let key = id as u64;
                if self
                    .interrupt_state
                    .cooldown_ok("production_complete", key, now, cooldown)
                {
                    self.interrupt_state
                        .mark_fired("production_complete", key, now);
                    fired = Some(format!("production_complete: id {}", id));
                    break;
                }
            }
        }

        if fired.is_none() && signals.contains("enemy_unit_spotted") {
            for &id in &cur_visible_enemy_units {
                if !self
                    .interrupt_state
                    .prev_visible_enemy_unit_ids
                    .contains(&id)
                {
                    let key = id as u64;
                    if self
                        .interrupt_state
                        .cooldown_ok("enemy_unit_spotted", key, now, cooldown)
                    {
                        self.interrupt_state
                            .mark_fired("enemy_unit_spotted", key, now);
                        fired = Some(format!("enemy_unit_spotted: id {}", id));
                        break;
                    }
                }
            }
        }

        if fired.is_none() && signals.contains("enemy_building_spotted") {
            for &id in &cur_visible_enemy_buildings {
                if !self
                    .interrupt_state
                    .prev_visible_enemy_building_ids
                    .contains(&id)
                {
                    let key = id as u64;
                    if self
                        .interrupt_state
                        .cooldown_ok("enemy_building_spotted", key, now, cooldown)
                    {
                        self.interrupt_state
                            .mark_fired("enemy_building_spotted", key, now);
                        fired = Some(format!("enemy_building_spotted: id {}", id));
                        break;
                    }
                }
            }
        }

        // Always update prev-frame snapshots so the next check sees a
        // proper transition. Crucial: a signal that we suppressed via
        // cooldown is still added to the prev-set so it doesn't keep
        // re-evaluating every check.
        self.interrupt_state.prev_visible_enemy_unit_ids = cur_visible_enemy_units;
        self.interrupt_state.prev_visible_enemy_building_ids = cur_visible_enemy_buildings;
        self.interrupt_state.prev_own_unit_ids = cur_own_unit_ids;
        self.interrupt_state.prev_attacking_pairs = cur_attacking_pairs;
        self.interrupt_state.prev_own_actor_ids = cur_own_actor_ids;
        self.interrupt_state.production_baseline_set = true;

        fired
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
            // Designed economy constraint from the scenario (default
            // 5000 = OpenRA skirmish default). Was hardcoded 0, which
            // starved all production/economy scenarios.
            starting_cash: self.map_def.starting_cash,
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
        // Phase 7 — load the vendored RA ruleset so building weapons
        // (TurretGun, TeslaZap, …) and proper footprints (pbox=1×1,
        // fact=3×2, proc=3×2) resolve correctly. Phase 8 also pulls a
        // typed `data_rules::Rules` view so we can attach Vehicle /
        // Turret typed components per actor below. Falls back to
        // `GameRules::defaults` when the vendor dir is missing.
        let (rules, typed_rules) = load_rules_with_fallback();
        let mut world = build_world(
            &ora,
            seed_i32,
            &lobby,
            Some(rules),
            0,
            self.map_def.spawn_mcvs,
        );

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
            // Resource source: a `mine`/`gmine` is a neutral map prop
            // (no owner). Scenario actors are injected here AFTER
            // build_world, so build_world's ore-seeding never sees
            // them — seed the ore patch into the live terrain now so
            // harvesters can find it.
            if sa.actor_type == "mine" || sa.actor_type == "gmine" {
                let (mx, my) = sa.position;
                let r: i32 = 5;
                for dy in -r..=r {
                    for dx in -r..=r {
                        if dx * dx + dy * dy > r * r || (dx == 0 && dy == 0) {
                            continue;
                        }
                        let (x, y) = (mx + dx, my + dy);
                        if world.terrain.contains(x, y)
                            && world.terrain.is_terrain_passable(x, y)
                        {
                            world.terrain.set_resource(
                                x,
                                y,
                                openra_sim::terrain::ResourceType::Ore,
                                50,
                            );
                        }
                    }
                }
                continue;
            }
            let owner = match sa.owner.as_str() {
                "agent" => agent_pid,
                "enemy" => enemy_pid,
                _ => continue,
            };
            let actor = build_scenario_actor(next_id, sa, owner, &world);
            world::insert_test_actor(&mut world, actor);
            // Honour an optional per-actor `stance:` from the scenario
            // (e.g. an enemy spawned on HoldFire so it won't pre-empt
            // the agent). Applied BEFORE the reset warmup frame so the
            // first auto-engage scan already respects it.
            if let Some(s) = sa.stance {
                world::set_actor_stance(&mut world, next_id, s);
            }
            // Phase 8 — attach Vehicle + Turret typed components for
            // vehicle actors (2tnk, 1tnk, 3tnk, jeep, apc, harv, mcv).
            // Static defenses with turrets (gun) carry their own
            // armament path via classify_defense; we only need the
            // typed component for query-by-tests / future visual aim.
            attach_typed_components(&mut world, next_id, &sa.actor_type, &typed_rules);
            next_id += 1;
        }

        // Lift the order-latency pause so the very first `step()`
        // advances the tick counter.
        set_test_unpaused(&mut world);

        // Attach a scripted opponent controller if the scenario set
        // `enemy: {bot: ...}` (else enemy stays stance-only reactive).
        if let Some(bt) = self.map_def.enemy_bot.as_deref() {
            match openra_sim::scripted_bot::ScriptedBehavior::parse(bt) {
                Some(b) => world.add_scripted_bot(
                    self.enemy_player_id,
                    self.agent_player_id,
                    b,
                ),
                None => eprintln!("unknown enemy.bot {bt:?} — ignored"),
            }
        }

        // Run one no-op frame so shroud reveals are recomputed with
        // the injected actors.
        world.process_frame(&[]);
        world
    }

    /// Translate `[Command]` into raw `GameOrder`s.
    /// Translate a player's `Command`s into engine `GameOrder`s.
    ///
    /// `issuing_player` is the player on whose behalf the commands are
    /// issued — every unit-ownership check and every player-subject
    /// order (Surrender / StartProduction / PlaceBuilding) is scoped to
    /// it. The single-controller `step` passes `agent_player_id`; the
    /// 1v1 path (`step_1v1`) calls this once per side so both players
    /// issue orders into the same frame.
    fn build_orders(
        &mut self,
        commands: &[Command],
        issuing_player: u32,
    ) -> Vec<GameOrder> {
        let mut orders = Vec::new();
        let world = match &self.world {
            Some(w) => w,
            None => return orders,
        };

        let issuer_owned: HashSet<u32> = world
            .actor_ids_for_player(issuing_player)
            .into_iter()
            .collect();

        for cmd in commands {
            match cmd {
                Command::Observe => {}
                Command::Surrender => {
                    orders.push(GameOrder {
                        order_string: "Surrender".into(),
                        subject_id: Some(issuing_player),
                        target_string: None,
                        extra_data: None,
                    });
                }
                Command::SetStance { unit_ids, stance } => {
                    let s = (*stance).clamp(0, 3) as u32;
                    for id in unit_ids {
                        if let Some(aid) =
                            resolve_owned(id, &issuer_owned, &mut self.last_warnings)
                        {
                            orders.push(GameOrder {
                                order_string: "SetStance".into(),
                                subject_id: Some(aid),
                                target_string: None,
                                extra_data: Some(s),
                            });
                        }
                    }
                }
                Command::Patrol { unit_ids } => {
                    for id in unit_ids {
                        if let Some(aid) =
                            resolve_owned(id, &issuer_owned, &mut self.last_warnings)
                        {
                            orders.push(GameOrder {
                                order_string: "Patrol".into(),
                                subject_id: Some(aid),
                                target_string: None,
                                extra_data: None,
                            });
                        }
                    }
                }
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
                        if !issuer_owned.contains(&aid) {
                            self.last_warnings
                                .push(format!("unit {aid} not owned by issuing player"));
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
                        if !issuer_owned.contains(&aid) {
                            self.last_warnings
                                .push(format!("unit {aid} not owned by issuing player"));
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
                Command::Guard { unit_ids, target_id } => {
                    let target_aid = match parse_actor_id(target_id) {
                        Some(v) => v,
                        None => {
                            self.last_warnings
                                .push(format!("invalid target_id {target_id:?}"));
                            continue;
                        }
                    };
                    for id in unit_ids {
                        if let Some(aid) =
                            resolve_owned(id, &issuer_owned, &mut self.last_warnings)
                        {
                            orders.push(GameOrder {
                                order_string: "Guard".into(),
                                subject_id: Some(aid),
                                target_string: None,
                                extra_data: Some(target_aid),
                            });
                        }
                    }
                }
                Command::SetPrimary { unit_ids } => {
                    for id in unit_ids {
                        if let Some(aid) =
                            resolve_owned(id, &issuer_owned, &mut self.last_warnings)
                        {
                            orders.push(GameOrder {
                                order_string: "SetPrimary".into(),
                                subject_id: Some(aid),
                                target_string: None,
                                extra_data: None,
                            });
                        }
                    }
                }
                Command::EnterTransport { unit_ids, target_id } => {
                    let target_aid = match parse_actor_id(target_id) {
                        Some(v) => v,
                        None => {
                            self.last_warnings
                                .push(format!("invalid target_id {target_id:?}"));
                            continue;
                        }
                    };
                    let cap = world.transport_capacity(target_aid) as usize;
                    let already = world.transport_cargo(target_aid).len();
                    let mut accepted = 0usize;
                    for id in unit_ids {
                        if let Some(aid) =
                            resolve_owned(id, &issuer_owned, &mut self.last_warnings)
                        {
                            if already + accepted >= cap {
                                self.last_warnings.push(format!(
                                    "transport {target_aid} full (capacity {cap}); \
                                     passenger {aid} rejected"
                                ));
                                continue;
                            }
                            accepted += 1;
                            orders.push(GameOrder {
                                order_string: "EnterTransport".into(),
                                subject_id: Some(aid),
                                target_string: None,
                                extra_data: Some(target_aid),
                            });
                        }
                    }
                }
                Command::Unload { unit_ids } => {
                    for id in unit_ids {
                        if let Some(aid) =
                            resolve_owned(id, &issuer_owned, &mut self.last_warnings)
                        {
                            orders.push(GameOrder {
                                order_string: "Unload".into(),
                                subject_id: Some(aid),
                                target_string: None,
                                extra_data: None,
                            });
                        }
                    }
                }
                Command::AttackMove { unit_ids, target_x, target_y } => {
                    for id in unit_ids {
                        if let Some(aid) =
                            resolve_owned(id, &issuer_owned, &mut self.last_warnings)
                        {
                            orders.push(GameOrder {
                                order_string: "AttackMove".into(),
                                subject_id: Some(aid),
                                target_string: Some(format!("{target_x},{target_y}")),
                                extra_data: None,
                            });
                        }
                    }
                }
                Command::Harvest { unit_ids, target_x, target_y } => {
                    for id in unit_ids {
                        if let Some(aid) =
                            resolve_owned(id, &issuer_owned, &mut self.last_warnings)
                        {
                            orders.push(GameOrder {
                                order_string: "Harvest".into(),
                                subject_id: Some(aid),
                                target_string: Some(format!("{target_x},{target_y}")),
                                extra_data: None,
                            });
                        }
                    }
                }
                Command::SetRallyPoint { unit_ids, target_x, target_y } => {
                    for id in unit_ids {
                        if let Some(aid) =
                            resolve_owned(id, &issuer_owned, &mut self.last_warnings)
                        {
                            orders.push(GameOrder {
                                order_string: "SetRallyPoint".into(),
                                subject_id: Some(aid),
                                target_string: Some(format!("{target_x},{target_y}")),
                                extra_data: None,
                            });
                        }
                    }
                }
                Command::Stop { unit_ids }
                | Command::Deploy { unit_ids }
                | Command::Sell { unit_ids }
                | Command::Repair { unit_ids }
                | Command::PowerDown { unit_ids } => {
                    let order_string = match cmd {
                        Command::Stop { .. } => "Stop",
                        Command::Deploy { .. } => "DeployTransform",
                        Command::Sell { .. } => "Sell",
                        Command::Repair { .. } => "RepairBuilding",
                        Command::PowerDown { .. } => "PowerDown",
                        _ => unreachable!(),
                    };
                    for id in unit_ids {
                        if let Some(aid) =
                            resolve_owned(id, &issuer_owned, &mut self.last_warnings)
                        {
                            orders.push(GameOrder {
                                order_string: order_string.into(),
                                subject_id: Some(aid),
                                target_string: None,
                                extra_data: None,
                            });
                        }
                    }
                }
                Command::Build { item } => {
                    orders.push(GameOrder {
                        order_string: "StartProduction".into(),
                        subject_id: Some(issuing_player),
                        target_string: Some(item.clone()),
                        extra_data: None,
                    });
                }
                Command::CancelProduction { item } => {
                    orders.push(GameOrder {
                        order_string: "CancelProduction".into(),
                        subject_id: Some(issuing_player),
                        target_string: Some(item.clone()),
                        extra_data: None,
                    });
                }
                Command::PlaceBuilding { item, target_x, target_y } => {
                    orders.push(GameOrder {
                        order_string: "PlaceBuilding".into(),
                        subject_id: Some(issuing_player),
                        target_string: Some(format!("{item},{target_x},{target_y}")),
                        extra_data: None,
                    });
                }
                Command::C4Detonate { unit_ids, target_id } => {
                    // Engine re-validates (subject must be `tanya`,
                    // target must be an enemy Building), but we filter
                    // here too so warnings surface early and don't get
                    // silently dropped.
                    let target_aid = match parse_actor_id(target_id) {
                        Some(v) => v,
                        None => {
                            self.last_warnings
                                .push(format!("invalid target_id {target_id:?}"));
                            continue;
                        }
                    };
                    // Target must be a building (alive); ownership check
                    // (must be enemy) happens at the engine layer where
                    // the issuer's actual owner id is in scope.
                    let target_is_building = world
                        .actor(target_aid)
                        .map(|a| a.kind == openra_sim::actor::ActorKind::Building)
                        .unwrap_or(false);
                    if !target_is_building {
                        self.last_warnings.push(format!(
                            "C4Detonate target {target_aid} is not a building; dropped"
                        ));
                        continue;
                    }
                    for id in unit_ids {
                        if let Some(aid) =
                            resolve_owned(id, &issuer_owned, &mut self.last_warnings)
                        {
                            let is_tanya = world
                                .actor(aid)
                                .and_then(|a| a.actor_type.as_deref())
                                == Some("tanya");
                            if !is_tanya {
                                self.last_warnings.push(format!(
                                    "C4Detonate subject {aid} is not a tanya; dropped"
                                ));
                                continue;
                            }
                            orders.push(GameOrder {
                                order_string: "C4Detonate".into(),
                                subject_id: Some(aid),
                                target_string: None,
                                extra_data: Some(target_aid),
                            });
                        }
                    }
                }
                Command::CaptureActor { unit_ids, target_id } => {
                    // Engineer (e6) walks to an enemy building and
                    // captures it. We accept any owned actor id here;
                    // the engine-side `order_capture_actor` gates on
                    // actor_type == "e6" and target being an enemy
                    // Building, so non-engineer / friendly targets are
                    // silently dropped at the engine.
                    let target_aid = match parse_actor_id(target_id) {
                        Some(v) => v,
                        None => {
                            self.last_warnings
                                .push(format!("invalid target_id {target_id:?}"));
                            continue;
                        }
                    };
                    for id in unit_ids {
                        if let Some(aid) =
                            resolve_owned(id, &issuer_owned, &mut self.last_warnings)
                        {
                            orders.push(GameOrder {
                                order_string: "CaptureActor".into(),
                                subject_id: Some(aid),
                                target_string: None,
                                extra_data: Some(target_aid),
                            });
                        }
                    }
                }
                Command::Infiltrate { unit_ids, target_id } => {
                    let target_aid = match parse_actor_id(target_id) {
                        Some(v) => v,
                        None => {
                            self.last_warnings
                                .push(format!("invalid target_id {target_id:?}"));
                            continue;
                        }
                    };
                    // Engine validates owner / building / infiltrator
                    // type at order time (`order_infiltrate`); the env
                    // just resolves agent-owned subjects.
                    for id in unit_ids {
                        if let Some(aid) =
                            resolve_owned(id, &issuer_owned, &mut self.last_warnings)
                        {
                            orders.push(GameOrder {
                                order_string: "Infiltrate".into(),
                                subject_id: Some(aid),
                                target_string: None,
                                extra_data: Some(target_aid),
                            });
                        }
                    }
                }
                Command::FireSuperweapon { kind, target_cell, target_id } => {
                    // Resolve and validate the (typed) actor id for
                    // iron-curtain / chrono targets. Nuke ignores
                    // `target_id`.
                    let resolved_target: Option<u32> = match target_id {
                        Some(id_str) => match parse_actor_id(id_str) {
                            Some(v) => Some(v),
                            None => {
                                self.last_warnings.push(format!(
                                    "FireSuperweapon: invalid target_id {id_str:?}"
                                ));
                                continue;
                            }
                        },
                        None => None,
                    };
                    // Encode payload into target_string for the engine
                    // order handler: "kind|tx,ty|tid" where any of tx,ty,
                    // tid can be empty (`-`) when not provided.
                    let cell_part = match target_cell {
                        Some((x, y)) => format!("{x},{y}"),
                        None => "-".into(),
                    };
                    let tid_part = match resolved_target {
                        Some(v) => v.to_string(),
                        None => "-".into(),
                    };
                    orders.push(GameOrder {
                        order_string: "FireSuperweapon".into(),
                        subject_id: Some(issuing_player),
                        target_string: Some(format!("{kind}|{cell_part}|{tid_part}")),
                        extra_data: None,
                    });
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
        // Fire any scheduled events that have reached their tick. We do
        // this AFTER process_frame so SpawnActors land in the just-
        // advanced world (and the next process_frame's shroud refresh
        // sees them). For consistency with the initial-spawn path we
        // refresh the typed shroud once more if any event fired.
        if self.fire_scheduled_events()
            && let Some(world) = self.world.as_mut()
        {
            world.update_typed_shroud_all_players();
        }
    }

    /// Walk `map_def.scheduled_events`, firing each entry whose `tick`
    /// has been reached and that hasn't fired yet in this episode.
    /// Returns `true` if at least one event fired.
    ///
    /// SpawnActors injects new actors using the same path as the
    /// initial-actors loop in `build_world_for_episode` (Mobile/Health
    /// traits for vehicles, Building/Health for structures, optional
    /// stance, typed Vehicle/Turret components for tanks).
    ///
    /// DestroyActors removes every actor matching the filter via
    /// `world::remove_test_actor` (also clears terrain occupancy).
    ///
    /// ShortenDeadline clamps `self.max_ticks` downward only — the
    /// deadline never grows, matching the design intent.
    fn fire_scheduled_events(&mut self) -> bool {
        if self.map_def.scheduled_events.is_empty() {
            return false;
        }
        let world_tick = match &self.world {
            Some(w) => w.world_tick,
            None => return false,
        };
        let mut any_fired = false;
        // Snapshot events to fire (indices) so we can release the
        // immutable borrow on `self` before mutating.
        let mut to_fire: Vec<usize> = Vec::new();
        for (idx, ev) in self.map_def.scheduled_events.iter().enumerate() {
            if self
                .fired_scheduled_events
                .get(idx)
                .copied()
                .unwrap_or(true)
            {
                continue;
            }
            if world_tick >= ev.tick {
                to_fire.push(idx);
            }
        }
        for idx in to_fire {
            // Clone the kind so we can drop the borrow on `self.map_def`.
            let kind = self.map_def.scheduled_events[idx].kind.clone();
            self.fired_scheduled_events[idx] = true;
            any_fired = true;
            match kind {
                openra_data::oramap::ScheduledEventKind::SpawnActors {
                    actors,
                } => {
                    self.apply_spawn_actors(&actors);
                }
                openra_data::oramap::ScheduledEventKind::DestroyActors {
                    filter,
                } => {
                    self.apply_destroy_actors(&filter);
                }
                openra_data::oramap::ScheduledEventKind::ShortenDeadline {
                    new_max_ticks,
                } => {
                    if new_max_ticks < self.max_ticks {
                        self.max_ticks = new_max_ticks.max(1);
                    }
                }
            }
        }
        any_fired
    }

    /// Inject the supplied actors into the live world. Mirrors the
    /// "scenario actors" loop in `build_world_for_episode`.
    fn apply_spawn_actors(&mut self, actors: &[ScenarioActor]) {
        let agent_pid = self.agent_player_id;
        let enemy_pid = self.enemy_player_id;
        let world = match self.world.as_mut() {
            Some(w) => w,
            None => return,
        };
        // Lazily resolve a typed ruleset so newly-spawned vehicles get
        // Vehicle/Turret components (otherwise turreted tanks would
        // never aim at targets). Cached across the episode.
        if self.typed_rules_cache.is_none() {
            let (_, typed_rules) = load_rules_with_fallback();
            self.typed_rules_cache = Some(typed_rules);
        }
        let typed_rules = self.typed_rules_cache.as_ref().expect("cached above");
        let mut next_id = scenario_id_seed(world);
        for sa in actors {
            let owner = match sa.owner.as_str() {
                "agent" => agent_pid,
                "enemy" => enemy_pid,
                _ => continue,
            };
            let actor = build_scenario_actor(next_id, sa, owner, world);
            world::insert_test_actor(world, actor);
            if let Some(s) = sa.stance {
                world::set_actor_stance(world, next_id, s);
            }
            attach_typed_components(world, next_id, &sa.actor_type, typed_rules);
            next_id += 1;
        }
    }

    /// Remove every actor matching `filter` from the world.
    fn apply_destroy_actors(
        &mut self,
        filter: &openra_data::oramap::ActorFilter,
    ) {
        let agent_pid = self.agent_player_id;
        let enemy_pid = self.enemy_player_id;
        let world = match self.world.as_mut() {
            Some(w) => w,
            None => return,
        };
        // Resolve owner-id filter (if any). Unknown owner tags drop the
        // whole event silently — matches the lenient parsing policy.
        let owner_pid: Option<u32> = match filter.owner.as_deref() {
            None => None,
            Some("agent") => Some(agent_pid),
            Some("enemy") => Some(enemy_pid),
            Some(_) => return,
        };
        // Enumerate candidate ids. With an owner filter we can restrict
        // to that player's actors; otherwise we sweep every owned actor.
        let candidate_ids: Vec<u32> = match owner_pid {
            Some(pid) => world.actor_ids_for_player(pid),
            None => {
                let mut v = world.actor_ids_for_player(agent_pid);
                v.extend(world.actor_ids_for_player(enemy_pid));
                v
            }
        };
        let region = filter.region;
        let mut to_remove: Vec<u32> = Vec::new();
        for aid in candidate_ids {
            if let Some(r) = region {
                let (ax, ay) = match world.actor_location(aid) {
                    Some(p) => p,
                    None => continue,
                };
                let dx = ax - r.x;
                let dy = ay - r.y;
                if dx * dx + dy * dy > r.radius * r.radius {
                    continue;
                }
            }
            to_remove.push(aid);
        }
        for id in to_remove {
            world::remove_test_actor(world, id);
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
    /// S9 spatial tensor: flat row-major `[y][x][c]`, 6 channels
    /// (passable, fog, own-unit density, visible-enemy density, own
    /// building, resource). Pure function of already-observed state.
    fn build_spatial(
        &self,
        world: &World,
        snap: &openra_sim::world::WorldSnapshot,
    ) -> (Vec<f32>, (i32, i32, i32)) {
        let (w, h) = (snap.map_width, snap.map_height);
        let c = crate::observation::SPATIAL_CHANNELS;
        if w <= 0 || h <= 0 {
            return (Vec::new(), (0, 0, c));
        }
        let mut t = vec![0f32; (w * h * c) as usize];
        let idx = |x: i32, y: i32, ch: i32| ((y * w + x) * c + ch) as usize;
        for y in 0..h {
            for x in 0..w {
                if world.terrain.is_terrain_passable(x, y) {
                    t[idx(x, y, 0)] = 1.0;
                }
                t[idx(x, y, 1)] = if self.is_visible_to_agent(world, x, y) {
                    1.0
                } else if self.explored_cells.contains(&(x, y)) {
                    0.5
                } else {
                    0.0
                };
                if world.terrain.has_resource(x, y) {
                    t[idx(x, y, 5)] = 1.0;
                }
            }
        }
        for a in &snap.actors {
            let (x, y) = (a.x, a.y);
            if x < 0 || x >= w || y < 0 || y >= h {
                continue;
            }
            let own = a.owner == self.agent_player_id;
            match a.kind {
                ActorKind::Building => {
                    if own {
                        t[idx(x, y, 4)] = 1.0;
                    }
                }
                ActorKind::Infantry
                | ActorKind::Vehicle
                | ActorKind::Mcv
                | ActorKind::Ship => {
                    if own {
                        t[idx(x, y, 2)] += 1.0;
                    } else if self.is_visible_to_agent(world, x, y) {
                        t[idx(x, y, 3)] += 1.0;
                    }
                }
                _ => {}
            }
        }
        (t, (h, w, c))
    }

    /// Single-controller observation: the agent player's view.
    fn observation(&self) -> Observation {
        self.observation_for(
            self.agent_player_id,
            self.enemy_player_id,
            &self.explored_cells,
            self.units_killed,
        )
    }

    /// Observation from the ENEMY player's perspective at the CURRENT
    /// state, without advancing the world. The 1v1 harness uses this to
    /// seed the enemy controller's first observation — `reset()` only
    /// surfaces the agent's view, so without this the harness would
    /// have to waste a decision turn on an idle bootstrap step.
    pub fn enemy_observation(&self) -> Observation {
        let enemy_kills = match &self.world {
            Some(w) => w.kills_for_player(self.enemy_player_id) as i32,
            None => 0,
        };
        let enemy_explored = self.explored_cells_for(self.enemy_player_id);
        self.observation_for(
            self.enemy_player_id,
            self.agent_player_id,
            &enemy_explored,
            enemy_kills,
        )
    }

    /// Build an `Observation` from `viewer`'s perspective: `viewer`'s
    /// actors as own units, `opponent`'s actors fog-limited to
    /// `viewer`'s shroud, and economy / production / buildings for
    /// `viewer`. `explored` and `units_killed` are passed in so the
    /// single-controller path reuses its cached agent state byte-for-
    /// byte, while the 1v1 path supplies the opponent's freshly-scanned
    /// values. NOTE: the `spatial` tensor is left agent-perspective —
    /// it is consumed only by the RL `step` path, never by the 1v1 LLM
    /// controllers, which read the text/minimap observation fields.
    fn observation_for(
        &self,
        viewer: u32,
        opponent: u32,
        explored: &HashSet<(i32, i32)>,
        units_killed: i32,
    ) -> Observation {
        let world = match &self.world {
            Some(w) => w,
            None => {
                return Observation {
                    unit_positions: Vec::new(),
                    unit_hp: Vec::new(),
                    enemy_positions: Vec::new(),
                    enemy_hp: Vec::new(),
                    enemy_buildings: Vec::new(),
                    units_killed: 0,
                    game_tick: 0,
                    explored_percent: 0.0,
                    explored_cells: Vec::new(),
                    economy: crate::observation::EconomyObs::default(),
                    own_buildings: Vec::new(),
                    production: Vec::new(),
                    map_info: crate::observation::MapInfo::default(),
                    spatial: Vec::new(),
                    spatial_shape: (0, 0, crate::observation::SPATIAL_CHANNELS),
                };
            }
        };

        let snap = world.snapshot();

        let mut unit_positions: Vec<(String, UnitPos)> = Vec::new();
        let mut unit_hp: Vec<(String, f32)> = Vec::new();
        let mut enemy_positions: Vec<EnemyPos> = Vec::new();
        let mut enemy_hp: Vec<(String, f32)> = Vec::new();
        let mut enemy_buildings: Vec<EnemyBuilding> = Vec::new();

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
            if a.owner == viewer {
                if matches!(a.kind, ActorKind::Building) {
                    // Own buildings — Phase 7 surfaces them too, though
                    // strategy scenarios don't pre-place any agent
                    // structures yet (the rush-hour env has none either).
                    // Still skip the unit lists since those carry combat
                    // unit positions only.
                    continue;
                }
                let target = match (a.target_x, a.target_y) {
                    (Some(tx), Some(ty)) => Some((tx, ty)),
                    _ => None,
                };
                let attacking_target_id = if a.activity == "attacking" {
                    a.target_id.map(|tid| tid.to_string())
                } else {
                    None
                };
                unit_positions.push((
                    id_str.clone(),
                    UnitPos {
                        cell_x: a.x,
                        cell_y: a.y,
                        target,
                        activity: a.activity.clone(),
                        attacking_target_id,
                        actor_type: a.actor_type.clone(),
                    },
                ));
                unit_hp.push((id_str, pct));
            } else if a.owner == opponent {
                let cell_visible = self.is_visible_to(world, viewer, a.x, a.y);
                let is_building = matches!(a.kind, ActorKind::Building);
                let infiltrated = is_building
                    && world.was_infiltration_revealed(viewer, a.id);
                if !cell_visible && !infiltrated {
                    continue;
                }
                if is_building {
                    enemy_buildings.push(EnemyBuilding {
                        cell_x: a.x,
                        cell_y: a.y,
                        id: id_str,
                        kind: a.actor_type.clone(),
                        hp_pct: pct,
                    });
                } else {
                    enemy_positions.push(EnemyPos {
                        cell_x: a.x,
                        cell_y: a.y,
                        id: id_str.clone(),
                        actor_type: a.actor_type.clone(),
                    });
                    enemy_hp.push((id_str, pct));
                }
            }
        }

        let explored_percent = if self.map_total_cells > 0 {
            (explored.len() as f32 / self.map_total_cells as f32) * 100.0
        } else {
            0.0
        };

        // Snapshot the cumulative explored set as a list. The Python
        // minimap renderer uses this as ground truth instead of
        // re-deriving from briefing-time unit positions (which misses
        // cells units transited between briefings).
        let explored_cells: Vec<(i32, i32)> =
            explored.iter().copied().collect();

        // S9 — economy / own buildings / production for the agent.
        let mut economy = crate::observation::EconomyObs::default();
        let mut production: Vec<crate::observation::ProductionObs> = Vec::new();
        if let Some(ps) = snap.players.iter().find(|p| p.index == viewer)
        {
            economy.cash = ps.cash;
            economy.power_provided = ps.power_provided;
            economy.power_drained = ps.power_drained;
            economy.resources = ps.resources;
            economy.resource_capacity = ps.resource_capacity;
            for q in &ps.production_queue {
                production.push(crate::observation::ProductionObs {
                    item: q.item_name.clone(),
                    progress: q.progress,
                    done: q.done,
                });
            }
        }
        let mut own_buildings: Vec<crate::observation::OwnBuilding> = Vec::new();
        for a in &snap.actors {
            if a.owner != viewer {
                continue;
            }
            if a.actor_type.to_ascii_lowercase().starts_with("harv") {
                economy.harvesters += 1;
            }
            if matches!(a.kind, ActorKind::Building) {
                let hp_pct = if a.max_hp > 0 {
                    (a.hp as f32 / a.max_hp as f32).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                own_buildings.push(crate::observation::OwnBuilding {
                    id: a.id.to_string(),
                    building_type: a.actor_type.clone(),
                    cell_x: a.x,
                    cell_y: a.y,
                    hp_pct,
                    is_primary: world.is_primary_building(a.id),
                });
            }
        }

        let (spatial, spatial_shape) = self.build_spatial(world, &snap);

        Observation {
            unit_positions,
            unit_hp,
            enemy_positions,
            enemy_hp,
            enemy_buildings,
            units_killed,
            game_tick: world.world_tick as i32,
            explored_percent,
            explored_cells,
            economy,
            own_buildings,
            production,
            map_info: crate::observation::MapInfo {
                width: snap.map_width,
                height: snap.map_height,
            },
            spatial,
            spatial_shape,
        }
    }

    /// Refresh `explored_cells` from the agent's typed shroud
    /// (`World::typed_shroud(player)`). The shroud's `is_explored`
    /// flag is sticky — once a cell has been seen by any agent unit
    /// it stays explored, matching OpenRA's `Shroud.IsExplored`.
    fn refresh_explored_cells(&mut self) {
        // `reveal_map` scenarios have no fog: the whole playable
        // rectangle counts as explored regardless of the shroud.
        let reveal = self.map_def.reveal_map;
        let world = match &self.world {
            Some(w) => w,
            None => return,
        };
        let shroud = match world.typed_shroud(self.agent_player_id) {
            Some(s) => s,
            None => return,
        };
        // Iterate the playable rectangle only (bounds = x,y,w,h).
        // Iterating the full map_size would let shroud.is_explored
        // mark cells in the 2-cell shroud border outside Bounds, which
        // then push explored_cells.len() past map_total_cells and make
        // explored_percent exceed 100. 100% must mean 'entire playable
        // region revealed'.
        let (bx, by, bw, bh) = self.map_def.bounds;
        let (mw, mh) = self.map_def.map_size;
        let (x_lo, x_hi, y_lo, y_hi) = if bw > 0 && bh > 0 {
            (bx, bx + bw, by, by + bh)
        } else {
            (0, mw, 0, mh)
        };
        for y in y_lo..y_hi {
            for x in x_lo..x_hi {
                if reveal || shroud.is_explored(x, y) {
                    self.explored_cells.insert((x, y));
                }
            }
        }
    }

    /// Cell visibility via a player's typed shroud `is_visible` flag —
    /// only counts cells currently in sight of one of `player`'s units.
    /// A `reveal_map: true` scenario disables fog for the AGENT player
    /// only: every cell is visible to it (the no-fog cells of the
    /// perception ablation grid). Other viewers (e.g. the 1v1 opponent
    /// controller) keep their own shroud.
    fn is_visible_to(
        &self,
        world: &World,
        player: u32,
        cx: i32,
        cy: i32,
    ) -> bool {
        if self.map_def.reveal_map && player == self.agent_player_id {
            return true;
        }
        match world.typed_shroud(player) {
            Some(s) => s.is_visible(cx, cy),
            None => false,
        }
    }

    /// Cell visibility for the agent player (single-controller path).
    fn is_visible_to_agent(&self, world: &World, cx: i32, cy: i32) -> bool {
        self.is_visible_to(world, self.agent_player_id, cx, cy)
    }

    /// The set of cells `viewer` has ever explored, scanned over the
    /// playable rectangle (mirrors `refresh_explored_cells`'s bounds
    /// logic). Used by the 1v1 path to build the opponent controller's
    /// observation from its own fog-of-war history.
    fn explored_cells_for(&self, viewer: u32) -> HashSet<(i32, i32)> {
        let mut out: HashSet<(i32, i32)> = HashSet::new();
        let world = match &self.world {
            Some(w) => w,
            None => return out,
        };
        let shroud = match world.typed_shroud(viewer) {
            Some(s) => s,
            None => return out,
        };
        let (bx, by, bw, bh) = self.map_def.bounds;
        let (mw, mh) = self.map_def.map_size;
        let (x_lo, x_hi, y_lo, y_hi) = if bw > 0 && bh > 0 {
            (bx, bx + bw, by, by + bh)
        } else {
            (0, mw, 0, mh)
        };
        for y in y_lo..y_hi {
            for x in x_lo..x_hi {
                if shroud.is_explored(x, y) {
                    out.insert((x, y));
                }
            }
        }
        out
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
        // Victory semantics — scenario-aware:
        //   - Agent: alive iff has combat units OR MustBeDestroyed
        //     buildings (most strategy scenarios are units-only on the
        //     agent side, but covered either way).
        //   - Enemy: TWO regimes, decided once at reset() based on the
        //     enemy's initial roster (`enemy_started_with_buildings`):
        //       * had buildings (e.g. maginot fact/proc): only those
        //         count. Defenders + remaining infantry are irrelevant
        //         — destroying the base ends the game even if an e3
        //         is still posted at a wall.
        //       * unit-only enemy (e.g. rush-hour): combat-units check.
        //         Without this branch, my earlier maginot fix collapses
        //         rush-hour to "no buildings → enemy_alive=False from
        //         turn 0 → instant terminal".
        // A surrendered agent is defeated regardless of remaining force.
        let agent_alive = !world.is_surrendered(self.agent_player_id)
            && (has_combat_units(world, self.agent_player_id)
                || has_must_be_destroyed_buildings(world, self.agent_player_id));
        let enemy_alive = if !self.enemy_started_present {
            // No enemy in this scenario: enemy-elimination is not a
            // victory/terminal condition. Termination is driven solely
            // by max_ticks, the agent being wiped, or the scenario's
            // declarative win_condition (evaluated bench-side).
            true
        } else if self.enemy_started_with_buildings {
            has_must_be_destroyed_buildings(world, self.enemy_player_id)
        } else {
            has_combat_units(world, self.enemy_player_id)
        };
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

/// Result of `Env::step_1v1` — one decision step advanced with
/// commands from BOTH players, returning each side's own fog-of-war
/// observation. `done` is true once either base is eliminated (or the
/// deadline is hit); the harness reads which side still has buildings
/// to decide the winner.
#[derive(Debug, Clone)]
pub struct Step1v1Result {
    /// Observation from the agent player's perspective.
    pub agent_obs: Observation,
    /// Observation from the enemy player's perspective.
    pub enemy_obs: Observation,
    pub done: bool,
    pub warnings: Vec<String>,
}

/// Result of `Env::step_until_event` — same shape as `StepResult` plus
/// fields describing whether (and why) the advance returned early.
#[derive(Debug, Clone)]
pub struct StepUntilEventResult {
    pub obs: Observation,
    pub reward: f32,
    pub done: bool,
    pub warnings: Vec<String>,
    /// True if the advance returned early because an interrupt fired.
    pub interrupted: bool,
    /// Human-readable reason (e.g. `"enemy_unit_spotted: id 1016"`).
    /// `None` if `interrupted == false`.
    pub interrupt_reason: Option<String>,
    /// How many ticks the advance actually consumed (≤ requested
    /// `max_ticks`). On interrupt this is < max_ticks.
    pub ticks_advanced: u32,
}

// ---- Helpers (private) -------------------------------------------------------

fn parse_actor_id(s: &str) -> Option<u32> {
    s.parse::<u32>().ok()
}

/// Resolve a Python unit-id string to an agent-owned actor id, pushing
/// a warning (and returning None) on a parse failure or ownership
/// violation. Shared by every unit-targeted command in `build_orders`.
fn resolve_owned(
    id_str: &str,
    issuer_owned: &HashSet<u32>,
    warnings: &mut Vec<String>,
) -> Option<u32> {
    match parse_actor_id(id_str) {
        None => {
            warnings.push(format!("invalid unit_id {id_str:?}"));
            None
        }
        Some(aid) if issuer_owned.contains(&aid) => Some(aid),
        Some(aid) => {
            warnings.push(format!("unit {aid} not owned by issuing player"));
            None
        }
    }
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

fn has_buildings(world: &World, player_id: u32) -> bool {
    for aid in world.actor_ids_for_player(player_id) {
        if matches!(world.actor_kind(aid), Some(ActorKind::Building)) {
            return true;
        }
    }
    false
}

/// Like `has_buildings` but only counts buildings whose actor type has the
/// C# `MustBeDestroyed` trait. Defenses (gun/tsla/pbox/...) and scenery
/// (powr/barr) are EXCLUDED — destroying those is not a victory condition.
fn has_must_be_destroyed_buildings(world: &World, player_id: u32) -> bool {
    for aid in world.actor_ids_for_player(player_id) {
        if !matches!(world.actor_kind(aid), Some(ActorKind::Building)) {
            continue;
        }
        let actor_type = match world.actor_type_name(aid) {
            Some(t) => t,
            None => continue,
        };
        if let Some(stats) = world.rules.actor(actor_type) {
            if stats.must_be_destroyed {
                return true;
            }
        }
    }
    false
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

/// Phase 8 — look up the unit's typed `UnitInfo` and attach a
/// `Vehicle` + (optionally) `Turret` typed component to the world's
/// `typed_components` map.
///
/// We keep the attach logic conservative: only actors whose
/// `UnitInfo.locomotor` is wheeled / tracked / heavy-tracked (i.e.
/// classic vehicles) get a `Vehicle` component, and only those with a
/// `Turreted.TurnSpeed` field also get a `Turret`. Infantry attaches
/// nothing — the typed-component bundle is left empty.
fn attach_typed_components(
    world: &mut World,
    actor_id: u32,
    actor_type: &str,
    typed_rules: &data_rules::Rules,
) {
    use openra_sim::math::WAngle;
    use openra_sim::traits::{Locomotor, Turret, Vehicle};
    use openra_sim::world::ActorTypedComponents;

    // Lookup uses uppercase keys (matches the YAML actor name).
    let unit_info = match typed_rules.unit(&actor_type.to_uppercase()) {
        Some(u) => u,
        None => return,
    };
    if unit_info.locomotor.is_empty() {
        return; // non-mobile — nothing to attach
    }
    let locomotor = Locomotor::from_yaml(&unit_info.locomotor);
    if !locomotor.is_ground() {
        return; // we don't yet ship aircraft / naval typed components
    }
    // Foot infantry (`dog`, `e1`, `e3`, `medi`) gets no Vehicle
    // typed component — `Vehicle` is reserved for wheeled / tracked /
    // heavy-tracked actors. Phase 8 keeps the type-safe split.
    if matches!(locomotor, Locomotor::Foot) {
        return;
    }
    let has_turret = unit_info.turret_turn_speed.is_some();
    // Initial chassis facing matches `build_scenario_actor` (south =
    // 512). The turret starts pointing the same way.
    let initial_facing = WAngle::new(512);
    let vehicle = Vehicle::new(locomotor, has_turret, initial_facing);
    let turret = if has_turret {
        let turn_speed = unit_info.turret_turn_speed.unwrap_or(28).max(0);
        Some(Turret::with_tolerance(initial_facing, turn_speed, 4))
    } else {
        None
    };
    world.set_typed_components(
        actor_id,
        ActorTypedComponents { vehicle: Some(vehicle), turret },
    );
}

/// Build a freshly-spawned `Actor` from a `ScenarioActor`.
///
/// Phase 7 changes
/// ---------------
/// * Buildings (`stats.kind == ActorKind::Building` or `stats.is_building`)
///   now produce a `TraitState::Building { top_left }` + `Health` trait
///   list and `kind = ActorKind::Building`. The world's tick loop sees
///   them as static defenses and attaches auto-target via
///   `traits::classify_defense` (see `tick_actors`).
/// * Vehicles attach a `TraitState::BodyOrientation` + `Mobile` as before.
fn build_scenario_actor(id: u32, sa: &ScenarioActor, owner: u32, world: &World) -> Actor {
    let stats = world.rules.actor(&sa.actor_type);
    let is_building = stats.map(|s| s.is_building).unwrap_or(false);
    let kind = match stats {
        Some(s) => s.kind,
        None => kind_for_unit_type(&sa.actor_type),
    };
    let max_hp = stats.map(|s| s.hp).unwrap_or(50000);
    // A scenario may pre-place a damaged actor via `health: N` (an HP
    // percentage, 1-100). Scale the spawned `Health` trait accordingly;
    // `None` ⇒ spawn at full HP. The result is clamped to ≥1 so a
    // rounding-to-zero never spawns an already-dead actor.
    let hp = match sa.health {
        Some(pct) => {
            let scaled = (max_hp as i64 * pct.clamp(1, 100) as i64) / 100;
            (scaled.max(1) as i32).min(max_hp)
        }
        None => max_hp,
    };
    let cell = CPos::new(sa.position.0, sa.position.1);

    if is_building || matches!(kind, ActorKind::Building) {
        // Static structure: no Mobile trait, immobile center.
        let center = center_of_cell(sa.position.0, sa.position.1);
        return Actor {
            id,
            kind: ActorKind::Building,
            owner_id: Some(owner),
            location: Some(sa.position),
            traits: vec![
                TraitState::BodyOrientation { quantized_facings: 1 },
                TraitState::Building { top_left: cell },
                TraitState::Immobile { top_left: cell, center_position: center },
                TraitState::Health { hp },
            ],
            activity: None,
            actor_type: Some(sa.actor_type.clone()),
            kills: 0,
            rank: 0,
        };
    }

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

/// Resolve and load the vendored RA ruleset. Phase 7 requires the real
/// ruleset so building weapons (TurretGun, TeslaZap, M60mg, …) can
/// resolve damage / range / reload from `weapons.yaml`. Phase 8 also
/// returns the typed `data_rules::Rules` view so the env loader can
/// attach `Vehicle` / `Turret` typed components per actor.
///
/// Falls back to `GameRules::defaults()` (and an empty typed Rules)
/// when the vendor dir is absent (e.g. CI without submodules).
fn load_rules_with_fallback() -> (GameRules, data_rules::Rules) {
    // Try common vendor locations relative to the runtime cwd, the
    // env's manifest dir, and HOME. The first hit wins.
    let mut candidates: Vec<PathBuf> = Vec::new();
    // Explicit env var override takes precedence (production deployments).
    if let Ok(p) = std::env::var("OPENRA_VENDOR_DIR") {
        candidates.push(PathBuf::from(p));
    }
    // Resolve relative to the openra-train crate's compile-time manifest
    // dir — this works regardless of where the wheel ends up installed,
    // as long as the source tree is intact alongside the binary.
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    candidates.push(crate_dir.join("../vendor/OpenRA/mods/ra"));
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(PathBuf::from(&home).join("Projects/OpenRA-Rust/vendor/OpenRA/mods/ra"));
        // Production deployment: workspace-rooted source tree.
        candidates.push(PathBuf::from(&home).join("workspace/OpenRA-Rust/vendor/OpenRA/mods/ra"));
    }
    candidates.push(PathBuf::from("vendor/OpenRA/mods/ra"));
    candidates.push(PathBuf::from("../vendor/OpenRA/mods/ra"));
    candidates.push(PathBuf::from("../../vendor/OpenRA/mods/ra"));
    for c in &candidates {
        if c.exists()
            && let Ok(rs) = data_rules::load_ruleset(c)
        {
            let typed = data_rules::Rules::from_ruleset(&rs);
            return (GameRules::from_ruleset(&rs), typed);
        }
    }
    (
        GameRules::defaults(),
        data_rules::Rules {
            units: std::collections::BTreeMap::new(),
            weapons: std::collections::BTreeMap::new(),
            buildings: std::collections::BTreeMap::new(),
            buildables: std::collections::BTreeMap::new(),
        },
    )
}

fn kind_for_unit_type(t: &str) -> ActorKind {
    match t {
        // MCV is its own kind — the world.rs DeployTransform handler
        // gates on `actor.kind == ActorKind::Mcv`, so a scenario-YAML
        // mcv that fell through to ActorKind::Infantry would silently
        // refuse to deploy. (Auto-seeded MCVs via spawn_mcvs:true get
        // their kind set explicitly in world.rs:4572; this fallback
        // is the path scenario-declared `{type: mcv}` actors take.)
        "mcv" => ActorKind::Mcv,
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
            stance: None,
            health: None,
        }],
        spawn_mcvs: true,
        starting_cash: 5000,
        enemy_bot: None,
        scheduled_events: Vec::new(),
        reveal_map: false,
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
        interrupt_state: InterruptState::default(),
        enabled_signals: HashSet::new(),
        cooldown_ticks: DEFAULT_INTERRUPT_COOLDOWN_TICKS,
        enemy_started_with_buildings: false,
        enemy_started_present: false,
        typed_rules_cache: None,
        fired_scheduled_events: Vec::new(),
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
        #[pyo3(signature = (scenario_path, seed, ticks_per_step=None, max_ticks=None, enabled_signals=None, cooldown_ticks=None, spawn_point=None))]
        fn new(
            scenario_path: String,
            seed: u64,
            ticks_per_step: Option<u32>,
            max_ticks: Option<u32>,
            enabled_signals: Option<Vec<String>>,
            cooldown_ticks: Option<i32>,
            spawn_point: Option<i32>,
        ) -> PyResult<Self> {
            let mut env = Env::new_with_spawn_point(&scenario_path, seed, spawn_point)
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
            if let Some(n) = ticks_per_step {
                env = env.with_ticks_per_step(n);
            }
            if let Some(n) = max_ticks {
                env = env.with_max_ticks(n);
            }
            if let Some(s) = enabled_signals {
                // Validate against the registry — surface typos early.
                for name in &s {
                    if !INTERRUPT_SIGNAL_NAMES.contains(&name.as_str()) {
                        return Err(PyValueError::new_err(format!(
                            "Unknown interrupt signal: {:?}. Valid: {:?}",
                            name, INTERRUPT_SIGNAL_NAMES
                        )));
                    }
                }
                env = env.with_enabled_signals(s);
            }
            if let Some(n) = cooldown_ticks {
                env = env.with_cooldown_ticks(n);
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

        /// Advance one decision step with commands from BOTH players —
        /// the agent-vs-agent 1v1 entry point. Returns
        /// `(agent_obs, enemy_obs, done, info)`, each observation from
        /// that player's own fog-of-war view; `info["warnings"]`
        /// carries order-rejection warnings from either side.
        fn step_1v1<'py>(
            &mut self,
            py: Python<'py>,
            agent_commands: Vec<PyRef<PyCommand>>,
            enemy_commands: Vec<PyRef<PyCommand>>,
        ) -> PyResult<(
            Bound<'py, PyDict>,
            Bound<'py, PyDict>,
            bool,
            Bound<'py, PyDict>,
        )> {
            let a_cmds: Vec<Command> = agent_commands
                .into_iter()
                .map(|c| c.inner.clone())
                .collect();
            let e_cmds: Vec<Command> = enemy_commands
                .into_iter()
                .map(|c| c.inner.clone())
                .collect();
            let result = self.inner.step_1v1(&a_cmds, &e_cmds);

            let agent_obs = result.agent_obs.to_pydict(py)?;
            let enemy_obs = result.enemy_obs.to_pydict(py)?;
            let info = PyDict::new_bound(py);
            let warnings = PyList::empty_bound(py);
            for w in &result.warnings {
                warnings.append(w)?;
            }
            info.set_item("warnings", warnings)?;
            info.set_item("game_tick", result.agent_obs.game_tick)?;
            Ok((agent_obs, enemy_obs, result.done, info))
        }

        /// Advance up to `max_ticks`, returning early if an enabled
        /// interrupt signal fires. Returns
        /// `(obs, reward, done, info, interrupted, interrupt_reason, ticks_advanced)`.
        /// `info["warnings"]` carries the same per-step warnings as
        /// `step()`.
        ///
        /// `enabled_signals_override` lets a single call narrow the
        /// constructor-set `enabled_signals` to a subset (e.g. silence
        /// `engage_start` during pure-recon phases). Pass `None` to
        /// inherit the env-level set.
        #[pyo3(signature = (commands, max_ticks=None, check_every=5, enabled_signals_override=None))]
        fn step_until_event<'py>(
            &mut self,
            py: Python<'py>,
            commands: Vec<PyRef<PyCommand>>,
            max_ticks: Option<u32>,
            check_every: u32,
            enabled_signals_override: Option<Vec<String>>,
        ) -> PyResult<(
            Bound<'py, PyDict>,
            f32,
            bool,
            Bound<'py, PyDict>,
            bool,
            Option<String>,
            u32,
        )> {
            let cmds: Vec<Command> = commands.into_iter().map(|c| c.inner.clone()).collect();
            let max_ticks = max_ticks.unwrap_or_else(|| self.inner.ticks_per_step());

            let override_set: Option<HashSet<String>> = enabled_signals_override.map(|v| {
                v.into_iter().collect()
            });
            // Validate override names if provided.
            if let Some(set) = override_set.as_ref() {
                for name in set {
                    if !INTERRUPT_SIGNAL_NAMES.contains(&name.as_str()) {
                        return Err(PyValueError::new_err(format!(
                            "Unknown interrupt signal: {:?}. Valid: {:?}",
                            name, INTERRUPT_SIGNAL_NAMES
                        )));
                    }
                }
            }

            let result = self.inner.step_until_event(
                &cmds,
                max_ticks,
                check_every,
                override_set,
            );

            let obs = result.obs.to_pydict(py)?;
            let info = PyDict::new_bound(py);
            let warnings = PyList::empty_bound(py);
            for w in &result.warnings {
                warnings.append(w)?;
            }
            info.set_item("warnings", warnings)?;
            info.set_item("game_tick", result.obs.game_tick)?;
            info.set_item("ticks_advanced", result.ticks_advanced)?;
            Ok((
                obs,
                result.reward,
                result.done,
                info,
                result.interrupted,
                result.interrupt_reason,
                result.ticks_advanced,
            ))
        }

        fn render(&self) -> String {
            self.inner.render()
        }

        /// Observation from the enemy player's perspective at the
        /// current state (no step). Seeds the enemy controller's first
        /// observation in the 1v1 harness.
        fn enemy_observation<'py>(
            &self,
            py: Python<'py>,
        ) -> PyResult<Bound<'py, PyDict>> {
            self.inner.enemy_observation().to_pydict(py)
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

        /// Return parsed actor + weapon stats for the requested actor types.
        ///
        /// `types` — list of actor-type strings (e.g. ["jeep", "tsla"]).
        /// Pass an empty list (default) to return every type the engine
        /// knows about (useful for offline debugging).
        ///
        /// The returned dict maps `type -> stats` where each stats dict has:
        ///   hp, kind ("Building"/"Vehicle"/"Infantry"/"Mcv"/...),
        ///   speed, sight_range, footprint=[fw,fh], armor_type, is_building,
        ///   must_be_destroyed, weapons=[{name, damage, range_cells, range_wdist,
        ///                                 reload_delay, burst, dps, splash_radius_cells,
        ///                                 versus={armor_class -> pct}}]
        ///
        /// Only types parsed by the active ruleset are returned; unknown
        /// types are silently skipped (matches engine fallback behavior).
        #[pyo3(signature = (types=None))]
        fn unit_codex<'py>(
            &self,
            py: Python<'py>,
            types: Option<Vec<String>>,
        ) -> PyResult<Bound<'py, PyDict>> {
            let world = match self.inner.world() {
                Some(w) => w,
                None => {
                    // No live world (env not reset yet) — return empty dict.
                    return Ok(PyDict::new_bound(py));
                }
            };
            let rules = &world.rules;
            let out = PyDict::new_bound(py);
            let want: Vec<String> = match types {
                Some(t) if !t.is_empty() => t.into_iter().map(|s| s.to_lowercase()).collect(),
                _ => rules.actors.keys().cloned().collect(),
            };
            for t in &want {
                let stats = match rules.actor(t) {
                    Some(s) => s,
                    None => continue,
                };
                let entry = PyDict::new_bound(py);
                entry.set_item("hp", stats.hp)?;
                entry.set_item("kind", format!("{:?}", stats.kind))?;
                entry.set_item("speed", stats.speed)?;
                entry.set_item("sight_range", stats.sight_range)?;
                entry.set_item("footprint", (stats.footprint.0, stats.footprint.1))?;
                entry.set_item("armor_type", format!("{:?}", stats.armor_type))?;
                entry.set_item("is_building", stats.is_building)?;
                entry.set_item("must_be_destroyed", stats.must_be_destroyed)?;
                entry.set_item("cost", stats.cost)?;
                let weapons = PyList::empty_bound(py);
                for wname in &stats.weapons {
                    let w = match rules.weapon(wname) {
                        Some(w) => w,
                        None => continue,
                    };
                    let wentry = PyDict::new_bound(py);
                    wentry.set_item("name", wname)?;
                    wentry.set_item("damage", w.damage)?;
                    wentry.set_item("range_wdist", w.range)?;
                    wentry.set_item("range_cells", (w.range as f32) / 1024.0)?;
                    wentry.set_item("reload_delay", w.reload_delay)?;
                    wentry.set_item("burst", w.burst)?;
                    // DPS = damage * burst / reload_delay (ticks ≈ 25/sec). Convert
                    // to dmg/sec by dividing reload_delay by 25 (engine ticks/sec).
                    let dps = if w.reload_delay > 0 {
                        (w.damage as f32) * (w.burst as f32) * 25.0 / (w.reload_delay as f32)
                    } else {
                        0.0
                    };
                    wentry.set_item("dps", dps)?;
                    wentry.set_item("splash_radius_cells", (w.splash_radius as f32) / 1024.0)?;
                    wentry.set_item("projectile_speed", w.projectile_speed)?;
                    let versus = PyDict::new_bound(py);
                    for (armor, pct) in &w.versus {
                        versus.set_item(format!("{:?}", armor), pct)?;
                    }
                    wentry.set_item("versus", versus)?;
                    weapons.append(wentry)?;
                }
                entry.set_item("weapons", weapons)?;
                out.set_item(t, entry)?;
            }
            Ok(out)
        }
    }
}

#[cfg(feature = "python")]
pub use py::OpenRAEnv;
