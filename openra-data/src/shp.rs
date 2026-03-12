//! SHP TD format sprite decoder.
//!
//! Decodes SHP files from the Tiberian Dawn / Red Alert engine format.
//! Each SHP file contains multiple frames of 8-bit indexed pixel data.
//! Global width/height apply to all frames.
//!
//! Reference: OpenRA.Mods.Common/SpriteLoaders/ShpTDLoader.cs

/// A single sprite frame.
#[derive(Debug, Clone)]
pub struct SpriteFrame {
    pub width: u16,
    pub height: u16,
    /// 8-bit indexed pixel data, row-major. Length = width * height.
    pub pixels: Vec<u8>,
}

/// A decoded SHP file containing multiple frames.
#[derive(Debug, Clone)]
pub struct ShpFile {
    pub width: u16,
    pub height: u16,
    pub frames: Vec<SpriteFrame>,
}

/// Decode an SHP TD format file from raw bytes.
///
/// File layout:
/// - Header: u16 num_images, u16 x, u16 y, u16 width, u16 height (10 bytes)
/// - Offset table: (num_images + 2) entries, each 8 bytes
///   - 3 bytes offset (LE) + 1 byte format + 3 bytes ref_offset + 1 byte ref_format
/// - Frame data: Format80 (LCW) compressed pixel data at each offset
pub fn decode(data: &[u8]) -> Result<ShpFile, String> {
    if data.len() < 14 {
        return Err("SHP file too small".into());
    }

    // Header: 5 x u16
    let num_images = read_u16(data, 0) as usize;
    let _x = read_u16(data, 2);
    let _y = read_u16(data, 4);
    let width = read_u16(data, 6);
    let height = read_u16(data, 8);

    if num_images == 0 {
        return Err("SHP file has zero frames".into());
    }

    // Offset table starts at byte 10
    let table_start = 10;
    let entry_size = 8;
    let table_end = table_start + (num_images + 2) * entry_size;

    if data.len() < table_end {
        return Err("SHP file truncated in offset table".into());
    }

    let mut offsets = Vec::with_capacity(num_images + 2);
    for i in 0..(num_images + 2) {
        let base = table_start + i * entry_size;
        let offset = read_u24(data, base) as usize;
        let format = data[base + 3];
        let ref_offset = read_u24(data, base + 4) as usize;
        let ref_format = data[base + 7];
        offsets.push(FrameHeader {
            offset,
            format,
            ref_offset,
            ref_format,
        });
    }

    let pixel_count = width as usize * height as usize;
    let mut frames = Vec::with_capacity(num_images);

    for i in 0..num_images {
        let hdr = &offsets[i];

        if pixel_count == 0 {
            frames.push(SpriteFrame {
                width,
                height,
                pixels: Vec::new(),
            });
            continue;
        }

        if hdr.offset >= data.len() {
            return Err(format!("Frame {} offset {} out of bounds (file size {})",
                i, hdr.offset, data.len()));
        }

        // Compressed data starts directly at the offset (no per-frame header)
        let compressed_data = &data[hdr.offset..];

        let pixels = if hdr.format & 0x80 != 0 {
            // Format80 (LCW)
            decode_format80(compressed_data, pixel_count)?
        } else if hdr.format & 0x40 != 0 {
            // XOR with base reference frame
            let base_pixels = decode_format80(compressed_data, pixel_count)?;
            let ref_data = &data[hdr.ref_offset..];
            let ref_pixels = decode_format80(ref_data, pixel_count)?;
            xor_buffers(&ref_pixels, &base_pixels)
        } else if hdr.format & 0x20 != 0 && i > 0 {
            // XOR with previous frame
            let base_pixels = decode_format80(compressed_data, pixel_count)?;
            let prev = &frames[i - 1].pixels;
            xor_buffers(prev, &base_pixels)
        } else {
            // Default: Format80
            decode_format80(compressed_data, pixel_count)?
        };

        frames.push(SpriteFrame {
            width,
            height,
            pixels,
        });
    }

    Ok(ShpFile { width, height, frames })
}

#[derive(Debug)]
struct FrameHeader {
    offset: usize,
    format: u8,
    ref_offset: usize,
    #[allow(dead_code)]
    ref_format: u8,
}

/// Decode Format80 (LCW) compressed data.
///
/// Command byte encoding:
/// - 0x80: end of data
/// - 0xFF nn nn pp pp: copy nn bytes from absolute dest position pp
/// - 0xFE nn nn vv: fill nn bytes with value vv
/// - 11cccccc pp pp: copy (c+3) bytes from absolute dest position pp
/// - 10cccccc dd: copy (c+3) bytes from relative position (current - dd)
///   Note: the 10xxxxxx form is NOT present in standard Format80.
///   Actually in Westwood's Format80, 10cccccc is just a shorter form:
///   copy (c+3) from relative offset stored in next byte as: (dest_pos - byte)
/// - 0ccccccc: copy c bytes directly from source stream
fn decode_format80(src: &[u8], max_output: usize) -> Result<Vec<u8>, String> {
    let mut dest = Vec::with_capacity(max_output);
    let mut i = 0;

    while i < src.len() && dest.len() < max_output {
        let cmd = src[i];
        i += 1;

        if cmd == 0x80 {
            break;
        } else if cmd == 0xFF {
            // Long copy from absolute dest position: 0xFF count_lo count_hi pos_lo pos_hi
            if i + 4 > src.len() { break; }
            let count = read_u16(src, i) as usize;
            let pos = read_u16(src, i + 2) as usize;
            i += 4;
            copy_from_dest(&mut dest, pos, count, max_output);
        } else if cmd == 0xFE {
            // Long fill: 0xFE count_lo count_hi value
            if i + 3 > src.len() { break; }
            let count = read_u16(src, i) as usize;
            let value = src[i + 2];
            i += 3;
            let actual = count.min(max_output - dest.len());
            dest.extend(std::iter::repeat(value).take(actual));
        } else if cmd & 0x80 != 0 {
            if cmd & 0x40 != 0 {
                // 11cccccc pos_lo pos_hi: copy (c+3) from absolute dest position
                let count = ((cmd & 0x3F) as usize) + 3;
                if i + 2 > src.len() { break; }
                let pos = read_u16(src, i) as usize;
                i += 2;
                copy_from_dest(&mut dest, pos, count, max_output);
            } else {
                // 10cccccc dd: copy (c+3) from relative dest position (current - dd)
                // Note: count field is (cmd & 0x3F), actual count = field + 3
                // But if field is 0 and next byte gives more info... no, just (c+3)
                let count = ((cmd & 0x3F) as usize) + 3;
                if i >= src.len() { break; }
                let offset_byte = src[i] as usize;
                i += 1;
                let pos = if dest.len() >= offset_byte {
                    dest.len() - offset_byte
                } else {
                    0
                };
                copy_from_dest(&mut dest, pos, count, max_output);
            }
        } else {
            // 0ccccccc: copy c bytes from source
            let count = cmd as usize;
            if count == 0 { break; }
            let actual = count.min(src.len() - i).min(max_output - dest.len());
            dest.extend_from_slice(&src[i..i + actual]);
            i += actual;
        }
    }

    // Pad to expected size
    dest.resize(max_output, 0);
    Ok(dest)
}

/// Copy `count` bytes from dest[pos..] back into dest, byte-by-byte to handle overlapping.
fn copy_from_dest(dest: &mut Vec<u8>, pos: usize, count: usize, max_output: usize) {
    for j in 0..count {
        if dest.len() >= max_output { break; }
        let src_idx = pos + j;
        if src_idx < dest.len() {
            dest.push(dest[src_idx]);
        } else {
            dest.push(0);
        }
    }
}

/// XOR two buffers together.
fn xor_buffers(base: &[u8], overlay: &[u8]) -> Vec<u8> {
    base.iter()
        .zip(overlay.iter())
        .map(|(a, b)| a ^ b)
        .collect()
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_u24(data: &[u8], offset: usize) -> u32 {
    data[offset] as u32
        | ((data[offset + 1] as u32) << 8)
        | ((data[offset + 2] as u32) << 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_helpers() {
        let data = [0x34, 0x12, 0xAB];
        assert_eq!(read_u16(&data, 0), 0x1234);
        assert_eq!(read_u24(&data, 0), 0xAB1234);
    }

    #[test]
    fn decode_empty_shp() {
        let data = [0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let result = decode(&data);
        assert!(result.is_err());
    }

    #[test]
    fn format80_direct_copy() {
        // 0x03 = copy 3 bytes from source, then 0x80 = end
        let src = [0x03, 0xAA, 0xBB, 0xCC, 0x80];
        let result = decode_format80(&src, 3).unwrap();
        assert_eq!(result, vec![0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn format80_fill() {
        // 0xFE, count=4, value=0x42
        let src = [0xFE, 0x04, 0x00, 0x42, 0x80];
        let result = decode_format80(&src, 4).unwrap();
        assert_eq!(result, vec![0x42, 0x42, 0x42, 0x42]);
    }
}
