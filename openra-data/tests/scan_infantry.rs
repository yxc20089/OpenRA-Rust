//! Scan conquer.mix for infantry sprites by brute-force SHP decode.
use openra_data::{mix, shp};

const MIX_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/");

fn load_mix(name: &str) -> Option<mix::MixArchive> {
    let path = format!("{}{}", MIX_DIR, name);
    let data = std::fs::read(&path).ok()?;
    mix::MixArchive::parse(data).ok()
}

#[test]
fn scan_conquer_for_infantry_shp() {
    // Compute expected hashes for infantry sprites
    let infantry = ["e1.shp", "e2.shp", "e3.shp", "e4.shp", "e7.shp",
                     "spy.shp", "thf.shp", "shok.shp", "medi.shp", "mech.shp"];

    println!("Expected hashes:");
    for name in &infantry {
        let classic = mix::classic_hash(name);
        let crc = mix::crc32_hash(name);
        println!("  {}: classic=0x{:08X}, crc32=0x{:08X}", name, classic, crc);
    }

    let conquer = load_mix("conquer.mix").unwrap();
    let hashes = conquer.hash_ids();
    println!("\nconquer.mix has {} entries", hashes.len());

    // Check which infantry hashes exist
    for name in &infantry {
        let ch = mix::classic_hash(name);
        let cr = mix::crc32_hash(name);
        if hashes.contains(&ch) {
            println!("{} found via classic hash!", name);
        }
        if hashes.contains(&cr) {
            println!("{} found via crc32 hash!", name);
        }
    }

    // Brute force: try all entries, decode as SHP, look for infantry-sized sprites
    // Infantry SHP files typically have 176+ frames (8 facings * ~22 frames each)
    let mut infantry_candidates = Vec::new();
    for &hash in &hashes {
        if let Some(data) = conquer.get_by_hash(hash) {
            if data.len() > 1000 {
                if let Ok(shp_file) = shp::decode(data) {
                    let nframes = shp_file.frames.len();
                    // Infantry sprites are typically 50x39 with 176+ frames
                    if nframes >= 100 && shp_file.width < 60 && shp_file.height < 60 {
                        infantry_candidates.push((hash, nframes, shp_file.width, shp_file.height, data.len()));
                    }
                }
            }
        }
    }

    println!("\nInfantry-sized SHP candidates ({}):", infantry_candidates.len());
    for (hash, nf, w, h, size) in &infantry_candidates {
        println!("  hash=0x{:08X}, frames={}, {}x{}, {} bytes", hash, nf, w, h, size);
    }

    // Also check local.mix
    if let Some(local) = load_mix("local.mix") {
        let lhashes = local.hash_ids();
        println!("\nlocal.mix has {} entries", lhashes.len());
        for name in &infantry {
            if local.contains(name) {
                let data = local.get(name).unwrap();
                println!("{} FOUND in local.mix ({} bytes)", name, data.len());
                if let Ok(shp_file) = shp::decode(data) {
                    println!("  SHP: {} frames, {}x{}", shp_file.frames.len(), shp_file.width, shp_file.height);
                }
            }
        }
        // Also check dog.shp to verify (we know it's in conquer.mix)
        if local.contains("dog.shp") {
            println!("dog.shp also in local.mix");
        }
        // Check for nested mix files
        for nested in &["main.mix", "conquer.mix", "redalert.mix", "general.mix", "expand.mix", "expand2.mix"] {
            if let Some(nested_data) = local.get(nested) {
                println!("{} found in local.mix ({} bytes)", nested, nested_data.len());
                if let Ok(nested_mix) = mix::MixArchive::parse(nested_data.to_vec()) {
                    println!("  Contains {} entries", nested_mix.len());
                    for name in &infantry {
                        if nested_mix.contains(name) {
                            println!("  {} found!", name);
                        }
                    }
                }
            }
        }
    }

    // Check hires.mix and allies.mix and russian.mix
    for mix_name in &["hires.mix", "allies.mix", "russian.mix"] {
        if let Some(archive) = load_mix(mix_name) {
            for name in &infantry {
                if archive.contains(name) {
                    let data = archive.get(name).unwrap();
                    println!("{} FOUND in {} ({} bytes)", name, mix_name, data.len());
                }
            }
        }
    }
}
