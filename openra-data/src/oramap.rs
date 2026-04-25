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
///    (jitter is ignored — tests want a deterministic count). For agent
///    actors, only entries matching the chosen `spawn_point` (default 0)
///    are kept; enemy actors do not have spawn_points and are always kept.
pub fn load_rush_hour_map(path: &Path) -> Result<MapDef, MapLoadError> {
    load_rush_hour_map_with_spawn(path, 0)
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

    // Expand actors.
    let actors = expand_scenario_actors(&scenario.actors, spawn_point);

    Ok(MapDef {
        title: base.title,
        tileset: base.tileset,
        map_size: base.map_size,
        bounds: base.bounds,
        tiles: base.tiles,
        agent_faction: scenario.agent_faction,
        enemy_faction: scenario.enemy_faction,
        actors,
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
}

#[derive(Debug, Clone)]
struct RawScenarioActor {
    actor_type: String,
    owner: String,
    position: (i32, i32),
    count: i32,
    spawn_point: Option<i32>,
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
                    let (faction, ni) = read_block_faction(&lines, i + 1);
                    out.agent_faction = faction;
                    i = ni;
                    continue;
                }
                "enemy" => {
                    let (faction, ni) = read_block_faction(&lines, i + 1);
                    out.enemy_faction = faction;
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

/// Inside a `agent:` or `enemy:` top-level block, read indented `faction:` line.
/// Returns `(faction_string, next_line_index)`.
fn read_block_faction(lines: &[&str], start: usize) -> (String, usize) {
    let mut i = start;
    let mut faction = String::new();
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
        if let Some((k, v)) = split_key_value(trimmed)
            && k == "faction"
        {
            faction = v.to_string();
        }
        i += 1;
    }
    (faction, i)
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
                    Some(("stance", _)) => { /* sim doesn't use this yet */ }
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

/// Expand `count: N` into N copies and apply spawn_point filter for agent
/// actors. Enemy actors are kept regardless of spawn_point.
///
/// Phase 7: when *no* agent actor has a spawn_point set (i.e. the
/// scenario doesn't use the multi-spawn pattern at all — e.g. the
/// scout-* strategy scenarios), we keep every agent actor without
/// filtering. This preserves the rush-hour multi-spawn semantics while
/// allowing simpler scenarios to omit the `spawn_point:` field.
fn expand_scenario_actors(raw: &[RawScenarioActor], spawn_point: i32) -> Vec<ScenarioActor> {
    let any_agent_has_spawn = raw
        .iter()
        .any(|r| r.owner == "agent" && r.spawn_point.is_some());
    let mut out = Vec::new();
    for r in raw {
        // Filter agent actors by spawn_point selection ONLY if the
        // scenario actually uses spawn points. Enemy actors don't have
        // spawn_point set (None) and pass through unconditionally.
        if r.owner == "agent" && any_agent_has_spawn && r.spawn_point != Some(spawn_point) {
            continue;
        }
        let n = r.count.max(1);
        for _ in 0..n {
            out.push(ScenarioActor {
                actor_type: r.actor_type.clone(),
                owner: r.owner.clone(),
                position: r.position,
            });
        }
    }
    out
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
