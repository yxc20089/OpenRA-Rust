//! S7: SURRENDER — the agent concedes; the env terminates immediately
//! (agent treated as defeated regardless of remaining force).

use openra_train::{Command, Env};
use std::path::PathBuf;

fn scenario() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let p = PathBuf::from(&home)
        .join("Projects/OpenRA-RL-Training/scenarios/discovery/rush-hour.yaml");
    p.exists().then_some(p)
}

#[test]
fn surrender_terminates_the_episode() {
    let Some(path) = scenario() else {
        eprintln!("skip: rush-hour scenario not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).expect("Env::new");
    env.reset();

    // A normal step early on is not terminal.
    let r0 = env.step(&[Command::Observe]);
    assert!(!r0.done, "fresh episode must not be terminal");

    // Surrender → immediately terminal (agent defeated), even though
    // the agent still has its full force on the field.
    let r1 = env.step(&[Command::Surrender]);
    assert!(r1.done, "surrender must terminate the episode");
}
