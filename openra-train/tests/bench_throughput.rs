//! Manual throughput benchmark — target ≥1000 sim ticks/sec on a single
//! core. Documented in `STATUS_PHASE_5.md`.
//!
//! Not a proper criterion harness (we keep dev deps minimal), but an
//! ignored-by-default cargo test you can run with
//! `cargo test -p openra-train --test bench_throughput --release -- --ignored`.

use openra_train::{Command, Env};
use std::path::PathBuf;
use std::time::Instant;

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

#[test]
#[ignore = "throughput benchmark — run with --release --ignored"]
fn ticks_per_second_above_1000() {
    let path = scenario_path().expect("scenario yaml");
    let mut env = Env::new(path.to_str().unwrap(), 42)
        .expect("Env::new")
        .with_ticks_per_step(30);
    let _ = env.reset();

    // Run a fixed number of steps, count world ticks performed.
    const STEPS: u32 = 200;
    let start = Instant::now();
    let mut total_ticks: u32 = 0;
    for _ in 0..STEPS {
        let r = env.step(&[Command::Observe]);
        total_ticks += env.ticks_per_step();
        if r.done {
            // Reset and keep going — we want a steady-state benchmark
            // that doesn't bail early on terminal episodes.
            let _ = env.reset();
        }
    }
    let elapsed = start.elapsed();
    let tps = total_ticks as f64 / elapsed.as_secs_f64();

    eprintln!(
        "throughput: {} ticks in {:.3}s = {:.0} ticks/sec",
        total_ticks,
        elapsed.as_secs_f64(),
        tps
    );

    assert!(
        tps > 1000.0,
        "expected ≥ 1000 ticks/sec, got {tps:.0} ticks/sec"
    );
}
