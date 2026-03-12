//! Extract infantry SHP sprites from MIX archives
use openra_data::mix;

#[test]
fn extract_infantry_sprites() {
    let out_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/extracted-sprites/");
    let mix_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/");

    let infantry = ["e1", "e2", "e3", "e4", "e7", "spy", "medi", "thf"];
    let mix_files = ["conquer.mix", "hires.mix", "lores.mix", "allies.mix", "russian.mix", "local.mix"];

    for mix_name in &mix_files {
        let mix_path = format!("{}{}", mix_dir, mix_name);
        let mix_data = match std::fs::read(&mix_path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let archive = match mix::MixArchive::parse(mix_data) {
            Ok(a) => a,
            Err(_) => continue,
        };

        for name in &infantry {
            let filename = format!("{}.shp", name);
            if let Some(data) = archive.get(&filename) {
                if let Ok(shp) = openra_data::shp::decode(data) {
                    let out_path = format!("{}{}", out_dir, filename);
                    std::fs::write(&out_path, data).unwrap();
                    println!("Found {} in {} ({}x{}, {} frames)", filename, mix_name, shp.width, shp.height, shp.frames.len());
                }
            }
        }
    }
}
