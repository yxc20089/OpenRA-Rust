//! Determinism: same seed + same command sequence ⇒ same observation
//! hash. Different seed ⇒ different hash.
//!
//! Uses `Observation::deterministic_hash` (FNV-1a over the sorted
//! observation fields).

use openra_train::{Command, Env};
use std::path::PathBuf;

fn scenario_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RUSH_HOUR_SCENARIO") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(home)
            .join("Projects/OpenRA-RL-Training/scenarios/discovery/rush-hour.yaml");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// One trace per step. We mix three signals into the trace so seed
/// divergence is detectable even though the public observation
/// surface is deliberately RNG-poor:
///   - `Observation::deterministic_hash` (positions / HP / kills)
///   - `Env::world_sync_hash` (rng.last + actor identity hashes)
///   - the spawn-assignment side channel (different seeds produce
///     different spawn picks via `assign_spawn_points`'s player_rng,
///     which the env preserves by allocating actor ids deterministically
///     after spawn assignment).
fn run_episode(seed: u64) -> Vec<(u64, i32)> {
    let path = scenario_path().expect("scenario yaml present");
    let mut env = Env::new(path.to_str().unwrap(), seed).expect("Env::new");
    let mut hashes = Vec::new();

    let initial = env.reset();
    hashes.push((initial.deterministic_hash(), env.world_sync_hash()));

    let unit_ids: Vec<String> = initial
        .unit_positions
        .iter()
        .map(|(id, _)| id.clone())
        .collect();

    // Issue 5 fixed Move commands (same target, same units) so the
    // command sequence is identical between runs of the same seed.
    let target = (60, 20);
    let cmd = Command::MoveUnits {
        unit_ids: unit_ids.clone(),
        target_x: target.0,
        target_y: target.1,
    };
    for _ in 0..5 {
        let r = env.step(&[cmd.clone()]);
        hashes.push((r.obs.deterministic_hash(), env.world_sync_hash()));
    }
    hashes
}

/// Pick two seeds that are guaranteed to diverge through the spawn
/// assignment side channel: seeds where the first `next_range(0,2)`
/// player_rng draw differs. We brute-force a small window since
/// MersenneTwister output is too cheap to script around.
fn pick_diverging_seeds() -> (u64, u64) {
    use openra_sim::rng::MersenneTwister;
    let mut a = None;
    let mut b = None;
    for s in 0i64..200 {
        let mut rng = MersenneTwister::new(s as i32);
        let v = rng.next_range(0, 2);
        if v == 0 && a.is_none() {
            a = Some(s as u64);
        } else if v == 1 && b.is_none() {
            b = Some(s as u64);
        }
        if a.is_some() && b.is_some() {
            break;
        }
    }
    (a.expect("seed with v=0"), b.expect("seed with v=1"))
}

#[test]
fn same_seed_same_command_sequence_same_hashes() {
    if scenario_path().is_none() {
        eprintln!("skipping — rush-hour scenario yaml not found");
        return;
    }

    let a = run_episode(123);
    let b = run_episode(123);
    assert_eq!(
        a, b,
        "two episodes with seed=123 should produce byte-identical observation hashes"
    );
}

#[test]
fn different_seed_yields_different_hash() {
    if scenario_path().is_none() {
        eprintln!("skipping — rush-hour scenario yaml not found");
        return;
    }

    // Strict assertion (tightened post Agent B merge): pick a pair of
    // seeds that we know diverge at the very first `assign_spawn_points`
    // RNG draw. Both seeds run the same command script; the (observation
    // × sync_hash) trace must differ on at least one step.
    let (sa, sb) = pick_diverging_seeds();
    let a = run_episode(sa);
    let b = run_episode(sb);
    assert_ne!(
        a, b,
        "seeds {sa} / {sb} must produce divergent (observation × sync-hash) traces"
    );
}
