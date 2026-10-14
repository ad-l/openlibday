//! String tables and bitmaps from PE resources (`EXE\LIBDAYEN.DLL` holds UI
//! text, `EXE\LIBDAY.EXE` some interface icons).

use std::collections::BTreeMap;

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PeError {
    #[error("not a PE image")]
    NotPe,
    #[error("corrupt PE image: {0}")]
    Corrupt(&'static str),
}

const RT_STRING: u32 = 6;

fn u16_at(d: &[u8], p: usize) -> Result<u16, PeError> {
    d.get(p..p + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or(PeError::Corrupt("read past end"))
}

fn u32_at(d: &[u8], p: usize) -> Result<u32, PeError> {
    d.get(p..p + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or(PeError::Corrupt("read past end"))
}

struct Image<'a> {
    data: &'a [u8],
    /// `(virtual address, virtual size, raw offset, raw size)`.
    sections: Vec<(u32, u32, u32, u32)>,
    resources: u32,
}

impl<'a> Image<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, PeError> {
        if data.get(..2) != Some(b"MZ") {
            return Err(PeError::NotPe);
        }
        let pe = u32_at(data, 0x3C)? as usize;
        if data.get(pe..pe + 4) != Some(b"PE\0\0") {
            return Err(PeError::NotPe);
        }
        let sections = u16_at(data, pe + 6)? as usize;
        let optional = pe + 24;
        let optional_size = u16_at(data, pe + 20)? as usize;
        let dirs = match u16_at(data, optional)? {
            0x10B => optional + 96,
            0x20B => optional + 112,
            _ => return Err(PeError::Corrupt("unknown optional header")),
        };
        let resources = u32_at(data, dirs + 2 * 8)?;
        let table = optional + optional_size;
        let sections = (0..sections)
            .map(|i| {
                let s = table + i * 40;
                Ok((
                    u32_at(data, s + 12)?,
                    u32_at(data, s + 8)?,
                    u32_at(data, s + 20)?,
                    u32_at(data, s + 16)?,
                ))
            })
            .collect::<Result<_, PeError>>()?;
        Ok(Self {
            data,
            sections,
            resources,
        })
    }

    fn offset(&self, rva: u32) -> Result<usize, PeError> {
        self.sections
            .iter()
            .find(|&&(va, vsize, _, rsize)| rva >= va && rva < va + vsize.max(rsize))
            .map(|&(va, _, raw, _)| (rva - va + raw) as usize)
            .ok_or(PeError::Corrupt("RVA outside sections"))
    }

    /// `(id, offset-to-data with subdirectory bit)` entries of a resource directory.
    fn directory(&self, base: usize, dir: usize) -> Result<Vec<(u32, u32)>, PeError> {
        let named = u16_at(self.data, base + dir + 12)? as usize;
        let ids = u16_at(self.data, base + dir + 14)? as usize;
        (0..named + ids)
            .map(|i| {
                let e = base + dir + 16 + i * 8;
                Ok((u32_at(self.data, e)?, u32_at(self.data, e + 4)?))
            })
            .collect()
    }
}

/// Loads every `RT_STRING` entry. When several languages define an id, the
/// first one in resource order wins.
pub fn string_table(data: &[u8]) -> Result<BTreeMap<u32, String>, PeError> {
    let image = Image::parse(data)?;
    let mut strings = BTreeMap::new();
    if image.resources == 0 {
        return Ok(strings);
    }
    let base = image.offset(image.resources)?;
    let sub = |off: u32| {
        if off & 0x8000_0000 != 0 {
            Ok((off & 0x7FFF_FFFF) as usize)
        } else {
            Err(PeError::Corrupt("expected a resource subdirectory"))
        }
    };
    for (kind, off) in image.directory(base, 0)? {
        if kind != RT_STRING {
            continue;
        }
        for (block, off) in image.directory(base, sub(off)?)? {
            if block & 0x8000_0000 != 0 {
                continue; // named string blocks do not occur
            }
            for (_lang, leaf) in image.directory(base, sub(off)?)? {
                if leaf & 0x8000_0000 != 0 {
                    return Err(PeError::Corrupt("unexpected resource depth"));
                }
                let entry = base + leaf as usize;
                let start = image.offset(u32_at(data, entry)?)?;
                let size = u32_at(data, entry + 4)? as usize;
                let bytes = data
                    .get(start..start + size)
                    .ok_or(PeError::Corrupt("string block overrun"))?;
                let mut pos = 0;
                for i in 0..16 {
                    let len = u16_at(bytes, pos)? as usize;
                    pos += 2;
                    let units = bytes
                        .get(pos..pos + len * 2)
                        .ok_or(PeError::Corrupt("string overrun"))?
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|c| u16::from_le_bytes(*c));
                    pos += len * 2;
                    if len > 0 {
                        strings.entry((block - 1) * 16 + i).or_insert_with(|| {
                            String::from_utf16_lossy(&units.collect::<Vec<_>>())
                        });
                    }
                }
            }
        }
    }
    Ok(strings)
}

const RT_BITMAP: u32 = 2;

/// A decoded `RT_BITMAP` resource.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bitmap {
    pub width: usize,
    pub height: usize,
    /// Opaque RGBA, top row first.
    pub rgba: Vec<u8>,
}

/// Raw data of resource `kind`/`id` (first language).
fn resource(data: &[u8], kind: u32, id: u32) -> Result<Option<&[u8]>, PeError> {
    let image = Image::parse(data)?;
    if image.resources == 0 {
        return Ok(None);
    }
    let base = image.offset(image.resources)?;
    let find = |dir: usize, want: u32| -> Result<Option<u32>, PeError> {
        Ok(image
            .directory(base, dir)?
            .into_iter()
            .find(|&(name, _)| name == want)
            .map(|(_, off)| off))
    };
    let sub = |off: u32| {
        if off & 0x8000_0000 != 0 {
            Ok((off & 0x7FFF_FFFF) as usize)
        } else {
            Err(PeError::Corrupt("expected a resource subdirectory"))
        }
    };
    let Some(types) = find(0, kind)? else {
        return Ok(None);
    };
    let Some(langs) = find(sub(types)?, id)? else {
        return Ok(None);
    };
    let Some(&(_, leaf)) = image.directory(base, sub(langs)?)?.first() else {
        return Ok(None);
    };
    if leaf & 0x8000_0000 != 0 {
        return Err(PeError::Corrupt("unexpected resource depth"));
    }
    let entry = base + leaf as usize;
    let start = image.offset(u32_at(data, entry)?)?;
    let size = u32_at(data, entry + 4)? as usize;
    data.get(start..start + size)
        .map(Some)
        .ok_or(PeError::Corrupt("resource overrun"))
}

/// Decodes bitmap resource `id` (uncompressed 8- or 24-bit DIB), if present.
pub fn bitmap(data: &[u8], id: u32) -> Result<Option<Bitmap>, PeError> {
    let Some(dib) = resource(data, RT_BITMAP, id)? else {
        return Ok(None);
    };
    let header = u32_at(dib, 0)? as usize;
    let width = u32_at(dib, 4)? as i32;
    let height = u32_at(dib, 8)? as i32;
    let bits = u16_at(dib, 14)?;
    let compression = u32_at(dib, 16)?;
    if width <= 0 || height == 0 || compression != 0 || !matches!(bits, 8 | 24) {
        return Err(PeError::Corrupt("unsupported bitmap"));
    }
    let (width, rows) = (width as usize, height.unsigned_abs() as usize);
    let colours = match (bits, u32_at(dib, 32)? as usize) {
        (8, 0) => 256,
        (8, n) => n.min(256),
        _ => 0,
    };
    let palette = dib
        .get(header..header + colours * 4)
        .ok_or(PeError::Corrupt("bitmap palette overrun"))?;
    let pixels = header + colours * 4;
    let stride = (width * bits as usize / 8).div_ceil(4) * 4;
    let mut rgba = Vec::with_capacity(width * rows * 4);
    for row in 0..rows {
        let src = if height > 0 { rows - 1 - row } else { row };
        let line = dib
            .get(pixels + src * stride..pixels + src * stride + stride)
            .ok_or(PeError::Corrupt("bitmap overrun"))?;
        for x in 0..width {
            let [b, g, r] = if bits == 8 {
                let i = line[x] as usize * 4;
                palette.get(i..i + 3).map_or([0; 3], |c| [c[0], c[1], c[2]])
            } else {
                [line[x * 3], line[x * 3 + 1], line[x * 3 + 2]]
            };
            rgba.extend([r, g, b, 255]);
        }
    }
    Ok(Some(Bitmap {
        width,
        height: rows,
        rgba,
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejects_non_pe() {
        assert_eq!(super::bitmap(b"hello", 1), Err(super::PeError::NotPe));
        assert_eq!(
            super::string_table(b"MZ"),
            Err(super::PeError::Corrupt("read past end"))
        );
        assert_eq!(super::string_table(b"hello"), Err(super::PeError::NotPe));
    }
}
