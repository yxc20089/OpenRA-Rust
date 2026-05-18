//! Typed command DSL for `OpenRAEnv::step`.
//!
//! Each variant maps to an OpenRA `GameOrder` the sim already handles
//! (see `world.rs::process_order`). Unit-targeted commands take actor
//! id strings (validated agent-owned in `env::build_orders`); economy
//! commands (`Build`/`CancelProduction`/`PlaceBuilding`) are owned by
//! the agent player's production, so they carry no actor id.
//!
//! Python sees one `Command` class with static constructors. Internally
//! a Rust enum keeps the env handler exhaustive.

#[cfg(feature = "python")]
use pyo3::prelude::*;

/// Native Rust representation. Used by integration tests and the PyO3
/// wrapper.
#[derive(Debug, Clone)]
pub enum Command {
    /// Move each unit to a cell (auto-fire opportunistically en route).
    MoveUnits { unit_ids: Vec<String>, target_x: i32, target_y: i32 },
    /// Pathfind to and focus-fire an enemy actor.
    AttackUnit { unit_ids: Vec<String>, target_id: String },
    /// Move toward a cell while engaging hostiles along the way.
    AttackMove { unit_ids: Vec<String>, target_x: i32, target_y: i32 },
    /// Follow and protect a friendly actor (C# Guard — follow subset).
    Guard { unit_ids: Vec<String>, target_id: String },
    /// Cancel current activity (go idle).
    Stop { unit_ids: Vec<String> },
    /// Transform an MCV into a construction yard.
    Deploy { unit_ids: Vec<String> },
    /// Enqueue production of `item` in the agent player's queue
    /// (covers both C# BUILD and TRAIN — same handler).
    Build { item: String },
    /// Cancel the last queued `item` (refunds remaining cost).
    CancelProduction { item: String },
    /// Place a completed building `item` at a cell.
    PlaceBuilding { item: String, target_x: i32, target_y: i32 },
    /// Send harvesters to a resource cell.
    Harvest { unit_ids: Vec<String>, target_x: i32, target_y: i32 },
    /// Sell a building (immediate refund).
    Sell { unit_ids: Vec<String> },
    /// Toggle repair on a building.
    Repair { unit_ids: Vec<String> },
    /// Toggle a building's power.
    PowerDown { unit_ids: Vec<String> },
    /// Set a production building's rally point.
    SetRallyPoint { unit_ids: Vec<String>, target_x: i32, target_y: i32 },
    /// Set engagement stance: 0=HoldFire, 1=ReturnFire, 2=Defend,
    /// 3=AttackAnything (clamped). HoldFire suppresses auto-engage.
    SetStance { unit_ids: Vec<String>, stance: i32 },
    /// C# parity: PATROL is defined but unimplemented — accepted as a
    /// no-op (does not warn / does not divert the unit).
    Patrol { unit_ids: Vec<String> },
    /// Concede the match (the agent loses immediately).
    Surrender,
    /// No-op; the env still ticks N frames.
    Observe,
}

/// Python-facing shim around `Command`.
#[cfg(feature = "python")]
#[pyclass(name = "Command", module = "openra_train")]
#[derive(Debug, Clone)]
pub struct PyCommand {
    pub(crate) inner: Command,
}

#[cfg(feature = "python")]
#[pymethods]
impl PyCommand {
    #[staticmethod]
    fn move_units(unit_ids: Vec<String>, target_x: i32, target_y: i32) -> Self {
        Self { inner: Command::MoveUnits { unit_ids, target_x, target_y } }
    }

    #[staticmethod]
    fn attack_unit(unit_ids: Vec<String>, target_id: String) -> Self {
        Self { inner: Command::AttackUnit { unit_ids, target_id } }
    }

    #[staticmethod]
    fn attack_move(unit_ids: Vec<String>, target_x: i32, target_y: i32) -> Self {
        Self { inner: Command::AttackMove { unit_ids, target_x, target_y } }
    }

    #[staticmethod]
    fn guard(unit_ids: Vec<String>, target_id: String) -> Self {
        Self { inner: Command::Guard { unit_ids, target_id } }
    }

    #[staticmethod]
    fn stop(unit_ids: Vec<String>) -> Self {
        Self { inner: Command::Stop { unit_ids } }
    }

    #[staticmethod]
    fn deploy(unit_ids: Vec<String>) -> Self {
        Self { inner: Command::Deploy { unit_ids } }
    }

    /// Enqueue production of `item` (BUILD/TRAIN).
    #[staticmethod]
    fn build(item: String) -> Self {
        Self { inner: Command::Build { item } }
    }

    #[staticmethod]
    fn cancel_production(item: String) -> Self {
        Self { inner: Command::CancelProduction { item } }
    }

    #[staticmethod]
    fn place_building(item: String, target_x: i32, target_y: i32) -> Self {
        Self { inner: Command::PlaceBuilding { item, target_x, target_y } }
    }

    #[staticmethod]
    fn harvest(unit_ids: Vec<String>, target_x: i32, target_y: i32) -> Self {
        Self { inner: Command::Harvest { unit_ids, target_x, target_y } }
    }

    #[staticmethod]
    fn sell(unit_ids: Vec<String>) -> Self {
        Self { inner: Command::Sell { unit_ids } }
    }

    #[staticmethod]
    fn repair(unit_ids: Vec<String>) -> Self {
        Self { inner: Command::Repair { unit_ids } }
    }

    #[staticmethod]
    fn power_down(unit_ids: Vec<String>) -> Self {
        Self { inner: Command::PowerDown { unit_ids } }
    }

    #[staticmethod]
    fn set_rally_point(unit_ids: Vec<String>, target_x: i32, target_y: i32) -> Self {
        Self { inner: Command::SetRallyPoint { unit_ids, target_x, target_y } }
    }

    #[staticmethod]
    fn observe() -> Self {
        Self { inner: Command::Observe }
    }

    #[staticmethod]
    fn surrender() -> Self {
        Self { inner: Command::Surrender }
    }

    #[staticmethod]
    fn set_stance(unit_ids: Vec<String>, stance: i32) -> Self {
        Self { inner: Command::SetStance { unit_ids, stance } }
    }

    #[staticmethod]
    fn patrol(unit_ids: Vec<String>) -> Self {
        Self { inner: Command::Patrol { unit_ids } }
    }

    fn __repr__(&self) -> String {
        format!("Command::{:?}", self.inner)
    }
}

#[cfg(feature = "python")]
impl PyCommand {
    pub fn into_inner(self) -> Command {
        self.inner
    }
}
