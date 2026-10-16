//! Front end: the main menu, the NEW GAME dialog (player name and
//! difficulty) that starts a campaign, and the keys and typing of the
//! network game dialogs (SESSIONS and the waiting area, `lobby.rs`).

use libday_view::ui::{Button, Rect, Screen};
use sdl3::keyboard::Keycode;

use crate::Viewer;
use crate::router::{PortMap, Wan};

/// Hardest difficulty; the slider runs from 0 (easy) to this.
/// The introduction video (`0x4ae014`), also played at start-up.
pub const INTRO_VIDEO: &str = "FH2Intro";

pub const DIFFICULTY_MAX: u8 = 10;
/// The original's default difficulty.
pub const DIFFICULTY_DEFAULT: u8 = 4;
/// Longest player name.
const NAME_MAX: usize = 20;
/// Name box and slider track in NEW GAME dialog coordinates.
pub const NAME_BOX: Rect = Rect::new(10, 55, 266, 24);
pub const SLIDER: Rect = Rect::new(39, 114, 210, 17);
/// SESSIONS name and address boxes in dialog coordinates.
pub const SESSION_NAME: Rect = Rect::new(120, 80, 220, 22);
pub const SESSION_ADDRESS: Rect = Rect::new(120, 112, 220, 22);
/// SESSIONS: the list of games found on the local network.
pub const GAMES_LIST: Rect = Rect::new(22, 158, 318, 4 * GAME_ROW + 4);
pub const GAMES_SHOWN: usize = 4;
const GAME_ROW: i32 = 18;
pub fn game_row(i: usize) -> Rect {
    Rect::new(
        GAMES_LIST.x + 2,
        GAMES_LIST.y + 2 + i as i32 * GAME_ROW,
        GAMES_LIST.w - 4,
        GAME_ROW,
    )
}
/// A row of the WAITING AREA's game information: its rectangle (dialog
/// coordinates), labels left of [`INFO_VALUE_X`].
pub fn info_row(row: usize) -> Rect {
    Rect::new(295, 76 + row as i32 * 22, 244, 18)
}
pub const INFO_VALUE_X: i32 = 359;
const INFO_RIGHT: i32 = 539;
const INFO_ARROW: i32 = 18;
const INFO_COPY: i32 = 44;

/// What a row of the game information shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoRow {
    Lan,
    Wan,
    Setting(crate::lobby::Setting),
}

/// The game information's rows: the addresses, then the settings (the
/// difficulty only for a campaign).
pub fn info_rows(campaign: bool) -> Vec<InfoRow> {
    use crate::lobby::Setting;
    let mut rows = vec![InfoRow::Lan, InfoRow::Wan, InfoRow::Setting(Setting::Mode)];
    if campaign {
        rows.push(InfoRow::Setting(Setting::Difficulty));
    }
    rows.push(InfoRow::Setting(Setting::TurnLimit));
    rows
}

/// A row's value area, and in it an address's box and COPY button or a
/// setting's arrows.
pub fn info_value(row: usize) -> Rect {
    let r = info_row(row);
    Rect::new(INFO_VALUE_X, r.y, INFO_RIGHT - INFO_VALUE_X, r.h)
}
pub fn info_address(row: usize) -> Rect {
    let v = info_value(row);
    Rect::new(v.x, v.y, v.w - INFO_COPY - 4, v.h)
}
pub fn info_copy(row: usize) -> Rect {
    let v = info_value(row);
    Rect::new(v.right() - INFO_COPY, v.y, INFO_COPY, v.h)
}
pub fn info_arrows(row: usize) -> (Rect, Rect) {
    let v = info_value(row);
    (
        Rect::new(v.x, v.y, INFO_ARROW, v.h),
        Rect::new(v.right() - INFO_ARROW, v.y, INFO_ARROW, v.h),
    )
}

/// Longest typed host address and chat line.
const ADDRESS_MAX: usize = 64;
const CHAT_MAX: usize = 120;

/// What the NEW GAME dialog edits.
pub struct Front {
    pub name: String,
    pub difficulty: u8,
    /// SESSIONS: the host to join, the port to host on, and whether the
    /// address (not the name) is being typed.
    pub address: String,
    pub port: u16,
    pub focus_address: bool,
    /// What went wrong, or is happening, with connecting.
    pub status: Option<String>,
    /// The chat line being typed, and whether the battle's chat is open.
    pub chat: String,
    pub chatting: bool,
    /// Text for the clipboard (the main loop puts it there).
    pub clipboard: Option<String>,
    /// Whether a host asks the router to open its port, and whether games
    /// are announced and looked for on the local network (both off in
    /// scripted runs unless the script turns them on).
    pub router: bool,
    pub discover: bool,
    /// SESSIONS: the games found on the local network.
    pub search: Option<crate::discovery::Search>,
}

impl Default for Front {
    fn default() -> Self {
        let name = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_default();
        Self {
            name: name.chars().take(NAME_MAX).collect(),
            difficulty: DIFFICULTY_DEFAULT,
            address: "127.0.0.1".into(),
            port: crate::link::DEFAULT_PORT,
            focus_address: false,
            status: None,
            chat: String::new(),
            chatting: false,
            clipboard: None,
            router: true,
            discover: true,
            search: None,
        }
    }
}

impl Viewer {
    pub(crate) fn open_main_menu(&mut self) {
        self.leave_network();
        self.flow = crate::skirmish::Flow::new();
        self.campaign = None;
        self.strategic = None;
        self.base_editor = None;
        self.screen = Screen::MainMenu;
    }

    pub(crate) fn front_enabled(&self, button: Button) -> bool {
        match button {
            Button::LobbyStart => self.net.online.as_ref().is_some_and(|o| o.is_host()),
            Button::LobbySend => !self.front.chat.trim().is_empty(),
            Button::HostGame | Button::JoinGame => {
                !self.front.name.trim().is_empty()
                    && self
                        .net
                        .online
                        .as_ref()
                        .is_none_or(|o| o.connecting.is_none())
            }
            Button::LoadCampaign => self.any_saves(),
            Button::DifficultyLeft => self.front.difficulty > 0,
            Button::DifficultyRight => self.front.difficulty < DIFFICULTY_MAX,
            Button::NewGameContinue => !self.front.name.trim().is_empty(),
            _ => true,
        }
    }

    pub(crate) fn front_activate(&mut self, button: Button) {
        match button {
            Button::NewCampaign => self.screen = Screen::NewGame,
            Button::LoadCampaign => self.open_saves(false),
            Button::MainSkirmish => {
                if let Err(err) = self.open_selector() {
                    self.error = Some(err);
                }
            }
            Button::Resolution => self.open_menu(),
            Button::Intro => self.play_video(INTRO_VIDEO),
            Button::Quit => self.quit = true,
            Button::DifficultyLeft => self.difficulty_step(-1),
            Button::DifficultyRight => self.difficulty_step(1),
            Button::NewGameBack => self.screen = Screen::MainMenu,
            Button::NewGameContinue => self.start_campaign(),
            Button::MultiPlayer => {
                self.front.status = None;
                self.open_sessions();
            }
            Button::HostGame => self.host_game(),
            Button::JoinGame => self.join_game(),
            Button::SessionsBack => self.open_main_menu(),
            Button::LobbySend => self.send_chat(),
            Button::LobbyBack => self.lobby_back(),
            Button::LobbyStart => self.lobby_start(),
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
        let name = self.front.name.trim().to_owned();
        self.new_campaign(&name, i64::from(self.front.difficulty));
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
            (Screen::Sessions, Keycode::Escape) => self.open_main_menu(),
            (Screen::Sessions, Keycode::Return | Keycode::KpEnter) => {
                if self.front_enabled(Button::JoinGame) {
                    self.join_game();
                }
            }
            (Screen::Sessions, Keycode::Tab | Keycode::Up | Keycode::Down) => {
                self.front.focus_address = !self.front.focus_address;
            }
            (Screen::Sessions, Keycode::Backspace) => {
                self.typed_field().pop();
            }
            (Screen::Sessions, _) => {}
            (Screen::Lobby, Keycode::Escape) => self.lobby_back(),
            (Screen::Lobby, Keycode::Return | Keycode::KpEnter) => self.send_chat(),
            (Screen::Lobby, Keycode::Backspace) => {
                self.front.chat.pop();
            }
            (Screen::Lobby, _) => {}
            _ if self.front.chatting => match key {
                Keycode::Escape => {
                    self.front.chatting = false;
                    self.front.chat.clear();
                }
                Keycode::Return | Keycode::KpEnter => {
                    self.send_chat();
                    self.front.chatting = false;
                }
                Keycode::Backspace => {
                    self.front.chat.pop();
                }
                _ => {}
            },
            _ => return None,
        }
        Some(true)
    }

    /// Whether typed text goes into a field (rather than being keys).
    pub(crate) fn typing(&self) -> bool {
        matches!(
            self.screen,
            Screen::NewGame | Screen::Sessions | Screen::Lobby
        ) || self.front.chatting
    }

    /// The SESSIONS field being typed in.
    fn typed_field(&mut self) -> &mut String {
        if self.front.focus_address {
            &mut self.front.address
        } else {
            &mut self.front.name
        }
    }

    /// Typed text goes into the NEW GAME dialog's name, the SESSIONS
    /// fields or the chat line.
    pub(crate) fn text_input(&mut self, text: &str) {
        if !self.typing() || self.cutscene.is_some() {
            return;
        }
        let max = match self.screen {
            Screen::NewGame => NAME_MAX,
            Screen::Sessions if self.front.focus_address => ADDRESS_MAX,
            Screen::Sessions => NAME_MAX,
            _ => CHAT_MAX,
        };
        let field = match self.screen {
            Screen::NewGame => &mut self.front.name,
            Screen::Sessions => self.typed_field(),
            _ => &mut self.front.chat,
        };
        for c in text.chars().filter(|c| !c.is_control()) {
            if field.chars().count() < max {
                field.push(c);
            }
        }
    }

    /// The game addresses shown in the waiting area: on the local
    /// network (or, for a client, the host's as typed) and on the internet.
    pub(crate) fn game_addresses(&self) -> (String, String) {
        let lan = match &self.net.online {
            Some(o) if o.is_host() => o.lan.clone().unwrap_or_default(),
            _ => self.front.address.trim().to_owned(),
        };
        let wan = match self.net.online.as_ref().and_then(|o| o.port_map.as_ref()) {
            Some(PortMap {
                wan: Wan::Open { address, .. },
                ..
            }) => address.to_string(),
            _ => String::new(),
        };
        (lan, wan)
    }

    /// What the waiting area shows when there is no internet address.
    pub(crate) fn wan_note(&self) -> &'static str {
        match &self.net.online {
            Some(o) if o.is_host() => match o.port_map.as_ref().map(|m| &m.wan) {
                Some(Wan::Searching) => "Asking the router...",
                Some(Wan::Open { .. }) => "",
                Some(Wan::Failed(_)) => "Router did not open the port",
                None => "Port mapping off",
            },
            _ => "Ask the host",
        }
    }

    /// The SESSIONS dialog's list of games on the local network.
    pub(crate) fn found_games(&self) -> &[crate::discovery::Found] {
        self.front.search.as_ref().map_or(&[], |s| &s.games)
    }

    /// Looks for games on the local network while SESSIONS is open.
    pub(crate) fn search_tick(&mut self) {
        if self.screen != Screen::Sessions || !self.front.discover {
            self.front.search = None;
            return;
        }
        if self.front.search.is_none() {
            self.front.search = crate::discovery::Search::new();
        }
        if let Some(search) = &mut self.front.search {
            search.tick();
        }
    }

    /// A click in the waiting area's game information: COPY an address,
    /// or (the host) an arrow of a setting.
    fn lobby_click(&mut self, x: i32, y: i32) {
        let Some(online) = &self.net.online else {
            return;
        };
        let campaign = online.settings.mode == crate::lobby::Mode::Campaign;
        let host = online.is_host();
        let layout = self.layout();
        for (i, row) in info_rows(campaign).into_iter().enumerate() {
            match row {
                InfoRow::Lan | InfoRow::Wan => {
                    if layout.in_lobby(info_copy(i)).contains(x, y) {
                        let (lan, wan) = self.game_addresses();
                        self.copy_text(if row == InfoRow::Lan { lan } else { wan });
                        return;
                    }
                }
                InfoRow::Setting(setting) if host => {
                    let (left, right) = info_arrows(i);
                    if layout.in_lobby(left).contains(x, y) {
                        self.change_setting(setting, -1);
                        return;
                    }
                    if layout.in_lobby(right).contains(x, y) {
                        self.change_setting(setting, 1);
                        return;
                    }
                }
                InfoRow::Setting(_) => {}
            }
        }
    }

    /// A click on the NEW GAME slider track sets the difficulty; in the
    /// network dialogs, on a field or the game information.
    pub(crate) fn front_click(&mut self, x: i32, y: i32) {
        let layout = self.layout();
        if self.screen == Screen::Lobby {
            self.lobby_click(x, y);
            return;
        }
        if self.screen == Screen::Sessions {
            if layout.in_sessions(SESSION_NAME).contains(x, y) {
                self.front.focus_address = false;
            } else if layout.in_sessions(SESSION_ADDRESS).contains(x, y) {
                self.front.focus_address = true;
            } else if let Some(game) = (0..GAMES_SHOWN)
                .find(|&i| layout.in_sessions(game_row(i)).contains(x, y))
                .and_then(|i| self.found_games().get(i))
            {
                self.front.address = game.address.to_string();
                self.front.focus_address = true;
            }
            return;
        }
        if self.screen != Screen::NewGame {
            return;
        }
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
