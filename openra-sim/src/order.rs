//! Typed in-engine order enum used by the Phase-1/2 API.
//!
//! Distinct from `world::GameOrder`, which is the string-based replay
//! representation. `Order` is what the Python/Rust agent layer issues.
//! It is converted to a `GameOrder` (or directly applied) at the
//! engine boundary.

use crate::math::CPos;
use crate::world::GameOrder;

/// A typed agent order targeting a single subject actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Order {
    /// Move the subject actor to the given cell.
    Move { target_cpos: CPos },
    /// Cancel the subject's current activity.
    Stop,
    /// Attack a specific actor by id.
    Attack { target_actor_id: u32 },
}

impl Order {
    /// Encode the typed order into the string-based `GameOrder` consumed
    /// by `World::process_frame`. `subject_id` is the actor receiving
    /// the order.
    pub fn to_game_order(&self, subject_id: u32) -> GameOrder {
        match *self {
            Order::Move { target_cpos } => GameOrder {
                order_string: "Move".to_string(),
                subject_id: Some(subject_id),
                target_string: Some(format!("{},{}", target_cpos.x(), target_cpos.y())),
                extra_data: None,
            },
            Order::Stop => GameOrder {
                order_string: "Stop".to_string(),
                subject_id: Some(subject_id),
                target_string: None,
                extra_data: None,
            },
            Order::Attack { target_actor_id } => GameOrder {
                order_string: "Attack".to_string(),
                subject_id: Some(subject_id),
                target_string: None,
                extra_data: Some(target_actor_id),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_order_encodes_target_string() {
        let o = Order::Move { target_cpos: CPos::new(7, 11) };
        let g = o.to_game_order(42);
        assert_eq!(g.order_string, "Move");
        assert_eq!(g.subject_id, Some(42));
        assert_eq!(g.target_string.as_deref(), Some("7,11"));
        assert!(g.extra_data.is_none());
    }

    #[test]
    fn stop_order_carries_only_subject() {
        let g = Order::Stop.to_game_order(5);
        assert_eq!(g.order_string, "Stop");
        assert_eq!(g.subject_id, Some(5));
        assert!(g.target_string.is_none());
        assert!(g.extra_data.is_none());
    }

    #[test]
    fn attack_order_uses_extra_data_for_target() {
        let g = Order::Attack { target_actor_id: 99 }.to_game_order(3);
        assert_eq!(g.order_string, "Attack");
        assert_eq!(g.subject_id, Some(3));
        assert_eq!(g.extra_data, Some(99));
    }

    #[test]
    fn move_order_negative_cells() {
        let g = Order::Move { target_cpos: CPos::new(-2, -3) }.to_game_order(1);
        assert_eq!(g.target_string.as_deref(), Some("-2,-3"));
    }
}
