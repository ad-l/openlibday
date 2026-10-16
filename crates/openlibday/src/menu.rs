//! The OPTIONS dialog the bottom bar's MENU opens: a scrolling table of the
//! configurable options, applied at once (and saved to the options file),
//! and the original's RESTART SCENARIO (a surrender), SAVE, QUIT and BACK.

use libday_data::objective::Outcome;
use libday_view::ui::{Ask, Button, OPTIONS_ROWS, Rect, Screen};
use sdl3::keyboard::Keycode;

use crate::Viewer;
use crate::settings::{self, AiMode, MoveSpeed, ScrollSpeed};
use crate::skirmish::Phase;

/// A row of the options table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Heading(&'static str),
    Scale,
    Fullscreen,
    Scroll,
    Zoom,
    Grid,
    TerrainOnly,
    Moves,
    OverwatchZone,
    Volume,
    Ai,
}

pub const ITEMS: [Item; 14] = [
    Item::Heading("INTERFACE"),
    Item::Scale,
    Item::Fullscreen,
    Item::Scroll,
    Item::Heading("MAP VIEW"),
    Item::Zoom,
    Item::Grid,
    Item::TerrainOnly,
    Item::Moves,
    Item::OverwatchZone,
    Item::Heading("SOUND"),
    Item::Volume,
    Item::Heading("COMPUTER PLAYER"),
    Item::Ai,
];

/// Volume steps of the selector.
const VOLUME_STEP: i32 = 10;

/// What a row shows on its right.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Check(bool),
    /// A `‹ value ›` selector, and whether it can go down / up.
    Choice(String, bool, bool),
}

impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Item::Heading(text) => text,
            Item::Scale => "Interface scale",
            Item::Fullscreen => "Fullscreen",
            Item::Scroll => "Scroll speed",
            Item::Zoom => "Default zoom",
            Item::Grid => "Tile grid",
            Item::TerrainOnly => "Terrain only",
            Item::Moves => "Unit moves",
            Item::OverwatchZone => "Overwatch preview",
            Item::Volume => "Volume",
            Item::Ai => "Behaviour",
        }
    }

    pub fn is_heading(self) -> bool {
        matches!(self, Item::Heading(_))
    }

    /// Scripts' name for the row (`Scale`, `Grid`...).
    pub fn name(self) -> String {
        match self {
            Item::Heading(text) => text.to_owned(),
            other => format!("{other:?}"),
        }
    }
}

/// The table's scroll and keyboard focus, and the question asked.
#[derive(Clone, Debug, Default)]
pub struct Menu {
    pub scroll: usize,
    /// Index into [`ITEMS`].
    pub focus: Option<usize>,
    pub question: String,
    /// Opened from the main menu's RESOLUTION: no battle to restart or
    /// save, BACK returns to the main menu and QUIT leaves the game.
    pub over_main: bool,
}

/// Steps `current` (an index into `n` choices) by `step`, clamped.
fn stepped(current: usize, n: usize, step: i32) -> usize {
    (current as i32 + step).clamp(0, n as i32 - 1) as usize
}

impl Viewer {
    /// The interface scale chosen, in the options file's terms.
    fn chosen_scale(&self) -> i32 {
        ((self.scale as f32 / self.density).round() as i32).clamp(1, settings::MAX_SCALE)
    }

    pub(crate) fn option_value(&self, item: Item) -> Option<Value> {
        let choice = |text: String, i: usize, n: usize| Value::Choice(text, i > 0, i + 1 < n);
        Some(match item {
            Item::Heading(_) => return None,
            Item::Scale => {
                let s = self.chosen_scale();
                let fitted = (self.layout().scale as f32 / self.density).round() as i32;
                let text = if fitted < s {
                    format!("x{s} (FITS x{fitted})")
                } else {
                    format!("x{s}")
                };
                choice(text, (s - 1) as usize, settings::MAX_SCALE as usize)
            }
            Item::Fullscreen => Value::Check(self.settings.fullscreen),
            Item::Scroll => {
                let i = ScrollSpeed::ALL
                    .iter()
                    .position(|&m| m == self.settings.scroll);
                let i = i.unwrap_or(0);
                choice(self.settings.scroll.name().to_uppercase(), i, 3)
            }
            Item::Zoom => {
                let i = settings::ZOOMS
                    .iter()
                    .position(|&z| z == self.settings.zoom);
                let i = i.unwrap_or(0);
                choice(format!("{}%", self.settings.zoom), i, settings::ZOOMS.len())
            }
            Item::Grid => Value::Check(self.grid),
            Item::OverwatchZone => Value::Check(self.settings.overwatch_zone),
            Item::TerrainOnly => Value::Check(self.options.terrain_only),
            Item::Moves => {
                let all = MoveSpeed::ALL;
                let i = all.iter().position(|&m| m == self.settings.moves);
                choice(
                    self.settings.moves.name().to_uppercase(),
                    i.unwrap_or(0),
                    all.len(),
                )
            }
            Item::Volume => {
                let v = self.settings.volume as i32;
                let text = if v == 0 {
                    "OFF".to_owned()
                } else {
                    format!("{v}%")
                };
                Value::Choice(text, v > 0, v < 100)
            }
            Item::Ai => match self.ai_mode {
                Some(mode) => {
                    let i = AiMode::ALL.iter().position(|&m| m == mode).unwrap_or(0);
                    choice(mode.name().to_uppercase(), i, AiMode::ALL.len())
                }
                None => Value::Choice("OFF".into(), false, true),
            },
        })
    }

    /// Changes an option: `step` -1 / 1 moves a selector, any step toggles a
    /// check box.
    pub(crate) fn change_option(&mut self, item: Item, step: i32) {
        let mut save = true;
        match item {
            Item::Heading(_) => return,
            Item::Scale => {
                let s = (self.chosen_scale() + step).clamp(1, settings::MAX_SCALE);
                self.settings.scale = s;
                self.set_scale(((s as f32 * self.density).round() as i32).max(1));
            }
            Item::Fullscreen => {
                self.settings.fullscreen ^= true;
                self.fullscreen = Some(self.settings.fullscreen);
            }
            Item::Scroll => {
                let all = ScrollSpeed::ALL;
                let i = all.iter().position(|&m| m == self.settings.scroll);
                self.settings.scroll = all[stepped(i.unwrap_or(1), all.len(), step)];
            }
            Item::Zoom => {
                let all = settings::ZOOMS;
                let i = all.iter().position(|&z| z == self.settings.zoom);
                self.settings.zoom = all[stepped(i.unwrap_or(0), all.len(), step)];
            }
            Item::OverwatchZone => {
                self.settings.overwatch_zone ^= true;
                self.watch.clear();
            }
            Item::Grid => {
                self.grid ^= true;
                save = false;
            }
            Item::TerrainOnly => {
                self.options.terrain_only ^= true;
                self.rebuild();
                save = false;
            }
            Item::Moves => {
                let all = MoveSpeed::ALL;
                let i = all.iter().position(|&m| m == self.settings.moves);
                self.settings.moves = all[stepped(i.unwrap_or(0), all.len(), step)];
            }
            Item::Volume => {
                let v = (self.settings.volume as i32 + step * VOLUME_STEP).clamp(0, 100);
                self.settings.volume = v as u32;
                self.sound.set_volume(v as u32);
            }
            Item::Ai => {
                let all = AiMode::ALL;
                let mode = match self.ai_mode {
                    Some(m) => {
                        let i = all.iter().position(|&a| a == m).unwrap_or(0);
                        all[stepped(i, all.len(), step)]
                    }
                    None => all[0],
                };
                self.ai_mode = Some(mode);
                self.settings.ai = mode;
            }
        }
        if save
            && let Some(path) = &self.settings_path
            && let Err(e) = self.settings.save(path)
        {
            self.error = Some(format!("cannot save {}: {e}", path.display()));
        }
    }

    pub(crate) fn open_menu(&mut self) {
        self.menu = Menu {
            over_main: self.screen == Screen::MainMenu,
            ..Menu::default()
        };
        self.screen = Screen::Options { ask: None };
    }

    pub(crate) fn close_menu(&mut self) {
        self.screen = if self.menu.over_main {
            Screen::MainMenu
        } else {
            self.home()
        };
    }

    /// Scrolls the table by `rows`.
    pub(crate) fn menu_scroll(&mut self, rows: isize) {
        let last = ITEMS.len().saturating_sub(OPTIONS_ROWS);
        self.menu.scroll = self.menu.scroll.saturating_add_signed(rows).min(last);
    }

    /// A click on the table at output pixel `(x, y)`: a selector's left half
    /// steps down, the rest of a row steps up or toggles.
    pub(crate) fn menu_click(&mut self, x: i32, y: i32) {
        let layout = self.layout();
        let Some(row) = (0..OPTIONS_ROWS).find(|&i| layout.options_row(i).contains(x, y)) else {
            return;
        };
        let index = self.menu.scroll + row;
        let Some(&item) = ITEMS.get(index) else {
            return;
        };
        if item.is_heading() {
            return;
        }
        self.menu.focus = Some(index);
        let c = control(layout.options_row(row), layout.scale);
        let step = match self.option_value(item) {
            Some(Value::Choice(..)) if x < c.x + c.w / 3 => -1,
            _ => 1,
        };
        self.change_option(item, step);
    }

    /// Moves the keyboard focus to the next setting `step` rows away.
    fn menu_focus(&mut self, step: isize) {
        let settings: Vec<usize> = (0..ITEMS.len())
            .filter(|&i| !ITEMS[i].is_heading())
            .collect();
        let at = self
            .menu
            .focus
            .and_then(|f| settings.iter().position(|&i| i == f));
        let next = match at {
            Some(at) => (at as isize + step).clamp(0, settings.len() as isize - 1) as usize,
            None if step > 0 => 0,
            None => settings.len() - 1,
        };
        let index = settings[next];
        self.menu.focus = Some(index);
        // Keep the focused row (and its heading) on screen.
        if index < self.menu.scroll + 1 {
            self.menu.scroll = index.saturating_sub(1);
        } else if index >= self.menu.scroll + OPTIONS_ROWS {
            self.menu.scroll = index + 1 - OPTIONS_ROWS;
        }
    }

    /// Keys of the OPTIONS dialog.
    pub(crate) fn menu_key(&mut self, ask: Option<Ask>, key: Keycode) {
        if let Some(ask) = ask {
            match key {
                Keycode::Return | Keycode::KpEnter | Keycode::Y => self.answer(ask),
                Keycode::Escape | Keycode::N => self.screen = Screen::Options { ask: None },
                _ => {}
            }
            return;
        }
        let focused = self.menu.focus.map(|i| ITEMS[i]);
        match key {
            Keycode::Escape => self.close_menu(),
            Keycode::Up => self.menu_focus(-1),
            Keycode::Down => self.menu_focus(1),
            Keycode::PageUp => self.menu_scroll(-(OPTIONS_ROWS as isize)),
            Keycode::PageDown => self.menu_scroll(OPTIONS_ROWS as isize),
            Keycode::Left => focused.into_iter().for_each(|i| self.change_option(i, -1)),
            Keycode::Right | Keycode::Space | Keycode::Return | Keycode::KpEnter => {
                focused.into_iter().for_each(|i| self.change_option(i, 1))
            }
            _ => {}
        }
    }

    fn ask(&mut self, ask: Ask) {
        // LIBDAYEN.DLL strings, as the original asks.
        let (id, fallback) = match ask {
            Ask::Restart => (0x332, "Do you really want to surrender?"),
            Ask::Quit => (0x333, "Do you really want to quit the game?"),
        };
        let strings = self.game.strings().unwrap_or_default();
        self.menu.question = strings.get(&id).cloned().unwrap_or_else(|| fallback.into());
        self.screen = Screen::Options { ask: Some(ask) };
    }

    /// YES to the OPTIONS dialog's question.
    pub(crate) fn answer(&mut self, ask: Ask) {
        match ask {
            Ask::Restart => {
                self.close_menu();
                self.end_battle(Outcome::Lost);
            }
            // As in the original, quitting leaves for the main menu.
            Ask::Quit if self.menu.over_main => self.quit = true,
            Ask::Quit => {
                self.close_menu();
                self.open_main_menu();
            }
        }
    }

    pub(crate) fn menu_enabled(&self, button: Button) -> bool {
        match button {
            Button::OptionsRestart => !self.menu.over_main && self.flow.phase == Phase::Player,
            Button::OptionsSave => {
                !self.menu.over_main
                    && self.strategic.is_none()
                    && matches!(self.flow.phase, Phase::Free | Phase::Player)
                    && !self.flow.reinforcing
            }
            Button::Menu => {
                !self.menu.over_main
                    && matches!(
                        self.screen,
                        Screen::Battle
                            | Screen::Deploy
                            | Screen::AiTurn
                            | Screen::Strategic { .. }
                            | Screen::BaseEditor
                            | Screen::Options { ask: None }
                    )
            }
            Button::OptionsUp => self.menu.scroll > 0,
            Button::OptionsDown => self.menu.scroll < ITEMS.len().saturating_sub(OPTIONS_ROWS),
            _ => true,
        }
    }

    pub(crate) fn menu_activate(&mut self, button: Button) {
        match button {
            Button::Menu if matches!(self.screen, Screen::Options { .. }) => self.close_menu(),
            Button::Menu => self.open_menu(),
            Button::OptionsRestart => self.ask(Ask::Restart),
            Button::OptionsQuit => self.ask(Ask::Quit),
            Button::OptionsSave => {
                self.quick_save();
                self.close_menu();
                if self.error.is_none() {
                    self.flow.message = Some("The game has been saved".into());
                }
            }
            Button::OptionsBack => self.close_menu(),
            Button::OptionsUp => self.menu_scroll(-1),
            Button::OptionsDown => self.menu_scroll(1),
            Button::Yes => {
                if let Screen::Options { ask: Some(ask) } = self.screen {
                    self.answer(ask);
                }
            }
            Button::No => self.screen = Screen::Options { ask: None },
            _ => {}
        }
    }
}

/// The control (check box or selector) on the right of table row `row`.
pub fn control(row: Rect, scale: i32) -> Rect {
    let w = 120 * scale;
    Rect::new(
        row.x + row.w - w - 10 * scale,
        row.y + 3 * scale,
        w,
        row.h - 6 * scale,
    )
}
