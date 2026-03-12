//! Try to find infantry sprites with various naming conventions.

use openra_data::mix;

const MIX_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/");

fn load_mix(name: &str) -> Option<mix::MixArchive> {
    let path = format!("{}{}", MIX_DIR, name);
    let data = std::fs::read(&path).ok()?;
    mix::MixArchive::parse(data).ok()
}

#[test]
fn find_infantry_variants() {
    let mix_names = [
        "conquer.mix", "allies.mix", "russian.mix", "temperat.mix",
        "interior.mix", "hires.mix", "lores.mix", "local.mix",
    ];

    // Try different naming conventions
    let variants = [
        // With .shp
        "e1.shp", "e2.shp",
        // Without extension
        "e1", "e2",
        // With .tem
        "e1.tem", "e2.tem",
        // Uppercase
        "E1.SHP", "E2.SHP",
        // Infantry prefix
        "infantry.mix",
        // sam2 variants
        "sam2.shp", "sam.shp",
        // proctop
        "proctop.shp",
        // Expansion units
        "ttnk.shp", "ftrk.shp", "shok.shp",
        // nopower
        "nopower.shp",
    ];

    for mix_name in &mix_names {
        if let Some(mix) = load_mix(mix_name) {
            let mut found = Vec::new();
            for name in &variants {
                if mix.contains(name) {
                    found.push(*name);
                }
            }
            if !found.is_empty() {
                println!("{}: {:?}", mix_name, found);
            }
        }
    }

    // Check if conquer.mix has a nested infantry.mix
    let conquer = load_mix("conquer.mix").unwrap();
    if let Some(infantry_data) = conquer.get("infantry.mix") {
        println!("\nFound infantry.mix inside conquer.mix ({} bytes)", infantry_data.len());
        if let Ok(infantry_mix) = mix::MixArchive::parse(infantry_data.to_vec()) {
            println!("infantry.mix has {} files", infantry_mix.len());
            for name in &["e1.shp", "e2.shp", "e3.shp", "e4.shp", "e6.shp", "e7.shp", "spy.shp", "dog.shp"] {
                if infantry_mix.contains(name) {
                    println!("  Found {} in infantry.mix", name);
                }
            }
        }
    }

    // Also check for nested MIX files
    for inner in &["expand.mix", "expand2.mix", "redalert.mix", "main.mix", "general.mix"] {
        if conquer.contains(inner) {
            println!("conquer.mix contains nested: {}", inner);
        }
    }

    // Check local.mix for nested archives
    if let Some(local) = load_mix("local.mix") {
        for inner in &["expand.mix", "expand2.mix", "redalert.mix", "general.mix", "conquer.mix"] {
            if local.contains(inner) {
                println!("local.mix contains nested: {}", inner);
            }
        }
    }
}
