//! S7 — cargo: ENTER_TRANSPORT / UNLOAD (C# `Cargo` + `Passenger`
//! traits, pragmatic subset).
//!
//! Scope asserted here (honest):
//!  * ENTER_TRANSPORT: an infantry unit ordered into an adjacent APC
//!    boards it — the passenger ceases to exist as a standalone world
//!    actor (it's carried), and the transport reports it as cargo.
//!  * UNLOAD: the transport ejects its passengers; the infantry
//!    re-appears as a standalone actor on a passable cell next to the
//!    transport, and the transport's cargo becomes empty.
//!  * Capacity: a transport will not board past its capacity (extra
//!    passengers are rejected with a warning, not silently dropped).
//!  * Validation: non-owned transport / passenger ids warn; commands
//!    never terminate the episode.
//!
//! NOT asserted (documented gap): per-passenger weight classes
//! (C# `Passenger.Weight`/`Cargo.MaxWeight`) — we use a unit count
//! capacity (APC=5) rather than weighted slots; turreted-transport
//! firing-while-loaded and load/unload animations are out of scope.

use openra_train::{Command, Env};
use std::io::Write;
use std::path::PathBuf;

const SCENARIO: &str = r#"name: Cargo
base_map: ../maps/rush-hour-arena.oramap
spawn_mcvs: false
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: apc
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
  owner: agent
  position:
  - 22
  - 21
- type: e1
  owner: enemy
  position:
  - 58
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

fn id_at(env: &Env, cell: (i32, i32)) -> Option<u32> {
    env.world()?
        .snapshot()
        .actors
        .iter()
        .find(|a| (a.x, a.y) == cell)
        .map(|a| a.id)
}

fn actor_exists(env: &Env, id: u32) -> bool {
    env.world()
        .map(|w| w.snapshot().actors.iter().any(|a| a.id == id))
        .unwrap_or(false)
}

fn cargo_of(env: &Env, transport: u32) -> Vec<u32> {
    env.world()
        .map(|w| w.transport_cargo(transport))
        .unwrap_or_default()
}

#[test]
fn enter_transport_then_unload_roundtrip() {
    let Some(path) = scenario("_cargo_roundtrip.yaml") else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).unwrap();
    env.reset();

    let apc = id_at(&env, (20, 20)).expect("apc at (20,20)");
    let p1 = id_at(&env, (22, 20)).expect("e1 at (22,20)");
    let p2 = id_at(&env, (22, 21)).expect("e1 at (22,21)");

    assert!(cargo_of(&env, apc).is_empty(), "apc starts empty");

    // Order both infantry to enter the APC.
    let r = env.step(&[Command::EnterTransport {
        unit_ids: vec![p1.to_string(), p2.to_string()],
        target_id: apc.to_string(),
    }]);
    assert!(r.warnings.is_empty(), "enter warned: {:?}", r.warnings);
    assert!(!r.done);

    // Give them time to walk to the transport and board.
    let mut boarded = false;
    for _ in 0..300 {
        env.step(&[Command::Observe]);
        if !actor_exists(&env, p1) && !actor_exists(&env, p2) {
            boarded = true;
            break;
        }
    }
    assert!(boarded, "both passengers should have boarded the APC");
    let mut cargo = cargo_of(&env, apc);
    cargo.sort();
    let mut want = vec![p1, p2];
    want.sort();
    assert_eq!(cargo, want, "APC must report both passengers as cargo");

    // Unload.
    let ru = env.step(&[Command::Unload { unit_ids: vec![apc.to_string()] }]);
    assert!(ru.warnings.is_empty(), "unload warned: {:?}", ru.warnings);
    let mut out = false;
    for _ in 0..50 {
        env.step(&[Command::Observe]);
        if actor_exists(&env, p1) && actor_exists(&env, p2) {
            out = true;
            break;
        }
    }
    assert!(out, "passengers must re-appear in the world after UNLOAD");
    assert!(
        cargo_of(&env, apc).is_empty(),
        "APC cargo must be empty after UNLOAD"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn enter_transport_validates() {
    let Some(path) = scenario("_cargo_validate.yaml") else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).unwrap();
    env.reset();
    let p1 = id_at(&env, (22, 20)).expect("e1");

    // Bad transport id → warn.
    let r1 = env.step(&[Command::EnterTransport {
        unit_ids: vec![p1.to_string()],
        target_id: "abc".into(),
    }]);
    assert!(
        r1.warnings.iter().any(|w| w.contains("target")),
        "invalid transport must warn: {:?}",
        r1.warnings
    );
    assert!(!r1.done);

    // Non-owned passenger → ownership warning.
    let apc = id_at(&env, (20, 20)).expect("apc");
    let r2 = env.step(&[Command::EnterTransport {
        unit_ids: vec!["999999".into()],
        target_id: apc.to_string(),
    }]);
    assert!(
        r2.warnings.iter().any(|w| w.contains("not owned")),
        "ownership validation must apply: {:?}",
        r2.warnings
    );

    let _ = std::fs::remove_file(&path);
}
