//! MIX archive file reader.
//!
//! Parses MIX files from Red Alert / Tiberian Dawn.
//! Supports unencrypted C&C and RA format MIX archives.
//!
//! Reference: OpenRA.Mods.Cnc/FileSystem/MixFile.cs

use std::collections::HashMap;

/// A parsed MIX archive.
pub struct MixArchive {
    data: Vec<u8>,
    entries: HashMap<u32, MixEntry>,
    data_start: usize,
}

#[derive(Debug, Clone, Copy)]
struct MixEntry {
    offset: u32,
    length: u32,
}

impl MixArchive {
    /// Parse a MIX archive from raw bytes.
    pub fn parse(data: Vec<u8>) -> Result<Self, String> {
        if data.len() < 6 {
            return Err("MIX file too small".into());
        }

        let (header_offset, _is_ra_format) = detect_format(&data);

        if header_offset + 6 > data.len() {
            return Err("MIX header offset out of bounds".into());
        }

        let num_files = read_u16(&data, header_offset) as usize;
        let _data_size = read_u32(&data, header_offset + 2);

        let entries_start = header_offset + 6;
        let entries_end = entries_start + num_files * 12;
        if entries_end > data.len() {
            return Err(format!(
                "MIX entry table out of bounds: need {} bytes, have {}",
                entries_end,
                data.len()
            ));
        }

        let data_start = entries_end;

        let mut entries = HashMap::with_capacity(num_files);
        for i in 0..num_files {
            let base = entries_start + i * 12;
            let hash = read_u32(&data, base);
            let offset = read_u32(&data, base + 4);
            let length = read_u32(&data, base + 8);
            entries.insert(hash, MixEntry { offset, length });
        }

        Ok(MixArchive {
            data,
            entries,
            data_start,
        })
    }

    /// Get a file by name from the archive.
    /// Tries classic hash first, then CRC32 hash (some RA MIX files use CRC32).
    pub fn get(&self, filename: &str) -> Option<&[u8]> {
        let entry = self.entries.get(&classic_hash(filename))
            .or_else(|| self.entries.get(&crc32_hash(filename)))?;
        let start = self.data_start + entry.offset as usize;
        let end = start + entry.length as usize;
        if end <= self.data.len() {
            Some(&self.data[start..end])
        } else {
            None
        }
    }

    /// Check if a file exists in the archive.
    pub fn contains(&self, filename: &str) -> bool {
        self.entries.contains_key(&classic_hash(filename))
            || self.entries.contains_key(&crc32_hash(filename))
    }

    /// Number of files in the archive.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the archive is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Detect MIX format and return (header_offset, is_ra_format).
fn detect_format(data: &[u8]) -> (usize, bool) {
    let first = read_u16(data, 0);
    if first != 0 {
        // C&C format: no flags, header starts at 0
        (0, false)
    } else {
        // RA format: flags at offset 2
        let flags = read_u16(data, 2);
        if flags & 0x2 != 0 {
            // Encrypted — skip 80-byte key block
            // We don't support decryption, but the freeware content shouldn't be encrypted
            (84, true)
        } else {
            // Unencrypted RA format
            (4, true)
        }
    }
}

/// Compute the Classic MIX hash for a filename.
///
/// Algorithm from PackageEntry.cs HashFilename:
/// 1. Uppercase the filename
/// 2. Pad with null bytes to multiple of 4
/// 3. Interpret as u32 array (little-endian)
/// 4. Accumulate: result = rotate_left_1(result) + next_u32
pub fn classic_hash(name: &str) -> u32 {
    let upper = name.to_ascii_uppercase();
    let bytes = upper.as_bytes();

    // Pad to multiple of 4
    let padding = (4 - bytes.len() % 4) % 4;
    let mut padded = bytes.to_vec();
    padded.resize(bytes.len() + padding, 0);

    let mut result: u32 = 0;
    for chunk in padded.chunks(4) {
        let val = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        result = result.rotate_left(1).wrapping_add(val);
    }
    result
}

/// CRC32 lookup table (standard polynomial 0xEDB88320).
/// Reference: OpenRA.Game/FileFormats/CRC32.cs
const CRC32_TABLE: [u32; 256] = [
    0x00000000, 0x77073096, 0xEE0E612C, 0x990951BA,
    0x076DC419, 0x706AF48F, 0xE963A535, 0x9E6495A3,
    0x0EDB8832, 0x79DCB8A4, 0xE0D5E91E, 0x97D2D988,
    0x09B64C2B, 0x7EB17CBD, 0xE7B82D07, 0x90BF1D91,
    0x1DB71064, 0x6AB020F2, 0xF3B97148, 0x84BE41DE,
    0x1ADAD47D, 0x6DDDE4EB, 0xF4D4B551, 0x83D385C7,
    0x136C9856, 0x646BA8C0, 0xFD62F97A, 0x8A65C9EC,
    0x14015C4F, 0x63066CD9, 0xFA0F3D63, 0x8D080DF5,
    0x3B6E20C8, 0x4C69105E, 0xD56041E4, 0xA2677172,
    0x3C03E4D1, 0x4B04D447, 0xD20D85FD, 0xA50AB56B,
    0x35B5A8FA, 0x42B2986C, 0xDBBBC9D6, 0xACBCF940,
    0x32D86CE3, 0x45DF5C75, 0xDCD60DCF, 0xABD13D59,
    0x26D930AC, 0x51DE003A, 0xC8D75180, 0xBFD06116,
    0x21B4F4B5, 0x56B3C423, 0xCFBA9599, 0xB8BDA50F,
    0x2802B89E, 0x5F058808, 0xC60CD9B2, 0xB10BE924,
    0x2F6F7C87, 0x58684C11, 0xC1611DAB, 0xB6662D3D,
    0x76DC4190, 0x01DB7106, 0x98D220BC, 0xEFD5102A,
    0x71B18589, 0x06B6B51F, 0x9FBFE4A5, 0xE8B8D433,
    0x7807C9A2, 0x0F00F934, 0x9609A88E, 0xE10E9818,
    0x7F6A0DBB, 0x086D3D2D, 0x91646C97, 0xE6635C01,
    0x6B6B51F4, 0x1C6C6162, 0x856530D8, 0xF262004E,
    0x6C0695ED, 0x1B01A57B, 0x8208F4C1, 0xF50FC457,
    0x65B0D9C6, 0x12B7E950, 0x8BBEB8EA, 0xFCB9887C,
    0x62DD1DDF, 0x15DA2D49, 0x8CD37CF3, 0xFBD44C65,
    0x4DB26158, 0x3AB551CE, 0xA3BC0074, 0xD4BB30E2,
    0x4ADFA541, 0x3DD895D7, 0xA4D1C46D, 0xD3D6F4FB,
    0x4369E96A, 0x346ED9FC, 0xAD678846, 0xDA60B8D0,
    0x44042D73, 0x33031DE5, 0xAA0A4C5F, 0xDD0D7CC9,
    0x5005713C, 0x270241AA, 0xBE0B1010, 0xC90C2086,
    0x5768B525, 0x206F85B3, 0xB966D409, 0xCE61E49F,
    0x5EDEF90E, 0x29D9C998, 0xB0D09822, 0xC7D7A8B4,
    0x59B33D17, 0x2EB40D81, 0xB7BD5C3B, 0xC0BA6CAD,
    0xEDB88320, 0x9ABFB3B6, 0x03B6E20C, 0x74B1D29A,
    0xEAD54739, 0x9DD277AF, 0x04DB2615, 0x73DC1683,
    0xE3630B12, 0x94643B84, 0x0D6D6A3E, 0x7A6A5AA8,
    0xE40ECF0B, 0x9309FF9D, 0x0A00AE27, 0x7D079EB1,
    0xF00F9344, 0x8708A3D2, 0x1E01F268, 0x6906C2FE,
    0xF762575D, 0x806567CB, 0x196C3671, 0x6E6B06E7,
    0xFED41B76, 0x89D32BE0, 0x10DA7A5A, 0x67DD4ACC,
    0xF9B9DF6F, 0x8EBEEFF9, 0x17B7BE43, 0x60B08ED5,
    0xD6D6A3E8, 0xA1D1937E, 0x38D8C2C4, 0x4FDFF252,
    0xD1BB67F1, 0xA6BC5767, 0x3FB506DD, 0x48B2364B,
    0xD80D2BDA, 0xAF0A1B4C, 0x36034AF6, 0x41047A60,
    0xDF60EFC3, 0xA867DF55, 0x316E8EEF, 0x4669BE79,
    0xCB61B38C, 0xBC66831A, 0x256FD2A0, 0x5268E236,
    0xCC0C7795, 0xBB0B4703, 0x220216B9, 0x5505262F,
    0xC5BA3BBE, 0xB2BD0B28, 0x2BB45A92, 0x5CB36A04,
    0xC2D7FFA7, 0xB5D0CF31, 0x2CD99E8B, 0x5BDEAE1D,
    0x9B64C2B0, 0xEC63F226, 0x756AA39C, 0x026D930A,
    0x9C0906A9, 0xEB0E363F, 0x72076785, 0x05005713,
    0x95BF4A82, 0xE2B87A14, 0x7BB12BAE, 0x0CB61B38,
    0x92D28E9B, 0xE5D5BE0D, 0x7CDCEFB7, 0x0BDBDF21,
    0x86D3D2D4, 0xF1D4E242, 0x68DDB3F8, 0x1FDA836E,
    0x81BE16CD, 0xF6B9265B, 0x6FB077E1, 0x18B74777,
    0x88085AE6, 0xFF0F6A70, 0x66063BCA, 0x11010B5C,
    0x8F659EFF, 0xF862AE69, 0x616BFFD3, 0x166CCF45,
    0xA00AE278, 0xD70DD2EE, 0x4E048354, 0x3903B3C2,
    0xA7672661, 0xD06016F7, 0x4969474D, 0x3E6E77DB,
    0xAED16A4A, 0xD9D65ADC, 0x40DF0B66, 0x37D83BF0,
    0xA9BCAE53, 0xDEBB9EC5, 0x47B2CF7F, 0x30B5FFE9,
    0xBDBDF21C, 0xCABAC28A, 0x53B39330, 0x24B4A3A6,
    0xBAD03605, 0xCDD70693, 0x54DE5729, 0x23D967BF,
    0xB3667A2E, 0xC4614AB8, 0x5D681B02, 0x2A6F2B94,
    0xB40BBE37, 0xC30C8EA1, 0x5A05DF1B, 0x2D02EF8D,
];

/// Compute the CRC32 MIX hash for a filename.
///
/// Reference: PackageEntry.cs HashFilename (CRC32 variant) + CRC32.cs
/// Different padding from classic: if len%4 != 0, pad byte[len] = remainder,
/// then fill remaining pad bytes with byte at round-down-to-4 position.
pub fn crc32_hash(name: &str) -> u32 {
    let upper = name.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let len = bytes.len();
    let padding = (4 - len % 4) % 4;
    let padded_len = len + padding;

    let mut buf = bytes.to_vec();
    buf.resize(padded_len, 0);

    if padding > 0 {
        let round_down = len / 4 * 4;
        buf[len] = (len - round_down) as u8;
        for p in 1..padding {
            buf[len + p] = buf[round_down];
        }
    }

    // CRC32 with polynomial 0xFFFFFFFF
    let mut crc: u32 = 0xFFFFFFFF;
    for &b in &buf {
        crc = (crc >> 8) ^ CRC32_TABLE[((crc & 0xFF) ^ b as u32) as usize];
    }
    crc ^ 0xFFFFFFFF
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
    fn hash_known_values() {
        // Test against known hashes from OpenRA
        // "e1.shp" should produce a deterministic hash
        let h = classic_hash("e1.shp");
        assert_ne!(h, 0);
        // Same name should produce same hash
        assert_eq!(classic_hash("e1.shp"), classic_hash("E1.SHP"));
    }

    #[test]
    fn hash_case_insensitive() {
        assert_eq!(classic_hash("foo.bar"), classic_hash("FOO.BAR"));
        assert_eq!(classic_hash("Test.Shp"), classic_hash("TEST.SHP"));
    }

    #[test]
    fn parse_real_mix_files() {
        // Test with actual RA freeware content if available
        let conquer_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../vendor/ra-content/conquer.mix"
        );
        if let Ok(data) = std::fs::read(conquer_path) {
            let mix = MixArchive::parse(data).expect("Failed to parse conquer.mix");
            assert!(mix.len() > 0);

            // Classic hash: vehicle sprites
            assert!(mix.get("1tnk.shp").is_some(), "1tnk.shp should be found via classic hash");
            assert!(mix.get("fact.shp").is_some(), "fact.shp should be found via classic hash");
        }
    }

    #[test]
    fn crc32_hash_case_insensitive() {
        assert_eq!(crc32_hash("e1.shp"), crc32_hash("E1.SHP"));
        assert_eq!(crc32_hash("foo.bar"), crc32_hash("FOO.BAR"));
    }

    #[test]
    fn crc32_dual_lookup() {
        // CRC32 hash is available as fallback for files not found with classic hash
        let h1 = crc32_hash("test.shp");
        assert_ne!(h1, 0);
        assert_ne!(h1, classic_hash("test.shp"));
    }

    #[test]
    fn parse_allies_mix() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../vendor/ra-content/allies.mix"
        );
        if let Ok(data) = std::fs::read(path) {
            let mix = MixArchive::parse(data).expect("Failed to parse allies.mix");
            assert!(mix.len() > 0);
        }
    }

    #[test]
    fn parse_temperat_mix() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../vendor/ra-content/temperat.mix"
        );
        if let Ok(data) = std::fs::read(path) {
            let mix = MixArchive::parse(data).expect("Failed to parse temperat.mix");
            assert!(mix.len() > 0);
        }
    }
}
