//! Training runtime for RL agents.
//!
//! Manages parallel game simulations and exposes a Python API via PyO3
//! for direct integration with training pipelines (TRL GRPOTrainer, etc.).
//!
//! Phase 5 surface area:
//!   * `OpenRAEnv` — gym-like wrapper over a single `World` running the
//!     rush-hour scenario.
//!   * `Command` — typed command DSL mirrored 1:1 to a Python class.
//!
//! The `python` feature gates all PyO3-touching code so the crate still
//! builds (and integration tests still link) when no Python interpreter
//! is available.

pub mod command;
pub mod env;
pub mod observation;

pub use command::Command;
pub use env::Env;

#[cfg(feature = "python")]
use pyo3::prelude::*;

/// PyO3 module entry point.
///
/// `maturin develop --release` produces an importable `openra_train`
/// Python package with `OpenRAEnv` and `Command` exposed.
#[cfg(feature = "python")]
#[pymodule]
fn openra_train(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<env::OpenRAEnv>()?;
    m.add_class::<command::PyCommand>()?;
    Ok(())
}
