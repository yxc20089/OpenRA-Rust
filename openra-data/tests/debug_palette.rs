//! Debug palette and sprite rendering issues.

use openra_data::{mix, shp, palette};

#[test]
fn inspect_palette() {
    let pal_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vendor/OpenRA/mods/ra/maps/chernobyl/temperat.pal"
    );
    let data = std::fs::read(pal_path).unwrap();
    println!("Palette file size: {} bytes", data.len());

    // Check if values are 6-bit (0-63) or 8-bit (0-255)
    let max_val = *data.iter().take(768).max().unwrap();
    let vals_over_63 = data.iter().take(768).filter(|&&v| v > 63).count();
    println!("Max palette value: {}", max_val);
    println!("Values > 63: {} out of 768", vals_over_63);

    // Show first 16 entries raw
    for i in 0..16 {
        let r = data[i * 3];
        let g = data[i * 3 + 1];
        let b = data[i * 3 + 2];
        println!("  [{:3}] raw: ({:3}, {:3}, {:3})", i, r, g, b);
    }

    // Show some middle entries
    for i in [80, 81, 82, 100, 120, 150, 200, 255] {
        let r = data[i * 3];
        let g = data[i * 3 + 1];
        let b = data[i * 3 + 2];
        println!("  [{:3}] raw: ({:3}, {:3}, {:3})", i, r, g, b);
    }
}

#[test]
fn inspect_sprite_pixels() {
    let mix_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/");
    let data = std::fs::read(format!("{}conquer.mix", mix_dir)).unwrap();
    let conquer = mix::MixArchive::parse(data).unwrap();

    // Check MCV sprite frame 0
    let mcv_data = conquer.get("mcv.shp").unwrap();
    let mcv = shp::decode(mcv_data).unwrap();
    let frame = &mcv.frames[0];

    println!("MCV frame 0: {}x{}, {} pixels", frame.width, frame.height, frame.pixels.len());

    // Count unique pixel values
    let mut counts = [0u32; 256];
    for &px in &frame.pixels {
        counts[px as usize] += 1;
    }

    let nonzero: Vec<_> = counts.iter().enumerate()
        .filter(|&(_, c)| *c > 0)
        .map(|(i, c)| (i, *c))
        .collect();

    println!("Unique pixel indices: {}", nonzero.len());
    println!("Index 0 (transparent): {} pixels", counts[0]);
    for &(idx, count) in &nonzero {
        if idx > 0 && count > 10 {
            println!("  Index {:3}: {:4} pixels", idx, count);
        }
    }

    // Also check fact.shp
    let fact_data = conquer.get("fact.shp").unwrap();
    let fact = shp::decode(fact_data).unwrap();
    println!("\nFACT: {}x{}, {} frames", fact.width, fact.height, fact.frames.len());
    let f0 = &fact.frames[0];
    let mut fcounts = [0u32; 256];
    for &px in &f0.pixels {
        fcounts[px as usize] += 1;
    }
    let fnonzero: Vec<_> = fcounts.iter().enumerate()
        .filter(|&(_, c)| *c > 0)
        .map(|(i, c)| (i, *c))
        .collect();
    println!("Unique pixel indices: {}", fnonzero.len());
    println!("Index 0 (transparent): {} pixels", fcounts[0]);

    // Check 1tnk.shp
    let tnk_data = conquer.get("1tnk.shp").unwrap();
    let tnk = shp::decode(tnk_data).unwrap();
    println!("\n1TNK: {}x{}, {} frames", tnk.width, tnk.height, tnk.frames.len());
    let t0 = &tnk.frames[0];
    let mut tcounts = [0u32; 256];
    for &px in &t0.pixels {
        tcounts[px as usize] += 1;
    }
    let total_nonzero: u32 = tcounts.iter().skip(1).sum();
    println!("Non-transparent pixels: {}/{}", total_nonzero, t0.pixels.len());
}

#[test]
fn render_test_sprite_to_ppm() {
    let pal_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vendor/OpenRA/mods/ra/maps/chernobyl/temperat.pal"
    );
    let pal_data = std::fs::read(pal_path).unwrap();

    // Try both 6-bit and 8-bit
    let pal_6bit = palette::Palette::from_bytes(&pal_data).unwrap();
    let pal_8bit = palette::Palette::from_bytes_8bit(&pal_data).unwrap();

    let mix_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/");
    let data = std::fs::read(format!("{}conquer.mix", mix_dir)).unwrap();
    let conquer = mix::MixArchive::parse(data).unwrap();

    let mcv_data = conquer.get("mcv.shp").unwrap();
    let mcv = shp::decode(mcv_data).unwrap();
    let frame = &mcv.frames[0];

    // Print a few pixels with both palettes
    println!("Comparing palette rendering for MCV frame 0:");
    for y in 0..5 {
        for x in 0..10 {
            let idx = frame.pixels[y * frame.width as usize + x];
            if idx > 0 {
                let c6 = pal_6bit.rgba(idx);
                let c8 = pal_8bit.rgba(idx);
                println!("  px[{},{}] idx={:3} 6bit=({:3},{:3},{:3}) 8bit=({:3},{:3},{:3})",
                    x, y, idx, c6[0], c6[1], c6[2], c8[0], c8[1], c8[2]);
            }
        }
    }

    // Write PPM files for visual comparison
    let out_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/extracted-sprites/");
    for (name, pal) in [("mcv_6bit.ppm", &pal_6bit), ("mcv_8bit.ppm", &pal_8bit)] {
        let w = frame.width as usize;
        let h = frame.height as usize;
        let mut ppm = format!("P3\n{} {}\n255\n", w, h);
        for &px in &frame.pixels {
            let c = pal.rgba(px);
            if c[3] == 0 {
                ppm += "40 80 40 "; // green background for transparent
            } else {
                ppm += &format!("{} {} {} ", c[0], c[1], c[2]);
            }
        }
        std::fs::write(format!("{}{}", out_dir, name), &ppm).ok();
        println!("Wrote {}", name);
    }
}
