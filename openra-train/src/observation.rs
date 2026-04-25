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
}

#[derive(Debug, Clone)]
pub struct EnemyPos {
    pub cell_x: i32,
    pub cell_y: i32,
    pub id: String,
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
}

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

            // unit_positions: {id: {cell_x, cell_y}}
            let unit_positions = PyDict::new_bound(py);
            for (id, pos) in &self.unit_positions {
                let entry = PyDict::new_bound(py);
                entry.set_item("cell_x", pos.cell_x)?;
                entry.set_item("cell_y", pos.cell_y)?;
                unit_positions.set_item(id, entry)?;
            }
            d.set_item("unit_positions", unit_positions)?;

            // unit_hp: {id: float}
            let unit_hp = PyDict::new_bound(py);
            for (id, hp) in &self.unit_hp {
                unit_hp.set_item(id, *hp)?;
            }
            d.set_item("unit_hp", unit_hp)?;

            // enemy_positions: [{cell_x, cell_y, id}]
            let enemy_positions = PyList::empty_bound(py);
            for ep in &self.enemy_positions {
                let entry = PyDict::new_bound(py);
                entry.set_item("cell_x", ep.cell_x)?;
                entry.set_item("cell_y", ep.cell_y)?;
                entry.set_item("id", &ep.id)?;
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
            Ok(d)
        }
    }
}
