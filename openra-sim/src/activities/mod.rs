//! Activity implementations for the Phase-2 trait-based stack.
//!
//! These wrap the existing data-driven `actor::Activity` enum where
//! possible, providing the `Activity` trait surface so higher layers
//! can compose activity stacks idiomatically.

pub mod attack;
pub mod move_;
pub mod wait;

pub use attack::AttackActivity;
pub use move_::MoveActivity;
pub use wait::WaitActivity;
