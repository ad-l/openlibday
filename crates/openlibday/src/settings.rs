//! User options read from `openlibday.ini`.
//!
//! The file lives in the per-user preferences directory (`%APPDATA%\openlibday\`
//! on Windows, `~/.local/share/openlibday/` on Linux) unless `--options FILE`
//! names another one. A commented default is written when it is missing.
//! Unknown keys and invalid values are ignored (and reported on stderr).

use std::path::{Path, PathBuf};

use libday_formats::Ini;

pub const FILE_NAME: &str = "openlibday.ini";
/// Largest interface scale.
pub const MAX_SCALE: i32 = 4;

pub const DEFAULT_FILE: &str = "\
; openlibday options. Delete this file to restore the defaults.

[interface]
; Size of the interface (sidebar, bottom bar, dialogs) as a multiple of the
; original 1024x768 artwork: 1 (original) to 4. The I key changes it in game.
scale = 2

[window]
; Initial window size in screen pixels; 0 fits the window to the screen.
width = 0
height = 0
; Start in borderless fullscreen (true/false).
fullscreen = false

[sound]
; Volume of the animation sounds: 0 (off) to 100.
volume = 100
";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub scale: i32,
    /// `0` means fit to the screen.
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    /// 0 (off) to 100.
    pub volume: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            scale: 2,
            width: 0,
            height: 0,
            fullscreen: false,
            volume: 100,
        }
    }
}

impl Settings {
    /// Parses `text`, keeping defaults for missing keys; returns the settings
    /// and a warning per invalid value.
    pub fn parse(text: &str) -> (Self, Vec<String>) {
        let ini = Ini::parse(text);
        let mut settings = Self::default();
        let mut warnings = Vec::new();
        let mut read = |section: &str, key: &str, apply: &mut dyn FnMut(&str) -> bool| {
            if let Some(value) = ini.get(section, key)
                && !apply(value)
            {
                warnings.push(format!(
                    "[{section}] {key} = {value}: invalid value, ignored"
                ));
            }
        };
        read("interface", "scale", &mut |v| {
            parse_in(v, 1, MAX_SCALE)
                .map(|s| settings.scale = s)
                .is_some()
        });
        read("window", "width", &mut |v| {
            v.parse().map(|w| settings.width = w).is_ok()
        });
        read("window", "height", &mut |v| {
            v.parse().map(|h| settings.height = h).is_ok()
        });
        read("window", "fullscreen", &mut |v| {
            parse_bool(v).map(|f| settings.fullscreen = f).is_some()
        });
        read("sound", "volume", &mut |v| {
            parse_in(v, 0, 100)
                .map(|n| settings.volume = n as u32)
                .is_some()
        });
        (settings, warnings)
    }

    /// Loads `path`, writing the default file first if it does not exist.
    /// Problems are reported on stderr and never fatal.
    pub fn load(path: &Path) -> Self {
        if !path.exists()
            && let Err(e) = std::fs::write(path, DEFAULT_FILE)
        {
            eprintln!("openlibday: cannot write {}: {e}", path.display());
        }
        match std::fs::read(path) {
            Ok(bytes) => {
                let (settings, warnings) = Self::parse(&String::from_utf8_lossy(&bytes));
                for warning in warnings {
                    eprintln!("openlibday: {}: {warning}", path.display());
                }
                settings
            }
            Err(e) => {
                eprintln!("openlibday: cannot read {}: {e}", path.display());
                Self::default()
            }
        }
    }
}

/// The default options file in the user's preferences directory.
pub fn default_path() -> Option<PathBuf> {
    sdl3::filesystem::get_pref_path("", "openlibday")
        .ok()
        .map(|dir| dir.join(FILE_NAME))
}

fn parse_in(value: &str, min: i32, max: i32) -> Option<i32> {
    value.parse().ok().filter(|v| (min..=max).contains(v))
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_matches_defaults() {
        assert_eq!(Settings::parse(DEFAULT_FILE), (Settings::default(), vec![]));
        assert_eq!(Settings::parse(""), (Settings::default(), vec![]));
    }

    #[test]
    fn values_and_invalid_values() {
        let (s, warnings) = Settings::parse(
            "[Interface]\nScale = 1\n[window]\nwidth=1600\nheight = x\nfullscreen = yes\n",
        );
        assert_eq!(
            s,
            Settings {
                scale: 1,
                width: 1600,
                height: 0,
                fullscreen: true,
                volume: 100,
            }
        );
        assert_eq!(warnings.len(), 1);
        let (s, warnings) = Settings::parse("[interface]\nscale = 9\n");
        assert_eq!((s.scale, warnings.len()), (2, 1));
    }
}
