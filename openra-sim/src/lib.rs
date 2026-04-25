//! OpenRA game simulation engine.
//!
//! Deterministic, zero-dependency core that replays OpenRA (Red Alert) games
//! tick-by-tick. Used by both the browser replay viewer (openra-wasm) and
//! the training runtime (openra-train).

pub mod activities;
pub mod activity;
pub mod actor;
pub mod ai;
pub mod gamerules;
pub mod math;
pub mod order;
pub mod pathfinder;
pub mod rng;
pub mod sync;
pub mod terrain;
pub mod traits;
pub mod world;
