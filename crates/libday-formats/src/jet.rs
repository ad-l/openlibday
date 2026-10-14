//! Read-only Jet 3 (Access 97) database reader.
//!
//! All Liberation Day databases (`GLOBAL.MDB`, `GAMEINFO.MDB`, scenario
//! catalogs and the scenario MDBs inside the ZIP archives) are Jet 3 files
//! with 2048-byte pages and CP1252 text. Database passwords only live in
//! the obfuscated header and do not encrypt pages, so no password is needed.
//!
//! References: the MDB Tools `HACKING` notes and Jackcess. Layout summary:
//!
//! * Page 0: header. Bytes `0x18..0x96` are RC4-masked with key
//!   `0x6b39dac7`; inside are the code page (`0x3C`), page encryption key
//!   (`0x3E`, 0 = none) and password (`0x42`, 20 bytes).
//! * Table definition (`0x02`) pages: column descriptors and names,
//!   continued through `next_pg`; page 2 is the `MSysObjects` catalog.
//! * Data (`0x01`) pages: row offsets after a 10-byte header, rows packed
//!   from the end of the page. Offset flag `0x8000` = deleted,
//!   `0x4000` = overflow (row holds a pointer to the real row).
//! * Usage maps (inline bitmap or bitmap pages) list a table's data pages.

use thiserror::Error;

use crate::cp1252;

pub const PAGE_SIZE: usize = 2048;
const HEADER_KEY: u32 = 0x6b39_dac7;
const HEADER_MASKED: std::ops::Range<usize> = 0x18..0x18 + 126;
const OFFSET_MASK: u16 = 0x1fff;
const DELETED: u16 = 0x8000;
const OVERFLOW: u16 = 0x4000;
const TDEF_COLUMNS: usize = 43;
const COLUMN_ENTRY: usize = 18;
const CATALOG_PAGE: u32 = 2;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum JetError {
    #[error("not a Jet database")]
    NotJet,
    #[error("unsupported Jet version {0} (only Jet 3 / Access 97 is supported)")]
    UnsupportedVersion(u8),
    #[error("database pages are encrypted")]
    Encrypted,
    #[error("page {0} is out of range")]
    PageOutOfRange(u32),
    #[error("corrupt database: {what} (page {page})")]
    Corrupt { what: &'static str, page: u32 },
    #[error("no table named {0}")]
    NoTable(String),
}

type Result<T> = std::result::Result<T, JetError>;

fn corrupt<T>(what: &'static str, page: u32) -> Result<T> {
    Err(JetError::Corrupt { what, page })
}

fn u16_at(d: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([d[p], d[p + 1]])
}

fn u32_at(d: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(d[p..p + 4].try_into().unwrap())
}

/// `(page, row)` from a packed row pointer (row in the low byte).
fn row_pointer(raw: u32) -> (u32, u8) {
    (raw >> 8, raw as u8)
}

fn rc4(key: &[u8], data: &mut [u8]) {
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    for byte in data {
        i = i.wrapping_add(1);
        j = j.wrapping_add(s[i as usize]);
        s.swap(i as usize, j as usize);
        *byte ^= s[s[i as usize].wrapping_add(s[j as usize]) as usize];
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnType {
    Bool,
    Byte,
    Int,
    Long,
    Money,
    Float,
    Double,
    DateTime,
    Binary,
    Text,
    Ole,
    Memo,
    Guid,
    Numeric,
    Unknown(u8),
}

impl ColumnType {
    fn from_code(code: u8) -> Self {
        match code {
            0x01 => Self::Bool,
            0x02 => Self::Byte,
            0x03 => Self::Int,
            0x04 => Self::Long,
            0x05 => Self::Money,
            0x06 => Self::Float,
            0x07 => Self::Double,
            0x08 => Self::DateTime,
            0x09 => Self::Binary,
            0x0A => Self::Text,
            0x0B => Self::Ole,
            0x0C => Self::Memo,
            0x0F => Self::Guid,
            0x10 => Self::Numeric,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub kind: ColumnType,
    /// Column number (null-mask bit; includes deleted columns).
    pub number: u16,
    /// Declared size in bytes (0 for memo/OLE).
    pub size: u16,
    pub flags: u8,
    var_index: u16,
    fixed_offset: u16,
}

impl Column {
    pub fn is_fixed(&self) -> bool {
        self.flags & 0x01 != 0
    }

    pub fn is_autonumber(&self) -> bool {
        self.flags & 0x04 != 0
    }
}

/// A decoded field value. Variants preserve the stored representation.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Byte(u8),
    Int(i16),
    Long(i32),
    /// Currency in 1/10000 units.
    Money(i64),
    Float(f32),
    Double(f64),
    /// Days since 1899-12-30 (OLE automation date).
    DateTime(f64),
    Binary(Vec<u8>),
    Text(String),
    Ole(Vec<u8>),
    Memo(String),
    Guid([u8; 16]),
    /// Types this reader does not interpret, kept as raw bytes.
    Raw(Vec<u8>),
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Integer view of integral and boolean values (Jet `True` is -1).
    pub fn as_i64(&self) -> Option<i64> {
        Some(match *self {
            Value::Bool(b) => -(b as i64),
            Value::Byte(v) => v as i64,
            Value::Int(v) => v as i64,
            Value::Long(v) => v as i64,
            _ => return None,
        })
    }

    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Value::Float(v) => Some(v as f64),
            Value::Double(v) | Value::DateTime(v) => Some(v),
            Value::Money(v) => Some(v as f64 / 10_000.0),
            _ => self.as_i64().map(|v| v as f64),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(s) | Value::Memo(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Value::Bool(b) => Some(b),
            _ => self.as_i64().map(|v| v != 0),
        }
    }
}

/// Calendar parts `(year, month, day, hour, minute, second)` of an OLE
/// date, rounded to the nearest second. The fraction is the absolute time
/// of day, also for dates before 1899-12-30.
pub fn datetime_parts(days: f64) -> (i64, u32, u32, u32, u32, u32) {
    let whole = days.trunc();
    let mut seconds = ((days - whole).abs() * 86_400.0).round() as i64;
    let mut day = whole as i64 - 25_569; // days since 1970-01-01
    if seconds >= 86_400 {
        seconds -= 86_400;
        day += 1;
    }
    // Howard Hinnant's civil_from_days.
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    let s = seconds as u32;
    (y, m, d, s / 3600, s / 60 % 60, s % 60)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableEntry {
    pub name: String,
    pub tdef_page: u32,
    /// `MSysObjects.Flags` marks it as a system/hidden table.
    pub system: bool,
}

#[derive(Clone, Debug)]
pub struct Table {
    pub name: String,
    pub tdef_page: u32,
    /// Columns in column-number order.
    pub columns: Vec<Column>,
    /// Row count recorded in the table definition.
    pub row_count: u32,
    num_var_cols: u16,
    usage_map: u32,
}

impl Table {
    /// Case-insensitive column index.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(name))
    }
}

pub struct Database {
    data: Vec<u8>,
    pub code_page: u16,
    /// Database password from the header, if any (informational only).
    pub password: Option<String>,
    catalog: Vec<TableEntry>,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("pages", &(self.data.len() / PAGE_SIZE))
            .field("tables", &self.catalog.len())
            .finish()
    }
}

impl Database {
    pub fn open(data: Vec<u8>) -> Result<Self> {
        if data.len() < PAGE_SIZE || data[..4] != [0, 1, 0, 0] || &data[4..19] != b"Standard Jet DB"
        {
            return Err(JetError::NotJet);
        }
        if data[0x14] != 0 {
            return Err(JetError::UnsupportedVersion(data[0x14]));
        }
        let mut header = data[HEADER_MASKED].to_vec();
        rc4(&HEADER_KEY.to_le_bytes(), &mut header);
        let at = |p: usize| p - HEADER_MASKED.start;
        if u32_at(&header, at(0x3E)) != 0 {
            return Err(JetError::Encrypted);
        }
        let code_page = u16_at(&header, at(0x3C));
        let raw = &header[at(0x42)..at(0x42) + 20];
        let raw = &raw[..raw.iter().position(|&b| b == 0).unwrap_or(raw.len())];
        let password = (!raw.is_empty()).then(|| cp1252::decode(raw));
        let mut db = Self {
            data,
            code_page,
            password,
            catalog: Vec::new(),
        };
        db.catalog = db.read_catalog()?;
        Ok(db)
    }

    pub fn page_count(&self) -> u32 {
        (self.data.len() / PAGE_SIZE) as u32
    }

    fn page(&self, n: u32) -> Result<&[u8]> {
        let start = n as usize * PAGE_SIZE;
        self.data
            .get(start..start + PAGE_SIZE)
            .filter(|_| n != 0)
            .ok_or(JetError::PageOutOfRange(n))
    }

    /// Raw row bytes and offset flags.
    fn row(&self, page: u32, row: u8) -> Result<(&[u8], u16)> {
        let p = self.page(page)?;
        if p[0] != 0x01 {
            return corrupt("row pointer to a non-data page", page);
        }
        let count = u16_at(p, 8) as usize;
        let row = row as usize;
        if row >= count || 10 + count * 2 > PAGE_SIZE {
            return corrupt("row number out of range", page);
        }
        let raw = u16_at(p, 10 + row * 2);
        let start = (raw & OFFSET_MASK) as usize;
        let end = if row == 0 {
            PAGE_SIZE
        } else {
            (u16_at(p, 10 + (row - 1) * 2) & OFFSET_MASK) as usize
        };
        if start < 10 + count * 2 || start > end || end > PAGE_SIZE {
            return corrupt("invalid row offsets", page);
        }
        Ok((&p[start..end], raw & !OFFSET_MASK))
    }

    /// Live row contents, following overflow pointers. Overflow targets are
    /// themselves flagged deleted (so scans skip them); only the first hop
    /// is checked for deletion.
    fn row_data(&self, mut page: u32, mut row: u8) -> Result<Option<&[u8]>> {
        for hop in 0..16 {
            let (bytes, flags) = self.row(page, row)?;
            if hop == 0 && flags & DELETED != 0 {
                return Ok(None);
            }
            if flags & OVERFLOW == 0 {
                return Ok(Some(bytes));
            }
            if bytes.len() < 4 {
                return corrupt("short overflow pointer", page);
            }
            (page, row) = row_pointer(u32_at(bytes, 0));
        }
        corrupt("overflow pointer loop", page)
    }

    fn read_tdef(&self, first: u32) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        let mut page = first;
        for _ in 0..self.page_count() {
            let p = self.page(page)?;
            if p[0] != 0x02 {
                return corrupt("expected a table definition page", page);
            }
            buf.extend_from_slice(if buf.is_empty() { p } else { &p[8..] });
            page = u32_at(p, 4);
            if page == 0 {
                return Ok(buf);
            }
        }
        corrupt("table definition loop", first)
    }

    fn load_table(&self, name: &str, tdef_page: u32) -> Result<Table> {
        let t = self.read_tdef(tdef_page)?;
        let bad = |what| JetError::Corrupt {
            what,
            page: tdef_page,
        };
        let row_count = u32_at(&t, 12);
        let num_var_cols = u16_at(&t, 23);
        let num_cols = u16_at(&t, 25) as usize;
        let num_real_idx = u32_at(&t, 31) as usize;
        let usage_map = u32_at(&t, 35);
        let mut pos = TDEF_COLUMNS + num_real_idx.checked_mul(8).ok_or(bad("index count"))?;
        let names_start = pos + num_cols * COLUMN_ENTRY;
        if names_start > t.len() {
            return Err(bad("column descriptors overrun"));
        }
        let mut columns = Vec::with_capacity(num_cols);
        let mut name_pos = names_start;
        for _ in 0..num_cols {
            let c = &t[pos..pos + COLUMN_ENTRY];
            let len = *t.get(name_pos).ok_or(bad("column names overrun"))? as usize;
            let raw = t
                .get(name_pos + 1..name_pos + 1 + len)
                .ok_or(bad("column names overrun"))?;
            columns.push(Column {
                name: cp1252::decode(raw),
                kind: ColumnType::from_code(c[0]),
                number: u16_at(c, 1),
                var_index: u16_at(c, 3),
                flags: c[13],
                fixed_offset: u16_at(c, 14),
                size: u16_at(c, 16),
            });
            pos += COLUMN_ENTRY;
            name_pos += 1 + len;
        }
        columns.sort_by_key(|c| c.number);
        Ok(Table {
            name: name.to_owned(),
            tdef_page,
            columns,
            row_count,
            num_var_cols,
            usage_map,
        })
    }

    /// Data pages of a table, ascending, from its usage map.
    fn data_pages(&self, table: &Table) -> Result<Vec<u32>> {
        let (page, row) = row_pointer(table.usage_map);
        let map = self.row_data(page, row)?.ok_or(JetError::Corrupt {
            what: "deleted usage map",
            page,
        })?;
        let mut pages = Vec::new();
        let mut collect = |base: u32, bits: &[u8]| {
            for (i, &byte) in bits.iter().enumerate() {
                for bit in 0..8 {
                    if byte & (1 << bit) != 0 {
                        pages.push(base + (i * 8 + bit) as u32);
                    }
                }
            }
        };
        match map.first() {
            Some(0) if map.len() >= 5 => collect(u32_at(map, 1), &map[5..]),
            Some(1) => {
                let per_page = ((PAGE_SIZE - 4) * 8) as u32;
                for (i, chunk) in map[1..].as_chunks::<4>().0.iter().enumerate() {
                    let bitmap_page = u32::from_le_bytes(*chunk);
                    if bitmap_page != 0 {
                        let p = self.page(bitmap_page)?;
                        if p[0] != 0x05 {
                            return corrupt("expected a usage bitmap page", bitmap_page);
                        }
                        collect(i as u32 * per_page, &p[4..]);
                    }
                }
            }
            _ => return corrupt("unknown usage map type", page),
        }
        Ok(pages)
    }

    /// Long value (memo/OLE) contents.
    fn long_value(&self, field: &[u8], page: u32) -> Result<Vec<u8>> {
        if field.len() < 12 {
            return corrupt("short long-value field", page);
        }
        let header = u32_at(field, 0);
        let len = (header & 0x3fff_ffff) as usize;
        if header & 0x8000_0000 != 0 {
            return field
                .get(12..12 + len)
                .map(<[u8]>::to_vec)
                .ok_or(JetError::Corrupt {
                    what: "inline long value overrun",
                    page,
                });
        }
        let (mut lpage, mut lrow) = row_pointer(u32_at(field, 4));
        if header & 0x4000_0000 != 0 {
            let (bytes, _) = self.row(lpage, lrow)?;
            return bytes
                .get(..len)
                .map(<[u8]>::to_vec)
                .ok_or(JetError::Corrupt {
                    what: "long value overrun",
                    page: lpage,
                });
        }
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            let (bytes, _) = self.row(lpage, lrow)?;
            if bytes.len() < 4 {
                return corrupt("short long-value chain row", lpage);
            }
            let take = (len - out.len()).min(bytes.len() - 4);
            out.extend_from_slice(&bytes[4..4 + take]);
            let next = u32_at(bytes, 0);
            if out.len() < len && next == 0 {
                return corrupt("truncated long-value chain", lpage);
            }
            (lpage, lrow) = row_pointer(next);
        }
        Ok(out)
    }

    fn crack(&self, table: &Table, row: &[u8], page: u32) -> Result<Vec<Value>> {
        let bad = |what| JetError::Corrupt { what, page };
        if row.is_empty() {
            return Err(bad("empty row"));
        }
        let row_cols = row[0] as usize;
        let mask_size = row_cols.div_ceil(8);
        let len = row.len();
        if len < 1 + mask_size + 1 {
            return Err(bad("row shorter than its null mask"));
        }
        let end = len - 1; // inclusive, as in MDB Tools
        let mask = &row[len - mask_size..];
        let var_cols = if table.num_var_cols > 0 {
            row[end - mask_size] as usize
        } else {
            0
        };
        let fixed_cols = row_cols.saturating_sub(var_cols);
        let mut var_offsets = Vec::new();
        if table.num_var_cols > 0 {
            let mut jumps = (len - 1) / 256;
            let ptr = end
                .checked_sub(mask_size + jumps + 1)
                .ok_or(bad("row offset table underflow"))?;
            if (ptr - var_cols) / 256 < jumps {
                jumps -= 1;
            }
            let mut used = 0;
            for i in 0..=var_cols {
                while used < jumps && i == row[end - mask_size - used - 1] as usize {
                    used += 1;
                }
                let at = ptr
                    .checked_sub(i)
                    .ok_or(bad("row offset table underflow"))?;
                var_offsets.push(row[at] as usize + used * 256);
            }
        }
        let mut fixed_found = 0;
        let mut values = Vec::with_capacity(table.columns.len());
        for col in &table.columns {
            let n = col.number as usize;
            let present = mask.get(n / 8).is_some_and(|b| b & (1 << (n % 8)) != 0);
            if col.kind == ColumnType::Bool {
                if col.is_fixed() {
                    fixed_found += 1;
                }
                values.push(Value::Bool(present));
                continue;
            }
            let field = if col.is_fixed() {
                if fixed_found >= fixed_cols {
                    None
                } else {
                    fixed_found += 1;
                    let start = 1 + col.fixed_offset as usize;
                    Some(
                        row.get(start..start + col.size as usize)
                            .ok_or(bad("fixed column overrun"))?,
                    )
                }
            } else if (col.var_index as usize) < var_cols {
                let i = col.var_index as usize;
                let (start, stop) = (var_offsets[i], var_offsets[i + 1]);
                Some(row.get(start..stop).ok_or(bad("variable column overrun"))?)
            } else {
                None
            };
            let Some(f) = field.filter(|_| present) else {
                values.push(Value::Null);
                continue;
            };
            let need = |n: usize| {
                if f.len() < n {
                    Err(bad("fixed column too short"))
                } else {
                    Ok(())
                }
            };
            values.push(match col.kind {
                ColumnType::Byte => {
                    need(1)?;
                    Value::Byte(f[0])
                }
                ColumnType::Int => {
                    need(2)?;
                    Value::Int(i16::from_le_bytes([f[0], f[1]]))
                }
                ColumnType::Long => {
                    need(4)?;
                    Value::Long(u32_at(f, 0) as i32)
                }
                ColumnType::Money => {
                    need(8)?;
                    Value::Money(i64::from_le_bytes(f[..8].try_into().unwrap()))
                }
                ColumnType::Float => {
                    need(4)?;
                    Value::Float(f32::from_bits(u32_at(f, 0)))
                }
                ColumnType::Double | ColumnType::DateTime => {
                    need(8)?;
                    let v = f64::from_bits(u64::from_le_bytes(f[..8].try_into().unwrap()));
                    if col.kind == ColumnType::Double {
                        Value::Double(v)
                    } else {
                        Value::DateTime(v)
                    }
                }
                ColumnType::Guid => {
                    need(16)?;
                    Value::Guid(f[..16].try_into().unwrap())
                }
                ColumnType::Text => Value::Text(cp1252::decode(f)),
                ColumnType::Binary => Value::Binary(f.to_vec()),
                ColumnType::Memo => Value::Memo(cp1252::decode(&self.long_value(f, page)?)),
                ColumnType::Ole => Value::Ole(self.long_value(f, page)?),
                ColumnType::Bool | ColumnType::Numeric | ColumnType::Unknown(_) => {
                    Value::Raw(f.to_vec())
                }
            });
        }
        Ok(values)
    }

    fn read_catalog(&self) -> Result<Vec<TableEntry>> {
        let sys = self.load_table("MSysObjects", CATALOG_PAGE)?;
        let col = |name| {
            sys.column_index(name).ok_or(JetError::Corrupt {
                what: "MSysObjects lacks a required column",
                page: CATALOG_PAGE,
            })
        };
        let (id, name, kind, flags) = (col("Id")?, col("Name")?, col("Type")?, col("Flags")?);
        let mut out = Vec::new();
        for row in self.rows(&sys)? {
            if row[kind].as_i64() != Some(1) {
                continue;
            }
            let (Some(id), Some(name)) = (row[id].as_i64(), row[name].as_str()) else {
                continue;
            };
            let flags = row[flags].as_i64().unwrap_or(0) as u32;
            out.push(TableEntry {
                name: name.to_owned(),
                tdef_page: id as u32 & 0x00ff_ffff,
                system: flags & 0x8000_0002 != 0,
            });
        }
        Ok(out)
    }

    /// Local tables from `MSysObjects` (including system tables).
    pub fn catalog(&self) -> &[TableEntry] {
        &self.catalog
    }

    /// Names of user tables in catalog order.
    pub fn table_names(&self) -> impl Iterator<Item = &str> {
        self.catalog
            .iter()
            .filter(|t| !t.system)
            .map(|t| t.name.as_str())
    }

    /// Table definition by case-insensitive name.
    pub fn table(&self, name: &str) -> Result<Table> {
        let entry = self
            .catalog
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| JetError::NoTable(name.to_owned()))?;
        self.load_table(&entry.name, entry.tdef_page)
    }

    /// All live rows in storage order, values in `table.columns` order.
    pub fn rows(&self, table: &Table) -> Result<Vec<Vec<Value>>> {
        let mut rows = Vec::new();
        for page in self.data_pages(table)? {
            let p = self.page(page)?;
            if p[0] != 0x01 || u32_at(p, 4) != table.tdef_page {
                continue;
            }
            for r in 0..u16_at(p, 8).min(255) as u8 {
                if let Some(bytes) = self.row_data(page, r)? {
                    rows.push(self.crack(table, bytes, page)?);
                }
            }
        }
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ole_dates() {
        assert_eq!(datetime_parts(0.0), (1899, 12, 30, 0, 0, 0));
        assert_eq!(datetime_parts(36526.5), (2000, 1, 1, 12, 0, 0));
        assert_eq!(datetime_parts(-1.25), (1899, 12, 29, 6, 0, 0));
        assert_eq!(datetime_parts(2.0 - 1e-9), (1900, 1, 1, 0, 0, 0));
    }

    #[test]
    fn rejects_non_jet() {
        assert_eq!(
            Database::open(vec![0; PAGE_SIZE]).unwrap_err(),
            JetError::NotJet
        );
    }
}
