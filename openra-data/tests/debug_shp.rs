//! Debug SHP decoding issues.

use openra_data::mix;

#[test]
fn debug_shp_format() {
    let mix_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/");
    let data = std::fs::read(format!("{}conquer.mix", mix_dir)).unwrap();
    let conquer = mix::MixArchive::parse(data).unwrap();

    for name in &["1tnk.shp", "mcv.shp", "fact.shp", "powr.shp"] {
        let shp_data = conquer.get(name).unwrap();
        println!("\n=== {} ({} bytes) ===", name, shp_data.len());

        // Parse header manually
        let num_images = u16::from_le_bytes([shp_data[0], shp_data[1]]);
        let x = u16::from_le_bytes([shp_data[2], shp_data[3]]);
        let y = u16::from_le_bytes([shp_data[4], shp_data[5]]);
        let w = u16::from_le_bytes([shp_data[6], shp_data[7]]);
        let h = u16::from_le_bytes([shp_data[8], shp_data[9]]);
        println!("Header: {} images, pos=({},{}), size={}x{}", num_images, x, y, w, h);

        // Parse first few offset entries
        let table_start = 10;
        for i in 0..std::cmp::min(4, num_images as usize + 2) {
            let base = table_start + i * 8;
            let offset = shp_data[base] as u32
                | ((shp_data[base + 1] as u32) << 8)
                | ((shp_data[base + 2] as u32) << 16);
            let format = shp_data[base + 3];
            let ref_offset = shp_data[base + 4] as u32
                | ((shp_data[base + 5] as u32) << 8)
                | ((shp_data[base + 6] as u32) << 16);
            let ref_format = shp_data[base + 7];
            println!("  Entry {}: offset={}, format=0x{:02X}, ref_offset={}, ref_format=0x{:02X}",
                i, offset, format, ref_offset, ref_format);

            // Show first bytes at offset
            if (offset as usize) < shp_data.len() {
                let start = offset as usize;
                let end = std::cmp::min(start + 20, shp_data.len());
                let bytes: Vec<String> = shp_data[start..end].iter()
                    .map(|b| format!("{:02X}", b))
                    .collect();
                println!("    Data at offset: {}", bytes.join(" "));
            }
        }

        // Try to understand the actual decompressed size
        let pixel_count = w as usize * h as usize;
        println!("Expected pixel count per frame: {}", pixel_count);
    }
}

#[test]
fn debug_bits_shp() {
    // Compare with a known-working SHP from bits/
    let bits_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/OpenRA/mods/ra/bits/");

    for name in &["fact.shp", "harv.shp", "e6.shp"] {
        let path = format!("{}{}", bits_dir, name);
        let shp_data = std::fs::read(&path).unwrap();
        println!("\n=== bits/{} ({} bytes) ===", name, shp_data.len());

        let num_images = u16::from_le_bytes([shp_data[0], shp_data[1]]);
        let x = u16::from_le_bytes([shp_data[2], shp_data[3]]);
        let y = u16::from_le_bytes([shp_data[4], shp_data[5]]);
        let w = u16::from_le_bytes([shp_data[6], shp_data[7]]);
        let h = u16::from_le_bytes([shp_data[8], shp_data[9]]);
        println!("Header: {} images, pos=({},{}), size={}x{}", num_images, x, y, w, h);

        let table_start = 10;
        for i in 0..std::cmp::min(4, num_images as usize + 2) {
            let base = table_start + i * 8;
            let offset = shp_data[base] as u32
                | ((shp_data[base + 1] as u32) << 8)
                | ((shp_data[base + 2] as u32) << 16);
            let format = shp_data[base + 3];
            println!("  Entry {}: offset={}, format=0x{:02X}", i, offset, format);

            if (offset as usize) < shp_data.len() {
                let start = offset as usize;
                let end = std::cmp::min(start + 20, shp_data.len());
                let bytes: Vec<String> = shp_data[start..end].iter()
                    .map(|b| format!("{:02X}", b))
                    .collect();
                println!("    Data: {}", bytes.join(" "));
            }
        }

        // Decode and check frame 0
        let decoded = openra_data::shp::decode(&shp_data).unwrap();
        let f = &decoded.frames[0];
        let nonzero: usize = f.pixels.iter().filter(|&&p| p != 0).count();
        let total = f.pixels.len();
        println!("Frame 0: {}/{} non-transparent ({:.1}%)", nonzero, total, 100.0 * nonzero as f64 / total as f64);
    }
}
