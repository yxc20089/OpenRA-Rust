//! Extract icon sprites from conquer.mix and hires.mix
use openra_data::mix;

const MIX_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/");
const OUT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/extracted-sprites/");

fn load_mix(name: &str) -> Option<mix::MixArchive> {
    let path = format!("{}{}", MIX_DIR, name);
    let data = std::fs::read(&path).ok()?;
    mix::MixArchive::parse(data).ok()
}

#[test]
fn extract_icon_sprites() {
    let icons = [
        // Building icons
        "facticon.shp", "powricon.shp", "apwricon.shp", "tenticon.shp",
        "barricon.shp", "procicon.shp", "weapicon.shp", "domeicon.shp",
        "fixicon.shp", "hpadicon.shp", "afldicon.shp", "samicon.shp",
        "agunicon.shp", "gunicon.shp", "fturicon.shp", "tslaicon.shp",
        "pboxicon.shp", "gapicon.shp", "ironicon.shp", "siloicon.shp",
        "atekicon.shp", "stekicon.shp", "kennicon.shp", "pdoxicon.shp",
        "spenicon.shp", "syrdicon.shp", "missicon.shp", "bioicon.shp",
        "fcomicon.shp", "brikicon.shp", "sbagicon.shp", "fencicon.shp",
        "cyclicon.shp", "barbicon.shp",
        // Vehicle icons
        "mcvicon.shp", "harvicon.shp", "1tnkicon.shp", "2tnkicon.shp",
        "3tnkicon.shp", "4tnkicon.shp", "v2rlicon.shp", "artyicon.shp",
        "jeepicon.shp", "apcicon.shp", "mnlyicon.shp", "mrjicon.shp",
        "mggicon.shp", "trukicon.shp",
        // Aircraft icons
        "migicon.shp", "yakicon.shp", "heliicon.shp", "hindicon.shp",
        "tranicon.shp",
        // Ship icons
        "ssicon.shp", "caicon.shp", "ddicon.shp", "pticon.shp", "lsticon.shp",
        // Infantry icons (some already extracted from hires.mix)
        "e1icon.shp", "e2icon.shp", "e3icon.shp", "e4icon.shp", "e6icon.shp",
        "e7icon.shp", "spyicon.shp", "thficon.shp", "mediicon.shp",
        "mechicon.shp", "shokicon.shp", "dogicon.shp",
    ];

    let mixes = ["conquer.mix", "hires.mix", "lores.mix", "allies.mix", "russian.mix"];
    let mut found = 0;
    let mut not_found = Vec::new();

    for icon in &icons {
        let out_path = format!("{}{}", OUT_DIR, icon);
        if std::path::Path::new(&out_path).exists() {
            found += 1;
            continue;
        }

        let mut extracted = false;
        for mix_name in &mixes {
            if let Some(archive) = load_mix(mix_name) {
                if let Some(data) = archive.get(icon) {
                    std::fs::write(&out_path, data).unwrap();
                    println!("Extracted {} from {} ({} bytes)", icon, mix_name, data.len());
                    found += 1;
                    extracted = true;
                    break;
                }
            }
        }
        if !extracted {
            not_found.push(*icon);
        }
    }

    println!("\nExtracted/found: {}, Not found: {}", found, not_found.len());
    for name in &not_found {
        println!("  MISSING: {}", name);
    }
}
