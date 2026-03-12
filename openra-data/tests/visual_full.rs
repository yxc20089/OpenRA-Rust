//! Generate a full scene rendering for visual verification.

use openra_data::{mix, shp, palette, tmp};

#[test]
fn render_full_scene() {
    let mix_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/");
    let conquer_data = std::fs::read(format!("{}conquer.mix", mix_dir)).unwrap();
    let conquer = mix::MixArchive::parse(conquer_data).unwrap();
    let temperat_data = std::fs::read(format!("{}temperat.mix", mix_dir)).unwrap();
    let temperat = mix::MixArchive::parse(temperat_data).unwrap();

    let pal_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vendor/OpenRA/mods/ra/maps/chernobyl/temperat.pal"
    );
    let pal = palette::Palette::from_bytes(&std::fs::read(pal_path).unwrap()).unwrap();

    // Decode clear terrain
    let clear_data = temperat.get("clear1.tem").unwrap();
    let clear_tiles = tmp::decode(clear_data).unwrap();

    // Scene: 12x8 cells = 288x192 pixels
    let cols = 12;
    let rows = 8;
    let cell = 24;
    let img_w = cols * cell;
    let img_h = rows * cell;
    let mut pixels = vec![[0u8; 3]; img_w * img_h];

    // Fill with terrain tiles
    for cy in 0..rows {
        for cx in 0..cols {
            let tile_idx = ((cx * 7 + cy * 13) % clear_tiles.tiles.len()) as usize;
            if let Some(tile_data) = &clear_tiles.tiles[tile_idx] {
                for ty in 0..cell {
                    for tx in 0..cell {
                        let pi = ty * cell + tx;
                        let px_idx = tile_data[pi];
                        let c = pal.rgba(px_idx);
                        let dest = (cy * cell + ty) * img_w + (cx * cell + tx);
                        if c[3] > 0 {
                            pixels[dest] = [c[0], c[1], c[2]];
                        }
                    }
                }
            }
        }
    }

    // Draw some sprites on top
    let units = [
        ("fact", 1, 1),  // Factory at (1,1)
        ("powr", 5, 1),  // Power plant at (5,1)
        ("1tnk", 3, 4),  // Tank
        ("2tnk", 5, 5),  // Heavy tank
        ("mcv", 8, 3),   // MCV
        ("harv", 7, 5),  // Harvester
        ("jeep", 10, 4), // Jeep
    ];

    for (name, cx, cy) in &units {
        if let Some(shp_data) = conquer.get(&format!("{}.shp", name)) {
            if let Ok(shp_file) = shp::decode(shp_data) {
                let frame = &shp_file.frames[0];
                // Center sprite on cell
                let ox = cx * cell as i32 + (cell as i32 - frame.width as i32) / 2;
                let oy = cy * cell as i32 + (cell as i32 - frame.height as i32) / 2;
                for sy in 0..frame.height as usize {
                    for sx in 0..frame.width as usize {
                        let pi = sy * frame.width as usize + sx;
                        let idx = frame.pixels[pi];
                        if idx != 0 {
                            let c = pal.rgba(idx);
                            let dx = ox as usize + sx;
                            let dy = oy as usize + sy;
                            if dx < img_w && dy < img_h {
                                pixels[dy * img_w + dx] = [c[0], c[1], c[2]];
                            }
                        }
                    }
                }
            }
        }
    }

    // Write PPM
    let mut ppm = format!("P3\n{} {}\n255\n", img_w, img_h);
    for [r, g, b] in &pixels {
        ppm += &format!("{} {} {} ", r, g, b);
    }

    let out_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vendor/extracted-sprites/full_scene.ppm"
    );
    std::fs::write(out_path, &ppm).unwrap();
    println!("Wrote full scene to {}", out_path);
}
