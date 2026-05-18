//! S7 — GUARD command (C# `Guard` activity / `GuardActivity.cs`
//! parity, pragmatic subset).
//!
//! Scope asserted here (honest):
//!  * GUARD makes the guard unit FOLLOW the guarded actor: when the
//!    guarded friendly moves away, the guard repositions to stay
//!    within a small leash radius of it (it does not stand still).
//!  * GUARD is accepted for an owned unit with no warning and does
//!    not terminate the episode.
//!  * Targeting a non-owned / invalid actor warns (validation intact).
//!
//! NOT asserted (documented gap): C# Guard also re-acquires the
//! guarded actor's attackers (AttackFollow). We exercise the
//! follow/leash behaviour only; opportunistic re-engagement around
//! the guarded actor falls back to the normal stance auto-engage and
//! is not separately asserted here.

use openra_train::{Command, Env};
use std::io::Write;
use std::path::PathBuf;

const SCENARIO: &str = r#"name: Guard
base_map: ../maps/rush-hour-arena.oramap
spawn_mcvs: false
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: e1
  owner: agent
  position:
  - 20
  - 20
- type: e1
  owner: agent
  position:
  - 22
  - 20
- type: e1
  owner: enemy
  position:
  - 60
  - 32
  stance: 0
termination:
  max_ticks: 40000
"#;

fn scenario(name: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(&home).join("Projects/OpenRA-RL-Training/scenarios/discovery");
    if !dir.join("rush-hour.yaml").exists() {
        return None;
    }
    let p = dir.join(name);
    std::fs::File::create(&p).ok()?.write_all(SCENARIO.as_bytes()).ok()?;
    Some(p)
}

// Resolve a world actor id by its spawn cell (shroud-independent).
fn uid_at(env: &Env, cell: (i32, i32)) -> u32 {
    env.world()
        .expect("world")
        .snapshot()
        .actors
        .iter()
        .find(|a| (a.x, a.y) == cell)
        .map(|a| a.id)
        .unwrap_or_else(|| panic!("no actor at {cell:?}"))
}

fn pos(env: &Env, id: u32) -> (i32, i32) {
    env.world()
        .expect("world")
        .snapshot()
        .actors
        .iter()
        .find(|a| a.id == id)
        .map(|a| (a.x, a.y))
        .expect("actor exists")
}

fn cheb(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

#[test]
fn guard_unit_follows_guarded_actor() {
    let Some(path) = scenario("_guard_follow.yaml") else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).unwrap();
    env.reset();

    let guard = uid_at(&env, (20, 20));
    let guarded = uid_at(&env, (22, 20));

    // Order the guard to guard the guarded actor.
    let rg = env.step(&[Command::Guard {
        unit_ids: vec![guard.to_string()],
        target_id: guarded.to_string(),
    }]);
    assert!(rg.warnings.is_empty(), "guard warned: {:?}", rg.warnings);
    assert!(!rg.done, "guard must not terminate the episode");

    // Move the guarded actor far away.
    env.step(&[Command::MoveUnits {
        unit_ids: vec![guarded.to_string()],
        target_x: 40,
        target_y: 30,
    }]);

    // Let the world run; the guard must close on the moving guarded
    // actor and end up within a small leash radius of it.
    for _ in 0..400 {
        env.step(&[Command::Observe]);
        if cheb(pos(&env, guard), pos(&env, guarded)) <= 3 {
            break;
        }
    }

    let gp = pos(&env, guard);
    let tp = pos(&env, guarded);
    let d = cheb(gp, tp);
    assert!(
        d <= 4,
        "GUARD must follow: guard at {gp:?}, guarded at {tp:?}, cheb={d}"
    );
    // And it must actually have moved from its spawn (not a no-op).
    assert_ne!(gp, (20, 20), "guard never moved");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn guard_validates_target_and_owner() {
    let Some(path) = scenario("_guard_validate.yaml") else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).unwrap();
    env.reset();
    let guard = uid_at(&env, (20, 20));

    // Bad target id → warn, no termination.
    let r1 = env.step(&[Command::Guard {
        unit_ids: vec![guard.to_string()],
        target_id: "abc".into(),
    }]);
    assert!(
        r1.warnings.iter().any(|w| w.contains("target")),
        "invalid target must warn: {:?}",
        r1.warnings
    );
    assert!(!r1.done);

    // Non-owned guard unit → ownership warning.
    let r2 = env.step(&[Command::Guard {
        unit_ids: vec!["999999".into()],
        target_id: guard.to_string(),
    }]);
    assert!(
        r2.warnings.iter().any(|w| w.contains("not owned")),
        "ownership validation must apply: {:?}",
        r2.warnings
    );

    let _ = std::fs::remove_file(&path);
}
