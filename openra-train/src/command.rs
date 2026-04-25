//! Typed command DSL for `OpenRAEnv::step`.
//!
//! Three commands map to OpenRA orders:
//!   * `MoveUnits { unit_ids, target_x, target_y }` → one `Move` order
//!     per unit.
//!   * `AttackUnit { unit_ids, target_id }` → one `Attack` order per
//!     unit (target is an enemy actor id).
//!   * `Observe` → no-op; the env still ticks N frames.
//!
//! Python sees a single `Command` class with three static constructors
//! (`Command.move_units(...)`, `Command.attack_unit(...)`,
//! `Command.observe()`). Internally we keep a Rust enum so the env
//! handler can `match` exhaustively.

#[cfg(feature = "python")]
use pyo3::prelude::*;

/// Native Rust representation. Used by integration tests and by the
/// PyO3 wrapper.
#[derive(Debug, Clone)]
pub enum Command {
    MoveUnits {
        unit_ids: Vec<String>,
        target_x: i32,
        target_y: i32,
    },
    AttackUnit {
        unit_ids: Vec<String>,
        target_id: String,
    },
    Observe,
}

/// Python-facing shim around `Command`.
///
/// PyO3 0.22 doesn't easily round-trip an enum with mixed-shape variants
/// without a discriminant, so we go with a single class plus three
/// classmethod constructors.
#[cfg(feature = "python")]
#[pyclass(name = "Command", module = "openra_train")]
#[derive(Debug, Clone)]
pub struct PyCommand {
    pub(crate) inner: Command,
}

#[cfg(feature = "python")]
#[pymethods]
impl PyCommand {
    /// Issue a Move order for each listed unit, all targeting the same cell.
    #[staticmethod]
    fn move_units(unit_ids: Vec<String>, target_x: i32, target_y: i32) -> Self {
        PyCommand {
            inner: Command::MoveUnits {
                unit_ids,
                target_x,
                target_y,
            },
        }
    }

    /// Issue an Attack order against the named enemy actor for each unit.
    #[staticmethod]
    fn attack_unit(unit_ids: Vec<String>, target_id: String) -> Self {
        PyCommand {
            inner: Command::AttackUnit {
                unit_ids,
                target_id,
            },
        }
    }

    /// No-op: env still advances N ticks.
    #[staticmethod]
    fn observe() -> Self {
        PyCommand {
            inner: Command::Observe,
        }
    }

    /// Pretty-print for Python `repr()`.
    fn __repr__(&self) -> String {
        match &self.inner {
            Command::MoveUnits {
                unit_ids,
                target_x,
                target_y,
            } => format!(
                "Command.move_units({:?}, {}, {})",
                unit_ids, target_x, target_y
            ),
            Command::AttackUnit {
                unit_ids,
                target_id,
            } => format!("Command.attack_unit({:?}, {:?})", unit_ids, target_id),
            Command::Observe => "Command.observe()".into(),
        }
    }
}

#[cfg(feature = "python")]
impl PyCommand {
    pub fn into_inner(self) -> Command {
        self.inner
    }
}
