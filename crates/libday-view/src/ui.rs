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

/// Sidebar width and the bottom bar height, in original pixels. The art is
/// 184 wide, but its last 48 columns are decoration (pipe, vent) that only
/// filled the fixed 1024-wide screen; with a resizable window they are cut
/// and the map gets the space.
pub const SIDEBAR_W: i32 = 136;
pub const SIDEBAR_H: i32 = 768;
/// The bar art is 40 high; its last 8 rows (wall below the text boxes) are
/// cut so the boxes sit flush with the window bottom.
pub const BOTTOM_H: i32 = 32;
pub const BOTTOM_W: i32 = 840;

/// Frame numbers of the elements.
pub mod frame {
    /// Deployment sidebar.
    pub const DEPLOY_SIDEBAR: u16 = 0;
    pub const DEPLOY_MAP: u16 = 5;
    pub const DEPLOY_GOAL: u16 = 9;
    pub const DEPLOY_UP: u16 = 13;
    pub const DEPLOY_DOWN: u16 = 17;
    pub const DEPLOY_CONTINUE: u16 = 21;
    pub const ARMY: u16 = 29;
    /// Base editor sidebar and its buttons.
    pub const BASE_SIDEBAR: u16 = 100;
    pub const BASE_MAP: u16 = 105;
    pub const BASE_BUILDINGS: u16 = 109;
    pub const BASE_DEFENSES: u16 = 115;
    pub const BASE_UP: u16 = 121;
    pub const BASE_DOWN: u16 = 125;
    pub const BASE_MOVE: u16 = 129;
    pub const CAMPAIGN: u16 = 135;
    pub const BASE_ARMY: u16 = 139;
    pub const RECYCLE: u16 = 147;
    /// Map overview sidebar.
    pub const OVERVIEW_SIDEBAR: u16 = 41;
    /// First colour swatch (six frames per colour: red, blue, yellow, magenta, green).
    pub const SWATCH: u16 = 46;
    pub const UP: u16 = 76;
    pub const DOWN: u16 = 80;
    pub const BACK: u16 = 84;
    /// Technology dialog and its elements.
    pub const TECH_DIALOG: u16 = 157;
    pub const TECH_CONTINUE: u16 = 158;
    pub const TECH_UPGRADE: u16 = 162;
    pub const TECH_BACK: u16 = 168;
    pub const TECH_RIGHT: u16 = 176;
    pub const TECH_LEFT: u16 = 180;
    pub const TECH_DOWN: u16 = 184;
    pub const TECH_UP: u16 = 188;
    /// Continent (strategic) sidebar and its buttons.
    pub const STRATEGIC_SIDEBAR: u16 = 192;
    pub const INTELLIGENCE: u16 = 198;
    pub const ATTACK: u16 = 202;
    pub const STRATEGIC_BACK: u16 = 210;
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
    /// Sidebar during the other players' turns.
    pub const AI_SIDEBAR: u16 = 310;
    /// YES / NO question dialog and its buttons.
    pub const QUESTION: u16 = 477;
    pub const YES: u16 = 478;
    pub const NO: u16 = 482;
    pub const BOTTOM_BAR: u16 = 498;
    pub const CHAT: u16 = 499;
    /// AUTOMATED UNITS sidebar and its buttons.
    pub const AUTOMATED_SIDEBAR: u16 = 507;
    pub const AUTOMATED_BACK: u16 = 512;
    pub const AUTOMATED_MAP: u16 = 520;
    pub const AUTOMATED_GOAL: u16 = 524;
    pub const AUTOMATED_UP: u16 = 528;
    pub const AUTOMATED_DOWN: u16 = 532;
    /// Battle report shown when a battle is over, and its CONTINUE.
    pub const REPORT_DIALOG: u16 = 365;
    pub const REPORT_CONTINUE: u16 = 366;
    pub const GOAL_DIALOG: u16 = 633;
    pub const CONTINUE: u16 = 634;
    pub const OBJECTIVES: u16 = 638;
    /// OPTIONS dialog (the bottom bar's MENU) and its buttons.
    pub const OPTIONS_DIALOG: u16 = 648;
    pub const OPTIONS_QUIT: u16 = 649;
    pub const OPTIONS_BACK: u16 = 653;
    pub const OPTIONS_SAVE: u16 = 661;
    pub const OPTIONS_RESTART: u16 = 689;
    pub const MENU: u16 = 693;
    /// Small CONTINUE of the overview shown before deployment.
    pub const OVERVIEW_CONTINUE: u16 = 700;
    /// Skirmish selector dialog and its buttons.
    pub const SKIRMISH_DIALOG: u16 = 841;
    pub const SKIRMISH_CONTINUE: u16 = 842;
    pub const SKIRMISH_BACK: u16 = 846;
    pub const SKIRMISH_UP: u16 = 850;
    pub const SKIRMISH_DOWN: u16 = 854;
    /// Main menu panel and its buttons (four frames each).
    pub const MAIN_MENU: u16 = 710;
    pub const NEW_CAMPAIGN: u16 = 711;
    pub const LOAD_CAMPAIGN: u16 = 715;
    pub const MULTI_PLAYER: u16 = 719;
    pub const INTRO: u16 = 723;
    pub const RESOLUTION: u16 = 731;
    pub const QUIT: u16 = 735;
    pub const MAIN_SKIRMISH: u16 = 765;
    /// NEW GAME dialog (player name and difficulty) and its buttons.
    pub const NEW_GAME: u16 = 939;
    pub const NEW_GAME_BACK: u16 = 940;
    pub const NEW_GAME_CONTINUE: u16 = 944;
    /// Multiplayer SESSIONS dialog (hosting or joining) and its buttons.
    pub const SESSIONS: u16 = 917;
    pub const SESSIONS_BACK: u16 = 918;
    pub const JOIN_GAME: u16 = 930;
    pub const HOST_GAME: u16 = 934;
    /// Multiplayer WAITING AREA (lobby) and its buttons.
    pub const LOBBY: u16 = 1013;
    pub const LOBBY_START: u16 = 1014;
    pub const LOBBY_SEND: u16 = 1018;
    pub const LOBBY_BACK: u16 = 1022;
    /// Continent INTELLIGENCE dialog and its BACK.
    pub const INTEL_DIALOG: u16 = 557;
    pub const INTEL_BACK: u16 = 558;
}

/// `(first frame, last frame, width, height)` of every element used.
const CROPS: &[(u16, u16, i32, i32)] = &[
    (0, 0, SIDEBAR_W, SIDEBAR_H),
    (5, 20, 53, 17),
    (21, 24, 110, 17),
    (29, 32, 110, 17),
    (41, 41, SIDEBAR_W, SIDEBAR_H),
    (46, 75, 19, 19),
    (76, 87, 53, 17),
    (100, 100, SIDEBAR_W, SIDEBAR_H),
    (105, 108, 110, 17),
    (109, 120, 19, 19),
    (121, 152, 53, 17),
    (157, 157, 588, 451),
    (158, 171, 110, 17),
    (172, 172, 60, 55),
    (176, 191, 19, 17),
    (192, 192, SIDEBAR_W, SIDEBAR_H),
    (198, 205, 110, 17),
    (210, 213, 110, 17),
    (226, 226, SIDEBAR_W, SIDEBAR_H),
    (231, 238, 53, 17),
    (241, 246, 53, 17),
    (247, 250, 110, 17),
    (251, 256, 19, 19),
    (258, 263, 110, 17),
    (264, 269, 19, 19),
    (270, 277, 19, 17),
    (278, 281, 53, 17),
    (282, 289, 110, 17),
    (290, 293, 53, 17),
    (298, 301, 53, 17),
    (310, 310, SIDEBAR_W, SIDEBAR_H),
    (365, 365, 448, 480),
    (366, 369, 110, 17),
    (477, 477, 254, 144),
    (478, 489, 53, 17),
    (498, 498, BOTTOM_W, BOTTOM_H),
    (499, 502, 53, 17),
    (507, 507, SIDEBAR_W, SIDEBAR_H),
    (512, 515, 110, 17),
    (520, 535, 53, 17),
    (557, 557, 513, 311),
    (558, 561, 110, 17),
    (633, 633, 449, 268),
    (634, 637, 110, 17),
    (638, 643, 19, 19),
    (648, 648, 403, 371),
    (649, 664, 53, 17),
    (689, 692, 110, 17),
    (693, 696, 53, 17),
    (700, 703, 53, 17),
    (710, 710, 286, 281),
    (711, 738, 256, 28),
    (765, 768, 256, 28),
    (841, 841, 364, 416),
    (842, 849, 100, 24),
    (850, 857, 53, 17),
    (917, 917, 364, 322),
    (918, 937, 100, 24),
    (939, 939, 286, 228),
    (940, 947, 100, 24),
    (1013, 1013, 560, 469),
    (1014, 1025, 100, 24),
];

fn crop_of(frame: u16) -> Option<(i32, i32)> {
    CROPS
        .iter()
        .find(|&&(first, last, ..)| (first..=last).contains(&frame))
        .map(|&(_, _, w, h)| (w, h))
}

/// Covers the part of the bottom-right emblem left by the [`SIDEBAR_W`] cut
/// with the plain wall texture above it.
fn hide_emblem(image: &mut Image) {
    const X: usize = 116;
    const Y: usize = 667;
    const H: usize = 56;
    const FROM: usize = 590;
    let w = image.width;
    if w <= X || image.height < Y + H {
        return;
    }
    for row in 0..H {
        let (src, dst) = (((FROM + row) * w + X) * 4, ((Y + row) * w + X) * 4);
        image.rgba.copy_within(src..src + (w - X) * 4, dst);
    }
}

/// Covers the skirmish dialog's "SET TIME" label (the multiplayer turn timer,
/// which a single player skirmish does not offer) with camouflage copied from
/// further right in the same band.
fn hide_set_time(image: &mut Image) {
    const AT: (usize, usize) = (14, 346);
    const FROM_X: usize = 200;
    const SIZE: (usize, usize) = (84, 20);
    let w = image.width;
    if w < FROM_X + SIZE.0 || image.height < AT.1 + SIZE.1 {
        return;
    }
    for row in AT.1..AT.1 + SIZE.1 {
        let src = (row * w + FROM_X) * 4;
        image
            .rgba
            .copy_within(src..src + SIZE.0 * 4, (row * w + AT.0) * 4);
    }
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
            if matches!(
                index,
                frame::SIDEBAR
                    | frame::OVERVIEW_SIDEBAR
                    | frame::DEPLOY_SIDEBAR
                    | frame::AI_SIDEBAR
                    | frame::AUTOMATED_SIDEBAR
                    | frame::STRATEGIC_SIDEBAR
                    | frame::BASE_SIDEBAR
            ) {
                hide_emblem(&mut image);
            }
            if index == frame::SKIRMISH_DIALOG {
                hide_set_time(&mut image);
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
    // Deployment sidebar.
    DeployMap,
    DeployGoal,
    DeployUp,
    DeployDown,
    Army,
    DeployContinue,
    /// Overview before deployment (in place of BACK).
    OverviewContinue,
    // Technology dialog.
    TechLeft,
    TechUp,
    TechDown,
    TechRight,
    AutoLeft,
    AutoUp,
    AutoDown,
    AutoRight,
    TechUpgrade,
    TechContinue,
    TechBack,
    // Question dialog.
    Yes,
    No,
    // Skirmish selector.
    SkirmishUp,
    SkirmishDown,
    SkirmishBack,
    SkirmishContinue,
    // Battle report.
    ReportContinue,
    // Automated units sidebar.
    AutomatedMap,
    AutomatedGoal,
    AutomatedUp,
    AutomatedDown,
    AutomatedBack,
    // OPTIONS dialog.
    OptionsRestart,
    OptionsSave,
    OptionsQuit,
    OptionsBack,
    /// Scroll the options table (the technology dialog's arrows).
    OptionsUp,
    OptionsDown,
    // Main menu.
    NewCampaign,
    LoadCampaign,
    MainSkirmish,
    MultiPlayer,
    Intro,
    Resolution,
    Quit,
    // NEW GAME dialog.
    DifficultyLeft,
    DifficultyRight,
    NewGameBack,
    NewGameContinue,
    // Multiplayer SESSIONS dialog.
    HostGame,
    JoinGame,
    SessionsBack,
    // Multiplayer WAITING AREA.
    LobbySend,
    LobbyBack,
    LobbyStart,
    // Continent (strategic) sidebar.
    Intelligence,
    Attack,
    StrategicBack,
    /// After a victory, in place of BACK.
    StrategicContinue,
    /// BACK of the INTELLIGENCE dialog.
    IntelBack,
    // Base editor sidebar.
    BaseMap,
    BaseBuildings,
    BaseDefenses,
    BaseUp,
    BaseDown,
    Recycle,
    BaseMove,
    Campaign,
    BaseArmy,
}

/// Which panel set is on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Battle,
    /// The MAP overview: whole map with unit positions and per-player lists.
    Overview,
    /// The GOAL dialog over the battle (or deployment) screen.
    Goal,
    /// The skirmish selector dialog.
    Skirmishes,
    /// Technology upgrades; read-only when opened from deployment (ARMY).
    Tech {
        read_only: bool,
    },
    /// Overview of the starting zones before deployment.
    Briefing,
    /// Placing units in the starting zones.
    Deploy,
    /// "Continue anyway?" over the deployment screen.
    Confirm,
    /// Another player's (the AI's) turn.
    AiTurn,
    /// The AUTOMATED UNITS sidebar: choosing a support and its target.
    Automated,
    /// The battle is over: the battle report.
    Report,
    /// The OPTIONS dialog (MENU), possibly asking to confirm a button.
    Options {
        ask: Option<Ask>,
    },
    /// The main menu shown at startup.
    MainMenu,
    /// NEW CAMPAIGN: the player's name and the difficulty.
    NewGame,
    /// MULTI PLAYER: the player's name and the host to join, or hosting.
    Sessions,
    /// The WAITING AREA of a network game: players, game information and
    /// chat until the host starts a skirmish.
    Lobby,
    /// The continent overview: choosing the province to attack; CONTINUE
    /// replaces BACK after a victory.
    Strategic {
        victory: bool,
    },
    /// Base building between battles.
    BaseEditor,
    /// ARMY in base building: the technology dialog over the greyed base.
    Army,
    /// INTELLIGENCE: a province's report over the greyed continent.
    Intelligence,
}

/// A question the OPTIONS dialog asks before acting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// RESTART SCENARIO: "Do you really want to surrender?"
    Restart,
    /// QUIT: "Do you really want to quit the game?"
    Quit,
}

impl Screen {
    /// Full-window dialogs drawn without the map and the bottom bar.
    pub fn is_dialog(self) -> bool {
        matches!(
            self,
            Screen::Skirmishes
                | Screen::Tech { .. }
                | Screen::MainMenu
                | Screen::NewGame
                | Screen::Sessions
                | Screen::Lobby
        )
    }
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
            Button::DeployMap => DEPLOY_MAP,
            Button::DeployGoal => DEPLOY_GOAL,
            Button::DeployUp => DEPLOY_UP,
            Button::DeployDown => DEPLOY_DOWN,
            Button::Army => ARMY,
            Button::DeployContinue => DEPLOY_CONTINUE,
            Button::OverviewContinue => OVERVIEW_CONTINUE,
            Button::TechLeft | Button::AutoLeft => TECH_LEFT,
            Button::TechUp | Button::AutoUp => TECH_UP,
            Button::TechDown | Button::AutoDown => TECH_DOWN,
            Button::TechRight | Button::AutoRight => TECH_RIGHT,
            Button::TechUpgrade => TECH_UPGRADE,
            Button::TechContinue => TECH_CONTINUE,
            Button::TechBack => TECH_BACK,
            Button::Yes => YES,
            Button::No => NO,
            Button::SkirmishUp => SKIRMISH_UP,
            Button::SkirmishDown => SKIRMISH_DOWN,
            Button::SkirmishBack => SKIRMISH_BACK,
            Button::SkirmishContinue => SKIRMISH_CONTINUE,
            Button::ReportContinue => REPORT_CONTINUE,
            Button::AutomatedMap => AUTOMATED_MAP,
            Button::AutomatedGoal => AUTOMATED_GOAL,
            Button::AutomatedUp => AUTOMATED_UP,
            Button::AutomatedDown => AUTOMATED_DOWN,
            Button::AutomatedBack => AUTOMATED_BACK,
            Button::OptionsRestart => OPTIONS_RESTART,
            Button::OptionsSave => OPTIONS_SAVE,
            Button::OptionsQuit => OPTIONS_QUIT,
            Button::OptionsBack => OPTIONS_BACK,
            Button::OptionsUp => TECH_UP,
            Button::OptionsDown => TECH_DOWN,
            Button::NewCampaign => NEW_CAMPAIGN,
            Button::LoadCampaign => LOAD_CAMPAIGN,
            Button::MainSkirmish => MAIN_SKIRMISH,
            Button::MultiPlayer => MULTI_PLAYER,
            Button::Intro => INTRO,
            Button::Resolution => RESOLUTION,
            Button::Quit => QUIT,
            Button::DifficultyLeft => TECH_LEFT,
            Button::DifficultyRight => TECH_RIGHT,
            Button::NewGameBack => NEW_GAME_BACK,
            Button::NewGameContinue => NEW_GAME_CONTINUE,
            Button::HostGame => HOST_GAME,
            Button::JoinGame => JOIN_GAME,
            Button::SessionsBack => SESSIONS_BACK,
            Button::LobbySend => LOBBY_SEND,
            Button::LobbyBack => LOBBY_BACK,
            Button::LobbyStart => LOBBY_START,
            Button::Intelligence => INTELLIGENCE,
            Button::Attack => ATTACK,
            Button::StrategicBack => STRATEGIC_BACK,
            Button::StrategicContinue => TECH_CONTINUE,
            Button::IntelBack => INTEL_BACK,
            Button::BaseMap => BASE_MAP,
            Button::BaseBuildings => BASE_BUILDINGS,
            Button::BaseDefenses => BASE_DEFENSES,
            Button::BaseUp => BASE_UP,
            Button::BaseDown => BASE_DOWN,
            Button::Recycle => RECYCLE,
            Button::BaseMove => BASE_MOVE,
            Button::Campaign => CAMPAIGN,
            Button::BaseArmy => BASE_ARMY,
        }
    }

    /// Toggle boxes have six frames: off (normal, hover, pressed),
    /// unavailable, on (normal, hover).
    pub fn is_toggle(self) -> bool {
        matches!(
            self,
            Button::Overwatch
                | Button::AutoFire
                | Button::Fire
                | Button::Objectives
                | Button::Group
                | Button::BaseBuildings
                | Button::BaseDefenses
                | Button::Recycle
                | Button::BaseMove
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
            Button::DeployMap => (9, 172),
            Button::DeployGoal => (68, 172),
            Button::DeployUp => (9, 401),
            Button::DeployDown => (73, 401),
            Button::Army => (9, 430),
            Button::DeployContinue => (9, 456),
            Button::AutomatedMap => (9, 172),
            Button::AutomatedGoal => (68, 172),
            Button::AutomatedUp => (9, 401),
            Button::AutomatedDown => (73, 401),
            Button::AutomatedBack => (9, 430),
            Button::OverviewContinue => (10, 458),
            // Relative to the technology dialog.
            Button::TechLeft => (247, 280),
            Button::TechUp => (269, 280),
            Button::TechDown => (291, 280),
            Button::TechRight => (313, 280),
            Button::AutoLeft => (185, 426),
            Button::AutoUp => (207, 426),
            Button::AutoDown => (229, 426),
            Button::AutoRight => (251, 426),
            Button::TechUpgrade => (471, 401),
            Button::TechContinue | Button::TechBack => (471, 428),
            // Relative to the question dialog.
            Button::Yes => (20, 119),
            Button::No => (181, 119),
            // Relative to the skirmish dialog.
            Button::SkirmishUp => (44, 189),
            Button::SkirmishDown => (267, 189),
            Button::SkirmishBack => (12, 373),
            Button::SkirmishContinue => (252, 373),
            // Relative to the battle report.
            Button::ReportContinue => (300, 419),
            // Relative to the OPTIONS dialog (measured on the original).
            Button::OptionsRestart => (10, 340),
            Button::OptionsSave => (132, 340),
            Button::OptionsQuit => (198, 340),
            Button::OptionsBack => (336, 340),
            // The technology dialog's arrows, between QUIT and BACK.
            Button::OptionsUp => (273, 340),
            Button::OptionsDown => (295, 340),
            // Relative to the main menu panel, one row apart.
            Button::NewCampaign => (15, 40),
            Button::LoadCampaign => (15, 70),
            Button::MainSkirmish => (15, 100),
            Button::MultiPlayer => (15, 130),
            Button::Intro => (15, 160),
            Button::Resolution => (15, 190),
            Button::Quit => (15, 220),
            // Relative to the NEW GAME dialog.
            Button::DifficultyLeft => (17, 114),
            Button::DifficultyRight => (250, 114),
            Button::NewGameBack => (10, 184),
            Button::NewGameContinue => (170, 184),
            // Relative to the SESSIONS dialog, in its camouflage band.
            Button::HostGame => (14, 280),
            Button::JoinGame => (132, 280),
            Button::SessionsBack => (250, 280),
            // Relative to the WAITING AREA.
            Button::LobbySend => (450, 391),
            Button::LobbyBack => (15, 432),
            Button::LobbyStart => (445, 432),
            Button::Intelligence => (11, 387),
            Button::Attack => (11, 407),
            Button::StrategicBack | Button::StrategicContinue => (11, 456),
            Button::IntelBack => (364, 250),
            Button::BaseMap => (11, 171),
            Button::BaseBuildings => (12, 198),
            Button::BaseDefenses => (12, 222),
            Button::BaseUp => (11, 407),
            Button::BaseDown => (71, 407),
            Button::Recycle => (11, 435),
            Button::BaseMove => (70, 435),
            Button::Campaign => (11, 455),
            Button::BaseArmy => (70, 455),
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
    /// Continent sidebar: tech points, credits and the province text.
    StrategicTech,
    StrategicCredits,
    StrategicZero,
    StrategicText,
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
            Field::StrategicTech => Rect::new(63, 26, 47, 14),
            Field::StrategicCredits => Rect::new(63, 56, 47, 14),
            Field::StrategicZero => Rect::new(63, 86, 47, 14),
            Field::StrategicText => Rect::new(9, 150, 120, 232),
        }
    }
}

/// Height of an overview list row, in original pixels.
pub const LIST_ROW: i32 = 15;
/// Rows the overview list shows at once.
pub const LIST_ROWS: usize = (331 / LIST_ROW) as usize;
/// Goal dialog position over the map viewport.
pub const GOAL_AT: (i32, i32) = (40, 40);
/// Battle report position over the map viewport (measured on the original).
pub const REPORT_AT: (i32, i32) = (44, 1);
/// Battle report text area (the dialog inset by 20, 20, 20, 30).
pub const REPORT_TEXT: Rect = Rect::new(20, 20, 408, 430);
/// Battle report: top of the column headings and their x offsets in the
/// text area (`0x45ded0`).
pub const REPORT_HEADER_Y: i32 = 185;
pub const REPORT_COLUMNS: [i32; 4] = [25, 115, 250, 360];
/// OPTIONS dialog position over the map viewport (measured on the original).
pub const OPTIONS_AT: (i32, i32) = (39, 41);
/// The OPTIONS dialog's black options table, relative to the dialog: it
/// covers the original's SOUNDS / MOVE ANIMATIONS / SCROLL SPEED boxes and
/// CD PLAYER panel.
pub const OPTIONS_TABLE: Rect = Rect::new(14, 40, 373, 286);
/// Height of an options table row.
pub const OPTIONS_ROW: i32 = 22;
/// Rows the options table shows at once.
pub const OPTIONS_ROWS: usize = (OPTIONS_TABLE.h / OPTIONS_ROW) as usize;
/// Question dialog position over the map viewport.
pub const QUESTION_AT: (i32, i32) = (50, 70);
/// INTELLIGENCE dialog position.
pub const INTEL_AT: (i32, i32) = (40, 40);
/// INTELLIGENCE dialog's map preview box, relative to the dialog.
pub const INTEL_MAP: Rect = Rect::new(10, 10, 200, 200);
/// INTELLIGENCE dialog's text area, relative to the dialog.
pub const INTEL_TEXT: Rect = Rect::new(220, 20, 273, 261);
/// Question dialog text area, relative to the dialog.
pub const QUESTION_TEXT: Rect = Rect::new(8, 8, 238, 104);
/// Skirmish list `(x, y, w, h)`, relative to the dialog.
pub const SKIRMISH_LIST: (i32, i32, i32, i32) = (12, 47, 340, 140);
/// Skirmish description box, relative to the dialog.
pub const SKIRMISH_TEXT: Rect = Rect::new(16, 216, 332, 122);
pub const SKIRMISH_ROW: i32 = 17;
pub const SKIRMISH_ROWS: usize = 8;
/// Technology unit grid.
pub const TECH_CELL: (i32, i32) = (135, 43);
pub const TECH_COLUMNS: usize = 4;
pub const TECH_ROWS: usize = 6;
pub const TECH_CELLS: usize = TECH_COLUMNS * TECH_ROWS;
/// Technology dialog's automated units grid.
pub const AUTO_COLUMNS: usize = 2;
pub const AUTO_ROWS: usize = 2;
pub const AUTO_CELLS: usize = AUTO_COLUMNS * AUTO_ROWS;
/// Technology dialog: selected unit name, stats and remaining points.
pub const TECH_NAME: (i32, i32) = (299, 316);
pub const TECH_STATS: (i32, i32, i32) = (299, 419, 358);
pub const TECH_STAT_ROW: i32 = 14;
pub const TECH_POINTS: Rect = Rect::new(470, 372, 92, 17);
pub const TECH_EMBLEM: (u16, i32, i32) = (172, 496, 309);
/// Base editor build list: rows of `BASE_ROW`, `BASE_ROWS` visible.
pub const BASE_ROW: i32 = 39;
pub const BASE_ROWS: usize = 4;
/// Deployment list: rows of `DEPLOY_ROW`, `DEPLOY_ROWS` visible.
pub const DEPLOY_ROW: i32 = 39;
pub const DEPLOY_ROWS: usize = 5;

const DEPLOY_BUTTONS: [Button; 6] = [
    Button::DeployMap,
    Button::DeployGoal,
    Button::DeployUp,
    Button::DeployDown,
    Button::Army,
    Button::DeployContinue,
];

/// Bottom bar slices: the left end, a tileable middle, and the right end.
pub const BOTTOM_LEFT: i32 = 100;
pub const BOTTOM_TILE: (i32, i32) = (100, 160);
pub const BOTTOM_RIGHT: i32 = 671;
/// Sidebar rows tiled to fill windows taller than 768 original pixels.
pub const SIDEBAR_TILE: (i32, i32) = (610, 650);
/// Smallest window, in original pixels, that shows the whole interface: the
/// sidebar down to END TURN, and the goal dialog left of the sidebar.
pub const MIN_SIZE: (i32, i32) = (GOAL_AT.0 + 449 + SIDEBAR_W, 480);

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

    /// Like [`Layout::new`], lowering `scale` until the sidebar buttons and
    /// the goal dialog fit in the window.
    pub fn fitted(width: i32, height: i32, scale: i32) -> Self {
        let (min_w, min_h) = MIN_SIZE;
        let fits = |s: i32| min_w * s <= width && min_h * s <= height;
        let scale = (1..=scale.max(1)).rev().find(|&s| fits(s)).unwrap_or(1);
        Self::new(width, height, scale)
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
            Button::TechLeft
            | Button::TechUp
            | Button::TechDown
            | Button::TechRight
            | Button::AutoLeft
            | Button::AutoUp
            | Button::AutoDown
            | Button::AutoRight
            | Button::TechUpgrade
            | Button::TechContinue
            | Button::TechBack => self.s(native).offset_by(self.tech_dialog()),
            Button::Yes | Button::No => self.s(native).offset_by(self.question_dialog()),
            Button::IntelBack => self.s(native).offset_by(self.intel_dialog()),
            Button::SkirmishUp
            | Button::SkirmishDown
            | Button::SkirmishBack
            | Button::SkirmishContinue => self.s(native).offset_by(self.skirmish_dialog()),
            Button::ReportContinue => self.s(native).offset_by(self.report_dialog()),
            Button::OptionsRestart
            | Button::OptionsSave
            | Button::OptionsQuit
            | Button::OptionsBack
            | Button::OptionsUp
            | Button::OptionsDown => self.s(native).offset_by(self.options_dialog()),
            Button::NewCampaign
            | Button::LoadCampaign
            | Button::MainSkirmish
            | Button::MultiPlayer
            | Button::Intro
            | Button::Resolution
            | Button::Quit => self.s(native).offset_by(self.main_menu()),
            Button::DifficultyLeft
            | Button::DifficultyRight
            | Button::NewGameBack
            | Button::NewGameContinue => self.s(native).offset_by(self.new_game_dialog()),
            Button::HostGame | Button::JoinGame | Button::SessionsBack => {
                self.s(native).offset_by(self.sessions_dialog())
            }
            Button::LobbySend | Button::LobbyBack | Button::LobbyStart => {
                self.s(native).offset_by(self.lobby_dialog())
            }
            _ => self.in_sidebar(native),
        }
    }

    /// A dialog of `frame`'s size centred in the window.
    fn centred(&self, frame: u16) -> Rect {
        let (w, h) = crop_of(frame).unwrap_or((0, 0));
        let r = self.s(Rect::new(0, 0, w, h));
        Rect::new((self.width - r.w) / 2, (self.height - r.h) / 2, r.w, r.h)
    }

    pub fn skirmish_dialog(&self) -> Rect {
        self.centred(frame::SKIRMISH_DIALOG)
    }

    pub fn main_menu(&self) -> Rect {
        self.centred(frame::MAIN_MENU)
    }

    pub fn new_game_dialog(&self) -> Rect {
        self.centred(frame::NEW_GAME)
    }

    /// A rectangle in NEW GAME dialog coordinates.
    pub fn in_new_game(&self, r: Rect) -> Rect {
        self.s(r).offset_by(self.new_game_dialog())
    }

    pub fn sessions_dialog(&self) -> Rect {
        self.centred(frame::SESSIONS)
    }

    /// A rectangle in SESSIONS dialog coordinates.
    pub fn in_sessions(&self, r: Rect) -> Rect {
        self.s(r).offset_by(self.sessions_dialog())
    }

    pub fn lobby_dialog(&self) -> Rect {
        self.centred(frame::LOBBY)
    }

    /// A rectangle in WAITING AREA coordinates.
    pub fn in_lobby(&self, r: Rect) -> Rect {
        self.s(r).offset_by(self.lobby_dialog())
    }

    pub fn report_dialog(&self) -> Rect {
        let (w, h) = crop_of(frame::REPORT_DIALOG).unwrap_or((0, 0));
        self.s(Rect::new(REPORT_AT.0, REPORT_AT.1, w, h))
    }

    /// A rectangle in battle report coordinates.
    pub fn in_report(&self, r: Rect) -> Rect {
        self.s(r).offset_by(self.report_dialog())
    }

    pub fn options_dialog(&self) -> Rect {
        let (w, h) = crop_of(frame::OPTIONS_DIALOG).unwrap_or((0, 0));
        self.s(Rect::new(OPTIONS_AT.0, OPTIONS_AT.1, w, h))
    }

    /// A rectangle in OPTIONS dialog coordinates.
    pub fn in_options(&self, r: Rect) -> Rect {
        self.s(r).offset_by(self.options_dialog())
    }

    /// Row `i` (on screen) of the options table.
    pub fn options_row(&self, i: usize) -> Rect {
        let t = OPTIONS_TABLE;
        self.in_options(Rect::new(
            t.x,
            t.y + 4 + i as i32 * OPTIONS_ROW,
            t.w,
            OPTIONS_ROW,
        ))
    }

    pub fn tech_dialog(&self) -> Rect {
        self.centred(frame::TECH_DIALOG)
    }

    /// The continent INTELLIGENCE dialog, at a fixed position.
    pub fn intel_dialog(&self) -> Rect {
        let (w, h) = crop_of(frame::INTEL_DIALOG).unwrap_or((0, 0));
        self.s(Rect::new(INTEL_AT.0, INTEL_AT.1, w, h))
    }

    /// A rectangle in INTELLIGENCE dialog coordinates.
    pub fn in_intel(&self, r: Rect) -> Rect {
        self.s(r).offset_by(self.intel_dialog())
    }

    pub fn question_dialog(&self) -> Rect {
        let (w, h) = crop_of(frame::QUESTION).unwrap_or((0, 0));
        self.s(Rect::new(QUESTION_AT.0, QUESTION_AT.1, w, h))
    }

    /// A rectangle in skirmish dialog coordinates.
    pub fn in_skirmish(&self, r: Rect) -> Rect {
        self.s(r).offset_by(self.skirmish_dialog())
    }

    /// A rectangle in technology dialog coordinates.
    pub fn in_tech(&self, r: Rect) -> Rect {
        self.s(r).offset_by(self.tech_dialog())
    }

    /// A rectangle in question dialog coordinates.
    pub fn in_question(&self, r: Rect) -> Rect {
        self.s(r).offset_by(self.question_dialog())
    }

    /// A rectangle in sidebar coordinates.
    pub fn in_side(&self, r: Rect) -> Rect {
        self.in_sidebar(r)
    }

    /// Row `i` (on screen) of the skirmish list.
    pub fn skirmish_row(&self, i: usize) -> Rect {
        let (x, y, w, _) = SKIRMISH_LIST;
        self.in_skirmish(Rect::new(
            x + 1,
            y + 1 + i as i32 * SKIRMISH_ROW,
            w - 2,
            SKIRMISH_ROW,
        ))
    }

    /// Cell `i` of the technology unit grid.
    pub fn tech_cell(&self, i: usize) -> Rect {
        let (c, r) = ((i % TECH_COLUMNS) as i32, (i / TECH_COLUMNS) as i32);
        self.in_tech(Rect::new(
            8 + TECH_CELL.0 * c,
            10 + TECH_CELL.1 * r,
            TECH_CELL.0,
            TECH_CELL.1,
        ))
    }

    /// Cell `i` of the technology dialog's automated units.
    pub fn tech_automated_cell(&self, i: usize) -> Rect {
        let (c, r) = ((i % AUTO_COLUMNS) as i32, (i / AUTO_COLUMNS) as i32);
        self.in_tech(Rect::new(
            8 + TECH_CELL.0 * c,
            316 + TECH_CELL.1 * r,
            TECH_CELL.0,
            TECH_CELL.1,
        ))
    }

    /// Row `i` (on screen) of the deployment list.
    pub fn deploy_row(&self, i: usize) -> Rect {
        self.in_sidebar(Rect::new(9, 204 + i as i32 * DEPLOY_ROW, 120, DEPLOY_ROW))
    }

    /// Row `i` (on screen) of the base editor's build list.
    pub fn base_row(&self, i: usize) -> Rect {
        self.in_sidebar(Rect::new(9, 248 + i as i32 * BASE_ROW, 120, BASE_ROW))
    }

    /// Player row `i` of the sidebar shown during the AI's turn.
    pub fn ai_row(&self, i: usize) -> Rect {
        let y = [204, 233, 261, 289, 317][i.min(4)];
        self.in_sidebar(Rect::new(8, y, 122, 19))
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
        let overview = |buttons: &mut Vec<Button>| {
            buttons.extend((0..swatches.min(MAX_SWATCHES)).map(Button::Swatch));
            buttons.extend([Button::Up, Button::Down]);
        };
        match screen {
            Screen::Battle => buttons.extend(BATTLE_BUTTONS),
            Screen::Overview => {
                overview(&mut buttons);
                buttons.push(Button::Back);
            }
            Screen::Briefing => {
                overview(&mut buttons);
                buttons.push(Button::OverviewContinue);
            }
            Screen::Goal => buttons.extend([Button::Continue, Button::Objectives]),
            Screen::Skirmishes => buttons.extend([
                Button::SkirmishUp,
                Button::SkirmishDown,
                Button::SkirmishBack,
                Button::SkirmishContinue,
            ]),
            Screen::Tech { read_only } => {
                buttons.extend([
                    Button::TechLeft,
                    Button::TechUp,
                    Button::TechDown,
                    Button::TechRight,
                    Button::AutoLeft,
                    Button::AutoUp,
                    Button::AutoDown,
                    Button::AutoRight,
                    Button::TechUpgrade,
                ]);
                buttons.push(if read_only {
                    Button::TechBack
                } else {
                    Button::TechContinue
                });
            }
            Screen::Army => buttons.extend([
                Button::TechLeft,
                Button::TechUp,
                Button::TechDown,
                Button::TechRight,
                Button::AutoLeft,
                Button::AutoUp,
                Button::AutoDown,
                Button::AutoRight,
                Button::TechUpgrade,
                Button::TechBack,
            ]),
            Screen::Deploy => buttons.extend(DEPLOY_BUTTONS),
            Screen::Automated => buttons.extend([
                Button::AutomatedMap,
                Button::AutomatedGoal,
                Button::AutomatedUp,
                Button::AutomatedDown,
                Button::AutomatedBack,
            ]),
            Screen::Confirm => buttons.extend([Button::Yes, Button::No]),
            Screen::AiTurn => buttons.extend([Button::Map, Button::Goal]),
            Screen::Report => buttons.push(Button::ReportContinue),
            Screen::Options { ask: None } => buttons.extend([
                Button::OptionsRestart,
                Button::OptionsSave,
                Button::OptionsQuit,
                Button::OptionsBack,
                Button::OptionsUp,
                Button::OptionsDown,
            ]),
            Screen::Options { ask: Some(_) } => buttons.extend([Button::Yes, Button::No]),
            Screen::MainMenu => buttons.extend([
                Button::NewCampaign,
                Button::LoadCampaign,
                Button::MainSkirmish,
                Button::MultiPlayer,
                Button::Intro,
                Button::Resolution,
                Button::Quit,
            ]),
            Screen::NewGame => buttons.extend([
                Button::DifficultyLeft,
                Button::DifficultyRight,
                Button::NewGameBack,
                Button::NewGameContinue,
            ]),
            Screen::Sessions => {
                buttons.extend([Button::HostGame, Button::JoinGame, Button::SessionsBack])
            }
            Screen::Lobby => {
                buttons.extend([Button::LobbySend, Button::LobbyBack, Button::LobbyStart])
            }
            Screen::Strategic { victory } => buttons.extend([
                Button::Intelligence,
                Button::Attack,
                if victory {
                    Button::StrategicContinue
                } else {
                    Button::StrategicBack
                },
            ]),
            Screen::Intelligence => buttons.push(Button::IntelBack),
            Screen::BaseEditor => buttons.extend([
                Button::BaseMap,
                Button::BaseBuildings,
                Button::BaseDefenses,
                Button::BaseUp,
                Button::BaseDown,
                Button::Recycle,
                Button::BaseMove,
                Button::Campaign,
                Button::BaseArmy,
            ]),
        }
        if !matches!(
            screen,
            Screen::Goal
                | Screen::Skirmishes
                | Screen::Tech { .. }
                | Screen::Confirm
                | Screen::Report
                | Screen::Options { ask: Some(_) }
                | Screen::MainMenu
                | Screen::NewGame
                | Screen::Sessions
                | Screen::Lobby
        ) {
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

/// Right-button unit info, drawn over the minimap well (offsets relative to
/// the well, original pixels; measured from the original and `LIBDAY.EXE`).
pub mod unit_info {
    /// Portrait position and size.
    pub const PORTRAIT: (i32, i32, i32) = (8, 6, 38);
    /// Text: left edge beside the portrait, left edge below it, first line
    /// top and line height. The first three lines sit beside the portrait.
    pub const TEXT_BESIDE: i32 = 49;
    pub const TEXT_LEFT: i32 = 8;
    pub const TEXT_TOP: i32 = 6;
    pub const LINE: i32 = 14;
    /// Capability icons: first position, size and horizontal step.
    pub const ICONS: (i32, i32) = (8, 87);
    pub const ICON: i32 = 25;
    pub const ICON_STEP: i32 = 30;

    /// `LIBDAY.EXE` bitmap resources of the capability icons.
    pub const ANTI_AIR: u32 = 185;
    pub const MINE_DETECTION: u32 = 184;
    pub const FOREST: u32 = 182;
    pub const STEALTH: u32 = 181;
    pub const INDIRECT: u32 = 183;
    pub const LOW_SIGNATURE: u32 = 309;
    pub const ALL: [u32; 6] = [
        ANTI_AIR,
        MINE_DETECTION,
        FOREST,
        STEALTH,
        INDIRECT,
        LOW_SIGNATURE,
    ];

    /// Icons shown for a unit type, left to right (as in `LIBDAY.EXE`).
    pub fn icons(kind: &libday_data::defs::UnitType) -> Vec<u32> {
        [
            (kind.anti_air, ANTI_AIR),
            (kind.mine_detection > 0, MINE_DETECTION),
            (kind.movement == 2, FOREST),
            (kind.jammer <= 0, STEALTH),
            (kind.weapon_type == 1, INDIRECT),
            ((1..100).contains(&kind.jammer), LOW_SIGNATURE),
        ]
        .into_iter()
        .filter_map(|(on, id)| on.then_some(id))
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_anchors_sidebar_and_bottom_bar() {
        let layout = Layout::new(1024, 768, 1);
        assert_eq!(layout.viewport(), Rect::new(0, 0, 888, 736));
        assert_eq!(layout.button(Button::Map), Rect::new(898, 169, 53, 17));
        assert_eq!(layout.button(Button::Menu), Rect::new(828, 744, 53, 17));
        assert_eq!(layout.field(Field::Timer), Rect::new(5, 743, 45, 19));

        let big = Layout::new(2560, 1600, 2);
        assert_eq!(big.viewport(), Rect::new(0, 0, 2288, 1536));
        assert_eq!(
            big.button(Button::Goal),
            Rect::new(2288 + 142, 338, 106, 34)
        );
        assert_eq!(big.button(Button::Chat).right(), 2288 - 240 + 106);
        assert_eq!(
            big.hit(Screen::Battle, 0, 2288 + 150, 340),
            Some(Button::Goal)
        );
        assert_eq!(big.hit(Screen::Goal, 0, 2288 + 150, 340), None);
        assert!(big.on_panel(2300, 10) && !big.on_panel(10, 10));
    }

    #[test]
    fn overview_hits_swatches_and_rows() {
        let layout = Layout::new(1024, 768, 1);
        assert_eq!(
            layout.hit(Screen::Overview, 2, 898 + 30, 75),
            Some(Button::Swatch(1))
        );
        assert_eq!(layout.hit(Screen::Overview, 1, 898 + 30, 75), None);
        assert_eq!(layout.list_row(1), Rect::new(897, 111, 120, 15));
    }

    #[test]
    fn fitted_scale_keeps_the_interface_on_screen() {
        assert_eq!(Layout::fitted(2048, 1536, 2).scale, 2);
        assert_eq!(Layout::fitted(1280, 800, 2).scale, 1);
        assert_eq!(Layout::fitted(1250, 960, 2).scale, 2);
        assert_eq!(Layout::fitted(1249, 960, 2).scale, 1);
        assert_eq!(Layout::fitted(3840, 2160, 4).scale, 4);
        assert_eq!(Layout::fitted(100, 100, 3).scale, 1);
    }

    #[test]
    fn skirmish_setup_dialogs() {
        let layout = Layout::new(1024, 768, 1);
        let dialog = layout.skirmish_dialog();
        assert_eq!(dialog, Rect::new(330, 176, 364, 416));
        assert_eq!(
            layout.hit(Screen::Skirmishes, 0, 330 + 260, 176 + 380),
            Some(Button::SkirmishContinue)
        );
        assert_eq!(layout.skirmish_row(2).y, 176 + 48 + 34);
        let tech = layout.tech_dialog();
        assert_eq!(tech, Rect::new(218, 158, 588, 451));
        assert_eq!(layout.tech_cell(5), Rect::new(218 + 143, 158 + 53, 135, 43));
        let read_only = layout.buttons(Screen::Tech { read_only: true }, 0);
        assert!(read_only.contains(&Button::TechBack));
        assert!(!read_only.contains(&Button::TechContinue));
        assert!(!read_only.contains(&Button::Menu));
        let big = Layout::new(2048, 1536, 2);
        assert_eq!(big.question_dialog(), Rect::new(100, 140, 508, 288));
        assert_eq!(
            big.hit(Screen::Confirm, 0, 100 + 40 + 2, 140 + 238 + 2),
            Some(Button::Yes)
        );
        assert_eq!(
            big.hit(Screen::Deploy, 0, 2048 - 272 + 18 + 2, 860 + 2),
            Some(Button::Army)
        );
        assert_eq!(big.deploy_row(1).y, (204 + 39) * 2);
    }

    #[test]
    fn options_dialog() {
        let layout = Layout::new(1024, 768, 1);
        assert_eq!(layout.options_dialog(), Rect::new(39, 41, 403, 371));
        assert_eq!(
            layout.hit(Screen::Options { ask: None }, 0, 39 + 200, 41 + 345),
            Some(Button::OptionsQuit)
        );
        let ask = Screen::Options {
            ask: Some(Ask::Quit),
        };
        assert_eq!(layout.hit(ask, 0, 39 + 200, 41 + 345), None);
        assert!(!layout.buttons(ask, 0).contains(&Button::Menu));
        let big = Layout::new(2048, 1536, 2);
        assert_eq!(big.options_row(1).y, (41 + 40 + 4 + OPTIONS_ROW) * 2);
        const { assert!(OPTIONS_ROWS >= 12) };
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
