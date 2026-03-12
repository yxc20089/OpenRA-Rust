//! TMP RA terrain tile decoder.
//!
//! Decodes .tem/.sno/.des terrain tile files from Red Alert.
//!
//! Reference: OpenRA.Mods.Cnc/SpriteLoaders/TmpRALoader.cs

/// A decoded terrain tile file containing multiple sub-tiles.
#[derive(Debug, Clone)]
pub struct TmpFile {
    pub width: u16,
    pub height: u16,
    /// Sub-tiles. Each is `width * height` bytes of indexed pixel data, or None if empty.
    pub tiles: Vec<Option<Vec<u8>>>,
}

/// Decode a TMP RA format terrain tile file.
///
/// Header:
/// - offset 0: u16 width
/// - offset 2: u16 height
/// - offset 12: u32 imgStart
/// - offset 24: i32 indexEnd
/// - offset 28: i32 indexStart
/// (some bytes between are unused/padding)
pub fn decode(data: &[u8]) -> Result<TmpFile, String> {
    if data.len() < 32 {
        return Err("TMP file too small".into());
    }

    let width = read_u16(data, 0);
    let height = read_u16(data, 2);

    if width == 0 || height == 0 {
        return Err("TMP has zero dimensions".into());
    }

    // C# TmpRALoader reads: skip 12 bytes, u32 imgStart, skip 8, i32 indexEnd, skip 4, i32 indexStart
    // That means: imgStart at offset 16, indexEnd at offset 28, indexStart at offset 36
    let img_start = read_u32(data, 16) as usize;
    let index_end = read_i32(data, 28) as usize;
    let index_start = read_i32(data, 36) as usize;

    if index_start >= data.len() || index_end > data.len() || index_end < index_start {
        return Err(format!(
            "TMP index range out of bounds: start={}, end={}, file_size={}",
            index_start, index_end, data.len()
        ));
    }

    let count = index_end - index_start;
    let tile_size = width as usize * height as usize;
    let mut tiles = Vec::with_capacity(count);

    for i in 0..count {
        let idx = data[index_start + i];
        if idx == 255 {
            tiles.push(None);
        } else {
            let offset = img_start + idx as usize * tile_size;
            if offset + tile_size <= data.len() {
                tiles.push(Some(data[offset..offset + tile_size].to_vec()));
            } else {
                tiles.push(None);
            }
        }
    }

    Ok(TmpFile {
        width,
        height,
        tiles,
    })
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn read_i32(data: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_clear_terrain() {
        let mix_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/");
        let mix_data = std::fs::read(format!("{}temperat.mix", mix_dir));
        if let Ok(data) = mix_data {
            let mix = crate::mix::MixArchive::parse(data).unwrap();
            if let Some(clear_data) = mix.get("clear1.tem") {
                let tmp = decode(clear_data).unwrap();
                println!("clear1.tem: {}x{}, {} tiles", tmp.width, tmp.height, tmp.tiles.len());
                let non_empty: usize = tmp.tiles.iter().filter(|t| t.is_some()).count();
                println!("Non-empty tiles: {}/{}", non_empty, tmp.tiles.len());
                assert!(non_empty > 0, "clear1.tem should have tile data");
            }
        }
    }

    #[test]
    fn decode_gold_as_shp() {
        // Gold/gem/mine .tem files are actually in SHP format, not TMP RA format
        let mix_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/");
        let mix_data = std::fs::read(format!("{}temperat.mix", mix_dir));
        if let Ok(data) = mix_data {
            let mix = crate::mix::MixArchive::parse(data).unwrap();
            for name in &["gold01.tem", "gold02.tem", "gem01.tem", "mine.tem"] {
                if let Some(d) = mix.get(name) {
                    let shp = crate::shp::decode(d).unwrap();
                    assert!(shp.width == 24 && shp.height == 24,
                        "{} should be 24x24, got {}x{}", name, shp.width, shp.height);
                    assert!(!shp.frames.is_empty(), "{} should have frames", name);
                }
            }
        }
    }
}
