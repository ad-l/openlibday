//! The battle interface: artwork from `INTERF\1024X768.SMK` and the layout of
//! the sidebar, bottom bar, map overview and goal dialog.
//!
//! The Smacker file is a sprite sheet rather than a movie. Each screen group
//! starts with a full 184x768 sidebar frame; every following frame draws one
//! element (button state, toggle box, dialog...) at the frame origin, so an
//! element is the `(0, 0, w, h)` crop of its frame. Magenta is transparent.
//!
//! Positions are in original 1024x768 pixels relative to their panel and are
//! multiplied by an integer UI scale.

use std::collections::BTreeMap;

use libday_formats::Smacker;

use crate::Image;

/// File name in the `INTERF` directory.
pub const FILE: &str = "1024X768.SMK";

/// Sidebar width and the bottom bar height, in original pixels.
pub const SIDEBAR_W: i32 = 184;
pub const SIDEBAR_H: i32 = 768;
pub const BOTTOM_H: i32 = 40;
pub const BOTTOM_W: i32 = 840;

/// Frame numbers of the elements.
pub mod frame {
    /// Map overview sidebar.
    pub const OVERVIEW_SIDEBAR: u16 = 41;
    /// First colour swatch (six frames per colour: red, blue, yellow, magenta, green).
    pub const SWATCH: u16 = 46;
    pub const UP: u16 = 76;
    pub const DOWN: u16 = 80;
    pub const BACK: u16 = 84;
    /// Battle sidebar (already includes the OVERWATCH / AUTO FIRE labels).
    pub const SIDEBAR: u16 = 226;
    pub const MAP: u16 = 231;
    pub const GOAL: u16 = 235;
    pub const GROUP: u16 = 241;
    pub const MOVE_ALL: u16 = 247;
    pub const OVERWATCH: u16 = 251;
    pub const FIRE: u16 = 258;
    pub const AUTO_FIRE: u16 = 264;
    pub const LEFT: u16 = 270;
    pub const RIGHT: u16 = 274;
    pub const GUARD: u16 = 278;
    pub const REINFORCEMENT: u16 = 282;
    pub const AUTOMATED: u16 = 286;
    pub const END_TURN: u16 = 290;
    pub const CENTER: u16 = 298;
    pub const BOTTOM_BAR: u16 = 498;
    pub const CHAT: u16 = 499;
    pub const GOAL_DIALOG: u16 = 633;
    pub const CONTINUE: u16 = 634;
    pub const OBJECTIVES: u16 = 638;
    pub const MENU: u16 = 693;
}

/// `(first frame, last frame, width, height)` of every element used.
const CROPS: &[(u16, u16, i32, i32)] = &[
    (41, 41, SIDEBAR_W, SIDEBAR_H),
    (46, 75, 19, 19),
    (76, 87, 53, 17),
    (226, 226, SIDEBAR_W, SIDEBAR_H),
    (231, 238, 53, 17),
    (241, 244, 53, 17),
    (247, 250, 110, 17),
    (251, 256, 19, 19),
    (258, 263, 110, 17),
    (264, 269, 19, 19),
    (270, 277, 19, 17),
    (278, 281, 53, 17),
    (282, 289, 110, 17),
    (290, 293, 53, 17),
    (298, 301, 53, 17),
    (498, 498, BOTTOM_W, BOTTOM_H),
    (499, 502, 53, 17),
    (633, 633, 449, 268),
    (634, 637, 110, 17),
    (638, 643, 19, 19),
    (693, 696, 53, 17),
];

fn crop_of(frame: u16) -> Option<(i32, i32)> {
    CROPS
        .iter()
        .find(|&&(first, last, ..)| (first..=last).contains(&frame))
        .map(|&(_, _, w, h)| (w, h))
}

/// The decoded interface elements.
#[derive(Clone, Debug, Default)]
pub struct UiArt {
    frames: BTreeMap<u16, Image>,
}

impl UiArt {
    /// Decodes the needed frames (streaming: the whole file would take ~800 MB).
    pub fn from_smacker(bytes: &[u8]) -> Result<Self, libday_formats::SmackerError> {
        let smk = Smacker::parse(bytes)?;
        let width = smk.header.width as usize;
        let last = CROPS.iter().map(|c| c.1).max().unwrap_or(0) as usize;
        let mut decoder = smk.decoder()?;
        let mut frames = BTreeMap::new();
        while decoder.position() <= last {
            let index = decoder.position() as u16;
            let Some(decoded) = decoder.next_frame()? else {
                break;
            };
            let Some((w, h)) = crop_of(index) else {
                continue;
            };
            let (w, h) = (w as usize, h as usize);
            let mut image = Image::new(w, h, [0, 0, 0, 0]);
            for y in 0..h {
                for x in 0..w {
                    let rgb = decoded.palette[decoded.pixels[y * width + x] as usize];
                    if rgb != [255, 0, 255] {
                        let at = (y * w + x) * 4;
                        image.rgba[at..at + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
                    }
                }
            }
            frames.insert(index, image);
        }
        Ok(Self { frames })
    }

    pub fn get(&self, frame: u16) -> Option<&Image> {
        self.frames.get(&frame)
    }

    pub fn frames(&self) -> impl Iterator<Item = (u16, &Image)> {
        self.frames.iter().map(|(&k, v)| (k, v))
    }
}

/// Integer pixel rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
}

/// Visual state of a button; the frame offset from its first (normal) frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Normal = 0,
    Hover = 1,
    Pressed = 2,
    Disabled = 3,
}

/// Every clickable interface element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    // Battle sidebar.
    Map,
    Goal,
    Group,
    Center,
    MoveAll,
    Overwatch,
    Fire,
    AutoFire,
    Left,
    Guard,
    Right,
    Automated,
    Reinforcement,
    EndTurn,
    // Bottom bar.
    Chat,
    Menu,
    // Map overview sidebar; swatches by slot.
    Swatch(usize),
    Up,
    Down,
    Back,
    // Goal dialog.
    Continue,
    Objectives,
}

/// Which panel set is on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Battle,
    /// The MAP overview: whole map with unit positions and per-player lists.
    Overview,
    /// The GOAL dialog over the battle screen.
    Goal,
}

const BATTLE_BUTTONS: [Button; 14] = [
    Button::Map,
    Button::Goal,
    Button::Group,
    Button::Center,
    Button::MoveAll,
    Button::Overwatch,
    Button::Fire,
    Button::AutoFire,
    Button::Left,
    Button::Guard,
    Button::Right,
    Button::Automated,
    Button::Reinforcement,
    Button::EndTurn,
];

/// Most player swatches the overview shows (AI + four players).
pub const MAX_SWATCHES: usize = 5;

impl Button {
    /// Frame of the normal state (toggle boxes: "off").
    pub fn frame(self) -> u16 {
        use frame::*;
        match self {
            Button::Map => MAP,
            Button::Goal => GOAL,
            Button::Group => GROUP,
            Button::Center => CENTER,
            Button::MoveAll => MOVE_ALL,
            Button::Overwatch => OVERWATCH,
            Button::Fire => FIRE,
            Button::AutoFire => AUTO_FIRE,
            Button::Left => LEFT,
            Button::Guard => GUARD,
            Button::Right => RIGHT,
            Button::Automated => AUTOMATED,
            Button::Reinforcement => REINFORCEMENT,
            Button::EndTurn => END_TURN,
            Button::Chat => CHAT,
            Button::Menu => MENU,
            Button::Swatch(_) => SWATCH,
            Button::Up => UP,
            Button::Down => DOWN,
            Button::Back => BACK,
            Button::Continue => CONTINUE,
            Button::Objectives => OBJECTIVES,
        }
    }

    /// Toggle boxes have six frames: off (normal, hover, pressed),
    /// unavailable, on (normal, hover).
    pub fn is_toggle(self) -> bool {
        matches!(
            self,
            Button::Overwatch | Button::AutoFire | Button::Objectives
        )
    }

    /// Frame to draw for a state; toggles also take their on/off value.
    pub fn frame_for(self, state: State, on: bool) -> u16 {
        let base = self.frame();
        if self.is_toggle() {
            return base
                + match (state, on) {
                    (State::Disabled, _) => 3,
                    (State::Hover, true) => 5,
                    (_, true) => 4,
                    (state, false) => state as u16,
                };
        }
        base + state as u16
    }

    /// Position and size in original pixels, relative to the button's panel
    /// (sidebar, bottom bar right end, or goal dialog).
    fn native(self) -> Rect {
        let (x, y) = match self {
            Button::Map => (10, 169),
            Button::Goal => (71, 169),
            Button::Group => (10, 252),
            Button::Center => (71, 252),
            Button::MoveAll => (12, 273),
            Button::Overwatch => (15, 295),
            Button::Fire => (12, 325),
            Button::AutoFire => (15, 344),
            Button::Left => (12, 374),
            Button::Guard => (41, 374),
            Button::Right => (103, 374),
            Button::Automated => (12, 402),
            Button::Reinforcement => (12, 428),
            Button::EndTurn => (11, 456),
            // Offsets from the bottom bar's right end.
            Button::Chat => (-120, 8),
            Button::Menu => (-60, 8),
            Button::Swatch(slot) => (10 + 19 * slot as i32, 73),
            Button::Up => (9, 431),
            Button::Down => (76, 431),
            Button::Back => (10, 458),
            Button::Continue => (308, 205),
            Button::Objectives => (298, 230),
        };
        let (w, h) = crop_of(self.frame()).unwrap_or((0, 0));
        Rect::new(x, y, w, h)
    }
}

/// Fixed boxes the game writes text or pictures into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// Battle sidebar.
    Credits,
    Minimap,
    UnitBox,
    Portrait,
    UnitText,
    Turn,
    /// Overview sidebar.
    PlayerName,
    List,
    OverviewBox,
    /// Bottom bar (relative to its left end).
    Timer,
    /// Goal dialog text area.
    GoalText,
}

impl Field {
    fn native(self) -> Rect {
        match self {
            Field::Credits => Rect::new(63, 25, 47, 14),
            Field::Minimap => Rect::new(8, 46, 121, 117),
            Field::UnitBox => Rect::new(6, 199, 122, 46),
            Field::Portrait => Rect::new(10, 203, 38, 38),
            Field::UnitText => Rect::new(52, 202, 74, 40),
            Field::Turn => Rect::new(70, 457, 61, 17),
            Field::PlayerName => Rect::new(5, 52, 123, 17),
            Field::List => Rect::new(8, 95, 122, 331),
            Field::OverviewBox => Rect::new(69, 457, 61, 17),
            Field::Timer => Rect::new(5, 7, 45, 19),
            Field::GoalText => Rect::new(20, 20, 409, 172),
        }
    }
}

/// Height of an overview list row, in original pixels.
pub const LIST_ROW: i32 = 15;
/// Rows the overview list shows at once.
pub const LIST_ROWS: usize = (331 / LIST_ROW) as usize;
/// Goal dialog position over the map viewport.
pub const GOAL_AT: (i32, i32) = (40, 40);

/// Bottom bar slices: the left end, a tileable middle, and the right end.
pub const BOTTOM_LEFT: i32 = 100;
pub const BOTTOM_TILE: (i32, i32) = (100, 160);
pub const BOTTOM_RIGHT: i32 = 671;
/// Sidebar rows tiled to fill windows taller than 768 original pixels.
pub const SIDEBAR_TILE: (i32, i32) = (610, 650);

/// Window-size-dependent placement of everything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Output size in pixels.
    pub width: i32,
    pub height: i32,
    /// Output pixels per original pixel.
    pub scale: i32,
}

impl Layout {
    pub fn new(width: i32, height: i32, scale: i32) -> Self {
        Self {
            width,
            height,
            scale: scale.max(1),
        }
    }

    fn s(&self, r: Rect) -> Rect {
        let s = self.scale;
        Rect::new(r.x * s, r.y * s, r.w * s, r.h * s)
    }

    /// Left edge of the sidebar.
    pub fn sidebar_x(&self) -> i32 {
        (self.width - SIDEBAR_W * self.scale).max(0)
    }

    /// The map area: everything left of the sidebar and above the bottom bar.
    pub fn viewport(&self) -> Rect {
        Rect::new(
            0,
            0,
            self.sidebar_x(),
            (self.height - BOTTOM_H * self.scale).max(0),
        )
    }

    pub fn bottom_bar(&self) -> Rect {
        let v = self.viewport();
        Rect::new(0, v.h, v.w, BOTTOM_H * self.scale)
    }

    fn in_sidebar(&self, r: Rect) -> Rect {
        let r = self.s(r);
        Rect::new(self.sidebar_x() + r.x, r.y, r.w, r.h)
    }

    pub fn field(&self, field: Field) -> Rect {
        let native = field.native();
        match field {
            Field::Timer => self.s(native).offset(0, self.bottom_bar().y),
            Field::GoalText => self.s(native).offset_by(self.goal_dialog()),
            _ => self.in_sidebar(native),
        }
    }

    pub fn goal_dialog(&self) -> Rect {
        let (w, h) = crop_of(frame::GOAL_DIALOG).unwrap_or((0, 0));
        self.s(Rect::new(GOAL_AT.0, GOAL_AT.1, w, h))
    }

    pub fn button(&self, button: Button) -> Rect {
        let native = button.native();
        match button {
            Button::Chat | Button::Menu => {
                let bar = self.bottom_bar();
                self.s(native).offset(bar.right(), bar.y)
            }
            Button::Continue | Button::Objectives => self.s(native).offset_by(self.goal_dialog()),
            _ => self.in_sidebar(native),
        }
    }

    /// Row `i` (on screen) of the overview list.
    pub fn list_row(&self, i: usize) -> Rect {
        let list = self.field(Field::List);
        let s = self.scale;
        Rect::new(
            list.x + s,
            list.y + s + i as i32 * LIST_ROW * s,
            list.w - 2 * s,
            LIST_ROW * s,
        )
    }

    /// Buttons of a screen; `swatches` is the overview's player count.
    pub fn buttons(&self, screen: Screen, swatches: usize) -> Vec<Button> {
        let mut buttons = Vec::new();
        match screen {
            Screen::Battle => buttons.extend(BATTLE_BUTTONS),
            Screen::Overview => {
                buttons.extend((0..swatches.min(MAX_SWATCHES)).map(Button::Swatch));
                buttons.extend([Button::Up, Button::Down, Button::Back]);
            }
            Screen::Goal => buttons.extend([Button::Continue, Button::Objectives]),
        }
        if screen != Screen::Goal {
            buttons.extend([Button::Chat, Button::Menu]);
        }
        buttons
    }

    /// The button under output pixel `(x, y)`.
    pub fn hit(&self, screen: Screen, swatches: usize, x: i32, y: i32) -> Option<Button> {
        self.buttons(screen, swatches)
            .into_iter()
            .find(|&b| self.button(b).contains(x, y))
    }

    /// Whether `(x, y)` is on the interface rather than the map.
    pub fn on_panel(&self, x: i32, y: i32) -> bool {
        !self.viewport().contains(x, y)
    }
}

impl Rect {
    fn offset(self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    fn offset_by(self, origin: Rect) -> Rect {
        self.offset(origin.x, origin.y)
    }
}

/// Greedy word wrap of `text` to `max` width; `##` starts a new line (the
/// first one a new paragraph, as the original goal dialog shows).
pub fn wrap(text: &str, max: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    for (i, paragraph) in text.split("##").enumerate() {
        if i == 1 {
            lines.push(String::new());
        }
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && measure(&candidate) > max {
                lines.push(std::mem::replace(&mut line, word.to_owned()));
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_anchors_sidebar_and_bottom_bar() {
        let layout = Layout::new(1024, 768, 1);
        assert_eq!(layout.viewport(), Rect::new(0, 0, 840, 728));
        assert_eq!(layout.button(Button::Map), Rect::new(850, 169, 53, 17));
        assert_eq!(layout.button(Button::Menu), Rect::new(780, 736, 53, 17));
        assert_eq!(layout.field(Field::Timer), Rect::new(5, 735, 45, 19));

        let big = Layout::new(2560, 1600, 2);
        assert_eq!(big.viewport(), Rect::new(0, 0, 2192, 1520));
        assert_eq!(
            big.button(Button::Goal),
            Rect::new(2192 + 142, 338, 106, 34)
        );
        assert_eq!(big.button(Button::Chat).right(), 2192 - 240 + 106);
        assert_eq!(
            big.hit(Screen::Battle, 0, 2192 + 150, 340),
            Some(Button::Goal)
        );
        assert_eq!(big.hit(Screen::Goal, 0, 2192 + 150, 340), None);
        assert!(big.on_panel(2200, 10) && !big.on_panel(10, 10));
    }

    #[test]
    fn overview_hits_swatches_and_rows() {
        let layout = Layout::new(1024, 768, 1);
        assert_eq!(
            layout.hit(Screen::Overview, 2, 850 + 30, 75),
            Some(Button::Swatch(1))
        );
        assert_eq!(layout.hit(Screen::Overview, 1, 850 + 30, 75), None);
        assert_eq!(layout.list_row(1), Rect::new(849, 111, 120, 15));
    }

    #[test]
    fn toggle_frames() {
        assert_eq!(Button::Overwatch.frame_for(State::Normal, false), 251);
        assert_eq!(Button::Overwatch.frame_for(State::Disabled, true), 254);
        assert_eq!(Button::Overwatch.frame_for(State::Normal, true), 255);
        assert_eq!(Button::Objectives.frame_for(State::Hover, true), 643);
        assert_eq!(Button::EndTurn.frame_for(State::Pressed, false), 292);
    }

    #[test]
    fn wraps_words_and_paragraphs() {
        let measure = |s: &str| s.len() as f32;
        let lines = wrap("aa bb cc dd##Objective: x##Do it", 5.0, measure);
        assert_eq!(
            lines,
            ["aa bb", "cc dd", "", "Objective:", "x", "Do it"].map(String::from)
        );
    }
}
