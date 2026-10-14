//! Windows-1252 text, the code page of the original databases.
//! Bytes undefined in CP1252 map to the matching C1 control code point, so
//! decoding is lossless and reversible.

const HIGH: [u16; 32] = [
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039,
    0x0152, 0x008D, 0x017D, 0x008F, 0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014,
    0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178,
];

pub fn decode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => char::from_u32(HIGH[(b - 0x80) as usize] as u32).unwrap(),
            _ => b as char,
        })
        .collect()
}

/// Encodes text; returns `None` if a character has no CP1252 byte.
pub fn encode(text: &str) -> Option<Vec<u8>> {
    text.chars()
        .map(|c| match c as u32 {
            0..=0x7F | 0xA0..=0xFF => Some(c as u8),
            code => HIGH
                .iter()
                .position(|&h| h as u32 == code)
                .map(|i| 0x80 + i as u8),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trips_every_byte() {
        let all: Vec<u8> = (0..=255).collect();
        let text = super::decode(&all);
        assert!(text.starts_with('\0'));
        assert_eq!(text.chars().nth(0x80), Some('€'));
        assert_eq!(super::encode(&text).unwrap(), all);
        assert_eq!(super::encode("日"), None);
    }
}
