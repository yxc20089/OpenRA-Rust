//! Episode-termination test: rig a scenario where the enemy team has
//! 0 units at start, then verify `done == true` immediately on first
//! `step()`.

use openra_train::{env, Command};

#[test]
fn no_enemy_units_means_done_on_first_step() {
    // 32x32 map, 1 own infantry, no enemies.
    let mut e = env::build_test_env_with_no_enemies((32, 32), 7);
    let result = e.step(&[Command::Observe]);

    assert!(
        result.done,
        "with 0 enemy combat units, first step should signal done=true"
    );
    // Tick should still have advanced.
    assert!(
        result.obs.game_tick > 0,
        "game_tick should advance even on a terminal step"
    );
}

#[test]
fn empty_world_obs_has_zero_enemies() {
    let mut e = env::build_test_env_with_no_enemies((32, 32), 1);
    let obs = e.reset();
    assert_eq!(obs.enemy_positions.len(), 0);
    assert_eq!(obs.enemy_hp.len(), 0);
    // Own infantry is present.
    assert_eq!(obs.unit_positions.len(), 1);
    assert_eq!(obs.unit_hp.len(), 1);
}
