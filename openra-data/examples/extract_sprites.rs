//! Extract SHP sprites from MIX archives for WASM bundling.

use openra_data::{mix, shp};
use std::fs;

const MIX_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/");
const OUT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/extracted-sprites/");

fn main() {
    let conquer_data = fs::read(format!("{}{}", MIX_DIR, "conquer.mix"))
        .expect("conquer.mix not found");
    let conquer = mix::MixArchive::parse(conquer_data).expect("Failed to parse conquer.mix");

    let temperat_data = fs::read(format!("{}{}", MIX_DIR, "temperat.mix"))
        .expect("temperat.mix not found");
    let temperat = mix::MixArchive::parse(temperat_data).expect("Failed to parse temperat.mix");

    // Sprites to extract from conquer.mix
    let conquer_sprites = [
        // Vehicles
        "1tnk.shp", "2tnk.shp", "3tnk.shp", "4tnk.shp",
        "harv.shp", "mcv.shp", "jeep.shp", "apc.shp",
        "v2rl.shp", "arty.shp", "mnly.shp", "mrj.shp",
        "truk.shp", "stnk.shp", "dog.shp",
        // Buildings
        "powr.shp", "apwr.shp", "barr.shp", "fact.shp",
        "proc.shp", "weap.shp", "dome.shp", "gun.shp",
        "tent.shp", "tsla.shp", "pbox.shp", "gap.shp",
        "iron.shp", "fix.shp", "silo.shp", "atek.shp",
        "stek.shp", "ftur.shp", "sam.shp",
        "weap2.shp",
        // Aircraft
        "heli.shp", "hind.shp", "yak.shp", "mig.shp", "tran.shp",
        // Naval
        "ss.shp", "dd.shp", "ca.shp", "pt.shp", "lst.shp",
    ];

    // Terrain from temperat.mix
    let terrain_tiles = [
        "clear1.tem",
    ];

    fs::create_dir_all(OUT_DIR).expect("Failed to create output dir");

    let mut extracted = 0;
    let mut total_bytes = 0;

    for name in &conquer_sprites {
        if let Some(data) = conquer.get(name) {
            let out_path = format!("{}{}", OUT_DIR, name);
            fs::write(&out_path, data).expect(&format!("Failed to write {}", name));
            // Verify it decodes
            match shp::decode(data) {
                Ok(shp_file) => {
                    println!("  {} => {}x{}, {} frames, {} bytes",
                        name, shp_file.width, shp_file.height,
                        shp_file.frames.len(), data.len());
                    extracted += 1;
                    total_bytes += data.len();
                }
                Err(e) => println!("  {} => DECODE ERROR: {}", name, e),
            }
        } else {
            println!("  {} => NOT FOUND", name);
        }
    }

    for name in &terrain_tiles {
        if let Some(data) = temperat.get(name) {
            let out_path = format!("{}{}", OUT_DIR, name);
            fs::write(&out_path, data).expect(&format!("Failed to write {}", name));
            println!("  {} => {} bytes (terrain)", name, data.len());
            extracted += 1;
            total_bytes += data.len();
        }
    }

    println!("\nExtracted {} files, {} total bytes", extracted, total_bytes);
}
