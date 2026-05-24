//! `.oramap` map file parser.
//!
//! An .oramap file is a ZIP archive containing:
//! - `map.yaml` — map metadata, player definitions, actor list
//! - `map.bin` — terrain tile data
//! - `map.png` — preview image (ignored)
//!
//! For the MVP we only parse map.yaml to get:
//! - Map dimensions and bounds
//! - Player definitions
//! - Initial actors (trees, mines, spawn points)
//!
//! For the rush-hour scenario, see [`load_rush_hour_map`] which combines a
//! base `.oramap` (for terrain + bounds) with a discovery scenario YAML
//! (for the per-faction actor list).

use crate::miniyaml;
use std::collections::HashSet;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// A player definition from the map
#[derive(Debug, Clone)]
pub struct PlayerDef {
    pub name: String,
    pub playable: bool,
    pub owns_world: bool,
    pub non_combatant: bool,
    pub faction: String,
    pub enemies: Vec<String>,
}

/// An actor placed on the map
#[derive(Debug, Clone)]
pub struct MapActor {
    pub id: String,
    pub actor_type: String,
    pub owner: String,
    pub location: (i32, i32),
}

/// A terrain tile reference (type + index within tileset).
#[derive(Debug, Clone, Copy, Default)]
pub struct TileReference {
    /// Terrain type ID from the tileset.
    pub type_id: u16,
    /// Variant index within the type.
    pub index: u8,
}

/// Parsed map data
#[derive(Debug, Clone)]
pub struct OraMap {
    pub title: String,
    pub tileset: String,
    pub map_size: (i32, i32),
    pub bounds: (i32, i32, i32, i32), // x, y, w, h
    pub players: Vec<PlayerDef>,
    pub actors: Vec<MapActor>,
    /// Terrain tile grid [row][col], dimensions = map_size.
    /// Empty if map.bin was not found or could not be parsed.
    pub tiles: Vec<Vec<TileReference>>,
}

/// Parse a simple MiniYaml value from a line like "Key: Value"
fn parse_kv(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    let colon = line.find(':')?;
    let key = line[..colon].trim();
    let value = line[colon + 1..].trim();
    Some((key, value))
}

/// Parse "X,Y" into (i32, i32)
fn parse_pair(s: &str) -> Option<(i32, i32)> {
    let mut parts = s.split(',');
    let x = parts.next()?.trim().parse().ok()?;
    let y = parts.next()?.trim().parse().ok()?;
    Some((x, y))
}

/// Parse "X,Y,W,H" into (i32, i32, i32, i32)
fn parse_quad(s: &str) -> Option<(i32, i32, i32, i32)> {
    let mut parts = s.split(',');
    let a = parts.next()?.trim().parse().ok()?;
    let b = parts.next()?.trim().parse().ok()?;
    let c = parts.next()?.trim().parse().ok()?;
    let d = parts.next()?.trim().parse().ok()?;
    Some((a, b, c, d))
}

/// Parse map.yaml content into an OraMap struct.
pub fn parse_map_yaml(yaml: &str) -> io::Result<OraMap> {
    let mut title = String::new();
    let mut tileset = String::new();
    let mut map_size = (0, 0);
    let mut bounds = (0, 0, 0, 0);
    let mut players = Vec::new();
    let mut actors = Vec::new();

    let lines: Vec<&str> = yaml.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        if let Some((key, value)) = parse_kv(trimmed) {
            match key {
                "Title" => title = value.to_string(),
                "Tileset" => tileset = value.to_string(),
                "MapSize" => {
                    if let Some(p) = parse_pair(value) {
                        map_size = p;
                    }
                }
                "Bounds" => {
                    if let Some(q) = parse_quad(value) {
                        bounds = q;
                    }
                }
                _ => {}
            }
        }

        // Parse Players section
        if trimmed == "Players:" {
            i += 1;
            while i < lines.len() {
                let pline = lines[i];
                // Check if we're still in Players section (indented with tab)
                if !pline.starts_with('\t') && !pline.starts_with("  ") {
                    break;
                }

                let ptrimmed = pline.trim();
                // PlayerReference@Name:
                if ptrimmed.starts_with("PlayerReference@") && ptrimmed.ends_with(':') {
                    let ref_name = &ptrimmed["PlayerReference@".len()..ptrimmed.len() - 1];
                    let mut player = PlayerDef {
                        name: ref_name.to_string(),
                        playable: false,
                        owns_world: false,
                        non_combatant: false,
                        faction: String::new(),
                        enemies: Vec::new(),
                    };

                    i += 1;
                    // Parse player properties (double-indented)
                    while i < lines.len() {
                        let ppline = lines[i];
                        let depth = ppline.len() - ppline.trim_start().len();
                        if depth < 2 && !ppline.trim().is_empty() {
                            break;
                        }
                        if ppline.trim().is_empty() {
                            i += 1;
                            continue;
                        }

                        if let Some((k, v)) = parse_kv(ppline.trim()) {
                            match k {
                                "Name" => player.name = v.to_string(),
                                "Playable" => player.playable = v == "True",
                                "OwnsWorld" => player.owns_world = v == "True",
                                "NonCombatant" => player.non_combatant = v == "True",
                                "Faction" => player.faction = v.to_lowercase(),
                                "Enemies" => {
                                    player.enemies = v.split(',').map(|s| s.trim().to_string()).collect();
                                }
                                _ => {}
                            }
                        }
                        i += 1;
                    }
                    players.push(player);
                    continue;
                }
                i += 1;
            }
            continue;
        }

        // Parse Actors section
        if trimmed == "Actors:" {
            i += 1;
            while i < lines.len() {
                let aline = lines[i];
                if !aline.starts_with('\t') && !aline.starts_with("  ") {
                    break;
                }

                let atrimmed = aline.trim();
                // Actor line: "ActorXXX: type_name"
                if atrimmed.contains(':') && !atrimmed.starts_with("Owner") && !atrimmed.starts_with("Location") {
                    if let Some((actor_key, actor_type)) = parse_kv(atrimmed) {
                        let mut actor = MapActor {
                            id: actor_key.to_string(),
                            actor_type: actor_type.to_string(),
                            owner: String::new(),
                            location: (0, 0),
                        };

                        i += 1;
                        // Parse actor properties
                        while i < lines.len() {
                            let apline = lines[i];
                            let depth = apline.len() - apline.trim_start().len();
                            if depth < 2 && !apline.trim().is_empty() {
                                break;
                            }
                            if apline.trim().is_empty() {
                                i += 1;
                                continue;
                            }

                            if let Some((k, v)) = parse_kv(apline.trim()) {
                                match k {
                                    "Owner" => actor.owner = v.to_string(),
                                    "Location" => {
                                        if let Some(p) = parse_pair(v) {
                                            actor.location = p;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            i += 1;
                        }
                        actors.push(actor);
                        continue;
                    }
                }
                i += 1;
            }
            continue;
        }

        i += 1;
    }

    Ok(OraMap {
        title,
        tileset,
        map_size,
        bounds,
        players,
        actors,
        tiles: Vec::new(),
    })
}

/// Parse terrain tiles from map.bin binary data.
///
/// Format v1: tiles start at offset 5, 3 bytes per cell (u16 type + u8 index), column-major
/// Format v2: header with offsets, tiles at TilesOffset, 3 bytes per cell, column-major
///
/// Reference: OpenRA.Game/Map/Map.cs BinaryDataHeader + PostInit()
fn parse_map_bin(data: &[u8], width: i32, height: i32) -> Vec<Vec<TileReference>> {
    let w = width as usize;
    let h = height as usize;
    let mut tiles = vec![vec![TileReference::default(); w]; h];

    if data.len() < 5 {
        return tiles;
    }

    let format = data[0];
    let tiles_offset = if format == 1 {
        5usize
    } else if format == 2 && data.len() >= 17 {
        u32::from_le_bytes([data[5], data[6], data[7], data[8]]) as usize
    } else {
        return tiles;
    };

    // Each tile is 3 bytes: u16 LE type_id + u8 index
    // Stored column-major: for x in 0..w { for y in 0..h { ... } }
    let bytes_per_tile = 3;
    let needed = tiles_offset + w * h * bytes_per_tile;
    if data.len() < needed {
        return tiles;
    }

    let mut offset = tiles_offset;
    for col in 0..w {
        for row in 0..h {
            let type_id = u16::from_le_bytes([data[offset], data[offset + 1]]);
            let index = data[offset + 2];
            tiles[row][col] = TileReference { type_id, index };
            offset += bytes_per_tile;
        }
    }

    tiles
}

// ---------------------------------------------------------------------------
// Rush-hour scenario loader.
//
// Combines a base `.oramap` (terrain + bounds) with a Python-style scenario
// YAML such as `OpenRA-RL-Training/scenarios/discovery/rush-hour.yaml`,
// expanding `count: N` and `spawn_point` selection into a flat actor list.
// ---------------------------------------------------------------------------

/// One actor placed on the map by the rush-hour scenario.
#[derive(Debug, Clone)]
pub struct ScenarioActor {
    /// Actor type, lowercase as written in the YAML (e.g. `"e1"`, `"dog"`).
    /// Matches `Rules::unit(name)` after uppercasing.
    pub actor_type: String,
    /// Either `"agent"` or `"enemy"` from the scenario's owner tag.
    pub owner: String,
    /// Cell coordinates `(x, y)`. The scenario YAML uses cell units (1 cell
    /// per integer step), the same convention as `MapActor::location`.
    pub position: (i32, i32),
    /// Optional initial engagement stance:
    /// 0=HoldFire, 1=ReturnFire, 2=Defend, 3=AttackAnything.
    /// `None` means "use the engine default" (AttackAnything).
    pub stance: Option<u8>,
    /// Optional initial health as a PERCENTAGE of the actor type's
    /// max HP (1-100). `None` ⇒ spawn at full HP. Used by scenarios
    /// that pre-place damaged buildings (repair-triage, disaster
    /// recovery). The spawned actor's `Health` trait is seeded with
    /// `max_hp * health / 100` (clamped to ≥1).
    pub health: Option<u8>,
}

impl ScenarioActor {
    /// Whether this actor is infantry-class (used by tests asserting the
    /// "13 enemy + 5 own infantry" rush-hour spec). Matches the C# OpenRA
    /// definition: any actor whose root chain is `^Infantry` (E1, E2, E3,
    /// E4, E6, E7, MEDI, MECH, SHOK, SPY, ENGI, THF, DOG, CIV*).
    pub fn is_infantry(&self) -> bool {
        let t = self.actor_type.to_ascii_lowercase();
        matches!(
            t.as_str(),
            "e1" | "e2"
                | "e3"
                | "e4"
                | "e6"
                | "e7"
                | "tanya"
                | "medi"
                | "mech"
                | "shok"
                | "spy"
                | "engi"
                | "thf"
                | "dog"
                | "c1"
                | "c2"
                | "c3"
                | "c4"
                | "c5"
                | "c6"
                | "c7"
                | "c8"
                | "c9"
                | "c10"
        )
    }
}

/// Combined map definition for the rush-hour scenario: terrain from the
/// base `.oramap`, actor placements from the scenario YAML.
#[derive(Debug, Clone)]
pub struct MapDef {
    pub title: String,
    pub tileset: String,
    pub map_size: (i32, i32),
    pub bounds: (i32, i32, i32, i32),
    pub tiles: Vec<Vec<TileReference>>,
    /// Player faction strings from the scenario YAML.
    pub agent_faction: String,
    pub enemy_faction: String,
    /// All actors placed by the scenario (own + enemy), flattened across
    /// `count:` expansion. The chosen `spawn_point` filter is already
    /// applied to player actors; enemy actors do not have spawn_point.
    pub actors: Vec<ScenarioActor>,
    /// If true, the engine auto-spawns one MCV per player at their
    /// assigned spawn-point. Scenarios that pre-place all units (e.g.,
    /// rush-hour) should set this to `false`. Defaults to `true` to
    /// preserve behaviour for production scenarios that rely on the
    /// MCV-deploy → Construction-Yard build chain.
    pub spawn_mcvs: bool,
    /// Per-scenario starting cash for every player (designed economy
    /// constraint). Defaults to 5000 (OpenRA skirmish default) when the
    /// scenario omits `starting_cash:`. Acts as the floor when neither
    /// `agent: {cash: N}` nor `enemy: {cash: M}` override it.
    pub starting_cash: i32,
    /// Per-player starting-cash override parsed from
    /// `agent: {cash: N}`. When `None`, the agent slot inherits
    /// `starting_cash`. Lets a scenario give the agent a different
    /// starting balance from the enemy (e.g. agent: 0, enemy: 2000 in
    /// the thief-steals-cash scenario).
    pub agent_starting_cash: Option<i32>,
    /// Per-player starting-cash override parsed from
    /// `enemy: {cash: M}`. When `None`, the enemy slot inherits
    /// `starting_cash`. See `agent_starting_cash`.
    pub enemy_starting_cash: Option<i32>,
    /// Scripted opponent behaviour for the enemy side, if the
    /// scenario set `enemy: {bot: ...}` (else None ⇒ stance-only).
    pub enemy_bot: Option<String>,
    /// Scripted mid-episode events (spawn waves, region destroys,
    /// deadline shorteners). Empty when the scenario omits a
    /// `scheduled_events:` block. The list preserves declaration order;
    /// downstream consumers (the env per-tick firing path) typically
    /// fire each event exactly once when `world_tick >= event.tick`.
    pub scheduled_events: Vec<ScheduledEvent>,
    /// If true, the agent player observes the entire map with no fog of
    /// war: every enemy actor is reported regardless of shroud, and
    /// `explored_cells` covers the whole playable rectangle. Defaults to
    /// `false` (normal fog). This is the no-fog half of the bench's
    /// perception ablation grid (vision/structured × fog/no-fog) — a
    /// perfect-information control cell, not a load-bearing scenario.
    pub reveal_map: bool,
    /// Ore patches declared directly in the scenario YAML via the
    /// top-level `ore_patches:` list. Each entry is materialised by the
    /// env layer into a disk of ore cells on the terrain map so
    /// harvesters can mine it. Scenarios that pre-place ore via the
    /// `mine` map prop still work unchanged (the env layer continues to
    /// handle that path); `ore_patches:` is the explicit, declarative
    /// alternative that makes the bench's economy scenarios
    /// load-bearing.
    pub ore_patches: Vec<OrePatchDef>,
    /// Water cells declared by the scenario YAML's `water_cells:` and/or
    /// `water_rect:` blocks. The engine marks each `(x, y)` as a water
    /// cell in the terrain map: ground actors (Infantry / Vehicle /
    /// Building) cannot enter; naval actors (Ship) can ONLY traverse
    /// water cells. Used by the naval MVP to declare water bands
    /// without touching the `map.bin` tile encoding.
    pub water_cells: Vec<(i32, i32)>,
    /// Termination flag: when `true` (default), the engine auto-`done`s
    /// the episode the moment the agent has no surviving combat units
    /// or MustBeDestroyed buildings. Set to `false` via
    /// `termination.agent_units_killed: false` in the scenario YAML to
    /// let the episode continue past an agent-side wipe (e.g. a
    /// suicide-charge pack scoring on enemy kills before the strike
    /// package dies). The win/fail predicates are still evaluated each
    /// turn; absent a predicate trigger the run ends at the tick
    /// deadline.
    pub terminate_on_agent_units_killed: bool,
    /// Termination flag: when `true` (default), the engine auto-`done`s
    /// the episode the moment the enemy has no surviving combat units
    /// or MustBeDestroyed buildings. Set to `false` via
    /// `termination.enemy_units_killed: false` in the scenario YAML to
    /// let the episode continue past an enemy-side wipe (e.g. a
    /// "deny-and-keep-playing" pack whose win clause has additional
    /// criteria beyond elimination). Win/fail predicates are still
    /// evaluated each turn.
    pub terminate_on_enemy_units_killed: bool,
    /// Scenario-declared hard tick deadline from
    /// `termination.max_ticks:`. `None` means the YAML did not declare
    /// one; the env layer falls back to `DEFAULT_MAX_TICKS`. When
    /// present the value is honoured EXACTLY (no clamp) — long-horizon
    /// packs (F11 vertical-strike, etc.) may declare any
    /// budget their capability requires. The env layer applies this in
    /// `Env::new_with_spawn_point`.
    pub max_ticks: Option<u32>,
}

/// A scenario-declared ore patch. Materialised by the env layer at
/// world-build time into a disk of harvestable ore centered at
/// `(x, y)` with roughly `amount` density units spread across cells.
/// `radius` controls the disk size (default 3, ≈28 cells).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrePatchDef {
    pub x: i32,
    pub y: i32,
    pub amount: i32,
    pub radius: i32,
}

/// One scripted mid-episode event. Parsed from a top-level
/// `scheduled_events:` block in the scenario YAML.
#[derive(Debug, Clone)]
pub struct ScheduledEvent {
    /// Absolute world tick at which to fire. Compared against
    /// `world.world_tick` (engine advances `NetFrameInterval == 3` per
    /// processed frame), so values are roughly 30 ticks per second.
    pub tick: u32,
    /// What to do when the event fires.
    pub kind: ScheduledEventKind,
}

/// Discriminated union of supported scheduled-event payloads. Adding a
/// new variant requires (a) parsing it in `read_scheduled_events`, (b)
/// handling it in the env's per-tick scheduled-event firing path
/// (`Env::fire_scheduled_events`), and (c) covering it in the parsing
/// test (`openra-data/tests/test_scheduled_events.rs`).
#[derive(Debug, Clone)]
pub enum ScheduledEventKind {
    /// Inject one or more new actors into the running world. Each
    /// `ScenarioActor` is expanded into a single actor at its declared
    /// `position`; `count: N` is pre-expanded at parse time (matches
    /// the initial-actors path in `expand_scenario_actors`).
    SpawnActors { actors: Vec<ScenarioActor> },
    /// Remove every actor that matches the supplied filter. Intended
    /// for mid-episode base-teardown / "the defenders retreat" scripts.
    DestroyActors { filter: ActorFilter },
    /// Shrink the episode's hard deadline. `new_max_ticks` is the new
    /// absolute cap; the env ignores any value higher than the current
    /// cap (deadline never *grows*).
    ShortenDeadline { new_max_ticks: u32 },
}

/// Filter for `DestroyActors` (and any future "find-and-act" variants).
/// All declared sub-filters AND together. `owner` matches the scenario
/// owner tag (`"agent"` or `"enemy"`). `region` is an inclusive
/// Euclidean radius around a cell.
#[derive(Debug, Clone, Default)]
pub struct ActorFilter {
    pub owner: Option<String>,
    pub region: Option<RegionFilter>,
}

/// Circular region filter — actors whose cell `(cx, cy)` satisfies
/// `(cx - x)^2 + (cy - y)^2 <= radius^2` match.
#[derive(Debug, Clone, Copy)]
pub struct RegionFilter {
    pub x: i32,
    pub y: i32,
    pub radius: i32,
}

impl MapDef {
    /// Actors owned by the agent (player).
    pub fn agent_actors(&self) -> impl Iterator<Item = &ScenarioActor> {
        self.actors.iter().filter(|a| a.owner == "agent")
    }
    /// Actors owned by the enemy (creeps).
    pub fn enemy_actors(&self) -> impl Iterator<Item = &ScenarioActor> {
        self.actors.iter().filter(|a| a.owner == "enemy")
    }
}

/// Errors loading the rush-hour map.
#[derive(Debug)]
pub enum MapLoadError {
    /// I/O failure reading either the scenario YAML or the base `.oramap`.
    Io(io::Error),
    /// The scenario YAML was missing a required field (e.g. `actors:`).
    BadScenario(String),
    /// The base map referenced by `base_map:` could not be located.
    MissingBaseMap(PathBuf),
}

impl std::fmt::Display for MapLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MapLoadError::Io(e) => write!(f, "i/o error: {e}"),
            MapLoadError::BadScenario(msg) => write!(f, "invalid scenario yaml: {msg}"),
            MapLoadError::MissingBaseMap(p) => write!(
                f,
                "base map not found at {} — set base_map: in the scenario yaml \
or place rush-hour-arena.oramap next to the scenario",
                p.display()
            ),
        }
    }
}

impl std::error::Error for MapLoadError {}

impl From<io::Error> for MapLoadError {
    fn from(e: io::Error) -> Self {
        MapLoadError::Io(e)
    }
}

/// Load the rush-hour scenario map.
///
/// `path` should point to the scenario YAML file (e.g.
/// `scenarios/discovery/rush-hour.yaml`). The function:
///
/// 1. Parses the scenario's flat fields (`agent.faction`, `enemy.faction`,
///    `base_map`).
/// 2. Resolves the base map: if the `path/base_map` reference exists, loads
///    that. Otherwise falls back to a sibling `rush-hour-arena.oramap`,
///    then to `~/Projects/openra-rl/maps/rush-hour-arena.oramap`,
///    then to `~/Projects/OpenRA-RL-Training/scenarios/maps/rush-hour-arena.oramap`.
///    On total failure returns [`MapLoadError::MissingBaseMap`].
/// 3. Expands the actor list: each entry's `count: N` is repeated N times
///    (jitter is ignored — tests want a deterministic count). The
///    `spawn_point` filter activates PER OWNER: if any agent actor
///    declares `spawn_point`, only matching agent actors pass; same for
///    enemy actors (Wave 9 — per-owner activation, see
///    [`expand_scenario_actors`]). Owners that don't use `spawn_point`
///    at all pass through unchanged (back-compat).
pub fn load_rush_hour_map(path: &Path) -> Result<MapDef, MapLoadError> {
    load_rush_hour_map_with_spawn(path, 0)
}

/// Distinct `spawn_point` values used by agent actors in the scenario,
/// sorted ascending. An empty result means the scenario does not use
/// the multi-spawn pattern (callers should treat that as a single
/// spawn_point=0). Used by the env to round-robin spawns across seeds
/// without forcing every caller to hard-code the spawn-point count.
pub fn distinct_agent_spawn_points(path: &Path) -> Result<Vec<i32>, MapLoadError> {
    let scenario_text = std::fs::read_to_string(path)?;
    let scenario = parse_scenario_yaml(&scenario_text)
        .map_err(|e| MapLoadError::BadScenario(e.to_string()))?;
    let mut sps: Vec<i32> = scenario
        .actors
        .iter()
        .filter(|a| a.owner == "agent")
        .filter_map(|a| a.spawn_point)
        .collect();
    sps.sort_unstable();
    sps.dedup();
    Ok(sps)
}

/// Distinct `spawn_point` values used by ENEMY actors in the scenario,
/// sorted ascending. Parallels [`distinct_agent_spawn_points`]: an
/// empty result means the scenario does not vary enemy composition by
/// spawn_point (every enemy actor passes through on every requested
/// spawn — pre-Wave-9 back-compat). When non-empty, the env can
/// round-robin the seed axis over enemy compositions even when the
/// agent base is fixed. The two owners activate INDEPENDENTLY in
/// [`expand_scenario_actors`].
pub fn distinct_enemy_spawn_points(path: &Path) -> Result<Vec<i32>, MapLoadError> {
    let scenario_text = std::fs::read_to_string(path)?;
    let scenario = parse_scenario_yaml(&scenario_text)
        .map_err(|e| MapLoadError::BadScenario(e.to_string()))?;
    let mut sps: Vec<i32> = scenario
        .actors
        .iter()
        .filter(|a| a.owner == "enemy")
        .filter_map(|a| a.spawn_point)
        .collect();
    sps.sort_unstable();
    sps.dedup();
    Ok(sps)
}

/// Variant of [`load_rush_hour_map`] that selects a specific spawn point
/// for the agent (0..=3 in the rush-hour scenario).
pub fn load_rush_hour_map_with_spawn(
    path: &Path,
    spawn_point: i32,
) -> Result<MapDef, MapLoadError> {
    let scenario_text = std::fs::read_to_string(path)?;
    let scenario = parse_scenario_yaml(&scenario_text)
        .map_err(|e| MapLoadError::BadScenario(e.to_string()))?;

    // Resolve the base_map path. Try, in order:
    //   1) {scenario_dir}/{base_map} (relative ref in scenario yaml)
    //   2) {scenario_dir}/rush-hour-arena.oramap
    //   3) ~/Projects/openra-rl/maps/rush-hour-arena.oramap
    //   4) ~/Projects/OpenRA-RL-Training/scenarios/maps/rush-hour-arena.oramap
    let scenario_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tried: Vec<PathBuf> = Vec::new();

    let mut base_path: Option<PathBuf> = None;
    if let Some(rel) = &scenario.base_map_ref {
        // 1. Try scenario_dir/base_map (legacy rush-hour)
        let p = scenario_dir.join(rel);
        if p.exists() {
            base_path = Some(p);
        } else {
            tried.push(p);
        }
        // 2. Phase-7 layout: scenarios/strategy/scout-*.yaml references a
        //    base_map that lives in `scenarios/maps/`. Try sibling
        //    `../maps/<base_map>`.
        if base_path.is_none() {
            let p = scenario_dir.join("..").join("maps").join(rel);
            if p.exists() {
                base_path = Some(p);
            } else {
                tried.push(p);
            }
        }
        // 3. Try a co-located maps/ subdir.
        if base_path.is_none() {
            let p = scenario_dir.join("maps").join(rel);
            if p.exists() {
                base_path = Some(p);
            } else {
                tried.push(p);
            }
        }
        // 4. Walk up to grandparent / scenarios/maps for very nested layouts.
        if base_path.is_none()
            && let Some(parent) = scenario_dir.parent()
        {
            let p = parent.join("maps").join(rel);
            if p.exists() {
                base_path = Some(p);
            } else {
                tried.push(p);
            }
        }
    }
    if base_path.is_none() {
        let p = scenario_dir.join("rush-hour-arena.oramap");
        if p.exists() {
            base_path = Some(p);
        } else {
            tried.push(p);
        }
    }
    if base_path.is_none()
        && let Ok(home) = std::env::var("HOME")
    {
        for candidate in [
            "Projects/openra-rl/maps/rush-hour-arena.oramap",
            "Projects/OpenRA-RL-Training/scenarios/maps/rush-hour-arena.oramap",
        ] {
            let p = PathBuf::from(&home).join(candidate);
            if p.exists() {
                base_path = Some(p);
                break;
            } else {
                tried.push(p);
            }
        }
    }

    let base_path = base_path
        .ok_or_else(|| MapLoadError::MissingBaseMap(tried.last().cloned().unwrap_or_default()))?;

    let base_bytes = std::fs::read(&base_path)?;
    let base = parse(&base_bytes)?;

    // Expand actors. Thread the base-map's playable rectangle so
    // `count: N` spirals reject out-of-bounds candidates (the engine
    // panics on out-of-bounds actor placement).
    let actors = expand_scenario_actors(&scenario.actors, spawn_point, Some(base.bounds));

    Ok(MapDef {
        title: base.title,
        tileset: base.tileset,
        map_size: base.map_size,
        bounds: base.bounds,
        tiles: base.tiles,
        agent_faction: scenario.agent_faction,
        enemy_faction: scenario.enemy_faction,
        actors,
        spawn_mcvs: scenario.spawn_mcvs.unwrap_or(true),
        starting_cash: scenario.starting_cash.unwrap_or(5000),
        agent_starting_cash: scenario.agent_starting_cash,
        enemy_starting_cash: scenario.enemy_starting_cash,
        enemy_bot: scenario.enemy_bot,
        scheduled_events: scenario.scheduled_events,
        reveal_map: scenario.reveal_map.unwrap_or(false),
        ore_patches: scenario.ore_patches,
        water_cells: scenario.water_cells,
        terminate_on_agent_units_killed: scenario
            .terminate_on_agent_units_killed
            .unwrap_or(true),
        terminate_on_enemy_units_killed: scenario
            .terminate_on_enemy_units_killed
            .unwrap_or(true),
        max_ticks: scenario.max_ticks,
    })
}

/// Internal: minimal in-memory representation of the discovery-style
/// scenario YAML. We don't try to handle every PyYAML feature — only what
/// the rush-hour scenarios use.
#[derive(Debug, Default)]
struct ScenarioYaml {
    base_map_ref: Option<String>,
    agent_faction: String,
    enemy_faction: String,
    actors: Vec<RawScenarioActor>,
    /// Top-level `spawn_mcvs:` flag. None ⇒ default (true on the
    /// engine side) for back-compat. Set `spawn_mcvs: false` in the
    /// scenario YAML to suppress the auto-MCV-per-player.
    spawn_mcvs: Option<bool>,
    /// Top-level `starting_cash:` — designed economy constraint. None ⇒
    /// engine default (5000, the OpenRA skirmish default). Contributors
    /// set this to gate economy/production difficulty.
    starting_cash: Option<i32>,
    /// Optional scripted opponent behaviour for the enemy side
    /// (`enemy: {bot: hunt|rusher|patrol|turtle}`).
    enemy_bot: Option<String>,
    /// Optional per-player starting-cash override parsed from
    /// `agent: {cash: N}` inside the agent block. When None, the agent
    /// slot inherits the scenario-level `starting_cash`.
    agent_starting_cash: Option<i32>,
    /// Optional per-player starting-cash override parsed from
    /// `enemy: {cash: M}` inside the enemy block. See
    /// `agent_starting_cash`.
    enemy_starting_cash: Option<i32>,
    /// Parsed `scheduled_events:` block (empty when omitted).
    scheduled_events: Vec<ScheduledEvent>,
    /// Top-level `reveal_map:` flag. None ⇒ default (false ⇒ normal fog
    /// of war). Set `reveal_map: true` to disable fog for the agent
    /// player — the no-fog cells of the perception ablation grid.
    reveal_map: Option<bool>,
    /// Parsed `ore_patches:` block (empty when omitted).
    ore_patches: Vec<OrePatchDef>,
    /// Water cells declared by `water_cells:` (a flat list of `[x, y]`
    /// pairs) and/or `water_rect:` (an `[x, y, w, h]` rectangle that
    /// is expanded into a flat cell list at parse time). MVP overlay
    /// path: lets a naval scenario declare water without touching
    /// `map.bin`. Empty when neither block is present.
    water_cells: Vec<(i32, i32)>,
    /// Top-level `termination.agent_units_killed:` flag. None ⇒
    /// default `true` (engine auto-`done`s on agent wipe). Set to
    /// `false` to keep the run alive past an agent-side wipe so the
    /// scenario's declarative win/fail predicates can still fire.
    terminate_on_agent_units_killed: Option<bool>,
    /// Top-level `termination.enemy_units_killed:` flag. None ⇒
    /// default `true` (engine auto-`done`s on enemy wipe). Set to
    /// `false` to keep the run alive past an enemy-side wipe.
    terminate_on_enemy_units_killed: Option<bool>,
    /// Top-level `termination.max_ticks:` — scenario-declared hard
    /// tick deadline. None ⇒ the env layer falls back to
    /// `DEFAULT_MAX_TICKS`. When set, the value is honoured exactly
    /// (no clamp): long-horizon packs may declare any budget.
    max_ticks: Option<u32>,
}

#[derive(Debug, Clone)]
struct RawScenarioActor {
    actor_type: String,
    owner: String,
    position: (i32, i32),
    count: i32,
    spawn_point: Option<i32>,
    stance: Option<u8>,
    /// Optional initial health percentage (1-100). See `ScenarioActor::health`.
    health: Option<u8>,
}

/// Parse the discovery-style scenario YAML.
///
/// This is *not* the OpenRA MiniYaml dialect — it's PyYAML output, with
/// list-of-dicts under `actors:`. We hand-roll a tiny parser for the
/// subset we need (string scalars, int scalars, 2-element int lists, and
/// list-of-dict).
///
/// Note: PyYAML emits list items at the *same* indent level as the
/// containing key (e.g. `actors:` at col 0, then `- type: foo` also at col
/// 0). The parser detects list items by their `- ` prefix rather than by
/// indent depth, and consumes them until a non-list, non-blank line at
/// indent 0 appears.
fn parse_scenario_yaml(text: &str) -> io::Result<ScenarioYaml> {
    let mut out = ScenarioYaml::default();

    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let trimmed = strip_yaml_comment(raw).trim_end();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        let indent = leading_spaces(raw);
        let trim_lead = trimmed.trim_start();

        // List items at top level belong to whatever key opened the most
        // recent list — but for our schema only `actors:` opens a list at
        // top-level, and we already consume the entire actors block when
        // we see the `actors:` line. Stray top-level list items here are
        // leftover `tags:` items, which we just skip.
        if trim_lead.starts_with("- ") && indent == 0 {
            i += 1;
            continue;
        }

        if indent == 0
            && let Some((k, v)) = split_key_value(trimmed)
        {
            match k {
                "base_map" => {
                    out.base_map_ref = Some(v.to_string());
                    i += 1;
                    continue;
                }
                "agent" => {
                    let (faction, _bot, cash, ni) = read_block_faction(&lines, i + 1);
                    out.agent_faction = faction;
                    out.agent_starting_cash = cash;
                    i = ni;
                    continue;
                }
                "enemy" => {
                    let (faction, bot, cash, ni) = read_block_faction(&lines, i + 1);
                    out.enemy_faction = faction;
                    out.enemy_bot = bot;
                    out.enemy_starting_cash = cash;
                    i = ni;
                    continue;
                }
                "actors" => {
                    // The `actors:` line itself; the list items begin on
                    // the next line at indent 0 (PyYAML compact form) or
                    // at indent > 0 (block form, e.g. scout-maginot uses
                    // 2-space indented list items). Detect the actual
                    // indent of the first `- ` item.
                    let detected_indent = detect_list_indent(&lines, i + 1).unwrap_or(0);
                    let (actors, ni) = read_actors_list(&lines, i + 1, detected_indent);
                    out.actors = actors;
                    i = ni;
                    continue;
                }
                "starting_cash" => {
                    out.starting_cash = v
                        .trim()
                        .trim_matches(|c: char| c == '"' || c == '\'')
                        .parse()
                        .ok();
                    i += 1;
                    continue;
                }
                "scheduled_events" => {
                    let detected_indent = detect_list_indent(&lines, i + 1).unwrap_or(0);
                    let (events, ni) =
                        read_scheduled_events(&lines, i + 1, detected_indent);
                    out.scheduled_events = events;
                    i = ni;
                    continue;
                }
                "ore_patches" => {
                    let detected_indent = detect_list_indent(&lines, i + 1).unwrap_or(0);
                    let (patches, ni) =
                        read_ore_patches(&lines, i + 1, detected_indent);
                    out.ore_patches = patches;
                    i = ni;
                    continue;
                }
                "spawn_mcvs" => {
                    let trimmed_v = v.trim().trim_matches(|c: char| c == '"' || c == '\'');
                    out.spawn_mcvs = match trimmed_v {
                        "true" | "True" | "yes" | "1" => Some(true),
                        "false" | "False" | "no" | "0" => Some(false),
                        _ => None,
                    };
                    i += 1;
                    continue;
                }
                "reveal_map" => {
                    let trimmed_v = v.trim().trim_matches(|c: char| c == '"' || c == '\'');
                    out.reveal_map = match trimmed_v {
                        "true" | "True" | "yes" | "1" => Some(true),
                        "false" | "False" | "no" | "0" => Some(false),
                        _ => None,
                    };
                    i += 1;
                    continue;
                }
                "water_rect" => {
                    // Two accepted shapes:
                    //   inline: `water_rect: [x, y, w, h]`
                    //   block:  `water_rect:\n  - x\n  - y\n  - w\n  - h`
                    // PyYAML's `safe_dump` emits the block form by
                    // default, so the bench's `_scenario_to_tmp_yaml`
                    // produces the block form and the engine must
                    // accept both.
                    let mut rect: Vec<i32> = if v.trim().is_empty() {
                        // Block form: 4 `- N` items at indent > 0.
                        let detected_indent =
                            detect_list_indent(&lines, i + 1).unwrap_or(0);
                        let (vals, ni) =
                            read_int_scalar_list(&lines, i + 1, detected_indent);
                        i = ni;
                        vals
                    } else {
                        // Inline `[x, y, w, h]`.
                        let r = parse_inline_int_list(v).unwrap_or_default();
                        i += 1;
                        r
                    };
                    if rect.len() == 4 {
                        let h = rect.pop().unwrap();
                        let w = rect.pop().unwrap();
                        let y = rect.pop().unwrap();
                        let x = rect.pop().unwrap();
                        for dy in 0..h {
                            for dx in 0..w {
                                out.water_cells.push((x + dx, y + dy));
                            }
                        }
                    }
                    continue;
                }
                "termination" => {
                    // Block form (PyYAML safe_dump default):
                    //   termination:
                    //     max_ticks: 5400
                    //     agent_units_killed: false
                    //     enemy_units_killed: false
                    // Inline form (`termination: {max_ticks: 6000}`) is
                    // ALSO accepted — PyYAML emits it for short blocks.
                    // We honour the two auto-`done` gating flags and
                    // `max_ticks` (the scenario's hard tick deadline,
                    // honoured exactly — no clamp). `max_time` is
                    // still parsed-but-ignored.
                    let inline_v = v.trim();
                    if !inline_v.is_empty()
                        && inline_v.starts_with('{')
                        && inline_v.ends_with('}')
                    {
                        // Inline flow-form: parse `{k: v, k: v}`.
                        let inner = &inline_v[1..inline_v.len() - 1];
                        for kv in inner.split(',') {
                            if let Some((k, vv)) = split_key_value(kv.trim()) {
                                let vv = vv
                                    .trim()
                                    .trim_matches(|c: char| c == '"' || c == '\'');
                                match k {
                                    "agent_units_killed" => {
                                        out.terminate_on_agent_units_killed =
                                            parse_bool_str(vv);
                                    }
                                    "enemy_units_killed" => {
                                        out.terminate_on_enemy_units_killed =
                                            parse_bool_str(vv);
                                    }
                                    "max_ticks" => {
                                        if let Ok(n) = vv.parse::<u32>() {
                                            out.max_ticks = Some(n);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        i += 1;
                        continue;
                    }
                    // Block form — indented child keys until the next
                    // unindented line.
                    let mut j = i + 1;
                    while j < lines.len() {
                        let raw_j = lines[j];
                        let tj = strip_yaml_comment(raw_j).trim_end();
                        if tj.is_empty() {
                            j += 1;
                            continue;
                        }
                        if leading_spaces(raw_j) == 0 {
                            break;
                        }
                        if let Some((k, vv)) = split_key_value(tj) {
                            let vv = vv
                                .trim()
                                .trim_matches(|c: char| c == '"' || c == '\'');
                            match k {
                                "agent_units_killed" => {
                                    out.terminate_on_agent_units_killed =
                                        parse_bool_str(vv);
                                }
                                "enemy_units_killed" => {
                                    out.terminate_on_enemy_units_killed =
                                        parse_bool_str(vv);
                                }
                                "max_ticks" => {
                                    if let Ok(n) = vv.parse::<u32>() {
                                        out.max_ticks = Some(n);
                                    }
                                }
                                _ => {}
                            }
                        }
                        j += 1;
                    }
                    i = j;
                    continue;
                }
                "water_cells" => {
                    // Flat list of `[x, y]` pairs (PyYAML block form):
                    //   water_cells:
                    //     - [10, 5]
                    //     - [10, 6]
                    // OR inline list-of-lists:
                    //   water_cells: [[10, 5], [10, 6]]
                    if !v.trim().is_empty() {
                        // Inline form (`[[x, y], [x, y]]`).
                        for pair in parse_inline_pair_list(v) {
                            out.water_cells.push(pair);
                        }
                        i += 1;
                        continue;
                    }
                    // Block form — list items at indent > 0.
                    let detected_indent = detect_list_indent(&lines, i + 1).unwrap_or(0);
                    let (cells, ni) =
                        read_water_cell_list(&lines, i + 1, detected_indent);
                    out.water_cells.extend(cells);
                    i = ni;
                    continue;
                }
                _ => {}
            }
        }
        i += 1;
    }

    if out.actors.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "scenario yaml has no actors: block (or it is empty)",
        ));
    }
    Ok(out)
}

/// Read a `key: value` line. Returns the trimmed key and value, or `None`.
fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    let colon = line.find(':')?;
    let key = line[..colon].trim();
    let value = line[colon + 1..].trim();
    Some((key, value))
}

/// Strip everything after the first `#` not preceded by `\`.
fn strip_yaml_comment(s: &str) -> &str {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'#' && (i == 0 || bytes[i - 1] != b'\\') {
            return &s[..i];
        }
    }
    s
}

fn leading_spaces(line: &str) -> usize {
    line.bytes().take_while(|&b| b == b' ').count()
}

/// Parse YAML's permissive booleans into `Option<bool>`. Mirrors the
/// inline `match` used by `spawn_mcvs:` / `reveal_map:`.
fn parse_bool_str(v: &str) -> Option<bool> {
    match v.trim() {
        "true" | "True" | "yes" | "1" => Some(true),
        "false" | "False" | "no" | "0" => Some(false),
        _ => None,
    }
}

/// Inside a `agent:` or `enemy:` top-level block, read indented `faction:` line.
/// Returns `(faction_string, next_line_index)`.
fn read_block_faction(
    lines: &[&str],
    start: usize,
) -> (String, Option<String>, Option<i32>, usize) {
    let mut i = start;
    let mut faction = String::new();
    let mut bot: Option<String> = None;
    let mut cash: Option<i32> = None;
    while i < lines.len() {
        let raw = lines[i];
        let trimmed = strip_yaml_comment(raw).trim_end();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        if leading_spaces(raw) == 0 {
            break;
        }
        if let Some((k, v)) = split_key_value(trimmed) {
            match k {
                "faction" => faction = v.to_string(),
                // `bot_type` survives the bench's training-schema
                // serialization; `bot` is an ergonomic alias.
                "bot" | "bot_type" => {
                    let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
                    if !v.is_empty() {
                        bot = Some(v);
                    }
                }
                // Per-player starting-cash override (`agent: {cash: N}`
                // / `enemy: {cash: M}`). Bench scenarios use this to
                // seed a thief's target enemy with cash while keeping
                // the agent's own balance separate.
                "cash" => {
                    if let Ok(n) = v
                        .trim()
                        .trim_matches(|c: char| c == '"' || c == '\'')
                        .parse::<i32>()
                    {
                        cash = Some(n);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    (faction, bot, cash, i)
}

/// Parse the `actors:` list. Each list item starts with `- type: NAME` at
/// `list_indent`. Within an item, lines indented further are properties
/// (`owner`, `position`, `count`, `spawn_point`). `position:` is a
/// 2-element list like `position:\n  - 5\n  - 6`. `randomize:` blocks are
/// skipped. The block ends at the first non-empty line at indent
/// `<list_indent` that is *not* a `- ` continuation, or when a top-level
/// scalar key appears.
fn read_actors_list(
    lines: &[&str],
    start: usize,
    list_indent: usize,
) -> (Vec<RawScenarioActor>, usize) {
    let mut actors = Vec::new();
    let mut i = start;

    while i < lines.len() {
        let raw = lines[i];
        let trimmed = strip_yaml_comment(raw).trim_end();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        let indent = leading_spaces(raw);
        let trim_lead = trimmed.trim_start();

        if indent < list_indent {
            // Indent decreased below the list — block is over.
            break;
        }

        if indent == list_indent && !trim_lead.starts_with("- ") {
            // A non-list line at the list's indent ends the block. This is
            // how we leave `actors:` and reach the next top-level key
            // (e.g. `reward:`).
            break;
        }

        // List items begin with `- type: X` at the actors-block indent.
        if let Some(rest) = trim_lead.strip_prefix("- ") {
            // Start a new actor. The first key on this line is `type:`.
            let mut actor = RawScenarioActor {
                actor_type: String::new(),
                owner: String::new(),
                position: (0, 0),
                count: 1,
                spawn_point: None,
                stance: None,
                health: None,
            };
            if let Some((k, v)) = split_key_value(rest)
                && k == "type"
            {
                // Strip surrounding quotes if present (PyYAML may emit
                // them for actor types like `"2tnk"`).
                actor.actor_type = strip_quotes(v).to_string();
            }
            i += 1;
            // Properties of this list item are lines indented STRICTLY
            // more than `list_indent` (PyYAML uses indent + 2 for the
            // item body when the `- ` prefix is at column `list_indent`).
            while i < lines.len() {
                let sub = lines[i];
                let sub_trim = strip_yaml_comment(sub).trim_end();
                if sub_trim.is_empty() {
                    i += 1;
                    continue;
                }
                let sub_indent = leading_spaces(sub);
                let sub_lead = sub_trim.trim_start();

                if sub_indent <= list_indent {
                    // Sibling list item (`- type:`) or block end.
                    break;
                }
                let sub_kv = split_key_value(sub_lead);
                match sub_kv {
                    Some(("owner", v)) => actor.owner = v.to_string(),
                    Some(("count", v)) => {
                        actor.count = v.parse().unwrap_or(1);
                    }
                    Some(("spawn_point", v)) => {
                        actor.spawn_point = v.parse().ok();
                    }
                    Some(("stance", v)) => {
                        actor.stance = v.trim().parse::<u8>().ok().map(|s| s.min(3));
                    }
                    Some(("health", v)) => {
                        // HP percentage, 1-100. Clamp into range; values
                        // outside (or unparseable) are treated as "full".
                        actor.health = v
                            .trim()
                            .parse::<i32>()
                            .ok()
                            .map(|h| h.clamp(1, 100) as u8);
                    }
                    Some(("position", "")) => {
                        // 2-element list on the next two lines, at indent
                        // sub_indent (sibling lines that start with `- N`).
                        let (xy, ni) = read_xy_list(lines, i + 1, sub_indent);
                        actor.position = xy;
                        i = ni;
                        continue;
                    }
                    // Phase-7: inline flow form `position: [18, 25]`.
                    Some(("position", v)) if v.starts_with('[') => {
                        if let Some(xy) = parse_inline_xy(v) {
                            actor.position = xy;
                        }
                    }
                    Some(("randomize", "")) => {
                        // Skip the entire block (we want deterministic
                        // counts; sim adds jitter via its own rng).
                        let block_indent = sub_indent;
                        i += 1;
                        while i < lines.len() {
                            let inner = lines[i];
                            let inner_trim = strip_yaml_comment(inner).trim_end();
                            if !inner_trim.is_empty()
                                && leading_spaces(inner) <= block_indent
                            {
                                break;
                            }
                            i += 1;
                        }
                        continue;
                    }
                    _ => {}
                }
                i += 1;
            }
            actors.push(actor);
            continue;
        }
        // Unrecognized content — skip.
        i += 1;
    }
    (actors, i)
}

/// Strip surrounding ASCII double or single quotes from a YAML scalar.
fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2 {
        let bytes = s.as_bytes();
        let first = bytes[0];
        let last = bytes[s.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &s[1..s.len() - 1];
        }
    }
    s
}

/// Find the indent of the first `- ` item in `lines[start..]`.
/// Used to support both PyYAML compact form (list at column 0) and
/// block form (list at indent > 0).
fn detect_list_indent(lines: &[&str], start: usize) -> Option<usize> {
    for i in start..lines.len() {
        let raw = lines[i];
        let trimmed = strip_yaml_comment(raw).trim_end();
        if trimmed.is_empty() {
            continue;
        }
        let trim_lead = trimmed.trim_start();
        if trim_lead.starts_with("- ") {
            return Some(leading_spaces(raw));
        }
        // Stop scanning at the next non-list, non-blank line — the
        // actors block hasn't started yet, so we shouldn't be here.
        return None;
    }
    None
}

/// Parse `[X, Y]` flow-list, e.g. `[18, 25]`. Returns `None` on malformed
/// input. Used by Phase-7 strategy scenarios which inline positions.
fn parse_inline_xy(s: &str) -> Option<(i32, i32)> {
    let trim = s.trim();
    let inner = trim.strip_prefix('[')?.strip_suffix(']')?;
    let mut parts = inner.split(',');
    let x: i32 = parts.next()?.trim().parse().ok()?;
    let y: i32 = parts.next()?.trim().parse().ok()?;
    Some((x, y))
}

/// Parse `[A, B, C, ...]` flow-list of ints. Empty `Vec` on malformed.
/// Used by naval-MVP `water_rect: [x, y, w, h]` parsing.
fn parse_inline_int_list(s: &str) -> Option<Vec<i32>> {
    let trim = s.trim();
    let inner = trim.strip_prefix('[')?.strip_suffix(']')?;
    let mut out = Vec::new();
    for tok in inner.split(',') {
        out.push(tok.trim().parse::<i32>().ok()?);
    }
    Some(out)
}

/// Parse `[[X, Y], [X, Y], ...]` flow-list of pairs. Returns an empty
/// `Vec` on malformed input. Used by naval-MVP inline `water_cells:`.
fn parse_inline_pair_list(s: &str) -> Vec<(i32, i32)> {
    let trim = s.trim();
    let inner = match trim.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
        Some(x) => x,
        None => return Vec::new(),
    };
    // Walk char-by-char, splitting on `]` then trimming the `, [`.
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut buf = String::new();
    for ch in inner.chars() {
        match ch {
            '[' => {
                depth += 1;
                if depth == 1 {
                    buf.clear();
                    continue;
                }
                buf.push(ch);
            }
            ']' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(pair) = parse_inline_xy(&format!("[{buf}]")) {
                        out.push(pair);
                    }
                    buf.clear();
                    continue;
                }
                buf.push(ch);
            }
            _ => {
                if depth >= 1 {
                    buf.push(ch);
                }
            }
        }
    }
    out
}

/// Read a flat list of `- N` integer entries (PyYAML block form for
/// a top-level scalar list). Stops at the first non-list, non-blank
/// line at indent `<= expected_indent`. Used by naval-MVP block-form
/// `water_rect:`.
fn read_int_scalar_list(
    lines: &[&str],
    start: usize,
    expected_indent: usize,
) -> (Vec<i32>, usize) {
    let mut out = Vec::new();
    let mut i = start;
    while i < lines.len() {
        let raw = lines[i];
        let trimmed = strip_yaml_comment(raw).trim_end();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        let indent = leading_spaces(raw);
        if indent < expected_indent {
            break;
        }
        let lead = trimmed.trim_start();
        if let Some(rest) = lead.strip_prefix("- ") {
            if let Ok(v) = rest.trim().parse::<i32>() {
                out.push(v);
                i += 1;
                continue;
            }
            break;
        }
        break;
    }
    (out, i)
}

/// Read a flat list of `- [X, Y]` water-cell entries (PyYAML block form).
/// Stops at the first non-list, non-blank line at indent `<= expected_indent`.
///
/// Two shapes accepted:
///   - `- [X, Y]`                   (inline-pair items)
///   - `- - X\n  - Y`               (PyYAML block-block form: each
///                                   item is itself a 2-element list)
fn read_water_cell_list(
    lines: &[&str],
    start: usize,
    expected_indent: usize,
) -> (Vec<(i32, i32)>, usize) {
    let mut out = Vec::new();
    let mut i = start;
    while i < lines.len() {
        let raw = lines[i];
        let trimmed = strip_yaml_comment(raw).trim_end();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        let indent = leading_spaces(raw);
        if indent < expected_indent {
            break;
        }
        let lead = trimmed.trim_start();
        if let Some(rest) = lead.strip_prefix("- ") {
            // Inline pair `[x, y]` after `- `.
            if let Some(pair) = parse_inline_xy(rest.trim()) {
                out.push(pair);
                i += 1;
                continue;
            }
            // PyYAML block-block form: `- - X` (the `- ` is followed
            // by another `- ` opening a nested 2-element scalar list).
            // The first scalar lives on the same line; the second lives
            // on the next sibling line at `indent` + 2 (or whatever
            // sub-indent PyYAML chose). Reuse `read_xy_list` on the
            // tail.
            if let Some(inner_first) = rest.strip_prefix("- ") {
                if let Ok(x) = inner_first.trim().parse::<i32>() {
                    // Parse the second scalar at the same nested-list
                    // indent as `- `, which sits at `indent + 2`.
                    let nested_indent = indent + 2;
                    let mut vals = vec![x];
                    i += 1;
                    while i < lines.len() && vals.len() < 2 {
                        let sub = lines[i];
                        let st = strip_yaml_comment(sub).trim_end();
                        if st.is_empty() {
                            i += 1;
                            continue;
                        }
                        let sind = leading_spaces(sub);
                        if sind < nested_indent {
                            break;
                        }
                        let sl = st.trim_start();
                        if let Some(r) = sl.strip_prefix("- ") {
                            if let Ok(v) = r.trim().parse::<i32>() {
                                vals.push(v);
                                i += 1;
                                continue;
                            }
                        }
                        break;
                    }
                    if vals.len() == 2 {
                        out.push((vals[0], vals[1]));
                    }
                    continue;
                }
            }
            i += 1;
            continue;
        }
        break;
    }
    (out, i)
}

/// Read a 2-element scalar list. Items must be at indent
/// `>= expected_indent` and begin with `- `. Returns the `(x, y)` pair and
/// the next-line index.
fn read_xy_list(lines: &[&str], start: usize, expected_indent: usize) -> ((i32, i32), usize) {
    let mut vals: Vec<i32> = Vec::new();
    let mut i = start;
    while i < lines.len() && vals.len() < 2 {
        let raw = lines[i];
        let trimmed = strip_yaml_comment(raw).trim_end();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        let indent = leading_spaces(raw);
        if indent < expected_indent {
            break;
        }
        let trimmed_lead = trimmed.trim_start();
        if let Some(rest) = trimmed_lead.strip_prefix("- ")
            && let Ok(v) = rest.trim().parse::<i32>()
        {
            vals.push(v);
            i += 1;
            continue;
        }
        break;
    }
    let xy = (
        *vals.first().unwrap_or(&0),
        *vals.get(1).unwrap_or(&0),
    );
    (xy, i)
}

/// Expand `count: N` into N copies and apply the spawn_point filter
/// PER OWNER. The agent and enemy sides activate the filter
/// independently:
///
/// - Phase 7 (agent): when no agent actor declares `spawn_point`, the
///   agent filter is inactive and every agent actor passes through.
///   When ANY agent actor declares `spawn_point`, ONLY agent actors
///   whose `spawn_point` matches the requested one are kept — agent
///   actors without `spawn_point` are filtered out (the established
///   "duplicate base/garrison across both spawn groups" idiom).
/// - Wave 9 (enemy): the same logic applies to enemy actors. When no
///   enemy actor declares `spawn_point`, every enemy actor passes
///   through (pre-Wave-9 back-compat). When ANY enemy actor declares
///   `spawn_point`, ONLY enemy actors whose `spawn_point` matches the
///   requested one are kept; "persistent every-seed" enemy actors
///   (e.g. a far-corner `fact` marker that prevents engine
///   auto-`done`) must be duplicated across every spawn group, just
///   like the agent side.
///
/// The two owners are independent: a scenario can fix the agent base
/// across all seeds (no agent declares `spawn_point` → agent filter
/// off) while rotating the enemy archetype (enemies declare
/// `spawn_point` → enemy filter on). This is the contract the
/// `adv-rps-counter-pick` pack relies on.
/// Nearest cell to `(ax, ay)` not already in `used`, searched in
/// outward Chebyshev rings (deterministic: row-major within each ring).
/// Used to spread a `count:` group so its units do not spawn stacked.
///
/// When `bounds` is `Some((bx, by, bw, bh))` candidate cells outside
/// the rectangle `[bx, bx+bw) × [by, by+bh)` are rejected — this
/// guards against `count: N` near a map edge silently placing actors
/// off-map (the engine panics on out-of-bounds actor placement).
///
/// Returns `None` if the spiral exhausts `r = 64` without finding a
/// valid cell — the caller is responsible for the fallback. The
/// historical behaviour (return the anchor itself on exhaustion) is
/// preserved at the [`push_count_spread`] call site, together with a
/// `eprintln!` warning so the underlying issue surfaces.
fn next_free_spiral(
    ax: i32,
    ay: i32,
    used: &HashSet<(i32, i32)>,
    bounds: Option<(i32, i32, i32, i32)>,
) -> Option<(i32, i32)> {
    let mut r = 1i32;
    while r <= 64 {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs().max(dy.abs()) != r {
                    continue;
                }
                let p = (ax + dx, ay + dy);
                if used.contains(&p) {
                    continue;
                }
                if let Some((bx, by, bw, bh)) = bounds {
                    if p.0 < bx || p.0 >= bx + bw || p.1 < by || p.1 >= by + bh {
                        continue;
                    }
                }
                return Some(p);
            }
        }
        r += 1;
    }
    None
}

/// Expand one raw actor's `count: N` into N `ScenarioActor`s. Copy 0
/// keeps the declared `position` exactly — so single-count actors and
/// the first unit of any group are unchanged — and copies 1..N are
/// placed on the nearest free cells in outward rings around the anchor.
/// `used` accumulates every placed cell so units of a group (and across
/// groups) never spawn stacked on one cell. A tile-based RTS gives each
/// ground unit its own cell; `count: N` previously copied `position`
/// verbatim, piling all N units on the anchor.
///
/// When `bounds` is `Some(map_rect)` the spiral search rejects
/// out-of-bounds cells (the engine panics on out-of-bounds actor
/// placement). If the spiral exhausts without finding any valid cell
/// we fall back to the anchor position and emit a warning — preserving
/// the historical behaviour but surfacing the underlying issue rather
/// than silently corrupting placement.
fn push_count_spread(
    out: &mut Vec<ScenarioActor>,
    used: &mut HashSet<(i32, i32)>,
    r: &RawScenarioActor,
    bounds: Option<(i32, i32, i32, i32)>,
) {
    let n = r.count.max(1);
    let (ax, ay) = r.position;
    for k in 0..n {
        let pos = if k == 0 {
            (ax, ay)
        } else {
            match next_free_spiral(ax, ay, used, bounds) {
                Some(p) => p,
                None => {
                    eprintln!(
                        "warning: scenario actor type={} owner={} count={} \
                         exhausted spiral search around anchor ({},{}) \
                         within bounds {:?} — falling back to anchor cell \
                         (copy {} of {}). Consider lowering count: or \
                         moving the anchor further from the map edge.",
                        r.actor_type, r.owner, n, ax, ay, bounds, k, n
                    );
                    (ax, ay)
                }
            }
        };
        used.insert(pos);
        out.push(ScenarioActor {
            actor_type: r.actor_type.clone(),
            owner: r.owner.clone(),
            position: pos,
            stance: r.stance,
            health: r.health,
        });
    }
}

fn expand_scenario_actors(
    raw: &[RawScenarioActor],
    spawn_point: i32,
    bounds: Option<(i32, i32, i32, i32)>,
) -> Vec<ScenarioActor> {
    let any_agent_has_spawn = raw
        .iter()
        .any(|r| r.owner == "agent" && r.spawn_point.is_some());
    let any_enemy_has_spawn = raw
        .iter()
        .any(|r| r.owner == "enemy" && r.spawn_point.is_some());
    let mut out = Vec::new();
    let mut used: HashSet<(i32, i32)> = HashSet::new();
    for r in raw {
        // Per-owner spawn_point filter. An owner's filter activates
        // when at least one actor of that owner declared
        // `spawn_point`. Then every actor of that owner must declare
        // a matching `spawn_point` to pass — the "persistent
        // every-seed" idiom requires duplicating the actor across
        // every spawn group at identical coords.
        if r.owner == "agent" && any_agent_has_spawn && r.spawn_point != Some(spawn_point) {
            continue;
        }
        if r.owner == "enemy" && any_enemy_has_spawn && r.spawn_point != Some(spawn_point) {
            continue;
        }
        // `count: N` expands to N actors on N distinct cells — units
        // of a group no longer spawn stacked on the anchor.
        push_count_spread(&mut out, &mut used, r, bounds);
    }
    out
}

/// Parse the `scheduled_events:` list. Each list item is a dict with
/// `tick:` + `type:` + a type-dependent payload. Unknown `type:` values
/// are tolerated (skipped) so adding a new event kind in YAML doesn't
/// break older engine builds. Returns the parsed events (in declaration
/// order) and the index of the first line past the block.
fn read_scheduled_events(
    lines: &[&str],
    start: usize,
    list_indent: usize,
) -> (Vec<ScheduledEvent>, usize) {
    let mut events = Vec::new();
    let mut i = start;

    while i < lines.len() {
        let raw = lines[i];
        let trimmed = strip_yaml_comment(raw).trim_end();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        let indent = leading_spaces(raw);
        let trim_lead = trimmed.trim_start();

        if indent < list_indent {
            break;
        }
        if indent == list_indent && !trim_lead.starts_with("- ") {
            break;
        }

        if let Some(rest) = trim_lead.strip_prefix("- ") {
            // Start a new event. The first key may be `tick:` or `type:`.
            let mut ev_tick: Option<u32> = None;
            let mut ev_type: Option<String> = None;
            let mut spawn_actors: Vec<ScenarioActor> = Vec::new();
            let mut filter: ActorFilter = ActorFilter::default();
            let mut new_max_ticks: Option<u32> = None;

            if let Some((k, v)) = split_key_value(rest) {
                match k {
                    "tick" => ev_tick = v.parse().ok(),
                    "type" => ev_type = Some(strip_quotes(v).to_string()),
                    _ => {}
                }
            }
            i += 1;

            // Body lines indented strictly more than `list_indent`.
            while i < lines.len() {
                let sub = lines[i];
                let sub_trim = strip_yaml_comment(sub).trim_end();
                if sub_trim.is_empty() {
                    i += 1;
                    continue;
                }
                let sub_indent = leading_spaces(sub);
                if sub_indent <= list_indent {
                    break;
                }
                let sub_lead = sub_trim.trim_start();
                match split_key_value(sub_lead) {
                    Some(("tick", v)) => {
                        ev_tick = v.parse().ok();
                        i += 1;
                    }
                    Some(("type", v)) => {
                        ev_type = Some(strip_quotes(v).to_string());
                        i += 1;
                    }
                    Some(("new_max_ticks", v)) => {
                        new_max_ticks = v.parse().ok();
                        i += 1;
                    }
                    Some(("actors", "")) => {
                        let nested_indent = detect_list_indent(lines, i + 1)
                            .unwrap_or(sub_indent + 2);
                        let (raw_actors, ni) =
                            read_actors_list(lines, i + 1, nested_indent);
                        // Expand `count:` exactly like the initial-spawn
                        // path — N actors on N distinct cells (no
                        // stacking). `spawn_point` isn't meaningful for a
                        // scheduled-event injection (the seed-axis filter
                        // is consumed once at episode start), so we drop
                        // it here.
                        // Scheduled-event expansion happens at YAML
                        // parse time, before the base map is loaded, so
                        // bounds aren't available here. Pass `None` —
                        // the spiral falls back to its legacy
                        // unbounded behaviour. Authors are responsible
                        // for keeping scheduled-event anchors safely
                        // inside the map; the same eprintln warning
                        // surfaces if the spiral exhausts.
                        let mut used: HashSet<(i32, i32)> = HashSet::new();
                        for r in &raw_actors {
                            push_count_spread(
                                &mut spawn_actors, &mut used, r, None,
                            );
                        }
                        i = ni;
                    }
                    Some(("filter", "")) => {
                        let (parsed, ni) =
                            read_actor_filter(lines, i + 1, sub_indent);
                        filter = parsed;
                        i = ni;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }

            // Assemble the event. Skip unknown / malformed entries.
            if let (Some(tick), Some(t)) = (ev_tick, ev_type.as_deref()) {
                let kind = match t {
                    "spawn_actors" => Some(ScheduledEventKind::SpawnActors {
                        actors: spawn_actors,
                    }),
                    "destroy_actors" => {
                        Some(ScheduledEventKind::DestroyActors { filter })
                    }
                    "shorten_deadline" => new_max_ticks.map(|n| {
                        ScheduledEventKind::ShortenDeadline { new_max_ticks: n }
                    }),
                    _ => None, // unknown event kind → tolerant skip
                };
                if let Some(k) = kind {
                    events.push(ScheduledEvent { tick, kind: k });
                }
            }
            continue;
        }
        i += 1;
    }
    (events, i)
}

/// Parse the `ore_patches:` list. Each list item is a dict with
/// `x:`, `y:`, `amount:` (required) and an optional `radius:`. Mirrors
/// the shape of `scheduled_events:` / `spawn_mcvs:` (top-level YAML
/// keys consumed by the env layer at world-build time). Returns the
/// parsed patches (in declaration order) and the index of the first
/// line past the block. Unknown sub-keys are tolerated.
fn read_ore_patches(
    lines: &[&str],
    start: usize,
    list_indent: usize,
) -> (Vec<OrePatchDef>, usize) {
    let mut patches = Vec::new();
    let mut i = start;

    while i < lines.len() {
        let raw = lines[i];
        let trimmed = strip_yaml_comment(raw).trim_end();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        let indent = leading_spaces(raw);
        let trim_lead = trimmed.trim_start();

        if indent < list_indent {
            break;
        }
        if indent == list_indent && !trim_lead.starts_with("- ") {
            break;
        }

        if let Some(rest) = trim_lead.strip_prefix("- ") {
            // Start a new patch. Anchor defaults; required `x`, `y`,
            // `amount` are validated at the end.
            let mut p_x: Option<i32> = None;
            let mut p_y: Option<i32> = None;
            let mut p_amount: Option<i32> = None;
            let mut p_radius: i32 = 3;

            // First key on the `- ` line itself (PyYAML often writes
            // `- x: 50`).
            if let Some((k, v)) = split_key_value(rest) {
                match k {
                    "x" => p_x = v.parse().ok(),
                    "y" => p_y = v.parse().ok(),
                    "amount" => p_amount = v.parse().ok(),
                    "radius" => {
                        if let Ok(r) = v.parse::<i32>() {
                            p_radius = r.max(0);
                        }
                    }
                    _ => {}
                }
            }
            i += 1;

            // Body lines indented strictly more than `list_indent`.
            while i < lines.len() {
                let sub = lines[i];
                let sub_trim = strip_yaml_comment(sub).trim_end();
                if sub_trim.is_empty() {
                    i += 1;
                    continue;
                }
                let sub_indent = leading_spaces(sub);
                if sub_indent <= list_indent {
                    break;
                }
                let sub_lead = sub_trim.trim_start();
                match split_key_value(sub_lead) {
                    Some(("x", v)) => p_x = v.parse().ok(),
                    Some(("y", v)) => p_y = v.parse().ok(),
                    Some(("amount", v)) => p_amount = v.parse().ok(),
                    Some(("radius", v)) => {
                        if let Ok(r) = v.parse::<i32>() {
                            p_radius = r.max(0);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }

            if let (Some(x), Some(y), Some(amount)) = (p_x, p_y, p_amount) {
                patches.push(OrePatchDef { x, y, amount, radius: p_radius });
            }
            continue;
        }
        i += 1;
    }
    (patches, i)
}

/// Parse an `ActorFilter` block. Lines belong to the filter while their
/// indent is strictly greater than `outer_indent` (the indent of the
/// `filter:` key line itself). Returns the filter and the index of the
/// first line past it.
fn read_actor_filter(
    lines: &[&str],
    start: usize,
    outer_indent: usize,
) -> (ActorFilter, usize) {
    let mut out = ActorFilter::default();
    let mut i = start;
    while i < lines.len() {
        let raw = lines[i];
        let trimmed = strip_yaml_comment(raw).trim_end();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        let indent = leading_spaces(raw);
        if indent <= outer_indent {
            break;
        }
        let lead = trimmed.trim_start();
        match split_key_value(lead) {
            Some(("owner", v)) => {
                out.owner = Some(strip_quotes(v).to_string());
                i += 1;
            }
            Some(("region", "")) => {
                let (region, ni) = read_region_filter(lines, i + 1, indent);
                out.region = region;
                i = ni;
            }
            _ => {
                i += 1;
            }
        }
    }
    (out, i)
}

/// Parse a `region:` sub-block (`x:`, `y:`, `radius:` keys). Returns
/// `None` when the block is empty or any required field is missing.
fn read_region_filter(
    lines: &[&str],
    start: usize,
    outer_indent: usize,
) -> (Option<RegionFilter>, usize) {
    let mut x: Option<i32> = None;
    let mut y: Option<i32> = None;
    let mut radius: Option<i32> = None;
    let mut i = start;
    while i < lines.len() {
        let raw = lines[i];
        let trimmed = strip_yaml_comment(raw).trim_end();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        let indent = leading_spaces(raw);
        if indent <= outer_indent {
            break;
        }
        let lead = trimmed.trim_start();
        match split_key_value(lead) {
            Some(("x", v)) => x = v.parse().ok(),
            Some(("y", v)) => y = v.parse().ok(),
            Some(("radius", v)) => radius = v.parse().ok(),
            _ => {}
        }
        i += 1;
    }
    let region = match (x, y, radius) {
        (Some(x), Some(y), Some(radius)) => Some(RegionFilter { x, y, radius }),
        _ => None,
    };
    (region, i)
}

// Suppress an unused-import warning when miniyaml isn't referenced from
// scenario parsing (we kept the import for future use).
#[allow(unused_imports)]
use miniyaml as _miniyaml;

/// Parse an .oramap file (ZIP archive) from bytes.
pub fn parse(data: &[u8]) -> io::Result<OraMap> {
    let cursor = io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let mut yaml_content = String::new();
    {
        let mut yaml_file = archive
            .by_name("map.yaml")
            .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;
        yaml_file.read_to_string(&mut yaml_content)?;
    }

    let mut map = parse_map_yaml(&yaml_content)?;

    // Try to parse map.bin for terrain data
    if let Ok(mut bin_file) = archive.by_name("map.bin") {
        let mut bin_data = Vec::new();
        if bin_file.read_to_end(&mut bin_data).is_ok() {
            map.tiles = parse_map_bin(&bin_data, map.map_size.0, map.map_size.1);
        }
    }

    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_yaml() {
        let yaml = r#"
Title: Test Map
Tileset: TEMPERAT
MapSize: 64,64
Bounds: 2,2,60,60

Players:
	PlayerReference@Neutral:
		Name: Neutral
		OwnsWorld: True
		NonCombatant: True
		Faction: allies
	PlayerReference@Multi0:
		Name: Multi0
		Playable: True
		Faction: Random

Actors:
	Actor0: mpspawn
		Owner: Neutral
		Location: 10,10
	Actor1: mine
		Owner: Neutral
		Location: 20,20
"#;
        let map = parse_map_yaml(yaml).unwrap();
        assert_eq!(map.title, "Test Map");
        assert_eq!(map.tileset, "TEMPERAT");
        assert_eq!(map.map_size, (64, 64));
        assert_eq!(map.bounds, (2, 2, 60, 60));
        assert_eq!(map.players.len(), 2);
        assert_eq!(map.players[0].name, "Neutral");
        assert!(map.players[0].owns_world);
        assert!(map.players[1].playable);
        assert_eq!(map.actors.len(), 2);
        assert_eq!(map.actors[0].actor_type, "mpspawn");
        assert_eq!(map.actors[0].location, (10, 10));
        assert_eq!(map.actors[1].actor_type, "mine");
        assert_eq!(map.actors[1].location, (20, 20));
    }
}
