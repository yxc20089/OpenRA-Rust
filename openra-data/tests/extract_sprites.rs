//! Comprehensive RA sprite extraction from MIX archives
//! Tries all known sprite filenames across all available MIX files.
use openra_data::mix;

#[test]
fn extract_all_sprites() {
    let out_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/extracted-sprites/");
    let mix_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/");

    // All known RA sprite filenames to try extracting
    let sprites: &[&str] = &[
        // Vehicles (not already extracted)
        "dtrk", "ctnk", "qtnk", "ttnk", "mgg",
        // Buildings (not already extracted)
        "hpad", "spen", "syrd", "agun", "hbox", "kenn", "miss", "pdox",
        "mslo", "fcom", "hosp", "bio", "oilb", "brik", "sbag", "fenc",
        "cycl", "afld", "barb",
        // Ships
        "msub", "turr", "mgun", "ssam",
        // Aircraft
        "badr", "u2", "lrotor", "rrotor",
        // Building construction animations
        "factmake", "procmake", "powrmake", "apwrmake", "barrmake",
        "domemake", "weapmake", "gunmake", "agunmake", "sammake",
        "fturmake", "tslamake", "pboxmake", "stekmake", "atekmake",
        "hpadmake", "fixmake", "gapmake", "ironmake", "spenmake",
        "syrdmake", "afldmake", "silomake", "tentmake", "kennmake",
        "pdoxmake", "fcommake", "hospmake", "biomake", "missmake",
        "mslomake",
        // Building death sprites
        "factdead", "powrdead", "apwrdead", "procdead",
        // Effects & explosions
        "piff", "piffpiff", "veh-hit1", "veh-hit2", "veh-hit3",
        "flak", "h2o_exp1", "h2o_exp2", "h2o_exp3", "art-exp1",
        "fball1", "frag1", "smoke_m", "burn-l", "burn-m", "burn-s",
        "fire1", "fire2", "fire3", "fire4", "clock", "speed",
        "120mm", "50cal", "v2", "minigun", "litning",
        // Resources
        "gold01", "gold02", "gold03", "gold04",
        "gem01", "gem02", "gem03", "gem04",
        // Misc
        "pips", "select", "flagfly", "parach", "bomblet",
        "scrate", "wcrate",
        // Building bibs
        "bib1", "bib2", "bib3",
        // Weapon overlays
        "gunfire2", "samfire",
        // Ship turrets
        "turr", "mgun", "ssam",
        // Additional vehicles/units
        "mcvhusk", "hhusk", "hhusk2",
        // Icons
        "e1icon", "e2icon", "e3icon", "e4icon", "e7icon",
        "dogicon", "spyicon", "thficon", "mediicon", "mechicon",
        "shokicon",
    ];

    let mix_files = [
        "conquer.mix", "allies.mix", "russian.mix",
        "hires.mix", "lores.mix", "local.mix", "interior.mix",
    ];

    let mut found = Vec::new();
    let mut not_found = Vec::new();

    // Load all MIX archives
    let mut archives = Vec::new();
    for mix_name in &mix_files {
        let mix_path = format!("{}{}", mix_dir, mix_name);
        if let Ok(data) = std::fs::read(&mix_path) {
            if let Ok(archive) = mix::MixArchive::parse(data) {
                archives.push((mix_name.to_string(), archive));
            }
        }
    }

    for name in sprites {
        let filename = format!("{}.shp", name);
        let out_path = format!("{}{}", out_dir, filename);

        // Skip if already extracted
        if std::path::Path::new(&out_path).exists() {
            found.push(format!("{} (already exists)", name));
            continue;
        }

        let mut extracted = false;
        for (mix_name, archive) in &archives {
            if let Some(data) = archive.get(&filename) {
                if let Ok(shp) = openra_data::shp::decode(data) {
                    std::fs::write(&out_path, data).unwrap();
                    found.push(format!("{} from {} ({}x{}, {} frames)",
                        name, mix_name, shp.width, shp.height, shp.frames.len()));
                    extracted = true;
                    break;
                }
            }
        }
        if !extracted {
            not_found.push(name.to_string());
        }
    }

    println!("\n=== EXTRACTION RESULTS ===");
    println!("\nFound ({}):", found.len());
    for f in &found {
        println!("  ✓ {}", f);
    }
    println!("\nNot found ({}):", not_found.len());
    for n in &not_found {
        println!("  ✗ {}", n);
    }
    println!("\nTotal: {} found, {} missing", found.len(), not_found.len());
}
