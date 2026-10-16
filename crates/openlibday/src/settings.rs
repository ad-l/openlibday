//! User options read from `openlibday.ini`.
//!
//! The file lives in the per-user preferences directory (`%APPDATA%\openlibday\`
//! on Windows, `~/.local/share/openlibday/` on Linux) unless `--options FILE`
//! names another one. A commented default is written when it is missing, and
//! the in-game OPTIONS menu rewrites it when an option changes. Unknown keys
//! and invalid values are ignored (and reported on stderr).

use std::path::{Path, PathBuf};

use libday_formats::Ini;

pub const FILE_NAME: &str = "openlibday.ini";
/// Largest interface scale.
pub const MAX_SCALE: i32 = 4;
/// Map zoom levels in percent, from the wheel and +/- keys.
pub const ZOOMS: [u32; 9] = [25, 50, 75, 100, 150, 200, 300, 400, 600];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub scale: i32,
    /// How far the arrow keys scroll the map.
    pub scroll: ScrollSpeed,
    /// Map zoom at start-up, in percent of the original map pixels (one of
    /// [`ZOOMS`]).
    pub zoom: u32,
    /// How unit moves are animated.
    pub moves: MoveSpeed,
    /// Inspecting an enemy shows the tiles it covers on overwatch.
    pub overwatch_zone: bool,
    /// Play the introduction video at start-up (the original's `Intro`).
    pub intro: bool,
    /// `0` means fit to the screen.
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    /// 0 (off) to 100.
    pub volume: u32,
    pub ai: AiMode,
}

/// How faithfully the computer player follows the original.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AiMode {
    /// The original's behaviour with its bugs fixed.
    #[default]
    Modern,
    /// Faithful to the original, bugs included.
    Legacy,
}

impl AiMode {
    pub const ALL: [Self; 2] = [Self::Modern, Self::Legacy];

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|m| m.name().eq_ignore_ascii_case(value))
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Modern => "modern",
            Self::Legacy => "legacy",
        }
    }
}

/// The original's SCROLL SPEED option.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollSpeed {
    Slow,
    #[default]
    Normal,
    Fast,
}

impl ScrollSpeed {
    pub const ALL: [Self; 3] = [Self::Slow, Self::Normal, Self::Fast];

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|m| m.name().eq_ignore_ascii_case(value))
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Slow => "slow",
            Self::Normal => "normal",
            Self::Fast => "fast",
        }
    }

    /// Map pixels an arrow key scrolls.
    pub fn step(self) -> f32 {
        match self {
            Self::Slow => 32.0,
            Self::Normal => 64.0,
            Self::Fast => 128.0,
        }
    }
}

/// How unit moves are animated (the original could only turn them off).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MoveSpeed {
    /// Every animation at its normal speed.
    #[default]
    Normal,
    /// The start and end animations at normal speed, the walk between
    /// them twice as fast.
    Fast,
    /// No animation: the unit appears at its destination.
    Instant,
}

impl MoveSpeed {
    pub const ALL: [Self; 3] = [Self::Normal, Self::Fast, Self::Instant];

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|m| m.name().eq_ignore_ascii_case(value))
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Fast => "fast",
            Self::Instant => "instant",
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            scale: 2,
            scroll: ScrollSpeed::Normal,
            zoom: 200,
            moves: MoveSpeed::Normal,
            overwatch_zone: true,
            intro: true,
            width: 0,
            height: 0,
            fullscreen: false,
            volume: 100,
            ai: AiMode::Modern,
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
        read("interface", "scroll", &mut |v| {
            ScrollSpeed::parse(v).map(|m| settings.scroll = m).is_some()
        });
        read("interface", "zoom", &mut |v| {
            v.parse()
                .ok()
                .filter(|z| ZOOMS.contains(z))
                .map(|z| settings.zoom = z)
                .is_some()
        });
        read("interface", "moves", &mut |v| {
            MoveSpeed::parse(v).map(|m| settings.moves = m).is_some()
        });
        read("interface", "overwatch_zone", &mut |v| {
            parse_bool(v).map(|b| settings.overwatch_zone = b).is_some()
        });
        read("interface", "intro", &mut |v| {
            parse_bool(v).map(|b| settings.intro = b).is_some()
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
        read("ai", "mode", &mut |v| {
            AiMode::parse(v).map(|m| settings.ai = m).is_some()
        });
        (settings, warnings)
    }

    /// The options file holding these settings, with comments.
    pub fn to_ini(&self) -> String {
        format!(
            "\
; openlibday options. Delete this file to restore the defaults.

[interface]
; Size of the interface (sidebar, bottom bar, dialogs) as a multiple of the
; original 1024x768 artwork: 1 (original) to 4. The I key changes it in game.
scale = {scale}
; How far the arrow keys scroll the map: slow, normal or fast.
scroll = {scroll}
; Map zoom at start-up, in percent: 25, 50, 75, 100, 150, 200, 300, 400 or
; 600. The mouse wheel and +/- change the zoom in game.
zoom = {zoom}
; Unit move animations: normal; fast (the start and end animations at normal
; speed, the walk between them twice as fast); or instant (no animation, the
; unit appears at its destination).
moves = {moves}
; Inspecting an enemy unit shows in red the tiles it covers on overwatch
; (true/false).
overwatch_zone = {overwatch_zone}
; Play the introduction video at start-up (true/false). The main menu's
; INTRO button plays it at any time.
intro = {intro}

[window]
; Initial window size in screen pixels; 0 fits the window to the screen.
width = {width}
height = {height}
; Start in borderless fullscreen (true/false).
fullscreen = {fullscreen}

[sound]
; Volume of the animation sounds: 0 (off) to 100.
volume = {volume}

[ai]
; Computer player: modern (the original's behaviour with its bugs fixed) or
; legacy (faithful to the original, bugs included).
mode = {ai}
",
            scale = self.scale,
            scroll = self.scroll.name(),
            zoom = self.zoom,
            moves = self.moves.name(),
            overwatch_zone = self.overwatch_zone,
            intro = self.intro,
            width = self.width,
            height = self.height,
            fullscreen = self.fullscreen,
            volume = self.volume,
            ai = self.ai.name(),
        )
    }

    /// Writes the options file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_ini())
    }

    /// Loads `path`, writing the default file first if it does not exist.
    /// Problems are reported on stderr and never fatal.
    pub fn load(path: &Path) -> Self {
        if !path.exists()
            && let Err(e) = Self::default().save(path)
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
        let defaults = Settings::default();
        assert_eq!(Settings::parse(&defaults.to_ini()), (defaults, vec![]));
        assert_eq!(Settings::parse(""), (Settings::default(), vec![]));
    }

    #[test]
    fn saved_values_read_back() {
        let s = Settings {
            scale: 3,
            scroll: ScrollSpeed::Fast,
            zoom: 300,
            moves: MoveSpeed::Instant,
            overwatch_zone: false,
            intro: false,
            width: 1600,
            height: 900,
            fullscreen: true,
            volume: 40,
            ai: AiMode::Legacy,
        };
        assert_eq!(Settings::parse(&s.to_ini()), (s, vec![]));
    }

    #[test]
    fn values_and_invalid_values() {
        let (s, warnings) = Settings::parse(
            "[Interface]\nScale = 1\nzoom = 150\nmoves = Fast\n[window]\nwidth=1600\nheight = x\nfullscreen = yes\n[ai]\nmode = Legacy\n",
        );
        assert_eq!(
            s,
            Settings {
                scale: 1,
                scroll: ScrollSpeed::Normal,
                zoom: 150,
                moves: MoveSpeed::Fast,
                overwatch_zone: true,
                intro: true,
                width: 1600,
                height: 0,
                fullscreen: true,
                volume: 100,
                ai: AiMode::Legacy,
            }
        );
        assert_eq!(warnings.len(), 1);
        let (s, warnings) = Settings::parse("[interface]\nscale = 9\nzoom = 120\nmoves = slow\n");
        assert_eq!(
            (s.scale, s.zoom, s.moves, warnings.len()),
            (2, 200, MoveSpeed::Normal, 3)
        );
    }
}
