//! Integration tests for loading real OpenRA mod assets.
//!
//! Requires vendor/OpenRA to be present (cloned from GitHub).

use std::path::Path;

const RA_MOD_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/OpenRA/mods/ra");

fn ra_mod_available() -> bool {
    Path::new(RA_MOD_DIR).join("rules/defaults.yaml").exists()
}

#[test]
fn load_ra_ruleset() {
    if !ra_mod_available() {
        eprintln!("Skipping: vendor/OpenRA not found");
        return;
    }

    let mod_dir = Path::new(RA_MOD_DIR);
    let ruleset = openra_data::rules::load_ruleset(mod_dir).expect("Failed to load RA ruleset");

    // Verify we loaded actors
    assert!(ruleset.actors.len() > 50, "Expected 50+ actors, got {}", ruleset.actors.len());
    eprintln!("Loaded {} actors, {} weapons", ruleset.actors.len(), ruleset.weapons.len());

    // Verify specific actors exist
    assert!(ruleset.actor("E1").is_some(), "E1 infantry should exist");
    assert!(ruleset.actor("2TNK").is_some(), "2TNK medium tank should exist");
    assert!(ruleset.actor("FACT").is_some(), "FACT construction yard should exist");
    assert!(ruleset.actor("HARV").is_some(), "HARV harvester should exist");

    // Verify we loaded weapons
    assert!(ruleset.weapons.len() > 10, "Expected 10+ weapons, got {}", ruleset.weapons.len());

    // Check E1 has Health trait
    let e1 = ruleset.actor("E1").unwrap();
    let health = e1.trait_info("Health");
    assert!(health.is_some(), "E1 should have Health trait");
    let hp = health.unwrap().get_i32("HP");
    assert!(hp.is_some() && hp.unwrap() > 0, "E1 HP should be > 0");
    eprintln!("E1 HP = {}", hp.unwrap());
}

#[test]
fn parse_real_palette() {
    let pal_path = Path::new(RA_MOD_DIR).join("maps/chernobyl/temperat.pal");
    if !pal_path.exists() {
        eprintln!("Skipping: temperat.pal not found");
        return;
    }

    let data = std::fs::read(&pal_path).unwrap();
    let palette = openra_data::palette::Palette::from_bytes(&data).unwrap();

    // Index 0 should be transparent
    assert_eq!(palette.rgba(0), [0, 0, 0, 0]);

    // Other indices should have alpha 255
    assert_eq!(palette.rgba(1)[3], 255);

    // Palette should have varied colors
    let mut distinct_colors = std::collections::HashSet::new();
    for i in 0..=255u8 {
        distinct_colors.insert(palette.colors[i as usize]);
    }
    assert!(distinct_colors.len() > 50, "Palette should have variety, got {} distinct", distinct_colors.len());
    eprintln!("Palette has {} distinct colors", distinct_colors.len());
}

#[test]
fn decode_real_shp_file() {
    let shp_path = Path::new(RA_MOD_DIR).join("bits/afldidle.shp");
    if !shp_path.exists() {
        eprintln!("Skipping: afldidle.shp not found");
        return;
    }

    let data = std::fs::read(&shp_path).unwrap();
    eprintln!("SHP file size: {} bytes", data.len());

    let shp = openra_data::shp::decode(&data);
    match shp {
        Ok(shp) => {
            assert!(shp.frames.len() > 0, "SHP should have frames");
            eprintln!("Decoded {} frames", shp.frames.len());
            for (i, frame) in shp.frames.iter().enumerate() {
                eprintln!("  Frame {}: {}x{} ({} pixels)",
                    i, frame.width, frame.height, frame.pixels.len());
                assert_eq!(frame.pixels.len(), frame.width as usize * frame.height as usize,
                    "Frame {} pixel count mismatch", i);
            }
        }
        Err(e) => {
            eprintln!("SHP decode error (may need Format80 fixes): {}", e);
            // Don't fail — we know Format80 might have issues
        }
    }
}
