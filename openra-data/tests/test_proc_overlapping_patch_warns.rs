//! Pin (currently FAILING / `#[ignore]`d): scenario-load validation
//! should emit a soft warning when a pre-placed `proc` (or any other
//! `Building`-footprint actor) overlaps a declared `ore_patches:`
//! disc.
//!
//! Bug: `env.rs::build_world_for_episode` seeds the ore patch FIRST,
//! then injects scenario actors. A `proc` placed inside a patch disc
//! occupies its footprint cells via `terrain.occupy_footprint` —
//! marking them ground-impassable WITHOUT clearing the resource
//! layer. `find_nearest_resource` keeps returning those cells, but
//! `find_path` cannot reach them, so the harvester loops
//! "FindingOre → can't path → fail" forever. The patch is silently
//! sterilised.
//!
//! Spec (option b in triage finding #7): emit a soft warning when
//! any scenario actor's `Building` footprint overlaps an
//! `OrePatchDef` disc, surfaced via the existing `last_warnings`
//! channel on `OpenRAEnv`. The warning string contains
//! `proc_overlaps_patch` (or `building_overlaps_patch` for non-proc
//! buildings) so bench tests can assert on it.
//!
//! Triaged in ENGINE_FOLLOWUPS_TRIAGE.md finding #7. The test is
//! `#[ignore]`d until the validator ships.

use openra_data::oramap::load_rush_hour_map_with_spawn;
use std::fs;
use std::path::PathBuf;

const BODY_PROC_INSIDE_PATCH: &str = r#"
base_map: rush-hour-arena.oramap
agent:
  faction: allies
enemy:
  faction: soviet
ore_patches:
- x: 40
  y: 10
  amount: 5000
  radius: 3
actors:
- type: proc
  owner: agent
  position:
  - 40
  - 10
"#;

fn load_scenario(text: &str) -> openra_data::oramap::MapDef {
    let tmpdir = tempfile::tempdir().expect("tempdir");
    let scenario_path: PathBuf = tmpdir.path().join("scen.yaml");
    fs::write(&scenario_path, text).unwrap();

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("rush-hour-arena.oramap");
    let dest = tmpdir.path().join("rush-hour-arena.oramap");
    if fixture.exists() {
        fs::copy(&fixture, &dest).unwrap();
    }

    load_rush_hour_map_with_spawn(&scenario_path, 0)
        .expect("scenario should parse")
}

#[test]
#[ignore = "TODO ENGINE_FOLLOWUPS_TRIAGE finding #7: needs scenario-load validator that emits proc_overlaps_patch warning"]
fn proc_overlapping_ore_patch_emits_warning() {
    // The validator does not exist yet; currently no warning is
    // surfaced and the patch is silently rendered unreachable.
    // Once the validator lands, `validate_layout(&map)` (or the
    // equivalent shape) should return a Vec<String> with at least
    // one entry containing `proc_overlaps_patch`.
    let map = load_scenario(BODY_PROC_INSIDE_PATCH);

    // Sanity: the scenario parsed both the proc and the patch.
    assert_eq!(map.ore_patches.len(), 1);
    let patch = map.ore_patches[0];
    assert_eq!((patch.x, patch.y, patch.radius), (40, 10, 3));

    // The (currently nonexistent) validator should flag this:
    // proc footprint at (40, 10) sits inside the radius-3 disc.
    //
    // Once `oramap::validate_layout` (or equivalent) is added, the
    // assertion below should hold. Until then, the test is
    // `#[ignore]`d so it doesn't break `cargo test`.
    let warnings: Vec<String> = validate_layout_placeholder(&map);
    assert!(
        warnings.iter().any(|w| w.contains("proc_overlaps_patch")),
        "validator should emit a proc_overlaps_patch warning when a \
         proc footprint overlaps an ore_patches disc; got {:?}",
        warnings,
    );
}

/// Placeholder for the not-yet-implemented `oramap::validate_layout`.
/// Returns an empty Vec so the assertion above fails — the
/// assertion is the test's pin. Remove this fn (and import the real
/// validator) once it lands.
fn validate_layout_placeholder(_map: &openra_data::oramap::MapDef) -> Vec<String> {
    Vec::new()
}
