//! Test extracting and decoding sprites from MIX archives.

use openra_data::{mix, shp, palette};

const MIX_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/");

fn load_mix(name: &str) -> Option<mix::MixArchive> {
    let path = format!("{}{}", MIX_DIR, name);
    let data = std::fs::read(&path).ok()?;
    mix::MixArchive::parse(data).ok()
}

#[test]
fn extract_and_decode_unit_sprites() {
    let conquer = load_mix("conquer.mix").expect("conquer.mix not found");

    // Try common unit sprite names
    let names = [
        "e1.shp", "e2.shp", "e3.shp", "e4.shp",
        "1tnk.shp", "2tnk.shp", "3tnk.shp", "4tnk.shp",
        "harv.shp", "mcv.shp", "jeep.shp", "apc.shp",
        "v2rl.shp", "arty.shp",
    ];

    let mut found = Vec::new();
    let mut not_found = Vec::new();

    for name in &names {
        if let Some(data) = conquer.get(name) {
            match shp::decode(data) {
                Ok(shp_file) => {
                    found.push(format!(
                        "{}: {}x{}, {} frames",
                        name, shp_file.width, shp_file.height, shp_file.frames.len()
                    ));
                }
                Err(e) => {
                    found.push(format!("{}: DECODE ERROR: {}", name, e));
                }
            }
        } else {
            not_found.push(*name);
        }
    }

    println!("\n=== Found in conquer.mix ===");
    for f in &found {
        println!("  {}", f);
    }
    if !not_found.is_empty() {
        println!("=== Not found in conquer.mix ===");
        for n in &not_found {
            println!("  {}", n);
        }
    }

    assert!(!found.is_empty(), "Should find at least some sprites in conquer.mix");
}

#[test]
fn extract_building_sprites() {
    let conquer = load_mix("conquer.mix").expect("conquer.mix not found");

    let buildings = [
        "powr.shp", "apwr.shp", "barr.shp", "fact.shp",
        "proc.shp", "weap.shp", "dome.shp", "gun.shp",
        "sam2.shp", "tsla.shp", "pbox.shp", "gap.shp",
        "iron.shp", "fix.shp", "silo.shp", "atek.shp",
        "stek.shp", "ftur.shp",
    ];

    let mut found = Vec::new();
    for name in &buildings {
        if let Some(data) = conquer.get(name) {
            match shp::decode(data) {
                Ok(shp_file) => {
                    found.push(format!(
                        "{}: {}x{}, {} frames",
                        name, shp_file.width, shp_file.height, shp_file.frames.len()
                    ));
                }
                Err(e) => {
                    found.push(format!("{}: DECODE ERROR: {}", name, e));
                }
            }
        }
    }

    println!("\n=== Buildings in conquer.mix ===");
    for f in &found {
        println!("  {}", f);
    }

    assert!(!found.is_empty(), "Should find building sprites");
}

#[test]
fn check_all_mix_files() {
    let mix_files = [
        "conquer.mix", "allies.mix", "russian.mix", "temperat.mix",
    ];

    let test_names = [
        "e1.shp", "1tnk.shp", "fact.shp", "powr.shp",
        "harv.shp", "mcv.shp", "tent.shp",
    ];

    for mix_name in &mix_files {
        if let Some(mix) = load_mix(mix_name) {
            let mut found: Vec<&str> = Vec::new();
            for name in &test_names {
                if mix.contains(name) {
                    found.push(name);
                }
            }
            if !found.is_empty() {
                println!("{}: contains {:?}", mix_name, found);
            }
        }
    }
}

#[test]
fn decode_sprite_to_rgba() {
    let conquer = load_mix("conquer.mix").expect("conquer.mix not found");
    let pal_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vendor/OpenRA/mods/ra/maps/chernobyl/temperat.pal"
    );
    let pal_data = std::fs::read(pal_path).expect("palette not found");
    let pal = palette::Palette::from_bytes(&pal_data).expect("palette parse failed");

    // Try to get e1.shp and decode to RGBA
    if let Some(shp_data) = conquer.get("e1.shp") {
        let shp_file = shp::decode(shp_data).expect("Failed to decode e1.shp");
        let frame = &shp_file.frames[0];

        // Convert to RGBA
        let mut rgba = Vec::with_capacity(frame.pixels.len() * 4);
        for &px in &frame.pixels {
            let c = pal.rgba(px);
            rgba.extend_from_slice(&c);
        }

        assert_eq!(rgba.len(), frame.width as usize * frame.height as usize * 4);
        println!(
            "e1.shp frame 0: {}x{}, {} RGBA bytes",
            frame.width,
            frame.height,
            rgba.len()
        );
    }
}
