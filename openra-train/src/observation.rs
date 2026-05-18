//! Observation snapshot — what the Python side sees after `reset()`/`step()`.
//!
//! The schema MUST match what `agent_rollout.py::_fork_start_snapshot`
//! consumes (see [`crate`] docs):
//!
//! ```text
//! {
//!   "unit_positions":   {actor_id_str: {"cell_x": int, "cell_y": int}},
//!   "unit_hp":          {actor_id_str: hp_pct_float},     # 0.0..1.0
//!   "enemy_positions":  [{"cell_x": int, "cell_y": int, "id": str}, ...],
//!   "enemy_hp":         {actor_id_str: hp_pct_float},     # 0.0..1.0
//!   "enemy_buildings_summary": [{"cell_x": int, "cell_y": int,
//!                                "type": str, "hp_pct": float, "id": str}, ...],
//!   "units_killed":     int,                              # cumulative
//!   "game_tick":        int,
//!   "explored_percent": float,                            # 0..100
//! }
//! ```
//!
//! Any drift here breaks the Python reward pipeline (`Φ` components,
//! discovery, exploration). The `to_pydict` method below is the single
//! source of truth.

// We deliberately avoid pulling `serde` into `openra-train` — the only
// consumer of these types is the PyO3 layer (which builds a PyDict
// directly) plus our integration tests (which compare native fields).

#[derive(Debug, Clone)]
pub struct UnitPos {
    pub cell_x: i32,
    pub cell_y: i32,
    /// Destination cell of the unit's current activity, if any
    /// (Move → final path cell; Attack → target's current cell).
    /// `None` means the unit is idle / turning / harvesting.
    pub target: Option<(i32, i32)>,
    /// Activity descriptor for the briefing: "idle", "moving",
    /// "attacking", "turning", "harvesting". Lets the Python side
    /// distinguish a Move from an Attack — both populate `target`,
    /// but they mean very different things to the agent.
    pub activity: String,
    /// If `activity == "attacking"`, the actor id of the target.
    /// `None` for any other activity. Surfaced so the briefing can
    /// render "attacking 1023" instead of the misleading
    /// "moving to (55,10)" (which is the target's cell, not a path).
    pub attacking_target_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EnemyPos {
    pub cell_x: i32,
    pub cell_y: i32,
    pub id: String,
    /// Actor type string from rules (e.g. "1tnk", "e1", "jeep"). Empty if
    /// unknown. Surfaced so the briefing can show "1tnk" instead of a
    /// generic "enemy" label.
    pub actor_type: String,
}

/// Phase-7 enemy-building entry surfaced through `enemy_buildings_summary`.
/// Buildings are visible to the agent only after their footprint (or
/// neighbouring cells) has entered the typed shroud.
#[derive(Debug, Clone)]
pub struct EnemyBuilding {
    pub cell_x: i32,
    pub cell_y: i32,
    pub id: String,
    pub kind: String,
    /// HP fraction in [0, 1].
    pub hp_pct: f32,
}

/// S9 — map dimensions (parity with C# RlMapInfo). Lets the bench use
/// true map size for region win-conditions / the minimap renderer
/// instead of synthesising bounds from observed cells.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MapInfo {
    pub width: i32,
    pub height: i32,
}

/// S9 — agent economy snapshot (parity with C# RlEconomy subset the
/// Rust sim can ground today: cash/power/harvesters; ore via resources).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EconomyObs {
    pub cash: i32,
    pub power_provided: i32,
    pub power_drained: i32,
    pub harvesters: i32,
    /// S1: stored resources and storage capacity (refineries/silos).
    pub resources: i32,
    pub resource_capacity: i32,
}

/// S9 — an agent-owned building (parity with C# RlBuildingInfo subset).
#[derive(Debug, Clone)]
pub struct OwnBuilding {
    pub id: String,
    pub building_type: String,
    pub cell_x: i32,
    pub cell_y: i32,
    pub hp_pct: f32,
}

/// S9 — a queued production item (parity with C# RlProductionInfo).
#[derive(Debug, Clone)]
pub struct ProductionObs {
    pub item: String,
    pub progress: f32,
    pub done: bool,
}

/// All fields the Python rollout pipeline reads off an observation.
///
/// Stored with native Rust types; the PyO3 boundary layer (in
/// `env::OpenRAEnv::step`) translates this into a `PyDict`. Keeping a
/// pure-Rust struct lets integration tests assert observation shape
/// without booting Python.
#[derive(Debug, Clone)]
pub struct Observation {
    /// Own units only. Key is the actor id rendered as a base-10 string
    /// (matches the Python convention of `str(actor_id)`).
    pub unit_positions: Vec<(String, UnitPos)>,
    /// Own units only. HP fraction in [0, 1].
    pub unit_hp: Vec<(String, f32)>,
    /// Visible enemies (fog-filtered through player_0's shroud).
    pub enemy_positions: Vec<EnemyPos>,
    /// Visible enemies. HP fraction in [0, 1].
    pub enemy_hp: Vec<(String, f32)>,
    /// Phase-7 visible enemy buildings, fog-filtered the same way as
    /// `enemy_positions`. Empty for scenarios with no enemy structures.
    pub enemy_buildings: Vec<EnemyBuilding>,
    /// Cumulative own-team kills.
    pub units_killed: i32,
    /// Current world tick.
    pub game_tick: i32,
    /// Percentage of map cells the agent has ever revealed (0..100).
    pub explored_percent: f32,
    /// All cells (x, y) the agent has ever revealed. Sticky — once a
    /// cell is in this set it stays. Accumulated tick-by-tick by the
    /// engine, so this captures cells that units transited *between*
    /// briefings (which a position-snapshot-based reveal mask would
    /// miss). Used by the minimap renderer to draw fog correctly.
    pub explored_cells: Vec<(i32, i32)>,
    /// S9 — agent economy (cash/power/harvesters).
    pub economy: EconomyObs,
    /// S9 — agent-owned buildings.
    pub own_buildings: Vec<OwnBuilding>,
    /// S9 — agent production queue items.
    pub production: Vec<ProductionObs>,
    /// S9 — true map dimensions.
    pub map_info: MapInfo,
    /// S9 — spatial tensor, flat row-major `[y][x][c]`, shape
    /// `spatial_shape = (h, w, c)` with c=6 channels:
    /// 0 passable, 1 fog (1 visible / 0.5 explored / 0 unknown),
    /// 2 own-unit density, 3 visible-enemy-unit density,
    /// 4 own building, 5 resource present. Enables grid/occupancy
    /// spatial reasoning (the ERQA-transfer axis).
    pub spatial: Vec<f32>,
    pub spatial_shape: (i32, i32, i32),
}

/// Channel count of the spatial tensor.
pub const SPATIAL_CHANNELS: i32 = 6;

impl Observation {
    /// Stable hash of the observation for determinism testing. Order is
    /// fixed by the natural sort of `unit_positions`/`enemy_positions`
    /// the env builder produces.
    pub fn deterministic_hash(&self) -> u64 {
        // Avoid the language-level `std::hash::Hash` trait — its
        // f32/f64 impls don't exist by default and we need to hash
        // floats deterministically. Roll a simple FNV-1a over a flat
        // byte representation.
        let mut h: u64 = 0xcbf29ce484222325;
        let mix = |h: &mut u64, b: u8| {
            *h ^= b as u64;
            *h = h.wrapping_mul(0x100000001b3);
        };

        let bytes_of_i32 = |v: i32, h: &mut u64| {
            for &b in &v.to_le_bytes() {
                mix(h, b);
            }
        };
        let bytes_of_f32 = |v: f32, h: &mut u64| {
            for &b in &v.to_le_bytes() {
                mix(h, b);
            }
        };
        let bytes_of_str = |s: &str, h: &mut u64| {
            for &b in s.as_bytes() {
                mix(h, b);
            }
            mix(h, 0);
        };

        for (id, pos) in &self.unit_positions {
            bytes_of_str(id, &mut h);
            bytes_of_i32(pos.cell_x, &mut h);
            bytes_of_i32(pos.cell_y, &mut h);
        }
        mix(&mut h, b'|');
        for (id, hp) in &self.unit_hp {
            bytes_of_str(id, &mut h);
            bytes_of_f32(*hp, &mut h);
        }
        mix(&mut h, b'|');
        for ep in &self.enemy_positions {
            bytes_of_str(&ep.id, &mut h);
            bytes_of_i32(ep.cell_x, &mut h);
            bytes_of_i32(ep.cell_y, &mut h);
        }
        mix(&mut h, b'|');
        for (id, hp) in &self.enemy_hp {
            bytes_of_str(id, &mut h);
            bytes_of_f32(*hp, &mut h);
        }
        mix(&mut h, b'|');
        for eb in &self.enemy_buildings {
            bytes_of_str(&eb.id, &mut h);
            bytes_of_str(&eb.kind, &mut h);
            bytes_of_i32(eb.cell_x, &mut h);
            bytes_of_i32(eb.cell_y, &mut h);
            bytes_of_f32(eb.hp_pct, &mut h);
        }
        mix(&mut h, b'|');
        bytes_of_i32(self.units_killed, &mut h);
        bytes_of_i32(self.game_tick, &mut h);
        bytes_of_f32(self.explored_percent, &mut h);
        mix(&mut h, b'|');
        bytes_of_i32(self.economy.cash, &mut h);
        bytes_of_i32(self.economy.power_provided, &mut h);
        bytes_of_i32(self.economy.power_drained, &mut h);
        bytes_of_i32(self.economy.harvesters, &mut h);
        bytes_of_i32(self.economy.resources, &mut h);
        bytes_of_i32(self.economy.resource_capacity, &mut h);
        for ob in &self.own_buildings {
            bytes_of_str(&ob.building_type, &mut h);
            bytes_of_i32(ob.cell_x, &mut h);
            bytes_of_i32(ob.cell_y, &mut h);
        }
        for p in &self.production {
            bytes_of_str(&p.item, &mut h);
            bytes_of_f32(p.progress, &mut h);
        }
        bytes_of_i32(self.map_info.width, &mut h);
        bytes_of_i32(self.map_info.height, &mut h);
        // Spatial tensor is a pure function of already-hashed state
        // (units / explored / terrain / resources); hashing shape +
        // length keeps the hash cheap while still detecting structural
        // divergence.
        let (sh, sw, sc) = self.spatial_shape;
        bytes_of_i32(sh, &mut h);
        bytes_of_i32(sw, &mut h);
        bytes_of_i32(sc, &mut h);
        bytes_of_i32(self.spatial.len() as i32, &mut h);
        h
    }
}

#[cfg(feature = "python")]
mod py {
    use super::*;
    use pyo3::prelude::*;
    use pyo3::types::{PyDict, PyList};

    impl Observation {
        /// Translate into a fresh PyDict that matches the schema in
        /// `agent_rollout.py::_fork_start_snapshot`.
        pub fn to_pydict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
            let d = PyDict::new_bound(py);

            // unit_positions: {id: {cell_x, cell_y, target?, activity, attacking_target_id?}}
            let unit_positions = PyDict::new_bound(py);
            for (id, pos) in &self.unit_positions {
                let entry = PyDict::new_bound(py);
                entry.set_item("cell_x", pos.cell_x)?;
                entry.set_item("cell_y", pos.cell_y)?;
                if let Some((tx, ty)) = pos.target {
                    let target = PyList::empty_bound(py);
                    target.append(tx)?;
                    target.append(ty)?;
                    entry.set_item("target", target)?;
                }
                entry.set_item("activity", &pos.activity)?;
                if let Some(tid) = pos.attacking_target_id.as_deref() {
                    entry.set_item("attacking_target_id", tid)?;
                }
                unit_positions.set_item(id, entry)?;
            }
            d.set_item("unit_positions", unit_positions)?;

            // unit_hp: {id: float}
            let unit_hp = PyDict::new_bound(py);
            for (id, hp) in &self.unit_hp {
                unit_hp.set_item(id, *hp)?;
            }
            d.set_item("unit_hp", unit_hp)?;

            // enemy_positions: [{cell_x, cell_y, id, actor_type}]
            // actor_type lets the briefing show "1tnk(1023)@(55,10)" instead
            // of a generic "enemy" label. Buildings have their own list
            // (`enemy_buildings_summary`) and are NOT mixed in here.
            let enemy_positions = PyList::empty_bound(py);
            for ep in &self.enemy_positions {
                let entry = PyDict::new_bound(py);
                entry.set_item("cell_x", ep.cell_x)?;
                entry.set_item("cell_y", ep.cell_y)?;
                entry.set_item("id", &ep.id)?;
                entry.set_item("actor_type", &ep.actor_type)?;
                enemy_positions.append(entry)?;
            }
            d.set_item("enemy_positions", enemy_positions)?;

            // enemy_hp: {id: float}
            let enemy_hp = PyDict::new_bound(py);
            for (id, hp) in &self.enemy_hp {
                enemy_hp.set_item(id, *hp)?;
            }
            d.set_item("enemy_hp", enemy_hp)?;

            // enemy_buildings_summary: [{cell_x, cell_y, id, type, hp_pct}]
            // Phase 7 — Python rollout pipeline reads this list to track
            // building discovery, `must_be_destroyed` win-condition state,
            // and per-building HP for combat/disruption rewards.
            let enemy_buildings = PyList::empty_bound(py);
            for eb in &self.enemy_buildings {
                let entry = PyDict::new_bound(py);
                entry.set_item("cell_x", eb.cell_x)?;
                entry.set_item("cell_y", eb.cell_y)?;
                entry.set_item("id", &eb.id)?;
                entry.set_item("type", &eb.kind)?;
                entry.set_item("hp_pct", eb.hp_pct)?;
                enemy_buildings.append(entry)?;
            }
            d.set_item("enemy_buildings_summary", enemy_buildings)?;

            d.set_item("units_killed", self.units_killed)?;
            d.set_item("game_tick", self.game_tick)?;
            d.set_item("explored_percent", self.explored_percent)?;

            // S9 economy: {cash, power_provided, power_drained, harvesters}
            let econ = PyDict::new_bound(py);
            econ.set_item("cash", self.economy.cash)?;
            econ.set_item("power_provided", self.economy.power_provided)?;
            econ.set_item("power_drained", self.economy.power_drained)?;
            econ.set_item("harvesters", self.economy.harvesters)?;
            econ.set_item("resources", self.economy.resources)?;
            econ.set_item("resource_capacity", self.economy.resource_capacity)?;
            d.set_item("economy", econ)?;

            // S9 own_buildings: [{id, type, cell_x, cell_y, hp_pct}]
            let own_b = PyList::empty_bound(py);
            for ob in &self.own_buildings {
                let e = PyDict::new_bound(py);
                e.set_item("id", &ob.id)?;
                e.set_item("type", &ob.building_type)?;
                e.set_item("cell_x", ob.cell_x)?;
                e.set_item("cell_y", ob.cell_y)?;
                e.set_item("hp_pct", ob.hp_pct)?;
                own_b.append(e)?;
            }
            d.set_item("own_buildings", own_b)?;

            // S9 production: [{item, progress, done}]
            let prod = PyList::empty_bound(py);
            for p in &self.production {
                let e = PyDict::new_bound(py);
                e.set_item("item", &p.item)?;
                e.set_item("progress", p.progress)?;
                e.set_item("done", p.done)?;
                prod.append(e)?;
            }
            d.set_item("production", prod)?;

            let mi = PyDict::new_bound(py);
            mi.set_item("width", self.map_info.width)?;
            mi.set_item("height", self.map_info.height)?;
            d.set_item("map_info", mi)?;

            // S9 spatial tensor: flat row-major [y][x][c] + shape.
            d.set_item("spatial", self.spatial.clone())?;
            let (sh, sw, sc) = self.spatial_shape;
            d.set_item("spatial_shape", (sh, sw, sc))?;

            // explored_cells: list[(x, y)] — accurate per-tick fog
            // accumulation. The renderer uses this instead of
            // approximating from per-briefing unit positions.
            let cells = PyList::empty_bound(py);
            for &(x, y) in &self.explored_cells {
                let pair = PyList::empty_bound(py);
                pair.append(x)?;
                pair.append(y)?;
                cells.append(pair)?;
            }
            d.set_item("explored_cells", cells)?;
            Ok(d)
        }
    }
}
