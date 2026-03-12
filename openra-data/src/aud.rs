//! AUD audio file decoder (IMA ADPCM).
//!
//! Red Alert .aud files use IMA ADPCM compression with chunk-based storage.
//! Decodes to 16-bit PCM samples.

/// Decoded AUD audio data.
pub struct AudSound {
    /// Sample rate in Hz (typically 22050).
    pub sample_rate: u16,
    /// Decoded 16-bit PCM samples (little-endian i16 values as bytes).
    pub pcm_data: Vec<u8>,
    /// Number of channels (1=mono, 2=stereo).
    pub channels: u8,
}

/// IMA ADPCM step size table (89 entries).
const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37,
    41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173,
    190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658,
    724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066,
    2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484,
    7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500,
    20350, 22385, 24623, 27086, 29794, 32767,
];

/// IMA ADPCM index adjustment table.
const INDEX_ADJUST: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

/// Decode one 4-bit IMA ADPCM sample.
fn decode_sample(nibble: u8, index: &mut i32, current: &mut i32) -> i16 {
    let code = (nibble & 7) as i32;
    let step = STEP_TABLE[*index as usize];
    let mut delta = step * code / 4 + step / 8;

    if nibble & 8 != 0 {
        delta = -delta;
    }

    *current += delta;
    *current = (*current).clamp(-32768, 32767);

    *index += INDEX_ADJUST[code as usize];
    *index = (*index).clamp(0, 88);

    *current as i16
}

/// Decode an AUD file from raw bytes.
///
/// Returns decoded PCM audio or an error message.
pub fn decode(data: &[u8]) -> Result<AudSound, String> {
    if data.len() < 12 {
        return Err("AUD file too short for header".into());
    }

    let sample_rate = u16::from_le_bytes([data[0], data[1]]);
    let _data_size = i32::from_le_bytes([data[2], data[3], data[4], data[5]]);
    let output_size = i32::from_le_bytes([data[6], data[7], data[8], data[9]]) as usize;
    let flags = data[10];
    let format = data[11];

    let channels: u8 = if flags & 1 != 0 { 2 } else { 1 };
    // flags & 2 => 16-bit (always for IMA ADPCM output)

    if format != 99 {
        return Err(format!("Unsupported AUD format: {} (expected 99 for IMA ADPCM)", format));
    }

    let mut pcm_data = Vec::with_capacity(output_size);
    let mut pos = 12usize;
    let mut index: i32 = 0;
    let mut current: i32 = 0;

    while pos + 8 <= data.len() && pcm_data.len() < output_size {
        // Read chunk header
        let compressed_size = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        let _chunk_output = u16::from_le_bytes([data[pos + 2], data[pos + 3]]);
        let marker = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);

        if marker != 0x0000deaf {
            return Err(format!("Invalid chunk marker: 0x{:08x} at offset {}", marker, pos));
        }

        pos += 8;
        let chunk_end = (pos + compressed_size).min(data.len());

        while pos < chunk_end && pcm_data.len() < output_size {
            let byte = data[pos];
            pos += 1;

            // Lower nibble first
            let sample1 = decode_sample(byte & 0x0F, &mut index, &mut current);
            let bytes1 = sample1.to_le_bytes();
            pcm_data.push(bytes1[0]);
            pcm_data.push(bytes1[1]);

            if pcm_data.len() < output_size {
                // Upper nibble
                let sample2 = decode_sample(byte >> 4, &mut index, &mut current);
                let bytes2 = sample2.to_le_bytes();
                pcm_data.push(bytes2[0]);
                pcm_data.push(bytes2[1]);
            }
        }
    }

    Ok(AudSound {
        sample_rate,
        pcm_data,
        channels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_sample_basic() {
        let mut index = 0i32;
        let mut current = 0i32;
        let s = decode_sample(0, &mut index, &mut current);
        // nibble=0, code=0, delta = 7*0/4 + 7/8 = 0, current=0
        assert_eq!(s, 0);
    }

    #[test]
    fn step_table_bounds() {
        assert_eq!(STEP_TABLE[0], 7);
        assert_eq!(STEP_TABLE[88], 32767);
        assert_eq!(STEP_TABLE.len(), 89);
    }
}
