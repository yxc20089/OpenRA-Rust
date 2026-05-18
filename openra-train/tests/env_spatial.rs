//! S9 spatial tensor: flat row-major [y][x][c], 6 channels
//! (passable, fog, own-unit density, visible-enemy density, own
//! building, resource). Enables grid/occupancy spatial reasoning.

use openra_train::{Command, Env};
use std::io::Write;
use std::path::PathBuf;

const SCENARIO: &str = r#"name: Spatial
base_map: ../maps/rush-hour-arena.oramap
spawn_mcvs: false
starting_cash: 200
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: proc
  owner: agent
  position:
  - 12
  - 18
- type: harv
  owner: agent
  position:
  - 14
  - 18
- type: mine
  owner: neutral
  position:
  - 22
  - 18
termination:
  max_ticks: 40000
"#;

fn scenario() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(&home).join("Projects/OpenRA-RL-Training/scenarios/discovery");
    if !dir.join("rush-hour.yaml").exists() {
        return None;
    }
    let p = dir.join("_spatial_test.yaml");
    std::fs::File::create(&p).ok()?.write_all(SCENARIO.as_bytes()).ok()?;
    Some(p)
}

const C: usize = 6;

#[test]
fn spatial_tensor_shape_channels_and_determinism() {
    let Some(path) = scenario() else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut a = Env::new(path.to_str().unwrap(), 7).unwrap();
    let oa = a.reset();
    let (h, w, c) = oa.spatial_shape;
    assert_eq!(c, 6, "channel count");
    assert!(w > 0 && h > 0);
    assert_eq!(w, oa.map_info.width, "tensor width == map width");
    assert_eq!(h, oa.map_info.height, "tensor height == map height");
    assert_eq!(oa.spatial.len(), (w * h * c) as usize, "flat length");

    let at = |x: i32, y: i32, ch: usize| {
        oa.spatial[((y * w + x) * c) as usize + ch]
    };
    // Channel 5 (resource): ore was seeded around the mine at (22,18).
    let ore: f32 = oa.spatial.iter().skip(5).step_by(C).sum();
    assert!(ore > 0.0, "resource channel must mark seeded ore");
    // Channel 2 (own units): the harvester at (14,18) is present.
    assert!(at(14, 18, 2) >= 1.0, "own-unit channel at harvester cell");
    // Channel 4 (own building): the refinery occupies its footprint.
    let own_b: f32 = oa.spatial.iter().skip(4).step_by(C).sum();
    assert!(own_b > 0.0, "own-building channel must mark the refinery");
    // Channel 1 (fog): some cells visible (1.0) at game start.
    assert!(
        oa.spatial.iter().skip(1).step_by(C).any(|&v| v == 1.0),
        "fog channel must show visible cells at reset"
    );
    // Channel 0 (passable): a non-trivial fraction of the map.
    let passable: f32 = oa.spatial.iter().step_by(C).sum();
    assert!(passable > (w * h) as f32 * 0.1, "passable channel sane");

    // Determinism: same seed -> identical tensor after identical steps.
    let mut b = Env::new(path.to_str().unwrap(), 7).unwrap();
    b.reset();
    let ra = a.step(&[Command::Observe]);
    let rb = b.step(&[Command::Observe]);
    assert_eq!(ra.obs.spatial, rb.obs.spatial, "spatial must be deterministic");
    assert_eq!(ra.obs.spatial_shape, rb.obs.spatial_shape);

    let _ = std::fs::remove_file(&path);
}
