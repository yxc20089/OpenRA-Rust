//! Episode-termination contract. A scenario that places NO enemy is an
//! agent-objective scenario (custom-map navigation, economy buildup,
//! base building): enemy-elimination must NOT count as victory, so the
//! run continues — termination is driven by max_ticks / the agent being
//! wiped / the bench-side declarative win_condition.

use openra_train::{env, Command};

#[test]
fn no_enemy_scenario_does_not_terminate_early() {
    // 32x32 map, 1 own infantry, no enemies.
    let mut e = env::build_test_env_with_no_enemies((32, 32), 7);
    for i in 0..10 {
        let result = e.step(&[Command::Observe]);
        assert!(
            !result.done,
            "no-enemy scenario must keep running (terminated at step {i})"
        );
        assert!(result.obs.game_tick > 0, "game_tick should advance");
    }
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
