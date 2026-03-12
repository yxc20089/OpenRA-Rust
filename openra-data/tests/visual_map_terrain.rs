//! Test rendering map terrain tiles using actual map data.

use openra_data::{mix, tmp, palette, oramap};

#[test]
fn render_map_terrain() {
    // Parse the bundled map
    let map_data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"), "/../tests/maps/singles.oramap"
    )).unwrap();
    let map = oramap::parse(&map_data).unwrap();
    println!("Map: {}x{}, tileset: {}, tiles: {}x{}",
        map.map_size.0, map.map_size.1, map.tileset,
        map.tiles.first().map(|r| r.len()).unwrap_or(0),
        map.tiles.len());

    // Parse tileset templates
    let tileset_yaml = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vendor/OpenRA/mods/ra/tilesets/temperat.yaml"
    )).unwrap();

    let mut templates: std::collections::HashMap<u16, String> = std::collections::HashMap::new();
    let mut current_id: Option<u16> = None;
    for line in tileset_yaml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Template@") {
            current_id = None;
        } else if trimmed.starts_with("Id:") && !trimmed.contains("TEMPERAT") {
            if let Ok(id) = trimmed[3..].trim().parse::<u16>() {
                current_id = Some(id);
            }
        } else if trimmed.starts_with("Images:") {
            if let Some(id) = current_id {
                templates.insert(id, trimmed[7..].trim().to_string());
            }
        }
    }
    println!("Templates: {}", templates.len());

    // Load temperat.mix
    let mix_data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/temperat.mix"
    )).unwrap();
    let tmix = mix::MixArchive::parse(mix_data).unwrap();

    // Decode all unique .tem files
    let mut tem_tiles: std::collections::HashMap<String, tmp::TmpFile> = std::collections::HashMap::new();
    for (_, filename) in &templates {
        if tem_tiles.contains_key(filename) { continue; }
        if let Some(data) = tmix.get(filename) {
            if let Ok(t) = tmp::decode(data) {
                tem_tiles.insert(filename.clone(), t);
            }
        }
    }
    println!("Decoded {} .tem files", tem_tiles.len());

    let pal = palette::Palette::from_bytes(&std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vendor/OpenRA/mods/ra/maps/chernobyl/temperat.pal"
    )).unwrap()).unwrap();

    // Render a 40x30 section of the map (around bounds)
    let bx = map.bounds.0 as usize;
    let by = map.bounds.1 as usize;
    let view_w = 40.min(map.bounds.2 as usize);
    let view_h = 30.min(map.bounds.3 as usize);
    let cell = 24;
    let img_w = view_w * cell;
    let img_h = view_h * cell;
    let mut pixels = vec![[40u8, 70, 40]; img_w * img_h];

    let mut drawn = 0;
    let mut missed = 0;

    for vy in 0..view_h {
        let row = by + vy;
        if row >= map.tiles.len() { continue; }
        for vx in 0..view_w {
            let col = bx + vx;
            if col >= map.tiles[row].len() { continue; }
            let tile_ref = map.tiles[row][col];

            let filename = templates.get(&tile_ref.type_id);
            if let Some(filename) = filename {
                if let Some(tmp_file) = tem_tiles.get(filename) {
                    let idx = tile_ref.index as usize;
                    if idx < tmp_file.tiles.len() {
                        if let Some(tile_data) = &tmp_file.tiles[idx] {
                            for ty in 0..cell {
                                for tx in 0..cell {
                                    let pi = ty * cell + tx;
                                    let px_idx = tile_data[pi];
                                    let c = if px_idx == 0 {
                                        [0, 0, 0]
                                    } else {
                                        pal.colors[px_idx as usize]
                                    };
                                    let dest = (vy * cell + ty) * img_w + (vx * cell + tx);
                                    pixels[dest] = c;
                                }
                            }
                            drawn += 1;
                            continue;
                        }
                    }
                }
            }
            missed += 1;
        }
    }

    println!("Drawn: {}, Missed: {}", drawn, missed);

    let mut ppm = format!("P3\n{} {}\n255\n", img_w, img_h);
    for [r, g, b] in &pixels {
        ppm += &format!("{} {} {} ", r, g, b);
    }

    let out_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vendor/extracted-sprites/map_terrain.ppm"
    );
    std::fs::write(out_path, &ppm).unwrap();
    println!("Wrote map terrain to {}", out_path);
}
