//! Front end: the main menu and the NEW GAME dialog (player name and
//! difficulty) that starts a campaign.

use libday_view::ui::{Button, Rect, Screen};
use sdl3::keyboard::Keycode;

use crate::Viewer;

/// Hardest difficulty; the slider runs from 0 (easy) to this.
pub const DIFFICULTY_MAX: u8 = 10;
/// The original's default difficulty.
const DIFFICULTY_DEFAULT: u8 = 4;
/// Longest player name.
const NAME_MAX: usize = 20;
/// Name box and slider track in NEW GAME dialog coordinates.
pub const NAME_BOX: Rect = Rect::new(10, 55, 266, 24);
pub const SLIDER: Rect = Rect::new(39, 114, 210, 17);

/// What the NEW GAME dialog edits.
pub struct Front {
    pub name: String,
    pub difficulty: u8,
}

impl Default for Front {
    fn default() -> Self {
        let name = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_default();
        Self {
            name: name.chars().take(NAME_MAX).collect(),
            difficulty: DIFFICULTY_DEFAULT,
        }
    }
}

impl Viewer {
    pub(crate) fn open_main_menu(&mut self) {
        self.flow = crate::skirmish::Flow::new();
        self.screen = Screen::MainMenu;
    }

    pub(crate) fn front_enabled(&self, button: Button) -> bool {
        match button {
            // Not implemented (no multiplayer, cutscenes or mode switch), and
            // no campaign to load yet.
            Button::MultiPlayer | Button::Intro | Button::Resolution | Button::LoadCampaign => {
                false
            }
            Button::DifficultyLeft => self.front.difficulty > 0,
            Button::DifficultyRight => self.front.difficulty < DIFFICULTY_MAX,
            Button::NewGameContinue => !self.front.name.trim().is_empty(),
            _ => true,
        }
    }

    pub(crate) fn front_activate(&mut self, button: Button) {
        match button {
            Button::NewCampaign => self.screen = Screen::NewGame,
            Button::MainSkirmish => {
                if let Err(err) = self.open_selector() {
                    self.error = Some(err);
                }
            }
            Button::Quit => self.quit = true,
            Button::DifficultyLeft => self.difficulty_step(-1),
            Button::DifficultyRight => self.difficulty_step(1),
            Button::NewGameBack => self.screen = Screen::MainMenu,
            Button::NewGameContinue => self.start_campaign(),
            _ => {}
        }
    }

    fn difficulty_step(&mut self, step: i8) {
        self.front.difficulty = self
            .front
            .difficulty
            .saturating_add_signed(step)
            .min(DIFFICULTY_MAX);
    }

    /// Starts a new campaign with the chosen name and difficulty.
    fn start_campaign(&mut self) {
        if !self.front_enabled(Button::NewGameContinue) {
            return;
        }
        self.notice = Some(format!(
            "campaign for {} at difficulty {}",
            self.front.name.trim(),
            self.front.difficulty
        ));
    }

    /// Keys of the main menu and the NEW GAME dialog; `None` elsewhere.
    pub(crate) fn front_key(&mut self, key: Keycode) -> Option<bool> {
        match (self.screen, key) {
            (Screen::MainMenu, Keycode::Escape) => return Some(false),
            (Screen::MainMenu, _) => {}
            (Screen::NewGame, Keycode::Escape) => self.screen = Screen::MainMenu,
            (Screen::NewGame, Keycode::Return | Keycode::KpEnter) => self.start_campaign(),
            (Screen::NewGame, Keycode::Backspace) => {
                self.front.name.pop();
            }
            (Screen::NewGame, Keycode::Left) => self.difficulty_step(-1),
            (Screen::NewGame, Keycode::Right) => self.difficulty_step(1),
            (Screen::NewGame, _) => {}
            _ => return None,
        }
        Some(true)
    }

    /// Typed text goes into the NEW GAME dialog's name.
    pub(crate) fn text_input(&mut self, text: &str) {
        if self.screen != Screen::NewGame {
            return;
        }
        for c in text.chars().filter(|c| !c.is_control()) {
            if self.front.name.chars().count() < NAME_MAX {
                self.front.name.push(c);
            }
        }
    }

    /// A click on the NEW GAME slider track sets the difficulty.
    pub(crate) fn front_click(&mut self, x: i32, y: i32) {
        if self.screen != Screen::NewGame {
            return;
        }
        let layout = self.layout();
        let track = layout.in_new_game(SLIDER);
        if track.contains(x, y) {
            let s = layout.scale;
            // Nearest step to the thumb's centre.
            let native = (x - track.x) / s + SLIDER.x - 44 - 4;
            let d = ((native * 10 + 94) / 189).clamp(0, DIFFICULTY_MAX as i32);
            self.front.difficulty = d as u8;
        }
    }
}
