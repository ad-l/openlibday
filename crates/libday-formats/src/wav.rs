//! RIFF WAVE files holding uncompressed PCM (the interface sounds in
//! `SOUNDS\`: mono, 8 or 16-bit).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum WavError {
    #[error("not a RIFF WAVE file")]
    NotWav,
    #[error("truncated {0} chunk")]
    Truncated(&'static str),
    #[error("unsupported format: tag {tag}, {bits} bits")]
    Unsupported { tag: u16, bits: u16 },
    #[error("no {0} chunk")]
    Missing(&'static str),
}

/// Decoded PCM, as signed 16-bit samples (channels interleaved).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wav {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

impl Wav {
    pub fn parse(bytes: &[u8]) -> Result<Self, WavError> {
        if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err(WavError::NotWav);
        }
        let mut format = None;
        let mut at = 12;
        while at + 8 <= bytes.len() {
            let id = &bytes[at..at + 4];
            let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
            let body = bytes.get(at + 8..at + 8 + len);
            match id {
                b"fmt " => {
                    let b = body
                        .filter(|b| b.len() >= 16)
                        .ok_or(WavError::Truncated("fmt"))?;
                    let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
                    let (tag, channels, bits) = (u16_at(0), u16_at(2), u16_at(14));
                    if tag != 1 || !matches!(bits, 8 | 16) || channels == 0 {
                        return Err(WavError::Unsupported { tag, bits });
                    }
                    let rate = u32::from_le_bytes(b[4..8].try_into().unwrap());
                    format = Some((rate, channels, bits));
                }
                b"data" => {
                    let (rate, channels, bits) = format.ok_or(WavError::Missing("fmt"))?;
                    // Some writers overstate the data length; keep what is there.
                    let b = body.unwrap_or(&bytes[at + 8..]);
                    let samples = if bits == 8 {
                        b.iter().map(|&v| (i16::from(v) - 128) << 8).collect()
                    } else {
                        b.as_chunks::<2>()
                            .0
                            .iter()
                            .map(|&s| i16::from_le_bytes(s))
                            .collect()
                    };
                    return Ok(Wav {
                        rate,
                        channels,
                        samples,
                    });
                }
                _ => {}
            }
            at += 8 + len + (len & 1);
        }
        Err(WavError::Missing("data"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(bits: u16, extra: &[u8], data: &[u8]) -> Vec<u8> {
        let mut v = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&11025u32.to_le_bytes());
        v.extend_from_slice(&(11025 * u32::from(bits / 8)).to_le_bytes());
        v.extend_from_slice(&(bits / 8).to_le_bytes());
        v.extend_from_slice(&bits.to_le_bytes());
        v.extend_from_slice(extra);
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn decodes_8_and_16_bit_pcm() {
        let w = Wav::parse(&wav(8, b"fact\x04\0\0\0\x03\0\0\0", &[0, 128, 255])).unwrap();
        assert_eq!((w.rate, w.channels), (11025, 1));
        assert_eq!(w.samples, [-32768, 0, 32512]);
        let w = Wav::parse(&wav(16, &[], &[0x34, 0x12, 0xff, 0xff])).unwrap();
        assert_eq!(w.samples, [0x1234, -1]);
        assert!(matches!(Wav::parse(b"RIFX"), Err(WavError::NotWav)));
    }
}
