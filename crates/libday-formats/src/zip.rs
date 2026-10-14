//! Read-only ZIP archives (scenario `SCENS\*.ZIP`, `COLONIES.ZIP`).
//! Supports stored and deflated entries; CRC-32 is verified on extraction.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ZipError {
    #[error("not a ZIP archive (no end of central directory)")]
    NotZip,
    #[error("corrupt ZIP archive: {0}")]
    Corrupt(&'static str),
    #[error("unsupported ZIP feature: {0}")]
    Unsupported(String),
    #[error("no archive member named {0}")]
    NotFound(String),
    #[error("CRC mismatch in {0}")]
    Crc(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub method: u16,
    pub flags: u16,
    pub crc32: u32,
    pub compressed_size: u32,
    pub size: u32,
    local_offset: u32,
}

#[derive(Debug)]
pub struct Zip<'a> {
    data: &'a [u8],
    entries: Vec<Entry>,
}

fn u16_at(d: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([d[p], d[p + 1]])
}

fn u32_at(d: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(d[p..p + 4].try_into().unwrap())
}

pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        std::array::from_fn(|i| {
            (0..8).fold(i as u32, |c, _| {
                if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                }
            })
        })
    });
    !data.iter().fold(!0u32, |c, &b| {
        table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8)
    })
}

impl<'a> Zip<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, ZipError> {
        let earliest = data.len().saturating_sub(22 + 0xFFFF);
        let eocd = (earliest..=data.len().saturating_sub(22))
            .rev()
            .find(|&p| data[p..p + 4] == *b"PK\x05\x06")
            .ok_or(ZipError::NotZip)?;
        let count = u16_at(data, eocd + 10) as usize;
        let mut pos = u32_at(data, eocd + 16) as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let h = data
                .get(pos..pos + 46)
                .filter(|h| h[..4] == *b"PK\x01\x02")
                .ok_or(ZipError::Corrupt("central directory entry"))?;
            let (name_len, extra_len, comment_len) = (
                u16_at(h, 28) as usize,
                u16_at(h, 30) as usize,
                u16_at(h, 32) as usize,
            );
            let name = data
                .get(pos + 46..pos + 46 + name_len)
                .ok_or(ZipError::Corrupt("entry name"))?;
            entries.push(Entry {
                name: crate::cp1252::decode(name),
                flags: u16_at(h, 8),
                method: u16_at(h, 10),
                crc32: u32_at(h, 16),
                compressed_size: u32_at(h, 20),
                size: u32_at(h, 24),
                local_offset: u32_at(h, 42),
            });
            pos += 46 + name_len + extra_len + comment_len;
        }
        Ok(Self { data, entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Case-insensitive lookup by member name.
    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>, ZipError> {
        let entry = self
            .find(name)
            .ok_or_else(|| ZipError::NotFound(name.to_owned()))?;
        self.extract(entry)
    }

    pub fn extract(&self, entry: &Entry) -> Result<Vec<u8>, ZipError> {
        if entry.flags & 0x0001 != 0 {
            return Err(ZipError::Unsupported(format!(
                "{} is encrypted",
                entry.name
            )));
        }
        let p = entry.local_offset as usize;
        let h = self
            .data
            .get(p..p + 30)
            .filter(|h| h[..4] == *b"PK\x03\x04")
            .ok_or(ZipError::Corrupt("local header"))?;
        let start = p + 30 + u16_at(h, 26) as usize + u16_at(h, 28) as usize;
        let raw = self
            .data
            .get(start..start + entry.compressed_size as usize)
            .ok_or(ZipError::Corrupt("entry data overrun"))?;
        let out = match entry.method {
            0 => raw.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, entry.size as usize)
                .map_err(|_| ZipError::Corrupt("deflate stream"))?,
            m => return Err(ZipError::Unsupported(format!("compression method {m}"))),
        };
        if out.len() != entry.size as usize || crc32(&out) != entry.crc32 {
            return Err(ZipError::Crc(entry.name.clone()));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ZIP with one stored member, built by hand.
    fn stored(name: &str, body: &[u8]) -> Vec<u8> {
        let crc = crc32(body);
        let mut z = Vec::new();
        z.extend_from_slice(b"PK\x03\x04");
        z.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&crc.to_le_bytes());
        z.extend_from_slice(&(body.len() as u32).to_le_bytes());
        z.extend_from_slice(&(body.len() as u32).to_le_bytes());
        z.extend_from_slice(&(name.len() as u16).to_le_bytes());
        z.extend_from_slice(&[0, 0]);
        z.extend_from_slice(name.as_bytes());
        z.extend_from_slice(body);
        let cd = z.len();
        z.extend_from_slice(b"PK\x01\x02");
        z.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&crc.to_le_bytes());
        z.extend_from_slice(&(body.len() as u32).to_le_bytes());
        z.extend_from_slice(&(body.len() as u32).to_le_bytes());
        z.extend_from_slice(&(name.len() as u16).to_le_bytes());
        z.extend_from_slice(&[0; 12]);
        z.extend_from_slice(&0u32.to_le_bytes());
        z.extend_from_slice(name.as_bytes());
        let cd_len = z.len() - cd;
        z.extend_from_slice(b"PK\x05\x06\0\0\0\0\x01\0\x01\0");
        z.extend_from_slice(&(cd_len as u32).to_le_bytes());
        z.extend_from_slice(&(cd as u32).to_le_bytes());
        z.extend_from_slice(&[0, 0]);
        z
    }

    #[test]
    fn crc_reference_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn reads_stored_member_case_insensitively() {
        let data = stored("Map.MDB", b"hello");
        let zip = Zip::parse(&data).unwrap();
        assert_eq!(zip.read("map.mdb").unwrap(), b"hello");
        let mut bad = data.clone();
        bad[30 + 7] ^= 1;
        assert_eq!(
            Zip::parse(&bad).unwrap().read("MAP.MDB").unwrap_err(),
            ZipError::Crc("Map.MDB".into())
        );
    }
}
