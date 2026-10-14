//! Minimal case-insensitive INI reader for `LIBDAY.INI`.

use std::collections::BTreeMap;

/// Sections and keys are stored lower-cased; values are trimmed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ini {
    sections: BTreeMap<String, BTreeMap<String, String>>,
}

impl Ini {
    pub fn parse(text: &str) -> Self {
        let mut ini = Self::default();
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.trim().to_ascii_lowercase();
                ini.sections.entry(section.clone()).or_default();
            } else if let Some((key, value)) = line.split_once('=') {
                ini.sections
                    .entry(section.clone())
                    .or_default()
                    .entry(key.trim().to_ascii_lowercase())
                    .or_insert_with(|| value.trim().to_owned());
            }
        }
        ini
    }

    /// Parses Windows-1252 bytes.
    pub fn parse_bytes(bytes: &[u8]) -> Self {
        Self::parse(&crate::cp1252::decode(bytes))
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .get(&section.to_ascii_lowercase())?
            .get(&key.to_ascii_lowercase())
            .map(String::as_str)
    }

    pub fn section(&self, section: &str) -> Option<&BTreeMap<String, String>> {
        self.sections.get(&section.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_case_insensitively() {
        let ini = Ini::parse(
            "; c\n[Installation]\r\nImgs = F:\\libday\\IMGS\\\nexe=x\n[User]\nName=A=B\n",
        );
        assert_eq!(ini.get("installation", "IMGS"), Some("F:\\libday\\IMGS\\"));
        assert_eq!(ini.get("USER", "name"), Some("A=B"));
        assert_eq!(ini.get("user", "missing"), None);
    }
}
