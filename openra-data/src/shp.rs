//! SHP TD format sprite decoder.
//!
//! Decodes SHP files from the Tiberian Dawn / Red Alert engine format.
//! Each SHP file contains multiple frames of 8-bit indexed pixel data.
//!
//! Reference: OpenRA.Mods.Cnc/SpriteLoaders/ShpTDLoader.cs

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

/// Format flag for each frame in the offset table.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Format {
    XorPrev = 0x20,
    XorLcw = 0x40,
    Lcw = 0x80,
}

impl Format {
    fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x20 => Some(Format::XorPrev),
            0x40 => Some(Format::XorLcw),
            0x80 => Some(Format::Lcw),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct FrameHeader {
    file_offset: u32,
    format: u8,
    ref_offset: u16,
    ref_format: u16,
}

/// Decode an SHP TD format file from raw bytes.
///
/// File layout (from OpenRA ShpTDLoader.cs):
/// - Header: u16 imageCount, u16 x, u16 y, u16 width, u16 height, u32 unknown (14 bytes)
/// - imageCount frame entries, each 8 bytes:
///   u32 (low 24 bits = offset, high 8 bits = format), u16 ref_offset, u16 ref_format
/// - 2 extra entries (eof marker + zeros) = 16 bytes
/// - Compressed frame data
pub fn decode(data: &[u8]) -> Result<ShpFile, String> {
    if data.len() < 14 {
        return Err("SHP file too small".into());
    }

    let num_images = read_u16(data, 0) as usize;
    let _x = read_u16(data, 2);
    let _y = read_u16(data, 4);
    let width = read_u16(data, 6);
    let height = read_u16(data, 8);
    // 4 unknown bytes at offset 10 (skip)

    if num_images == 0 {
        return Err("SHP file has zero frames".into());
    }

    let pixel_count = width as usize * height as usize;

    // Entries start at byte 14
    let entries_start = 14;
    let entry_size = 8;
    let entries_end = entries_start + num_images * entry_size;
    // After entries: 2 extra entries (eof + zeros) = 16 bytes
    let data_start = entries_end + 16;

    if entries_end > data.len() {
        return Err("SHP file truncated in offset table".into());
    }

    // Parse frame headers
    let mut headers: Vec<FrameHeader> = Vec::with_capacity(num_images);
    for i in 0..num_images {
        let base = entries_start + i * entry_size;
        let dword = read_u32(data, base);
        let file_offset = dword & 0xFFFFFF;
        let format = (dword >> 24) as u8;
        let ref_offset = read_u16(data, base + 4);
        let ref_format = read_u16(data, base + 6);
        headers.push(FrameHeader {
            file_offset,
            format,
            ref_offset,
            ref_format,
        });
    }

    // The compressed data is everything from data_start onwards.
    // FileOffset values are absolute from stream start.
    // shpBytesFileOffset = data_start
    let shp_bytes_offset = data_start;

    // Build a map from file_offset to header index for XORLCW resolution
    let mut offset_to_idx: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for (i, h) in headers.iter().enumerate() {
        offset_to_idx.entry(h.file_offset).or_insert(i);
    }

    // Resolve reference images for XOR formats
    let mut ref_images: Vec<Option<usize>> = vec![None; num_images];
    for i in 0..num_images {
        let fmt = headers[i].format;
        if fmt == Format::XorPrev as u8 {
            if i > 0 {
                ref_images[i] = Some(i - 1);
            }
        } else if fmt == Format::XorLcw as u8 {
            if let Some(&idx) = offset_to_idx.get(&(headers[i].ref_offset as u32)) {
                ref_images[i] = Some(idx);
            }
        }
    }

    // Decompress all frames
    let mut frame_data: Vec<Option<Vec<u8>>> = vec![None; num_images];

    fn decompress_frame(
        idx: usize,
        headers: &[FrameHeader],
        ref_images: &[Option<usize>],
        frame_data: &mut Vec<Option<Vec<u8>>>,
        data: &[u8],
        shp_bytes_offset: usize,
        pixel_count: usize,
        depth: usize,
    ) {
        if depth > headers.len() || frame_data[idx].is_some() || pixel_count == 0 {
            return;
        }

        let h = &headers[idx];
        let src_offset = if h.file_offset as usize >= shp_bytes_offset {
            h.file_offset as usize - shp_bytes_offset
        } else {
            // FileOffset is absolute from file start, but data array starts at shp_bytes_offset
            // If offset < shp_bytes_offset, the data is in the header area
            h.file_offset as usize
        };

        let fmt = h.format;

        if fmt == Format::Lcw as u8 {
            // Direct LCW decompression
            let mut dest = vec![0u8; pixel_count];
            if shp_bytes_offset + src_offset <= data.len() {
                lcw_decode_into(&data[shp_bytes_offset..], &mut dest, src_offset);
            }
            frame_data[idx] = Some(dest);
        } else if fmt == Format::XorPrev as u8 || fmt == Format::XorLcw as u8 {
            // XOR delta - need reference frame first
            if let Some(ref_idx) = ref_images[idx] {
                if frame_data[ref_idx].is_none() {
                    decompress_frame(
                        ref_idx, headers, ref_images, frame_data, data,
                        shp_bytes_offset, pixel_count, depth + 1,
                    );
                }
                // Copy reference data and XOR delta into it
                let mut dest = frame_data[ref_idx]
                    .as_ref()
                    .map(|d| d.clone())
                    .unwrap_or_else(|| vec![0u8; pixel_count]);
                if shp_bytes_offset + src_offset <= data.len() {
                    xor_delta_decode_into(&data[shp_bytes_offset..], &mut dest, src_offset);
                }
                frame_data[idx] = Some(dest);
            } else {
                // No reference found, produce empty frame
                frame_data[idx] = Some(vec![0u8; pixel_count]);
            }
        } else {
            // Unknown format, try LCW as fallback
            let mut dest = vec![0u8; pixel_count];
            if shp_bytes_offset + src_offset <= data.len() {
                lcw_decode_into(&data[shp_bytes_offset..], &mut dest, src_offset);
            }
            frame_data[idx] = Some(dest);
        }
    }

    for i in 0..num_images {
        decompress_frame(
            i, &headers, &ref_images, &mut frame_data, data,
            shp_bytes_offset, pixel_count, 0,
        );
    }

    let frames: Vec<SpriteFrame> = frame_data
        .into_iter()
        .map(|d| SpriteFrame {
            width,
            height,
            pixels: d.unwrap_or_else(|| vec![0u8; pixel_count]),
        })
        .collect();

    Ok(ShpFile { width, height, frames })
}

/// LCW (Format80) decompression — exact port of OpenRA's LCWCompression.DecodeInto.
///
/// Reference: OpenRA.Mods.Cnc/FileFormats/LCWCompression.cs
fn lcw_decode_into(src: &[u8], dest: &mut [u8], src_offset: usize) {
    let mut si = src_offset;
    let mut di = 0;
    let src_len = src.len();
    let dest_len = dest.len();

    while si < src_len && di < dest_len {
        let i = src[si];
        si += 1;

        if (i & 0x80) == 0 {
            // Case 2: Relative back-reference copy
            if si >= src_len { break; }
            let second_byte = src[si];
            si += 1;
            let count = (((i & 0x70) >> 4) + 3) as usize;
            let rpos = (((i & 0x0F) as usize) << 8) + second_byte as usize;

            if di + count > dest_len { break; }

            let src_start = if di >= rpos { di - rpos } else { break };
            replicate_previous(dest, di, src_start, count);
            di += count;
        } else if (i & 0x40) == 0 {
            // Case 1: Literal copy from source stream
            let count = (i & 0x3F) as usize;
            if count == 0 {
                // End marker
                return;
            }
            if si + count > src_len || di + count > dest_len { break; }
            dest[di..di + count].copy_from_slice(&src[si..si + count]);
            si += count;
            di += count;
        } else {
            // High two bits set (11xxxxxx)
            let count3 = (i & 0x3F) as usize;
            if count3 == 0x3E {
                // Case 4: Fill with repeated byte
                if si + 3 > src_len { break; }
                let count = read_u16(src, si) as usize;
                si += 2;
                let color = src[si];
                si += 1;
                let end = (di + count).min(dest_len);
                while di < end {
                    dest[di] = color;
                    di += 1;
                }
            } else {
                // Case 3 or Case 5: Absolute back-reference copy
                let count = if count3 == 0x3F {
                    // Case 5: Large count
                    if si + 2 > src_len { break; }
                    let c = read_u16(src, si) as usize;
                    si += 2;
                    c
                } else {
                    // Case 3: Small count
                    count3 + 3
                };

                if si + 2 > src_len { break; }
                let src_index = read_u16(src, si) as usize;
                si += 2;

                if src_index >= di || di + count > dest_len { break; }
                for j in 0..count {
                    dest[di + j] = dest[src_index + j];
                }
                di += count;
            }
        }
    }
}

/// Copy bytes from earlier in dest buffer (handles overlapping).
fn replicate_previous(dest: &mut [u8], dest_index: usize, src_index: usize, count: usize) {
    for i in 0..count {
        if dest_index - src_index == 1 {
            dest[dest_index + i] = dest[dest_index - 1];
        } else {
            dest[dest_index + i] = dest[src_index + i];
        }
    }
}

/// XOR Delta (Format40) decompression — exact port of OpenRA's XORDeltaCompression.DecodeInto.
///
/// Reference: OpenRA.Mods.Cnc/FileFormats/XORDeltaCompression.cs
fn xor_delta_decode_into(src: &[u8], dest: &mut [u8], src_offset: usize) {
    let mut si = src_offset;
    let mut di = 0;
    let src_len = src.len();
    let dest_len = dest.len();

    while si < src_len && di < dest_len {
        let i = src[si];
        si += 1;

        if (i & 0x80) == 0 {
            let count = (i & 0x7F) as usize;
            if count == 0 {
                // Case 6: XOR repeated value
                if si + 2 > src_len { break; }
                let count = src[si] as usize;
                si += 1;
                let value = src[si];
                si += 1;
                let end = (di + count).min(dest_len);
                while di < end {
                    dest[di] ^= value;
                    di += 1;
                }
            } else {
                // Case 5: XOR variable bytes
                let end = (di + count).min(dest_len);
                while di < end {
                    if si >= src_len { break; }
                    dest[di] ^= src[si];
                    si += 1;
                    di += 1;
                }
            }
        } else {
            let count = (i & 0x7F) as usize;
            if count == 0 {
                if si + 2 > src_len { break; }
                let word = read_u16(src, si) as usize;
                si += 2;
                if word == 0 {
                    // End marker
                    return;
                }
                if (word & 0x8000) == 0 {
                    // Case 2: Skip pixels
                    di += word & 0x7FFF;
                } else if (word & 0x4000) == 0 {
                    // Case 3: XOR variable bytes (large)
                    let n = word & 0x3FFF;
                    let end = (di + n).min(dest_len);
                    while di < end {
                        if si >= src_len { break; }
                        dest[di] ^= src[si];
                        si += 1;
                        di += 1;
                    }
                } else {
                    // Case 4: XOR repeated value (large)
                    let n = word & 0x3FFF;
                    if si >= src_len { break; }
                    let value = src[si];
                    si += 1;
                    let end = (di + n).min(dest_len);
                    while di < end {
                        dest[di] ^= value;
                        di += 1;
                    }
                }
            } else {
                // Case 1: Skip pixels
                di += count;
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcw_literal_copy() {
        // Case 1: 0x83 = 10_000011 = copy 3 bytes from source
        // Then 0x80 = 10_000000 = count 0 = end
        let src = [0x83, 0xAA, 0xBB, 0xCC, 0x80];
        let mut dest = vec![0u8; 3];
        lcw_decode_into(&src, &mut dest, 0);
        assert_eq!(dest, vec![0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn lcw_fill() {
        // Case 4: 0xFE = 11_111110, count=4, value=0x42
        let src = [0xFE, 0x04, 0x00, 0x42, 0x80];
        let mut dest = vec![0u8; 4];
        lcw_decode_into(&src, &mut dest, 0);
        assert_eq!(dest, vec![0x42, 0x42, 0x42, 0x42]);
    }

    #[test]
    fn decode_real_shp_from_mix() {
        let conquer_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../vendor/ra-content/conquer.mix"
        );
        if let Ok(mix_data) = std::fs::read(conquer_path) {
            let conquer = crate::mix::MixArchive::parse(mix_data).unwrap();

            // Test 1tnk.shp
            if let Some(shp_data) = conquer.get("1tnk.shp") {
                let shp = decode(shp_data).unwrap();
                assert_eq!(shp.width, 24);
                assert_eq!(shp.height, 24);
                assert_eq!(shp.frames.len(), 64);

                // Check that frames have actual pixel data
                let f0_nonzero: usize = shp.frames[0].pixels.iter()
                    .filter(|&&p| p != 0).count();
                println!("1tnk frame 0: {}/{} non-transparent", f0_nonzero, shp.frames[0].pixels.len());
                assert!(f0_nonzero > 50, "1tnk frame 0 should have significant pixel data, got {}", f0_nonzero);
            }

            // Test fact.shp
            if let Some(shp_data) = conquer.get("fact.shp") {
                let shp = decode(shp_data).unwrap();
                assert_eq!(shp.width, 72);
                assert_eq!(shp.height, 72);
                let f0_nonzero: usize = shp.frames[0].pixels.iter()
                    .filter(|&&p| p != 0).count();
                println!("fact frame 0: {}/{} non-transparent", f0_nonzero, shp.frames[0].pixels.len());
                assert!(f0_nonzero > 500, "fact frame 0 should have lots of pixel data, got {}", f0_nonzero);
            }

            // Test mcv.shp
            if let Some(shp_data) = conquer.get("mcv.shp") {
                let shp = decode(shp_data).unwrap();
                let f0_nonzero: usize = shp.frames[0].pixels.iter()
                    .filter(|&&p| p != 0).count();
                println!("mcv frame 0: {}/{} non-transparent", f0_nonzero, shp.frames[0].pixels.len());
                assert!(f0_nonzero > 200, "mcv frame 0 should have significant pixel data, got {}", f0_nonzero);
            }
        }
    }

    #[test]
    fn decode_bits_shp() {
        // Test with known-working SHP from bits/
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../vendor/OpenRA/mods/ra/bits/fact.shp"
        );
        if let Ok(data) = std::fs::read(path) {
            let shp = decode(&data).unwrap();
            assert_eq!(shp.width, 72);
            assert_eq!(shp.height, 72);
            let f0_nonzero: usize = shp.frames[0].pixels.iter()
                .filter(|&&p| p != 0).count();
            println!("bits/fact.shp frame 0: {}/{} non-transparent", f0_nonzero, shp.frames[0].pixels.len());
            assert!(f0_nonzero > 500, "bits/fact.shp should decode properly");
        }
    }
}
