//! Saved games: any number of them, listed by the LOAD GAME dialog (LOAD
//! CAMPAIGN on the main menu) and the SAVE GAME dialog (SAVE in the game's
//! OPTIONS). A save holds the campaign (if any), the battle being fought
//! (if any) and what its turn needs to resume, with a summary and a small
//! picture of the map for the list.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use libday_data::Battle;
use libday_data::scenario::{Deployment, Scenario};
use libday_view::Image;
use libday_view::minimap;
use libday_view::ui::{SAVES_ROWS, Screen};
use sdl3::keyboard::Keycode;
use serde::{Deserialize, Serialize};

use crate::Viewer;
use crate::campaign::{Session, Stage, State};
use crate::players::Players;
use crate::skirmish::{Phase, Setup};

const FORMAT: &str = "openlibday-save";
const VERSION: u32 = 1;
/// The saves' directory, in the saves directory.
const GAMES: &str = "games";
/// Side of a save's picture, in pixels.
pub const THUMB: usize = 48;
/// Two clicks on a row this close together open it.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Campaign,
    Skirmish,
    /// A map opened on the command line.
    Map,
}

/// What the list shows of a save.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub kind: Kind,
    /// The campaign's player name.
    pub name: String,
    pub continent: String,
    pub captured: usize,
    pub total: usize,
    pub players: usize,
    pub difficulty: i64,
    /// The battle's map title and turn (none between battles).
    pub title: String,
    pub turn: i64,
}

/// What a battle's turn needs besides the battle itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Turn {
    units: Vec<i64>,
    tech: Vec<i64>,
    automated: Vec<i64>,
    points: i64,
    used_once: Vec<i64>,
    ai_budget: i64,
    captured: BTreeSet<i64>,
    humans: usize,
    /// The deployment zones and points (not in the battle save).
    deployments: Vec<[i64; 5]>,
    deploy_points: i64,
}

/// A saved campaign state of any version ([`State::from_value`]).
fn campaign_state<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<State>, D::Error> {
    let value: Option<serde_json::Value> = Option::deserialize(d)?;
    value
        .filter(|v| !v.is_null())
        .map(State::from_value)
        .transpose()
        .map_err(serde::de::Error::custom)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SaveFile {
    format: String,
    version: u32,
    /// Unix seconds.
    created: i64,
    summary: Summary,
    #[serde(default, deserialize_with = "campaign_state")]
    campaign: Option<State>,
    /// Saved in the base editor (reopened on load).
    #[serde(default)]
    base_editor: bool,
    #[serde(default)]
    battle: Option<serde_json::Value>,
    #[serde(default)]
    turn: Option<Turn>,
    /// `THUMB`x`THUMB` RGB pixels in hex.
    #[serde(default)]
    thumbnail: String,
}

/// The head of a save, read for the list.
#[derive(Deserialize)]
struct Head {
    format: String,
    version: u32,
    created: i64,
    summary: Summary,
    #[serde(default)]
    thumbnail: String,
}

/// A row of the list.
pub struct Entry {
    pub path: PathBuf,
    pub created: i64,
    pub summary: Summary,
    pub thumbnail: Option<Image>,
    /// The campaign's own save, kept as it goes.
    pub autosave: bool,
}

/// A second SAVE / DELETE confirms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    Overwrite(usize),
    Delete(usize),
}

/// The LOAD GAME / SAVE GAME dialog's state.
#[derive(Default)]
pub struct SavesUi {
    pub entries: Vec<Entry>,
    /// Selected row (with SAVE GAME, row 0 is a new save) and the first
    /// row shown.
    pub selected: Option<usize>,
    pub scroll: usize,
    pub confirm: Option<Confirm>,
    last_click: Option<(usize, Duration)>,
}

/// A save's date and time in local time.
pub fn date(created: i64) -> String {
    chrono::DateTime::from_timestamp(created, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn now() -> Duration {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
}

fn hex(image: &Image) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(image.width * image.height * 6);
    for p in image.rgba.chunks(4) {
        for c in &p[..3] {
            let _ = write!(out, "{c:02x}");
        }
    }
    out
}

fn unhex(text: &str) -> Option<Image> {
    let bytes = text.as_bytes();
    if bytes.len() != THUMB * THUMB * 6 {
        return None;
    }
    let digit = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut image = Image::new(THUMB, THUMB, [0, 0, 0, 255]);
    for (i, pair) in bytes.chunks(2).enumerate() {
        image.rgba[i / 3 * 4 + i % 3] = digit(pair[0])? * 16 + digit(pair[1])?;
    }
    Some(image)
}

/// The map seen from above, its markers on top, cut to `owner`'s buildings
/// (or the whole map) and fitted in `THUMB`x`THUMB`.
fn picture(s: &Scenario, defs: &libday_data::Definitions, owner: Option<i64>) -> Image {
    let size = s.size as i64;
    let mut map = minimap::tile_colours(s, defs);
    let mut set = |x: i64, y: i64, c: [u8; 3]| {
        if (0..size).contains(&x) && (0..size).contains(&y) {
            let at = (y * size + x) as usize * 4;
            map.rgba[at..at + 3].copy_from_slice(&c);
        }
    };
    let markers = minimap::markers(s, defs);
    for m in &markers {
        let c = if m.building {
            minimap::building_colour(m.owner)
        } else {
            minimap::owner_colour(m.owner)
        };
        for y in m.y..m.y + m.height.max(1) {
            for x in m.x..m.x + m.width.max(1) {
                set(x, y, c);
            }
        }
    }
    let mine: Vec<_> = markers
        .iter()
        .filter(|m| m.building && Some(m.owner) == owner)
        .collect();
    let (mut x0, mut y0, mut x1, mut y1) = (0, 0, size, size);
    if !mine.is_empty() {
        const MARGIN: i64 = 6;
        x0 = mine.iter().map(|m| m.x).min().unwrap_or(0) - MARGIN;
        y0 = mine.iter().map(|m| m.y).min().unwrap_or(0) - MARGIN;
        x1 = mine.iter().map(|m| m.x + m.width).max().unwrap_or(size) + MARGIN;
        y1 = mine.iter().map(|m| m.y + m.height).max().unwrap_or(size) + MARGIN;
        // Square, inside the map.
        let side = (x1 - x0).max(y1 - y0).min(size);
        let fit = |a: i64, b: i64| {
            let a = (a + b - side) / 2;
            a.clamp(0, size - side)
        };
        (x0, y0) = (fit(x0, x1), fit(y0, y1));
        (x1, y1) = (x0 + side, y0 + side);
    }
    let (w, h) = ((x1 - x0).max(1) as usize, (y1 - y0).max(1) as usize);
    let mut crop = Image::new(w, h, [0, 0, 0, 255]);
    for y in 0..h {
        let from = ((y0 as usize + y) * size as usize + x0 as usize) * 4;
        crop.rgba[y * w * 4..(y + 1) * w * 4].copy_from_slice(&map.rgba[from..from + w * 4]);
    }
    minimap::downscale(&crop, THUMB, THUMB)
}

impl Viewer {
    fn games_dir(&self) -> Option<PathBuf> {
        self.saves.as_ref().map(|d| d.join(GAMES))
    }

    /// Whether LOAD CAMPAIGN has anything to load.
    pub(crate) fn any_saves(&self) -> bool {
        self.campaign_saved()
            || self
                .games_dir()
                .and_then(|d| std::fs::read_dir(d).ok())
                .is_some_and(|mut d| {
                    d.any(|e| e.is_ok_and(|e| e.path().extension().is_some_and(|x| x == "json")))
                })
    }

    /// Whether SAVE can save now: in a campaign between battles, or during
    /// one's own turn of a game at this computer.
    pub(crate) fn can_save(&self) -> bool {
        if self.saves.is_none() || self.busy() {
            return false;
        }
        let between = self.strategic.is_some() || self.base_editor.is_some();
        if self.networked() {
            // The host saves an online campaign between battles.
            return self.net.host.is_none() && self.campaign.is_some() && between;
        }
        if self.campaign.is_some() && between {
            return true;
        }
        match self.flow.phase {
            Phase::Player => self.my_turn() && !self.flow.reinforcing && self.motions.is_empty(),
            Phase::Free => self.campaign.is_none(),
            _ => false,
        }
    }

    /// The summary of a campaign state.
    fn campaign_summary(&self, state: &State, data: &libday_data::campaign::Campaign) -> Summary {
        Summary {
            kind: Kind::Campaign,
            name: state.name.clone(),
            continent: data
                .continents
                .get(state.continent)
                .map(|c| c.name.en.clone())
                .unwrap_or_default(),
            captured: state.provinces.iter().filter(|p| p.captured).count(),
            total: state.provinces.len(),
            players: state.members.len(),
            difficulty: state.difficulty,
            title: String::new(),
            turn: 0,
        }
    }

    /// The base's picture: of the base editor's map, else the campaign's
    /// base.
    fn base_picture(&mut self) -> Option<Image> {
        let me = Some(self.seats.local);
        if self.base_editor.is_some() {
            return Some(picture(&self.map.battle.scenario, &self.defs, me));
        }
        let error = self.error.clone();
        let base = self.load_base(false);
        self.error = error;
        Some(picture(&base?.scenario, &self.defs, me))
    }

    /// Reads the saves, newest first; the campaign's own save too when
    /// loading.
    fn list_saves(&mut self, saving: bool) -> Vec<Entry> {
        let mut entries = Vec::new();
        if let Some(dir) = self.games_dir()
            && let Ok(files) = std::fs::read_dir(dir)
        {
            for path in files.flatten().map(|e| e.path()) {
                if path.extension().is_none_or(|x| x != "json") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(head) = serde_json::from_str::<Head>(&text) else {
                    continue;
                };
                if head.format != FORMAT || head.version == 0 || head.version > VERSION {
                    continue;
                }
                entries.push(Entry {
                    path,
                    created: head.created,
                    summary: head.summary,
                    thumbnail: unhex(&head.thumbnail),
                    autosave: false,
                });
            }
        }
        if !saving && let Some(entry) = self.autosave_entry() {
            entries.push(entry);
        }
        entries.sort_by(|a, b| {
            b.created
                .cmp(&a.created)
                .then(b.autosave.cmp(&a.autosave))
                .then(b.path.cmp(&a.path))
        });
        entries
    }

    /// The campaign's own save as a row.
    fn autosave_entry(&mut self) -> Option<Entry> {
        let path = self.campaign_save_path().filter(|p| p.is_file())?;
        let text = std::fs::read_to_string(&path).ok()?;
        let state = State::from_json(&text).ok()?;
        let data = self.game.original_campaign().ok()?;
        let created = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);
        let summary = self.campaign_summary(&state, &data);
        let won = state.stage == Stage::Won;
        let saved = self.campaign.replace(Session { data, state });
        let thumbnail = if won { None } else { self.base_picture() };
        self.campaign = saved;
        Some(Entry {
            path,
            created,
            summary,
            thumbnail,
            autosave: true,
        })
    }

    /// Opens SAVE GAME (`saving`) or LOAD GAME.
    pub(crate) fn open_saves(&mut self, saving: bool) {
        let entries = self.list_saves(saving);
        let any = saving || !entries.is_empty();
        self.saves_ui = SavesUi {
            entries,
            selected: any.then_some(0),
            ..SavesUi::default()
        };
        self.screen = Screen::Saves { saving };
    }

    fn saving(&self) -> bool {
        matches!(self.screen, Screen::Saves { saving: true })
    }

    /// The list's rows: with SAVE GAME a new save first.
    pub(crate) fn saves_rows(&self) -> usize {
        self.saves_ui.entries.len() + usize::from(self.saving())
    }

    /// The save of row `row`, `None` for a new save.
    pub(crate) fn saves_entry(&self, row: usize) -> Option<&Entry> {
        let i = row.checked_sub(usize::from(self.saving()))?;
        self.saves_ui.entries.get(i)
    }

    pub(crate) fn saves_enabled(&self, button: libday_view::ui::Button) -> bool {
        use libday_view::ui::Button;
        let ui = &self.saves_ui;
        match button {
            Button::SavesOk => ui.selected.is_some_and(|r| r < self.saves_rows()),
            Button::SavesUp => ui.scroll > 0,
            Button::SavesDown => ui.scroll + SAVES_ROWS < self.saves_rows(),
            _ => true,
        }
    }

    pub(crate) fn saves_activate(&mut self, button: libday_view::ui::Button) {
        use libday_view::ui::Button;
        match button {
            Button::SavesBack => self.close_saves(),
            Button::SavesOk => self.saves_ok(),
            Button::SavesUp => self.saves_scroll(-1),
            Button::SavesDown => self.saves_scroll(1),
            _ => {}
        }
    }

    fn close_saves(&mut self) {
        self.screen = if self.saving() {
            self.home()
        } else {
            Screen::MainMenu
        };
    }

    pub(crate) fn saves_scroll(&mut self, rows: isize) {
        let last = self.saves_rows().saturating_sub(SAVES_ROWS);
        let ui = &mut self.saves_ui;
        ui.scroll = ui.scroll.saturating_add_signed(rows).min(last);
    }

    /// Selects row `row`, keeping it on screen.
    pub(crate) fn saves_select(&mut self, row: usize) {
        let rows = self.saves_rows();
        if rows == 0 {
            return;
        }
        let row = row.min(rows - 1);
        let ui = &mut self.saves_ui;
        if ui.selected != Some(row) {
            ui.confirm = None;
        }
        ui.selected = Some(row);
        if row < ui.scroll {
            ui.scroll = row;
        } else if row >= ui.scroll + SAVES_ROWS {
            ui.scroll = row + 1 - SAVES_ROWS;
        }
    }

    fn saves_step(&mut self, step: isize) {
        let row = self
            .saves_ui
            .selected
            .map_or(0, |r| r.saturating_add_signed(step));
        self.saves_select(row);
    }

    /// SAVE / LOAD on the selected row; overwriting asks first.
    pub(crate) fn saves_ok(&mut self) {
        let Some(row) = self.saves_ui.selected.filter(|&r| r < self.saves_rows()) else {
            return;
        };
        let path = self.saves_entry(row).map(|e| e.path.clone());
        if !self.saving() {
            if let Some(path) = path {
                self.load_game(&path);
            }
            return;
        }
        if path.is_some() && self.saves_ui.confirm != Some(Confirm::Overwrite(row)) {
            self.saves_ui.confirm = Some(Confirm::Overwrite(row));
            return;
        }
        self.saves_ui.confirm = None;
        match self.save_game(path.as_deref()) {
            Ok(_) => {
                self.screen = self.home();
                self.flow.message = Some("The game has been saved".into());
            }
            Err(e) => self.error = Some(format!("save failed: {e}")),
        }
    }

    /// DELETE: deletes the selected save after a second press.
    fn saves_delete(&mut self) {
        let Some(row) = self.saves_ui.selected else {
            return;
        };
        let Some(entry) = self.saves_entry(row).filter(|e| !e.autosave) else {
            return;
        };
        let path = entry.path.clone();
        if self.saves_ui.confirm != Some(Confirm::Delete(row)) {
            self.saves_ui.confirm = Some(Confirm::Delete(row));
            return;
        }
        if let Err(e) = std::fs::remove_file(&path) {
            self.error = Some(format!("cannot delete {}: {e}", path.display()));
        }
        let saving = self.saving();
        let (selected, scroll) = (row, self.saves_ui.scroll);
        self.open_saves(saving);
        self.saves_ui.scroll = scroll;
        if self.saves_rows() > 0 {
            self.saves_select(selected);
        } else {
            self.saves_ui.selected = None;
        }
        self.saves_scroll(0);
    }

    /// A click on the list at output pixel `(x, y)`: selects a row, and a
    /// second click soon after opens it.
    pub(crate) fn saves_click(&mut self, x: i32, y: i32) {
        let layout = self.layout();
        let Some(i) = (0..SAVES_ROWS).find(|&i| layout.saves_row(i).contains(x, y)) else {
            return;
        };
        let row = self.saves_ui.scroll + i;
        if row >= self.saves_rows() {
            return;
        }
        let now = self.clock.now();
        let double = self
            .saves_ui
            .last_click
            .is_some_and(|(r, t)| r == row && now.saturating_sub(t) <= DOUBLE_CLICK);
        self.saves_select(row);
        if double {
            self.saves_ui.last_click = None;
            self.saves_ok();
        } else {
            self.saves_ui.last_click = Some((row, now));
        }
    }

    pub(crate) fn saves_key(&mut self, key: Keycode) {
        match key {
            Keycode::Escape => self.close_saves(),
            Keycode::Up => self.saves_step(-1),
            Keycode::Down => self.saves_step(1),
            Keycode::PageUp => self.saves_step(-(SAVES_ROWS as isize)),
            Keycode::PageDown => self.saves_step(SAVES_ROWS as isize),
            Keycode::Home => self.saves_select(0),
            Keycode::End => self.saves_select(usize::MAX),
            Keycode::Return | Keycode::KpEnter => self.saves_ok(),
            Keycode::Delete => self.saves_delete(),
            _ => {}
        }
    }

    /// Writes the game to `path`, or a new save; returns the file.
    pub(crate) fn save_game(&mut self, path: Option<&Path>) -> Result<PathBuf, String> {
        if !self.can_save() {
            return Err("the game cannot be saved now".into());
        }
        let dir = self.games_dir().ok_or("no saves directory")?;
        let stamp = now();
        let path = match path {
            Some(p) => p.to_owned(),
            None => (0..)
                .map(|i| dir.join(format!("{}.json", stamp.as_millis() + i)))
                .find(|p| !p.exists())
                .unwrap_or_default(),
        };
        let between = self.strategic.is_some() || self.base_editor.is_some();
        let campaign = self.campaign.as_ref().map(|c| c.state.clone());
        let mut summary = match (&campaign, &self.campaign) {
            (Some(state), Some(session)) => self.campaign_summary(state, &session.data),
            _ => Summary {
                kind: if self.flow.phase == Phase::Free {
                    Kind::Map
                } else {
                    Kind::Skirmish
                },
                players: self.seats.humans().count(),
                difficulty: self.map.battle.difficulty,
                ..Summary::default()
            },
        };
        let (battle, turn, thumbnail) = if between {
            (None, None, self.base_picture())
        } else {
            let b = &self.map.battle;
            let base = b.source.load(&self.game).map_err(|e| e.to_string())?;
            let json = b.to_json(&base).map_err(|e| e.to_string())?;
            let value: serde_json::Value =
                serde_json::from_str(&json).map_err(|e| e.to_string())?;
            let s = &b.scenario;
            summary.title = if s.info.title.is_empty() {
                self.map.file.clone()
            } else {
                s.info.title.clone()
            };
            summary.turn = b.clock.turn;
            let setup = self.flow.setup.as_ref();
            let turn = Turn {
                units: setup.map(|s| s.units.clone()).unwrap_or_default(),
                tech: setup.map(|s| s.tech.clone()).unwrap_or_default(),
                automated: setup.map(|s| s.automated.clone()).unwrap_or_default(),
                points: setup.map_or(0, |s| s.points),
                used_once: self.flow.used_once.clone(),
                ai_budget: self.flow.ai_budget,
                captured: self.flow.tech.clone(),
                humans: self.seats.humans().count(),
                deployments: s
                    .deployments
                    .iter()
                    .map(|d| [d.counter, d.player, d.x, d.y, d.turn])
                    .collect(),
                deploy_points: s.info.points,
            };
            (Some(value), Some(turn), Some(picture(s, &self.defs, None)))
        };
        let file = SaveFile {
            format: FORMAT.into(),
            version: VERSION,
            created: stamp.as_secs() as i64,
            summary,
            campaign,
            base_editor: self.base_editor.is_some(),
            battle,
            turn,
            thumbnail: thumbnail.as_ref().map(hex).unwrap_or_default(),
        };
        let json = serde_json::to_string(&file).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        // Write then rename so a failed write keeps the previous save.
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&temp, &path).map_err(|e| e.to_string())?;
        Ok(path)
    }

    /// Loads the save at `path` (or the campaign's own save).
    pub(crate) fn load_game(&mut self, path: &Path) {
        if self.campaign_save_path().as_deref() == Some(path) {
            self.load_campaign();
            return;
        }
        let result = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|text| serde_json::from_str::<SaveFile>(&text).map_err(|e| e.to_string()))
            .and_then(|file| {
                if file.format != FORMAT || file.version == 0 || file.version > VERSION {
                    return Err(format!(
                        "unsupported save {} v{}",
                        file.format, file.version
                    ));
                }
                self.restore(file)
            });
        match result {
            Ok(()) => {
                self.error = None;
                self.flow.message = Some("The game has been loaded".into());
            }
            Err(e) => self.error = Some(format!("load failed: {e}")),
        }
    }

    fn restore(&mut self, file: SaveFile) -> Result<(), String> {
        if file.campaign.as_ref().is_some_and(|c| c.members.len() > 1) {
            return Err("an online campaign cannot be resumed alone".into());
        }
        let battle = match &file.battle {
            Some(value) => {
                let text = value.to_string();
                let source = Battle::source_of(&text).map_err(|e| e.to_string())?;
                let base = source.load(&self.game).map_err(|e| e.to_string())?;
                Some(Battle::from_json(&text, base).map_err(|e| e.to_string())?)
            }
            None => None,
        };
        let session = match file.campaign {
            Some(state) => Some(Session {
                data: self.game.original_campaign().map_err(|e| e.to_string())?,
                state,
            }),
            None => None,
        };
        self.open_main_menu();
        self.campaign = session;
        // The campaign goes on from here.
        self.save_campaign();
        let Some(mut battle) = battle else {
            if self.campaign.is_none() {
                return Err("nothing saved".into());
            }
            if file.base_editor {
                self.open_base_editor();
            } else {
                self.resume_campaign();
            }
            return Ok(());
        };
        let turn = file.turn.unwrap_or_default();
        let s = &mut battle.scenario;
        s.deployments = turn
            .deployments
            .iter()
            .map(|&[counter, player, x, y, turn]| Deployment {
                counter,
                player,
                x,
                y,
                turn,
            })
            .collect();
        s.info.points = turn.deploy_points;
        self.show_campaign_map(battle);
        if file.summary.kind == Kind::Map {
            self.flow.phase = Phase::Free;
            self.screen = Screen::Battle;
            self.rebuild();
            return Ok(());
        }
        self.flow.setup = Some(Setup {
            units: turn.units,
            tech: turn.tech,
            automated: turn.automated,
            points: turn.points,
            cell: 0,
            tech_scroll: 0,
            auto_cell: 0,
            auto_scroll: 0,
            row: 0,
            scroll: 0,
            deployed: Vec::new(),
        });
        self.flow.used_once = turn.used_once;
        self.flow.ai_budget = turn.ai_budget;
        self.flow.tech = turn.captured;
        self.seats = Players::new(turn.humans);
        self.players = self.seats.overview();
        self.player = 0;
        self.screen = Screen::Battle;
        let player = self.map.battle.clock.player;
        self.begin_player_turn(player);
        self.rebuild();
        Ok(())
    }

    /// The list for scripts' `state`.
    pub(crate) fn saves_state(&self) -> serde_json::Value {
        let ui = &self.saves_ui;
        serde_json::json!({
            "selected": ui.selected,
            "scroll": ui.scroll,
            "count": ui.entries.len(),
            "confirm": ui.confirm.map(|c| format!("{c:?}")),
            "entries": ui.entries.iter().map(|e| serde_json::json!({
                "file": e.path.file_name().map(|n| n.to_string_lossy().into_owned()),
                "autosave": e.autosave,
                "kind": e.summary.kind,
                "name": e.summary.name,
                "continent": e.summary.continent,
                "captured": e.summary.captured,
                "total": e.summary.total,
                "players": e.summary.players,
                "difficulty": e.summary.difficulty,
                "title": e.summary.title,
                "turn": e.summary.turn,
                "thumbnail": e.thumbnail.is_some(),
            })).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_hex_round_trips() {
        let mut image = Image::new(THUMB, THUMB, [0, 0, 0, 255]);
        for (i, p) in image.rgba.chunks_mut(4).enumerate() {
            p[0] = i as u8;
            p[1] = (i / 7) as u8;
            p[2] = 255 - i as u8;
        }
        assert_eq!(unhex(&hex(&image)), Some(image));
        assert_eq!(unhex("00"), None);
    }
}
