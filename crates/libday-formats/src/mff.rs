//! `.FF` animation containers (`ANMSUNIT\ANMSUNIT.FF`).
//!
//! Layout (little-endian): magic `MFF\0`, u32 entry count, then `count`
//! 264-byte directory entries (name[256] NUL-padded with backslash paths,
//! u32 absolute offset, u32 zero). Member data follow the directory
//! back to back; entries are not sorted by offset, so a member's size is
//! the distance to the next offset in file order (the last runs to EOF).

use std::collections::HashMap;

use thiserror::Error;

const MAGIC: &[u8; 4] = b"MFF\0";
const ENTRY: usize = 264;
const NAME: usize = 256;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MffError {
    #[error("not an MFF container")]
    BadMagic,
    #[error("MFF container is truncated")]
    Truncated,
    #[error("invalid MFF entry {0}")]
    InvalidEntry(String),
    #[error("duplicate MFF entry {0}")]
    Duplicate(String),
    #[error("no MFF entry named {0}")]
    NotFound(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MffEntry {
    pub name: String,
    pub offset: usize,
    pub size: usize,
}

/// A parsed container borrowing the file bytes.
#[derive(Debug)]
pub struct Mff<'a> {
    data: &'a [u8],
    entries: Vec<MffEntry>,
    index: HashMap<String, usize>,
}

fn key(name: &str) -> String {
    name.replace('/', "\\").to_ascii_uppercase()
}

impl<'a> Mff<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, MffError> {
        if data.len() < 8 || &data[..4] != MAGIC {
            return Err(MffError::BadMagic);
        }
        let count = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
        let start = count
            .checked_mul(ENTRY)
            .and_then(|n| n.checked_add(8))
            .filter(|&n| n <= data.len())
            .ok_or(MffError::Truncated)?;
        let mut entries = Vec::with_capacity(count);
        let mut index = HashMap::with_capacity(count);
        for i in 0..count {
            let raw = &data[8 + i * ENTRY..8 + (i + 1) * ENTRY];
            let end = raw[..NAME].iter().position(|&b| b == 0).unwrap_or(NAME);
            let name = String::from_utf8_lossy(&raw[..end]).into_owned();
            let offset = u32::from_le_bytes(raw[NAME..NAME + 4].try_into().unwrap()) as usize;
            if name.is_empty() || offset < start || offset > data.len() {
                return Err(MffError::InvalidEntry(name));
            }
            if index.insert(key(&name), i).is_some() {
                return Err(MffError::Duplicate(name));
            }
            entries.push(MffEntry {
                name,
                offset,
                size: 0,
            });
        }
        let mut order: Vec<usize> = (0..count).collect();
        order.sort_by_key(|&i| entries[i].offset);
        for (n, &i) in order.iter().enumerate() {
            let next = order.get(n + 1).map_or(data.len(), |&j| entries[j].offset);
            entries[i].size = next - entries[i].offset;
        }
        Ok(Self {
            data,
            entries,
            index,
        })
    }

    /// Entries in directory order.
    pub fn entries(&self) -> &[MffEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Looks up a member by case-insensitive name (`/` or `\` separators).
    pub fn entry(&self, name: &str) -> Option<&MffEntry> {
        self.index.get(&key(name)).map(|&i| &self.entries[i])
    }

    pub fn get(&self, name: &str) -> Result<&'a [u8], MffError> {
        let entry = self
            .entry(name)
            .ok_or_else(|| MffError::NotFound(name.to_owned()))?;
        Ok(&self.data[entry.offset..entry.offset + entry.size])
    }

    /// Rebuilds the container with some members replaced, keeping the
    /// directory order and the original data order of every member.
    pub fn rebuild(&self, replacements: &HashMap<String, Vec<u8>>) -> Result<Vec<u8>, MffError> {
        let mut replaced: HashMap<String, &[u8]> = HashMap::new();
        for (name, bytes) in replacements {
            let entry = self
                .entry(name)
                .ok_or_else(|| MffError::NotFound(name.clone()))?;
            replaced.insert(key(&entry.name), bytes);
        }
        let mut order: Vec<usize> = (0..self.entries.len()).collect();
        order.sort_by_key(|&i| self.entries[i].offset);
        let start = 8 + self.entries.len() * ENTRY;
        let mut offsets = vec![0usize; self.entries.len()];
        let mut body = Vec::with_capacity(self.data.len());
        for &i in &order {
            let entry = &self.entries[i];
            offsets[i] = start + body.len();
            match replaced.get(&key(&entry.name)) {
                Some(bytes) => body.extend_from_slice(bytes),
                None => body.extend_from_slice(&self.data[entry.offset..entry.offset + entry.size]),
            }
        }
        if start + body.len() > u32::MAX as usize {
            return Err(MffError::InvalidEntry("container exceeds 4 GiB".into()));
        }
        let mut out = Vec::with_capacity(start + body.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (i, offset) in offsets.iter().enumerate() {
            let raw = &self.data[8 + i * ENTRY..8 + (i + 1) * ENTRY];
            out.extend_from_slice(&raw[..NAME]);
            out.extend_from_slice(&(*offset as u32).to_le_bytes());
            out.extend_from_slice(&raw[NAME + 4..ENTRY]);
        }
        out.extend(body);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(members: &[(&str, &[u8])], reverse: bool) -> Vec<u8> {
        let start = 8 + members.len() * ENTRY;
        let mut offsets = Vec::new();
        let mut body = Vec::new();
        let order: Vec<usize> = if reverse {
            (0..members.len()).rev().collect()
        } else {
            (0..members.len()).collect()
        };
        offsets.resize(members.len(), 0);
        for &i in &order {
            offsets[i] = start + body.len();
            body.extend_from_slice(members[i].1);
        }
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&(members.len() as u32).to_le_bytes());
        for (i, (name, _)) in members.iter().enumerate() {
            let mut raw = [0u8; ENTRY];
            raw[..name.len()].copy_from_slice(name.as_bytes());
            raw[NAME..NAME + 4].copy_from_slice(&(offsets[i] as u32).to_le_bytes());
            out.extend_from_slice(&raw);
        }
        out.extend(body);
        out
    }

    #[test]
    fn sizes_follow_data_order() {
        let data = build(&[("A\\X.SMK", b"aaa"), ("B\\Y.SMK", b"bbbbb")], true);
        let mff = Mff::parse(&data).unwrap();
        assert_eq!(mff.get("a/x.smk").unwrap(), b"aaa");
        assert_eq!(mff.get("B\\Y.SMK").unwrap(), b"bbbbb");
        assert!(matches!(mff.get("C"), Err(MffError::NotFound(_))));
    }

    #[test]
    fn rebuild_replaces_and_round_trips() {
        let data = build(&[("A", b"aaa"), ("B", b"bbbbb"), ("C", b"c")], true);
        let mff = Mff::parse(&data).unwrap();
        assert_eq!(mff.rebuild(&HashMap::new()).unwrap(), data);
        let mut changes = HashMap::new();
        changes.insert("b".to_owned(), b"NEW".to_vec());
        let out = mff.rebuild(&changes).unwrap();
        let rebuilt = Mff::parse(&out).unwrap();
        assert_eq!(rebuilt.get("A").unwrap(), b"aaa");
        assert_eq!(rebuilt.get("B").unwrap(), b"NEW");
        assert_eq!(rebuilt.get("C").unwrap(), b"c");
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(Mff::parse(b"XXXX").unwrap_err(), MffError::BadMagic);
        let mut data = build(&[("A", b"a")], false);
        data.truncate(100);
        assert_eq!(Mff::parse(&data).unwrap_err(), MffError::Truncated);
    }
}
