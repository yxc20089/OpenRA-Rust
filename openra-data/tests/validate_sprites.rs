//! Validate all SHP sprites from both bits/ and extracted-sprites/ decode correctly.
use openra_data::shp;

#[test]
fn validate_all_shp_sprites() {
    let dirs = [
        concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/extracted-sprites/"),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/OpenRA/mods/ra/bits/"),
    ];

    let mut ok = 0;
    let mut fail = 0;

    for dir in &dirs {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().map(|e| e == "shp").unwrap_or(false) {
                let data = std::fs::read(&path).unwrap();
                let name = path.file_stem().unwrap().to_string_lossy();
                match shp::decode(&data) {
                    Ok(shp) => {
                        assert!(shp.frames.len() > 0, "{}: 0 frames", name);
                        assert!(shp.width > 0 && shp.width < 512, "{}: bad width {}", name, shp.width);
                        assert!(shp.height > 0 && shp.height < 512, "{}: bad height {}", name, shp.height);
                        ok += 1;
                    }
                    Err(e) => {
                        println!("FAIL {}: {}", name, e);
                        fail += 1;
                    }
                }
            }
        }
    }

    println!("\nSprite validation: {} ok, {} failed", ok, fail);
    assert_eq!(fail, 0, "{} sprites failed to decode", fail);
}
