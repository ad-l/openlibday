//! `.DBI` sprite databases (`IMGS\*.DBI`).
//!
//! Layout (little-endian):
//!
//! * 32-byte header: magic `dbi\0`, version 3, record count, cell width,
//!   cell height, palette count (0 or 1), two reserved words.
//! * 1024-byte palette of 256 BGRx entries. With palette count 0
//!   (`LISTBOX.DBI`) the slot holds the default Windows palette and the
//!   sprites are meant for the game palette instead.
//! * `count` fixed 8192-byte records: name[8], u16 width, u16 height,
//!   u16 payload size, u16 encoding, 4 unknown bytes, payload at +20.
//!   Encoding 1 (map art) stores one padding byte after the rows, or drops
//!   three bytes of the final transparent run. Encoding 0 (`LISTBOX.DBI`)
//!   uses the same rows with a payload size that ends exactly after them.
//! * Tail index of 12-byte entries: name[8] + u32 record number.
//!
//! Payload rows are `u32 length` (including itself) followed by runs of
//! `u16 count, u16 kind` where kind 1 skips transparent pixels and kind 2 is
//! followed by `count` palette indices.

use std::borrow::Cow;
use std::collections::HashMap;

use thiserror::Error;

const HEADER: usize = 32;
const PALETTE: usize = 1024;
const RECORD: usize = 8192;
const INDEX: usize = 12;
const RECORD_HEADER: usize = 20;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DbiError {
    #[error("DBI file is truncated")]
    Truncated,
    #[error("unsupported DBI header (magic/version/palette count)")]
    UnsupportedHeader,
    #[error("invalid DBI record count {0}")]
    InvalidCount(u32),
    #[error("DBI size does not match its fixed-record index")]
    SizeMismatch,
    #[error("invalid DBI record name {0:?}")]
    InvalidName(String),
    #[error("duplicate DBI record {0}")]
    DuplicateRecord(String),
    #[error("invalid DBI index entry {0}")]
    InvalidIndex(String),
    #[error("no DBI record named {0}")]
    NotFound(String),
    #[error("invalid sprite header for {0}")]
    InvalidSpriteHeader(String),
    #[error("invalid sprite data for {name}, row {row}: {reason}")]
    InvalidSprite {
        name: String,
        row: usize,
        reason: &'static str,
    },
    #[error("encoded sprite {0} does not fit in its 8192-byte record")]
    TooLarge(String),
}

/// 256-entry RGB palette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette(pub [[u8; 3]; 256]);

impl Palette {
    pub fn rgba(&self, index: u8) -> [u8; 4] {
        let [r, g, b] = self.0[index as usize];
        [r, g, b, 255]
    }
}

/// Decoded 8-bit sprite. `None` pixels are transparent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sprite {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Option<u8>>,
}

impl Sprite {
    /// Expands to RGBA8; transparent pixels are all-zero.
    pub fn to_rgba(&self, palette: &Palette) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for pixel in &self.pixels {
            out.extend_from_slice(&pixel.map_or([0; 4], |i| palette.rgba(i)));
        }
        out
    }
}

/// A parsed DBI database borrowing the file bytes.
#[derive(Debug)]
pub struct Dbi<'a> {
    data: Cow<'a, [u8]>,
    pub cell_width: u32,
    pub cell_height: u32,
    /// Embedded palette; only meaningful when `has_palette` is true.
    pub palette: Palette,
    pub has_palette: bool,
    names: Vec<String>,
    records: HashMap<String, usize>,
}

fn u16_at(data: &[u8], pos: usize) -> u16 {
    u16::from_le_bytes([data[pos], data[pos + 1]])
}

fn u32_at(data: &[u8], pos: usize) -> u32 {
    u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap())
}

fn record_name(raw: &[u8]) -> Result<String, DbiError> {
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let name = &raw[..end];
    let valid = !name.is_empty() && name.iter().all(|&b| b.is_ascii_alphanumeric() || b == b'_');
    let text = String::from_utf8_lossy(name).into_owned();
    if valid {
        Ok(text)
    } else {
        Err(DbiError::InvalidName(text))
    }
}

impl<'a> Dbi<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, DbiError> {
        if data.len() < HEADER + PALETTE {
            return Err(DbiError::Truncated);
        }
        let (version, count) = (u32_at(data, 4), u32_at(data, 8));
        let (cell_width, cell_height) = (u32_at(data, 12), u32_at(data, 16));
        if &data[..4] != b"dbi\0" || version != 3 || u32_at(data, 20) > 1 {
            return Err(DbiError::UnsupportedHeader);
        }
        if !(1..=10_000).contains(&count) {
            return Err(DbiError::InvalidCount(count));
        }
        let count = count as usize;
        let start = HEADER + PALETTE;
        let index_start = start + count * RECORD;
        if data.len() != index_start + count * INDEX {
            return Err(DbiError::SizeMismatch);
        }
        let mut palette = [[0u8; 3]; 256];
        for (i, entry) in palette.iter_mut().enumerate() {
            let p = HEADER + i * 4;
            *entry = [data[p + 2], data[p + 1], data[p]];
        }
        let mut names = Vec::with_capacity(count);
        let mut records = HashMap::with_capacity(count);
        for i in 0..count {
            let pos = start + i * RECORD;
            let name = record_name(&data[pos..pos + 8])?;
            if records.insert(name.to_ascii_uppercase(), pos).is_some() {
                return Err(DbiError::DuplicateRecord(name));
            }
            names.push(name);
        }
        let mut seen = std::collections::HashSet::with_capacity(count);
        for i in 0..count {
            let pos = index_start + i * INDEX;
            let name = record_name(&data[pos..pos + 8])?;
            let record = u32_at(data, pos + 8) as usize;
            let key = name.to_ascii_uppercase();
            if record >= count
                || !seen.insert(key.clone())
                || records.get(&key) != Some(&(start + record * RECORD))
            {
                return Err(DbiError::InvalidIndex(name));
            }
        }
        Ok(Self {
            data: Cow::Borrowed(data),
            cell_width,
            cell_height,
            palette: Palette(palette),
            has_palette: u32_at(data, 20) == 1,
            names,
            records,
        })
    }

    /// Parses a DBI that owns its bytes.
    pub fn from_vec(data: Vec<u8>) -> Result<Dbi<'static>, DbiError> {
        let Dbi {
            cell_width,
            cell_height,
            palette,
            has_palette,
            names,
            records,
            ..
        } = Dbi::parse(&data)?;
        Ok(Dbi {
            data: Cow::Owned(data),
            cell_width,
            cell_height,
            palette,
            has_palette,
            names,
            records,
        })
    }

    /// Record names in file order.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.records.contains_key(&name.to_ascii_uppercase())
    }

    /// Decodes a sprite by (case-insensitive) name.
    pub fn sprite(&self, name: &str) -> Result<Sprite, DbiError> {
        let record = *self
            .records
            .get(&name.to_ascii_uppercase())
            .ok_or_else(|| DbiError::NotFound(name.to_owned()))?;
        let d: &[u8] = &self.data;
        let width = u16_at(d, record + 8) as usize;
        let height = u16_at(d, record + 10) as usize;
        let size = u16_at(d, record + 12) as usize;
        let encoding = u16_at(d, record + 14);
        if width as u32 != self.cell_width
            || height as u32 != self.cell_height
            || encoding > 1
            || !(1..=RECORD - RECORD_HEADER).contains(&size)
        {
            return Err(DbiError::InvalidSpriteHeader(name.to_owned()));
        }
        let fail = |row, reason| DbiError::InvalidSprite {
            name: name.to_owned(),
            row,
            reason,
        };
        let mut pos = record + RECORD_HEADER;
        let limit = pos + size;
        let mut pixels = vec![None; width * height];
        for y in 0..height {
            if pos + 4 > limit {
                return Err(fail(y, "missing scanline"));
            }
            let length = u32_at(d, pos) as usize;
            let end = pos + length;
            pos += 4;
            if length < 8 || end > limit + 3 {
                return Err(fail(y, "invalid scanline length"));
            }
            let mut x = 0;
            while pos < end {
                if pos + 4 > limit {
                    // The encoder drops the last three bytes of a final
                    // transparent run; only the low count byte survives.
                    let quirk = y == height - 1
                        && end == limit + 3
                        && limit - pos == 1
                        && d[pos] as usize == width - x;
                    if !quirk {
                        return Err(fail(y, "truncated run"));
                    }
                    x = width;
                    pos = end;
                    break;
                }
                let count = u16_at(d, pos) as usize;
                let kind = u16_at(d, pos + 2);
                pos += 4;
                if count == 0 || x + count > width || !(kind == 1 || kind == 2) {
                    return Err(fail(y, "invalid run"));
                }
                if kind == 2 {
                    if pos + count > end.min(limit) {
                        return Err(fail(y, "truncated pixels"));
                    }
                    let row = y * width + x;
                    for (dst, &src) in pixels[row..row + count]
                        .iter_mut()
                        .zip(&d[pos..pos + count])
                    {
                        *dst = Some(src);
                    }
                    pos += count;
                }
                x += count;
            }
            if pos != end || x != width {
                return Err(fail(y, "scanline does not cover its width"));
            }
        }
        let complete = match encoding {
            0 => pos == limit,
            _ => pos == limit - 1 || pos == limit + 3,
        };
        if !complete {
            return Err(fail(height, "unexpected payload length"));
        }
        Ok(Sprite {
            width,
            height,
            pixels,
        })
    }

    /// Returns a copy of the whole file with sprite `name` replaced.
    ///
    /// The sprite must match the cell size; the record header's unknown bytes
    /// and encoding are kept.
    pub fn replace(&self, name: &str, sprite: &Sprite) -> Result<Vec<u8>, DbiError> {
        let record = *self
            .records
            .get(&name.to_ascii_uppercase())
            .ok_or_else(|| DbiError::NotFound(name.to_owned()))?;
        if sprite.width as u32 != self.cell_width
            || sprite.height as u32 != self.cell_height
            || sprite.pixels.len() != sprite.width * sprite.height
        {
            return Err(DbiError::InvalidSpriteHeader(name.to_owned()));
        }
        let encoding = u16_at(&self.data, record + 14);
        let mut payload = encode_rows(sprite);
        if encoding == 1 {
            payload.push(0);
        }
        if payload.len() > RECORD - RECORD_HEADER {
            return Err(DbiError::TooLarge(name.to_owned()));
        }
        let mut out = self.data.to_vec();
        let body = &mut out[record + RECORD_HEADER..record + RECORD];
        body.fill(0);
        body[..payload.len()].copy_from_slice(&payload);
        out[record + 12..record + 14].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        Ok(out)
    }
}

/// Encodes sprite rows as length-prefixed skip/literal runs.
fn encode_rows(sprite: &Sprite) -> Vec<u8> {
    let mut out = Vec::new();
    for row in sprite.pixels.chunks(sprite.width.max(1)) {
        let start = out.len();
        out.extend_from_slice(&[0; 4]);
        let mut x = 0;
        while x < row.len() {
            let opaque = row[x].is_some();
            let run = row[x..]
                .iter()
                .take_while(|p| p.is_some() == opaque)
                .take(u16::MAX as usize)
                .count();
            out.extend_from_slice(&(run as u16).to_le_bytes());
            out.extend_from_slice(&(if opaque { 2u16 } else { 1 }).to_le_bytes());
            if opaque {
                out.extend(row[x..x + run].iter().map(|p| p.unwrap_or(0)));
            }
            x += run;
        }
        let length = (out.len() - start) as u32;
        out[start..start + 4].copy_from_slice(&length.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a one-record DBI with the given payload rows.
    fn build(width: u16, height: u16, payload: &[u8]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(b"dbi\0");
        for v in [3u32, 1, width as u32, height as u32, 1, 0, 0] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        for i in 0..256u32 {
            data.extend_from_slice(&[i as u8, (i / 2) as u8, 255 - i as u8, 0]);
        }
        let mut record = vec![0u8; RECORD];
        record[..4].copy_from_slice(b"TEST");
        record[8..10].copy_from_slice(&width.to_le_bytes());
        record[10..12].copy_from_slice(&height.to_le_bytes());
        record[12..14].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        record[14..16].copy_from_slice(&1u16.to_le_bytes());
        record[20..20 + payload.len()].copy_from_slice(payload);
        data.extend_from_slice(&record);
        data.extend_from_slice(b"TEST\0\0\0\0");
        data.extend_from_slice(&0u32.to_le_bytes());
        data
    }

    fn row(runs: &[(u16, u16, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for &(count, kind, px) in runs {
            body.extend_from_slice(&count.to_le_bytes());
            body.extend_from_slice(&kind.to_le_bytes());
            body.extend_from_slice(px);
        }
        let mut out = ((body.len() + 4) as u32).to_le_bytes().to_vec();
        out.extend(body);
        out
    }

    #[test]
    fn decodes_runs_and_palette() {
        let mut payload = row(&[(1, 1, &[]), (2, 2, &[7, 9])]);
        payload.extend(row(&[(3, 2, &[1, 2, 3])]));
        payload.push(0); // stored size ends one byte past the last row
        let data = build(3, 2, &payload);
        let dbi = Dbi::parse(&data).unwrap();
        assert_eq!(dbi.names(), ["TEST"]);
        let sprite = dbi.sprite("test").unwrap();
        assert_eq!(
            sprite.pixels,
            [None, Some(7), Some(9), Some(1), Some(2), Some(3)]
        );
        assert_eq!(dbi.palette.rgba(7), [255 - 7, 3, 7, 255]);
        assert_eq!(
            &sprite.to_rgba(&dbi.palette)[..8],
            &[0, 0, 0, 0, 248, 3, 7, 255]
        );
    }

    #[test]
    fn accepts_truncated_final_transparent_run() {
        let mut payload = row(&[(2, 2, &[5, 6]), (2, 1, &[])]);
        let last = row(&[(1, 2, &[4]), (3, 1, &[])]);
        payload.extend_from_slice(&last[..last.len() - 3]);
        let sprite = Dbi::parse(&build(4, 2, &payload))
            .unwrap()
            .sprite("TEST")
            .unwrap();
        assert_eq!(&sprite.pixels[4..], [Some(4), None, None, None]);
    }

    #[test]
    fn encoding_zero_payload_ends_after_rows() {
        let payload = row(&[(2, 2, &[5, 6])]);
        let mut data = build(2, 1, &payload);
        data[HEADER + PALETTE + 14] = 0;
        let sprite = Dbi::parse(&data).unwrap().sprite("TEST").unwrap();
        assert_eq!(sprite.pixels, [Some(5), Some(6)]);
    }

    #[test]
    fn replace_round_trips() {
        let mut payload = row(&[(1, 1, &[]), (2, 2, &[7, 9])]);
        payload.extend(row(&[(3, 2, &[1, 2, 3])]));
        payload.push(0);
        let data = build(3, 2, &payload);
        let dbi = Dbi::parse(&data).unwrap();
        let original = dbi.sprite("TEST").unwrap();
        assert_eq!(dbi.replace("test", &original).unwrap(), data);
        let edited = Sprite {
            width: 3,
            height: 2,
            pixels: vec![Some(1), None, Some(2), None, None, None],
        };
        let out = dbi.replace("TEST", &edited).unwrap();
        assert_eq!(out.len(), data.len());
        assert_eq!(Dbi::parse(&out).unwrap().sprite("TEST").unwrap(), edited);
        let wrong = Sprite {
            width: 2,
            height: 2,
            pixels: vec![None; 4],
        };
        assert!(dbi.replace("TEST", &wrong).is_err());
    }

    #[test]
    fn rejects_short_rows_and_bad_index() {
        let mut payload = row(&[(2, 2, &[5, 6])]);
        payload.push(0);
        let dbi_data = build(3, 1, &payload);
        assert!(matches!(
            Dbi::parse(&dbi_data).unwrap().sprite("TEST"),
            Err(DbiError::InvalidSprite { .. })
        ));
        let mut bad = dbi_data.clone();
        let n = bad.len();
        bad[n - 4] = 1;
        assert_eq!(
            Dbi::parse(&bad).unwrap_err(),
            DbiError::InvalidIndex("TEST".into())
        );
    }
}
