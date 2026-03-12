//! Generate visual test images to verify sprite rendering.

use openra_data::{mix, shp, palette};

#[test]
fn render_sprite_sheet() {
    let mix_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/");
    let data = std::fs::read(format!("{}conquer.mix", mix_dir)).unwrap();
    let conquer = mix::MixArchive::parse(data).unwrap();

    let pal_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vendor/OpenRA/mods/ra/maps/chernobyl/temperat.pal"
    );
    let pal_data = std::fs::read(pal_path).unwrap();
    let pal = palette::Palette::from_bytes(&pal_data).unwrap();

    let sprites = ["1tnk.shp", "2tnk.shp", "mcv.shp", "fact.shp", "powr.shp", "harv.shp", "jeep.shp"];

    // Render a grid of first frames
    let grid_cols = 4;
    let cell_size = 96; // Max sprite will be 72x72
    let grid_rows = (sprites.len() + grid_cols - 1) / grid_cols;
    let img_w = grid_cols * cell_size;
    let img_h = grid_rows * cell_size;

    let mut ppm = format!("P3\n{} {}\n255\n", img_w, img_h);
    let mut pixels = vec![(40u8, 80u8, 40u8); img_w * img_h]; // green bg

    for (idx, name) in sprites.iter().enumerate() {
        let col = idx % grid_cols;
        let row = idx / grid_cols;
        let ox = col * cell_size;
        let oy = row * cell_size;

        if let Some(shp_data) = conquer.get(name) {
            let shp_file = shp::decode(shp_data).unwrap();
            let frame = &shp_file.frames[0];
            // Center in cell
            let fx = ox + (cell_size - frame.width as usize) / 2;
            let fy = oy + (cell_size - frame.height as usize) / 2;

            for py in 0..frame.height as usize {
                for px in 0..frame.width as usize {
                    let pi = py * frame.width as usize + px;
                    let idx = frame.pixels[pi];
                    if idx != 0 {
                        let c = pal.rgba(idx);
                        let dest = (fy + py) * img_w + (fx + px);
                        if dest < pixels.len() {
                            pixels[dest] = (c[0], c[1], c[2]);
                        }
                    }
                }
            }
        }
    }

    for (r, g, b) in &pixels {
        ppm += &format!("{} {} {} ", r, g, b);
    }

    let out_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vendor/extracted-sprites/sprite_sheet.ppm"
    );
    std::fs::write(out_path, &ppm).unwrap();
    println!("Wrote sprite sheet to {}", out_path);
}
