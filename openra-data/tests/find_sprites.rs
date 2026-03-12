//! Find which MIX file contains each sprite.

use openra_data::mix;

const MIX_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/");

fn load_mix(name: &str) -> Option<mix::MixArchive> {
    let path = format!("{}{}", MIX_DIR, name);
    let data = std::fs::read(&path).ok()?;
    mix::MixArchive::parse(data).ok()
}

#[test]
fn find_all_sprites() {
    let mix_names = [
        "conquer.mix", "allies.mix", "russian.mix", "temperat.mix",
        "interior.mix", "hires.mix", "lores.mix", "local.mix",
        "snow.mix",
    ];

    let sprites = [
        // Infantry
        "e1.shp", "e2.shp", "e3.shp", "e4.shp", "e6.shp", "e7.shp",
        "dog.shp", "spy.shp", "thf.shp", "medi.shp", "shok.shp",
        // Vehicles
        "1tnk.shp", "2tnk.shp", "3tnk.shp", "4tnk.shp",
        "harv.shp", "mcv.shp", "jeep.shp", "apc.shp",
        "v2rl.shp", "arty.shp", "mnly.shp", "mrj.shp",
        "ttnk.shp", "ftrk.shp", "truk.shp", "stnk.shp",
        // Buildings
        "powr.shp", "apwr.shp", "barr.shp", "fact.shp",
        "proc.shp", "weap.shp", "dome.shp", "gun.shp",
        "tent.shp", "sam2.shp", "tsla.shp", "pbox.shp",
        "gap.shp", "iron.shp", "fix.shp", "silo.shp",
        "atek.shp", "stek.shp", "ftur.shp",
        "proctop.shp", "weap2.shp",
        // Naval
        "ss.shp", "dd.shp", "ca.shp", "pt.shp", "lst.shp",
        // Aircraft
        "heli.shp", "hind.shp", "yak.shp", "mig.shp", "tran.shp",
        // Terrain
        "clear1.tem",
        // Misc
        "nopower.shp",
    ];

    let mixes: Vec<_> = mix_names
        .iter()
        .filter_map(|name| Some((*name, load_mix(name)?)))
        .collect();

    for sprite in &sprites {
        let mut locations = Vec::new();
        for (mix_name, mix) in &mixes {
            if mix.contains(sprite) {
                locations.push(*mix_name);
            }
        }
        if locations.is_empty() {
            println!("{}: NOT FOUND", sprite);
        } else {
            println!("{}: {}", sprite, locations.join(", "));
        }
    }
}
