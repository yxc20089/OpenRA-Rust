//! Phase-8 acceptance: the env loader attaches `Vehicle` + `Turret`
//! typed components to vehicle actors when constructing the world.
//!
//! Verified via `World::typed_components_of(actor_id)`. Phase 6's
//! `Vehicle` / `Turret` types live on the world's typed-component
//! map (added in Phase 8 to close the carry-forward TODO).
//!
//! Skipped silently when the rush-hour scenario isn't present on the
//! dev box.

use openra_sim::actor::ActorKind;
use openra_train::Env;
use std::path::PathBuf;

fn scenario_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RUSH_HOUR_SCENARIO") {
        let pb = PathBuf::from(p);
        if pb.exists() { return Some(pb); }
    }
    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(home)
            .join("Projects/OpenRA-RL-Training/scenarios/discovery/rush-hour.yaml");
        if p.exists() { return Some(p); }
    }
    None
}

#[test]
fn env_attaches_vehicle_and_turret_components_to_2tnk() {
    let path = match scenario_path() {
        Some(p) => p,
        None => {
            eprintln!("skipping — rush-hour scenario yaml not found on this box");
            return;
        }
    };
    let mut env = Env::new(path.to_str().unwrap(), 42).expect("Env::new");
    let _ = env.reset();
    let world = env.world().expect("world should exist after reset");

    // Find every vehicle in the world. Each one (loaded through the
    // env's scenario actor injection) should have an
    // `ActorTypedComponents` entry with `Vehicle` set; if its YAML
    // declares `Turreted: TurnSpeed: ...`, also `Turret`.
    let mut found_vehicle = false;
    let mut found_turret = false;
    let mut sample = String::new();
    for id in openra_sim::world::all_actor_ids(world) {
        if !matches!(world.actor_kind(id), Some(ActorKind::Vehicle)) {
            continue;
        }
        let actor_type = world.actor_type_name(id).map(|s| s.to_string()).unwrap_or_default();
        let bundle = world.typed_components_of(id);
        if let Some(b) = bundle {
            if b.vehicle.is_some() {
                found_vehicle = true;
                sample = actor_type.clone();
            }
            if b.turret.is_some() {
                found_turret = true;
            }
            eprintln!(
                "actor {id} ({actor_type}): vehicle={:?} turret={:?}",
                b.vehicle.as_ref().map(|v| v.locomotor),
                b.turret.as_ref().map(|t| t.facing.angle),
            );
        }
    }
    assert!(found_vehicle, "expected ≥1 Vehicle typed-component on a vehicle actor");
    assert!(found_turret, "expected ≥1 Turret typed-component (most rush-hour vehicles have turrets)");
    eprintln!("vehicle sample type: {sample}");
}

#[test]
fn env_does_not_attach_typed_components_to_infantry() {
    let path = match scenario_path() {
        Some(p) => p,
        None => return,
    };
    let mut env = Env::new(path.to_str().unwrap(), 42).expect("Env::new");
    let _ = env.reset();
    let world = env.world().expect("world after reset");

    // Pure foot infantry should NOT get a Vehicle typed component.
    for id in openra_sim::world::all_actor_ids(world) {
        if !matches!(world.actor_kind(id), Some(ActorKind::Infantry)) {
            continue;
        }
        if let Some(bundle) = world.typed_components_of(id) {
            assert!(
                bundle.vehicle.is_none(),
                "infantry actor {id} should not have a Vehicle component"
            );
        }
    }
}
