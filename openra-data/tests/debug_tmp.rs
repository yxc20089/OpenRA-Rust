//! Debug TMP file format.

use openra_data::mix;

#[test]
fn inspect_clear_tem() {
    let mix_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/");
    let data = std::fs::read(format!("{}temperat.mix", mix_dir)).unwrap();
    let mix = mix::MixArchive::parse(data).unwrap();

    let clear_data = mix.get("clear1.tem").unwrap();
    println!("clear1.tem: {} bytes", clear_data.len());

    // Dump first 40 bytes as hex
    for i in 0..40.min(clear_data.len()) {
        print!("{:02X} ", clear_data[i]);
        if (i + 1) % 16 == 0 { println!(); }
    }
    println!();

    // Read header fields
    let width = u16::from_le_bytes([clear_data[0], clear_data[1]]);
    let height = u16::from_le_bytes([clear_data[2], clear_data[3]]);
    println!("width={}, height={}", width, height);

    // Show u16/u32 at various offsets
    for off in (4..36).step_by(4) {
        if off + 4 <= clear_data.len() {
            let v16a = u16::from_le_bytes([clear_data[off], clear_data[off+1]]);
            let v16b = u16::from_le_bytes([clear_data[off+2], clear_data[off+3]]);
            let v32 = u32::from_le_bytes([clear_data[off], clear_data[off+1], clear_data[off+2], clear_data[off+3]]);
            println!("offset {:2}: u16={:5}, {:5} | u32={}", off, v16a, v16b, v32);
        }
    }

    // Try the C# offsets:
    // offset 12: u32 imgStart
    // offset 20: u32 (should be 0 per magic check)
    // offset 24: i32 indexEnd
    // offset 28: i32 indexStart
    let img_start = u32::from_le_bytes([clear_data[12], clear_data[13], clear_data[14], clear_data[15]]);
    let magic_a = u32::from_le_bytes([clear_data[20], clear_data[21], clear_data[22], clear_data[23]]);
    let magic_b = u16::from_le_bytes([clear_data[25], clear_data[26]]);
    let index_end = i32::from_le_bytes([clear_data[24], clear_data[25], clear_data[26], clear_data[27]]);
    let index_start = i32::from_le_bytes([clear_data[28], clear_data[29], clear_data[30], clear_data[31]]);

    println!("\nimgStart={}, magic_a=0x{:08X}, magic_b=0x{:04X}", img_start, magic_a, magic_b);
    println!("indexEnd={}, indexStart={}", index_end, index_start);

    // Calculate expected tile count: 9289 bytes total
    // With 24x24 tiles, each tile = 576 bytes
    // (9289 - header) / 576 = about 16 tiles
    println!("\nTile size: {}x{} = {} bytes per tile", width, height, width as usize * height as usize);
    println!("Approx tiles if data starts at imgStart={}: {}", img_start,
        (clear_data.len() - img_start as usize) / (width as usize * height as usize));
}
