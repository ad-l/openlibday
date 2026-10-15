//! openlibday application entry point.
//!
//! Without a scenario or save argument it starts a skirmish: the selector,
//! technology upgrades, deployment in the starting zones, then turns between
//! the player and the AI (which does nothing yet). With one it is a map
//! viewer where every unit can be moved. Maps draw terrain, roads, walls,
//! mines, buildings, units and starting zones in the original stacking
//! order, with pan and zoom, inside the original sidebar and bottom bar.
//!
//! Usage: `openlibday [--root DIR] [--load SAVE] [--script FILE [--headless]
//! [--size WxH]] [SCENARIO]`, where `SCENARIO` is a
//! scenario database name such as `s1-2-1` or `s1-2-1.mdb`,
//! `SAVE` an openlibday save, an original savegame directory or slot number,
//! and `FILE` a list of commands to run (see `script.rs`); `--headless` runs
//! it without a window, at `--size` (default 1024x768).

mod clock;
mod combat;
mod hud;
mod moves;
mod script;
mod settings;
mod skirmish;

use std::path::PathBuf;
use std::time::Duration;

use libday_data::{Battle, Definitions, Dir, GameData};
use libday_formats::Dbi;
use libday_view::minimap::{self, Entry, Marker};
use libday_view::scatter::Rand;
use libday_view::text::Fonts;
use libday_view::ui::{AUTO_CELLS, Button, LIST_ROWS, SKIRMISH_ROWS, Screen, TECH_CELLS};
use libday_view::{Art, Layer, Options, Scene, Sheet, map_pixels, sprite_origin, tile_at};
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::Keycode;
use sdl3::mouse::MouseButton;
use sdl3::pixels::Color;
use sdl3::render::{Canvas, FPoint, FRect, Texture};
use sdl3::video::Window;

use clock::Clock;
use hud::{Gfx, Result};
use skirmish::{Flow, Phase};

const ZOOMS: [f32; 9] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0];
const HELP: &str = "PgUp/PgDn map  wheel/+/- zoom  drag/arrows pan  Home recentre  \
                    G grid  D zones  U units  T terrain  [ ] reinforcements  \
                    M map overview  O goal  I interface scale  F5 quick save  F9 quick load  \
                    Enter end turn  Esc close/quit";
/// Quick save file in the saves directory.
const QUICK_SAVE: &str = "quick.json";
/// Window-coordinate movement below which a press counts as a click.
const CLICK_SLOP: f32 = 4.0;

struct Args {
    root: Option<String>,
    scenario: Option<String>,
    options: Option<PathBuf>,
    load: Option<String>,
    script: Option<PathBuf>,
    headless: bool,
    size: Option<(u32, u32)>,
}

fn parse_size(text: &str) -> Option<(u32, u32)> {
    let (w, h) = text.split_once(['x', 'X'])?;
    let (w, h) = (w.parse().ok()?, h.parse().ok()?);
    (w >= 640 && h >= 480).then_some((w, h))
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        root: None,
        scenario: None,
        options: None,
        load: None,
        script: None,
        headless: false,
        size: None,
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--root" => args.root = Some(iter.next().ok_or("--root needs a directory")?),
            "--options" => args.options = Some(iter.next().ok_or("--options needs a file")?.into()),
            "--load" => args.load = Some(iter.next().ok_or("--load needs a save")?),
            "--script" => args.script = Some(iter.next().ok_or("--script needs a file")?.into()),
            "--headless" => args.headless = true,
            "--size" => {
                let size = iter.next().ok_or("--size needs WIDTHxHEIGHT")?;
                args.size =
                    Some(parse_size(&size).ok_or("--size needs WIDTHxHEIGHT, at least 640x480")?);
            }
            "-h" | "--help" => {
                let file = settings::default_path()
                    .map_or_else(|| settings::FILE_NAME.into(), |p| p.display().to_string());
                println!(
                    "usage: openlibday [--root DIR] [--options FILE] [--load SAVE]\n\
                     \x20                 [--script FILE [--headless] [--size WxH]] [SCENARIO]\n\
                     SAVE: an openlibday save file, an original savegame directory, or\n\
                     \x20     the number of an original savegame slot (SAVEGAME\\savegame.NN)\n\
                     --script: run the commands in FILE on a virtual clock (see script.rs);\n\
                     \x20         --headless without a window, at --size (default 1024x768)\n\
                     options file: {file}\n{HELP}"
                );
                std::process::exit(0);
            }
            // M0 compatibility: a bare directory is the install root.
            _ if std::path::Path::new(&arg).is_dir() => args.root = Some(arg),
            _ => args.scenario = Some(arg),
        }
    }
    if args.headless && args.script.is_none() {
        return Err("--headless needs --script".into());
    }
    Ok(args)
}

/// The battle in play and its draw list.
struct Map {
    /// Scenario database or save it was loaded from.
    file: String,
    battle: Battle,
    scene: Scene,
}

struct Viewer {
    game: GameData,
    defs: Definitions,
    art: Art,
    fonts: Fonts,
    /// Unit portraits (`IMGS\LISTBOX.DBI`).
    listbox: Option<Dbi<'static>>,
    /// Scenario database names, in PgUp/PgDn order.
    maps: Vec<String>,
    index: usize,
    map: Map,
    /// Bumped whenever the scene changes (invalidates minimap pictures).
    generation: u64,
    options: Options,
    show_units: bool,
    grid: bool,
    /// Output pixels per map pixel.
    zoom: f32,
    /// Map pixel at the viewport's top-left corner.
    camera: (f32, f32),
    /// Output size in pixels.
    output: (f32, f32),
    /// Output pixels per window coordinate (mouse events use the latter).
    density: f32,
    /// Output pixels per original interface pixel.
    scale: i32,
    screen: Screen,
    /// Mouse position in output pixels.
    mouse: (i32, i32),
    /// Button held down (drawn pressed while the mouse stays on it).
    pressed: Option<Button>,
    /// Map drag in window coordinates, and where the press started.
    drag: Option<(f32, f32)>,
    press: Option<(f32, f32)>,
    minimap_drag: bool,
    /// Unit whose info replaces the minimap while the right button is held.
    info: Option<usize>,
    hover: Option<(i64, i64)>,
    /// Index into `map.battle.scenario.units`.
    selected: Option<usize>,
    preview: Option<moves::Preview>,
    motion: Option<moves::Motion>,
    /// Enemy locked as the selected unit's target, and a shot playing.
    lock: Option<combat::Lock>,
    shot: Option<combat::Shot>,
    /// The unit the FIRE button armed for a single shot at any tile.
    fire_mode: Option<usize>,
    /// `ANMSUNIT.FF` (walk animations).
    anims: Option<Vec<u8>>,
    objectives: bool,
    /// Units and buildings on the minimap; owners with an overview list.
    markers: Vec<Marker>,
    players: Vec<i64>,
    /// Overview: shown player slot, list scroll and selected (blinking) entry.
    player: usize,
    scroll: usize,
    entry: Option<usize>,
    /// Drives animations and timers (virtual in scripted runs).
    clock: Clock,
    /// The game's `rand()`: seeded from the time, or 1 in scripted runs.
    rng: Rand,
    /// Clock time the turn (or map) started; the bottom bar's timer.
    started: Duration,
    /// Where F5/F9 save and load.
    saves: Option<PathBuf>,
    /// Last save/load result, shown in the title.
    notice: Option<String>,
    error: Option<String>,
    /// Log events to stderr (`OPENLIBDAY_TRACE_EVENTS`).
    trace: bool,
    /// Skirmish phase and setup.
    flow: Flow,
    /// Set by a button that closes the program.
    quit: bool,
}

impl Viewer {
    fn load(&mut self, index: usize) {
        let file = self.maps[index].clone();
        match Battle::start(&self.game, &file) {
            Ok(battle) => {
                self.index = index;
                self.show(file, battle);
            }
            Err(err) => self.error = Some(format!("{file}: {err}")),
        }
    }

    fn show(&mut self, file: String, battle: Battle) {
        self.map = Map {
            file,
            scene: Scene::default(),
            battle,
        };
        self.options.arrival = self.show_units.then_some(0);
        self.reset();
        self.error = None;
        self.notice = None;
    }

    fn quick_save_path(&self) -> std::result::Result<PathBuf, String> {
        let dir = self.saves.as_ref().ok_or("no preferences directory")?;
        Ok(dir.join(QUICK_SAVE))
    }

    fn quick_save(&mut self) {
        let result = self.quick_save_path().and_then(|path| {
            self.map
                .battle
                .save(&self.game, &path)
                .map_err(|e| e.to_string())
                .map(|()| path)
        });
        match result {
            Ok(path) => self.notice = Some(format!("saved {}", path.display())),
            Err(err) => self.error = Some(format!("save failed: {err}")),
        }
    }

    fn quick_load(&mut self) {
        let result = self.quick_save_path().and_then(|path| {
            Battle::open(&self.game, &path)
                .map_err(|e| e.to_string())
                .map(|b| (path, b))
        });
        match result {
            Ok((path, battle)) => {
                self.show(path.display().to_string(), battle);
                self.notice = Some(format!("loaded {}", path.display()));
            }
            Err(err) => self.error = Some(format!("load failed: {err}")),
        }
    }

    /// Per-map state after loading a map.
    fn reset(&mut self) {
        self.rebuild();
        self.recentre();
        self.markers = minimap::markers(&self.map.battle.scenario, &self.defs);
        self.players = minimap::players(&self.map.battle.scenario, &self.markers);
        self.player = 0;
        self.scroll = 0;
        self.entry = None;
        self.selected = None;
        self.info = None;
        self.preview = None;
        self.lock = None;
        self.fire_mode = None;
        self.motion = None;
        self.shot = None;
        self.options.hidden_unit = None;
        self.hover = None;
        self.started = self.clock.now();
    }

    fn rebuild(&mut self) {
        self.map.scene = libday_view::build(
            &self.map.battle.scenario,
            &self.defs,
            &self.art,
            &self.options,
        );
        self.generation += 1;
        for name in &self.map.scene.missing {
            eprintln!("{}: missing {name}", self.map.file);
        }
    }

    fn recentre(&mut self) {
        let (w, h) = map_pixels(self.map.battle.scenario.size);
        let v = self.layout().viewport();
        self.camera = (
            w as f32 / 2.0 - v.w as f32 / (2.0 * self.zoom),
            h as f32 / 2.0 - v.h as f32 / (2.0 * self.zoom),
        );
    }

    /// Zooms one step, keeping the map point under output pixel `(px, py)` fixed.
    fn zoom_step(&mut self, step: i32, (px, py): (f32, f32)) {
        let current = ZOOMS
            .iter()
            .position(|&z| z >= self.zoom - 1e-3)
            .unwrap_or(ZOOMS.len() - 1) as i32;
        let next = ZOOMS[(current + step).clamp(0, ZOOMS.len() as i32 - 1) as usize];
        let (mx, my) = self.to_map(px, py);
        self.zoom = next;
        self.camera = (mx - px / next, my - py / next);
    }

    fn to_map(&self, px: f32, py: f32) -> (f32, f32) {
        (
            self.camera.0 + px / self.zoom,
            self.camera.1 + py / self.zoom,
        )
    }

    fn max_arrival(&self) -> i64 {
        let units = &self.map.battle.scenario.units;
        units.iter().map(|u| u.arrival).max().unwrap_or(0)
    }

    /// The shown player's overview list.
    fn entries(&self) -> Vec<Entry> {
        let owner = self.players.get(self.player).copied().unwrap_or(0);
        minimap::entries(&self.markers, &self.defs, owner)
    }

    /// Unit on the map tile under output pixel `(x, y)`.
    fn unit_under(&self, x: i32, y: i32) -> Option<usize> {
        let (mx, my) = self.to_map(x as f32, y as f32);
        let (tx, ty) = tile_at(self.map.battle.scenario.size, mx as f64, my as f64)?;
        self.unit_at(tx, ty)
    }

    /// Unit on tile `(x, y)` among the shown units.
    fn unit_at(&self, x: i64, y: i64) -> Option<usize> {
        let arrival = self.options.arrival?;
        if self.options.terrain_only {
            return None;
        }
        self.map.battle.scenario.units.iter().position(|u| {
            let (w, h) = self
                .defs
                .unit(u.kind)
                .map_or((1, 1), |k| (k.width, k.height));
            u.arrival == arrival && (u.x..u.x + w).contains(&x) && (u.y..u.y + h).contains(&y)
        })
    }

    /// Name, portrait image and the AP/AR lines of the selected unit.
    fn selected_unit(&self) -> Option<(String, String, String, String)> {
        let i = self.selected?;
        let unit = self.map.battle.scenario.units.get(i)?;
        let kind = self.kind_of(i)?;
        Some((
            kind.name.clone(),
            kind.image.clone(),
            format!(
                "AP {} / {}",
                unit.action_points.unwrap_or(kind.action_points),
                kind.action_points
            ),
            format!("AR {} / {}", unit.armor.unwrap_or(kind.armor), kind.armor),
        ))
    }

    fn centre_on_selected(&mut self) {
        let Some(unit) = self
            .selected
            .and_then(|i| self.map.battle.scenario.units.get(i))
        else {
            return;
        };
        let (w, h) = self
            .defs
            .unit(unit.kind)
            .map_or((1, 1), |k| (k.width, k.height));
        let (tx, ty) = (
            unit.x as f64 + w as f64 / 2.0,
            unit.y as f64 + h as f64 / 2.0,
        );
        self.centre_on_tile(tx, ty);
    }

    /// Units the next/previous buttons cycle through, in deployment order.
    fn cycle_units(&self) -> Vec<usize> {
        let Some(arrival) = self.options.arrival else {
            return Vec::new();
        };
        (0..self.map.battle.scenario.units.len())
            .filter(|&i| {
                self.map.battle.scenario.units[i].arrival == arrival && self.controllable(i)
            })
            .collect()
    }

    /// Selects the next (`step` 1) or previous (`step` -1) unit and centres on it.
    fn cycle_selected(&mut self, step: isize) {
        let units = self.cycle_units();
        if units.is_empty() || self.busy() {
            return;
        }
        let n = units.len() as isize;
        let next = match self
            .selected
            .and_then(|s| units.iter().position(|&u| u == s))
        {
            Some(at) => (at as isize + step).rem_euclid(n),
            None if step > 0 => 0,
            None => n - 1,
        };
        let unit = units[next as usize];
        if self.selected != Some(unit) {
            self.preview = None;
            self.lock = None;
            self.fire_mode = None;
        }
        self.selected = Some(unit);
        self.centre_on_selected();
    }

    /// Whether a button reacts to clicks (otherwise drawn greyed out).
    fn enabled(&self, button: Button) -> bool {
        let deploy_ready = self.flow.setup.is_some();
        match button {
            Button::Chat => false,
            Button::Overwatch => self.selected.is_some_and(|i| {
                self.controllable(i) && self.kind_of(i).is_some_and(|k| k.can_overwatch)
            }),
            Button::AutoFire => self.selected.is_some(),
            Button::Fire => !self.busy() && self.can_fire_mode(),
            Button::Left | Button::Right => !self.busy() && !self.cycle_units().is_empty(),
            Button::Up => self.scroll > 0,
            Button::Down => self.scroll + LIST_ROWS < self.entries().len(),
            Button::Menu => self.screen != Screen::Goal,
            Button::EndTurn => self.flow.phase == Phase::Free || self.can_end_turn(),
            Button::SkirmishUp => self.flow.pick > 0,
            Button::SkirmishDown => self.flow.pick + 1 < self.flow.catalog.len(),
            Button::TechLeft => self.tech_step_ok(-1),
            Button::TechRight => self.tech_step_ok(1),
            Button::TechUp => self.tech_step_ok(-Self::TECH_ROW),
            Button::TechDown => self.tech_step_ok(Self::TECH_ROW),
            Button::AutoLeft => self.auto_step_ok(-1),
            Button::AutoRight => self.auto_step_ok(1),
            Button::AutoUp => self.auto_step_ok(-Self::AUTO_ROW),
            Button::AutoDown => self.auto_step_ok(Self::AUTO_ROW),
            Button::TechUpgrade => self.can_upgrade(),
            Button::DeployUp => self.deploy_scroll_ok(-1),
            Button::DeployDown => self.deploy_scroll_ok(1),
            Button::Army | Button::DeployContinue => deploy_ready,
            _ => true,
        }
    }

    fn activate(&mut self, button: Button) {
        match button {
            Button::Map | Button::DeployMap => self.screen = Screen::Overview,
            Button::Goal | Button::DeployGoal => self.screen = Screen::Goal,
            Button::Back | Button::Continue => self.screen = self.home(),
            Button::Center => self.centre_on_selected(),
            Button::Left => self.cycle_selected(-1),
            Button::Right => self.cycle_selected(1),
            Button::Overwatch => self.toggle_overwatch(),
            Button::AutoFire => self.map.battle.auto_fire ^= true,
            Button::Fire => self.toggle_fire_mode(),
            Button::Objectives => self.objectives = !self.objectives,
            Button::Swatch(slot) => {
                self.player = slot;
                self.scroll = 0;
                self.entry = None;
            }
            Button::Up => self.scroll = self.scroll.saturating_sub(1),
            Button::Down => self.scroll += 1,
            Button::EndTurn => self.end_turn(),
            Button::SkirmishUp => self.pick_step(-1),
            Button::SkirmishDown => self.pick_step(1),
            Button::SkirmishBack => self.quit = true,
            Button::SkirmishContinue => self.choose_skirmish(),
            Button::TechLeft => self.tech_step(-1),
            Button::TechRight => self.tech_step(1),
            Button::TechUp => self.tech_step(-Self::TECH_ROW),
            Button::TechDown => self.tech_step(Self::TECH_ROW),
            Button::AutoLeft => self.auto_step(-1),
            Button::AutoRight => self.auto_step(1),
            Button::AutoUp => self.auto_step(-Self::AUTO_ROW),
            Button::AutoDown => self.auto_step(Self::AUTO_ROW),
            Button::TechUpgrade => self.upgrade(),
            Button::TechContinue => self.begin_briefing(),
            Button::TechBack => self.screen = Screen::Deploy,
            Button::OverviewContinue => self.begin_deploy(),
            Button::DeployUp => self.deploy_scroll(-1),
            Button::DeployDown => self.deploy_scroll(1),
            Button::Army => self.screen = Screen::Tech { read_only: true },
            Button::DeployContinue => self.finish_deploy(),
            Button::Yes => self.start_battle(),
            Button::No => self.screen = Screen::Deploy,
            _ => {}
        }
    }

    fn set_scale(&mut self, scale: i32) {
        let v = self.layout().viewport();
        let centre = self.to_map(v.w as f32 / 2.0, v.h as f32 / 2.0);
        self.scale = scale;
        let v = self.layout().viewport();
        self.camera = (
            centre.0 - v.w as f32 / (2.0 * self.zoom),
            centre.1 - v.h as f32 / (2.0 * self.zoom),
        );
    }

    fn title(&self) -> String {
        let battle = &self.map.battle;
        let info = &battle.scenario.info;
        let size = battle.scenario.size;
        let name = if info.title.is_empty() {
            "untitled"
        } else {
            &info.title
        };
        let mut title = format!(
            "openlibday - {name} ({}, {size}x{size}, {}/{})  turn {}  zoom {}%",
            self.map.file,
            self.index + 1,
            self.maps.len(),
            battle.clock.turn,
            (self.zoom / self.density * 100.0).round(),
        );
        match self.options.arrival {
            Some(0) => {}
            Some(turn) => title += &format!("  reinforcements of turn {turn}"),
            None => title += "  units hidden",
        }
        if self.options.terrain_only {
            title += "  terrain only";
        }
        if let Some((x, y)) = self.hover
            && let Some(cell) = self.map.battle.scenario.cell(x, y)
        {
            title += &format!("  [{x},{y}] {}", cell.terrain);
            let covers = |ox: i64, oy: i64, w: i64, h: i64| {
                (ox..ox + w).contains(&x) && (oy..oy + h).contains(&y)
            };
            let s = &self.map.battle.scenario;
            if !self.options.terrain_only {
                for b in &s.buildings {
                    if let Some(k) = self.defs.buildings.get(&b.kind)
                        && covers(b.x, b.y, k.width, k.height)
                    {
                        title += &format!(" - {} (P{})", k.name, b.owner);
                        break;
                    }
                }
                if let Some(u) = self.unit_at(x, y).map(|i| &s.units[i])
                    && let Some(k) = self.defs.unit(u.kind)
                {
                    title += &format!(" - {} #{}", k.name, u.id);
                    if !u.reinforcement {
                        title += &format!(" (P{})", u.owner);
                    }
                }
            }
        }
        if let Some(p) = &self.preview {
            let cost: u32 = p.steps.iter().map(|s| s.cost).sum();
            title += &format!("  path {} steps, {cost} AP", p.steps.len());
        }
        if let Some(notice) = &self.notice {
            title += &format!("  {notice}");
        }
        if let Some(err) = &self.error {
            title += &format!("  ERROR: {err}");
        }
        title
    }

    /// Keys of the full-window dialogs; `None` when not on one.
    fn dialog_key(&mut self, key: Keycode) -> Option<bool> {
        match (self.screen, key) {
            (Screen::Skirmishes, Keycode::Escape) => return Some(false),
            (Screen::Skirmishes, Keycode::Up) => self.pick_step(-1),
            (Screen::Skirmishes, Keycode::Down) => self.pick_step(1),
            (Screen::Skirmishes, Keycode::PageUp) => self.pick_step(-(SKIRMISH_ROWS as isize)),
            (Screen::Skirmishes, Keycode::PageDown) => self.pick_step(SKIRMISH_ROWS as isize),
            (Screen::Skirmishes, Keycode::Return | Keycode::KpEnter) => self.choose_skirmish(),
            (Screen::Tech { read_only }, _) => match key {
                Keycode::Left => self.tech_step(-1),
                Keycode::Right => self.tech_step(1),
                Keycode::Up => self.tech_step(-Self::TECH_ROW),
                Keycode::Down => self.tech_step(Self::TECH_ROW),
                Keycode::Space | Keycode::U => self.upgrade(),
                Keycode::Return | Keycode::KpEnter if !read_only => self.begin_briefing(),
                Keycode::Return | Keycode::KpEnter | Keycode::Escape if read_only => {
                    self.screen = Screen::Deploy
                }
                Keycode::Escape => {
                    self.screen = Screen::Skirmishes;
                    self.flow.setup = None;
                }
                _ => {}
            },
            (Screen::Briefing, Keycode::Return | Keycode::KpEnter | Keycode::Escape) => {
                self.begin_deploy()
            }
            (Screen::Confirm, Keycode::Return | Keycode::KpEnter | Keycode::Y) => {
                self.start_battle()
            }
            (Screen::Confirm, Keycode::Escape | Keycode::N) => self.screen = Screen::Deploy,
            (Screen::Skirmishes | Screen::Briefing | Screen::Confirm, _) => {}
            _ => return None,
        }
        Some(true)
    }

    fn key(&mut self, key: Keycode) -> bool {
        if let Some(running) = self.dialog_key(key) {
            return running;
        }
        let pan = 64.0 / self.zoom * self.density;
        let v = self.layout().viewport();
        let centre = (v.w as f32 / 2.0, v.h as f32 / 2.0);
        let count = self.maps.len();
        let free = self.flow.phase == Phase::Free;
        let home = self.home();
        match key {
            Keycode::Escape if self.screen != home => self.screen = home,
            Keycode::Escape => return false,
            Keycode::Return | Keycode::KpEnter if self.screen == Screen::Battle => self.end_turn(),
            Keycode::Return | Keycode::KpEnter if self.screen == Screen::Deploy => {
                self.finish_deploy()
            }
            Keycode::Left => self.camera.0 -= pan,
            Keycode::Right => self.camera.0 += pan,
            Keycode::Up => self.camera.1 -= pan,
            Keycode::Down => self.camera.1 += pan,
            Keycode::Home => self.recentre(),
            Keycode::Equals | Keycode::Plus | Keycode::KpPlus => self.zoom_step(1, centre),
            Keycode::Minus | Keycode::KpMinus => self.zoom_step(-1, centre),
            Keycode::PageDown | Keycode::PageUp => {
                // Browsing maps leaves the skirmish for the free viewer.
                self.flow = Flow::new();
                self.screen = Screen::Battle;
                self.options.deployment = false;
                let next = if key == Keycode::PageDown {
                    (self.index + 1) % count
                } else {
                    (self.index + count - 1) % count
                };
                self.load(next);
            }
            Keycode::F5 if matches!(self.flow.phase, Phase::Free | Phase::Player) => {
                self.quick_save()
            }
            Keycode::F9 if matches!(self.flow.phase, Phase::Free | Phase::Player) => {
                self.quick_load()
            }
            Keycode::G => self.grid = !self.grid,
            Keycode::M => {
                self.screen = match self.screen {
                    Screen::Overview => home,
                    _ => Screen::Overview,
                }
            }
            Keycode::O => {
                self.screen = match self.screen {
                    Screen::Goal => home,
                    _ => Screen::Goal,
                }
            }
            Keycode::I => {
                let max = (settings::MAX_SCALE as f32 * self.density).round() as i32;
                // Start from the scale in use, which may have been fitted down.
                self.set_scale(self.layout().scale % max + 1)
            }
            Keycode::D if free => {
                self.options.deployment = !self.options.deployment;
                self.rebuild();
            }
            Keycode::T => {
                self.options.terrain_only = !self.options.terrain_only;
                self.rebuild();
            }
            Keycode::U if free => {
                self.show_units = !self.show_units;
                self.options.arrival = self.show_units.then_some(0);
                self.selected = None;
                self.preview = None;
                self.lock = None;
                self.fire_mode = None;
                self.rebuild();
            }
            Keycode::LeftBracket | Keycode::RightBracket if free => {
                let step = if key == Keycode::RightBracket { 1 } else { -1 };
                let turn = (self.options.arrival.unwrap_or(0) + step).clamp(0, self.max_arrival());
                self.show_units = true;
                self.options.arrival = Some(turn);
                self.selected = None;
                self.preview = None;
                self.lock = None;
                self.fire_mode = None;
                self.rebuild();
            }
            _ => {}
        }
        true
    }

    fn output_point(&self, x: f32, y: f32) -> (i32, i32) {
        ((x * self.density) as i32, (y * self.density) as i32)
    }

    fn mouse_down(&mut self, button: MouseButton, x: f32, y: f32) {
        let (ox, oy) = self.output_point(x, y);
        self.mouse = (ox, oy);
        let layout = self.layout();
        let left = button == MouseButton::Left;
        if left && let Some(b) = layout.hit(self.screen, self.players.len(), ox, oy) {
            if self.enabled(b) {
                self.pressed = Some(b);
            }
            return;
        }
        match self.screen {
            Screen::Goal | Screen::Confirm => {}
            Screen::Skirmishes if left => {
                let layout = self.layout();
                if let Some(row) =
                    (0..SKIRMISH_ROWS).find(|&i| layout.skirmish_row(i).contains(ox, oy))
                {
                    let pick = self.flow.scroll + row;
                    if pick < self.flow.catalog.len() {
                        self.flow.pick = pick;
                    }
                }
            }
            Screen::Tech { .. } if left => {
                if let Some(setup) = &mut self.flow.setup {
                    if let Some(i) = (0..TECH_CELLS).find(|&i| layout.tech_cell(i).contains(ox, oy))
                    {
                        let cell = setup.tech_scroll * Self::TECH_ROW as usize + i;
                        if cell < setup.units.len() {
                            setup.cell = cell;
                        }
                    } else if let Some(i) =
                        (0..AUTO_CELLS).find(|&i| layout.tech_automated_cell(i).contains(ox, oy))
                    {
                        let cell = setup.auto_scroll * Self::AUTO_ROW as usize + i;
                        if cell < setup.automated.len() {
                            setup.auto_cell = cell;
                        }
                    }
                }
            }
            Screen::Skirmishes | Screen::Tech { .. } => {}
            Screen::Overview | Screen::Briefing if left => {
                let entries = self.entries().len();
                if let Some(row) = (0..LIST_ROWS).find(|&i| layout.list_row(i).contains(ox, oy)) {
                    let index = self.scroll + row;
                    if index < entries {
                        self.entry = (self.entry != Some(index)).then_some(index);
                    }
                } else if self.screen == Screen::Overview
                    && let Some((tx, ty)) = self.picture_tile(ox, oy)
                {
                    self.centre_on_tile(tx, ty);
                    self.screen = self.home();
                }
            }
            Screen::Overview | Screen::Briefing => {}
            Screen::Battle | Screen::Deploy | Screen::AiTurn => {
                let row = (self.screen == Screen::Deploy)
                    .then(|| {
                        (0..libday_view::ui::DEPLOY_ROWS)
                            .find(|&i| layout.deploy_row(i).contains(ox, oy))
                    })
                    .flatten();
                if left && let Some(row) = row {
                    let scroll = self.flow.setup.as_ref().map_or(0, |s| s.scroll);
                    self.deploy_select(scroll + row);
                } else if left && let Some((tx, ty)) = self.picture_tile(ox, oy) {
                    self.minimap_drag = true;
                    self.centre_on_tile(tx, ty);
                } else if !layout.on_panel(ox, oy) {
                    if button == MouseButton::Right
                        && let Some(i) = self.unit_under(ox, oy)
                    {
                        self.info = Some(i);
                        return;
                    }
                    self.drag = Some((x, y));
                    self.press = left.then_some((x, y));
                }
            }
        }
    }

    fn mouse_up(&mut self, x: f32, y: f32) {
        let (ox, oy) = self.output_point(x, y);
        if let Some(b) = self.pressed.take()
            && self.layout().hit(self.screen, self.players.len(), ox, oy) == Some(b)
        {
            self.activate(b);
        }
        if let Some((px, py)) = self.press.take()
            && (x - px).abs() < CLICK_SLOP
            && (y - py).abs() < CLICK_SLOP
        {
            let (mx, my) = self.to_map(ox as f32, oy as f32);
            match tile_at(self.map.battle.scenario.size, mx as f64, my as f64) {
                Some((tx, ty)) if self.screen == Screen::Deploy => self.deploy_click(tx, ty),
                Some((tx, ty)) if self.screen == Screen::Battle => self.click_tile(tx, ty),
                _ if !self.busy() => {
                    self.preview = None;
                    self.lock = None;
                    self.fire_mode = None;
                }
                _ => {}
            }
        }
        self.drag = None;
        self.minimap_drag = false;
        self.info = None;
    }

    fn mouse_motion(&mut self, x: f32, y: f32) {
        let (ox, oy) = self.output_point(x, y);
        self.mouse = (ox, oy);
        if self.minimap_drag {
            if let Some((tx, ty)) = self.picture_tile(ox, oy) {
                self.centre_on_tile(tx, ty);
            }
        } else if let Some((px, py)) = self.drag {
            let scale = self.density / self.zoom;
            self.camera.0 -= (x - px) * scale;
            self.camera.1 -= (y - py) * scale;
            self.drag = Some((x, y));
        }
        if let Some((px, py)) = self.press
            && ((x - px).abs() >= CLICK_SLOP || (y - py).abs() >= CLICK_SLOP)
        {
            self.press = None;
        }
        let on_map = matches!(
            self.screen,
            Screen::Battle | Screen::Deploy | Screen::AiTurn
        ) && !self.layout().on_panel(ox, oy);
        self.hover = on_map
            .then(|| {
                let (mx, my) = self.to_map(ox as f32, oy as f32);
                tile_at(self.map.battle.scenario.size, mx as f64, my as f64)
            })
            .flatten();
    }

    /// Handles one event; returns `false` to quit.
    fn handle(&mut self, event: Event) -> bool {
        if self.trace {
            eprintln!("{event:?}");
        }
        match event {
            Event::Quit { .. } => return false,
            Event::Window {
                win_event: WindowEvent::PixelSizeChanged(w, h),
                ..
            } => {
                // Keep the map point at the viewport centre fixed.
                let v = self.layout().viewport();
                let centre = self.to_map(v.w as f32 / 2.0, v.h as f32 / 2.0);
                self.output = (w as f32, h as f32);
                let v = self.layout().viewport();
                self.camera = (
                    centre.0 - v.w as f32 / (2.0 * self.zoom),
                    centre.1 - v.h as f32 / (2.0 * self.zoom),
                );
            }
            Event::KeyDown {
                keycode: Some(key), ..
            } => return self.key(key),
            Event::MouseWheel {
                y,
                mouse_x,
                mouse_y,
                ..
            } if y != 0.0 => {
                let (ox, oy) = self.output_point(mouse_x, mouse_y);
                let step = if y > 0.0 { 1 } else { -1 };
                match self.screen {
                    Screen::Battle | Screen::Deploy | Screen::AiTurn
                        if !self.layout().on_panel(ox, oy) =>
                    {
                        self.zoom_step(step, (ox as f32, oy as f32));
                    }
                    Screen::Deploy => self.deploy_scroll(-step as isize),
                    Screen::Skirmishes => {
                        let last = self.flow.catalog.len().saturating_sub(SKIRMISH_ROWS);
                        self.flow.scroll = self
                            .flow
                            .scroll
                            .saturating_add_signed(-step as isize)
                            .min(last);
                    }
                    Screen::Overview | Screen::Briefing => {
                        let last = self.entries().len().saturating_sub(LIST_ROWS);
                        self.scroll = if step > 0 {
                            self.scroll.saturating_sub(1)
                        } else {
                            (self.scroll + 1).min(last)
                        };
                    }
                    _ => {}
                }
            }
            Event::MouseButtonDown {
                mouse_btn: button @ (MouseButton::Left | MouseButton::Right | MouseButton::Middle),
                x,
                y,
                ..
            } => self.mouse_down(button, x, y),
            Event::MouseButtonUp { x, y, .. } => self.mouse_up(x, y),
            Event::MouseMotion { x, y, .. } => self.mouse_motion(x, y),
            _ => {}
        }
        !self.quit
    }
}

impl<'r> Gfx<'r> {
    fn sprite(&mut self, art: &Art, sheet: Sheet, name: &str) -> Option<&Texture<'r>> {
        let creator = self.creator();
        self.sprites
            .entry((sheet, name.to_owned()))
            .or_insert_with(|| {
                let (width, height, rgba) = art.rgba(sheet, name)?;
                hud::texture(
                    creator,
                    &libday_view::Image {
                        width,
                        height,
                        rgba,
                    },
                )
            })
            .as_ref()
    }
}

/// Draws the map into the viewport.
fn draw_map(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let v = viewer.layout().viewport();
    let (ow, oh) = (v.w as f32, v.h as f32);
    canvas.set_clip_rect(sdl3::rect::Rect::new(v.x, v.y, v.w as u32, v.h as u32));
    let (zoom, (cx, cy)) = (viewer.zoom, viewer.camera);
    let size = viewer.map.battle.scenario.size;
    let screen = |mx: i32, my: i32| ((mx as f32 - cx) * zoom, (my as f32 - cy) * zoom);
    let point = |(px, py): (i32, i32)| {
        let (x, y) = screen(px, py);
        FPoint::new(x, y)
    };
    let outside = |p: &FPoint| p.x < 0.0 || p.y < 0.0 || p.x > ow || p.y > oh;
    let screen_f = |mx: f64, my: f64| ((mx as f32 - cx) * zoom, (my as f32 - cy) * zoom);
    let footprint = |i: usize| -> Vec<(i64, i64)> {
        let Some(unit) = viewer.map.battle.scenario.units.get(i) else {
            return Vec::new();
        };
        let (w, h) = viewer
            .defs
            .unit(unit.kind)
            .map_or((1, 1), |k| (k.width, k.height));
        (0..h)
            .flat_map(|dy| (0..w).map(move |dx| (unit.x + dx, unit.y + dy)))
            .collect()
    };
    // Red grid under a selectable unit the mouse is over, drawn beneath it.
    let hover_tiles = if !viewer.busy() && matches!(viewer.screen, Screen::Battle) {
        viewer
            .hover
            .and_then(|(x, y)| viewer.unit_at(x, y))
            .filter(|&i| viewer.controllable(i))
            .map(footprint)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let cell =
        |canvas: &mut Canvas<Window>, gfx: &mut Gfx, left: i32, top: i32, name: &str| -> Result {
            let (x0, y0) = screen(left, top);
            let (x1, y1) = screen(left + 54, top + 56);
            if x1 < 0.0 || y1 < 0.0 || x0 > ow || y0 > oh {
                return Ok(());
            }
            if let Some(texture) = gfx.sprite(&viewer.art, Sheet::Symbols, name) {
                let (x0, y0, x1, y1) = (x0.floor(), y0.floor(), x1.floor(), y1.floor());
                canvas.copy(texture, None, Some(FRect::new(x0, y0, x1 - x0, y1 - y0)))?;
            }
            Ok(())
        };
    let mut walker = viewer.walker_depth().map(|(x, y)| (x + y, x));
    // Tile overlays drawn in depth order, after their tile's ground and
    // buildings but before its units, so objects in front cover them: the
    // selection grid, path dots and the move cursor. Target cursors go on
    // top of everything, over the unit or building they aim at.
    let (move_cursor, target_cursor) = match viewer.battle_cursor() {
        Some(c) if c.1 == "CMOVE" => (Some(c), None),
        c => (None, c),
    };
    let mut overlays: Vec<(i64, i64, &str)> = hover_tiles
        .into_iter()
        .map(|(x, y)| (x, y, "CSELC"))
        .chain(moves::path_dots(viewer))
        .chain(move_cursor.map(|((x, y), name)| (x, y, name)))
        .collect();
    overlays.sort_by_key(|&(x, y, _)| (x + y, x));
    let mut overlays = overlays.into_iter().peekable();
    for item in &viewer.map.scene.items {
        if let Some(depth) = walker
            && (item.x + item.y, item.x) > depth
        {
            // Path dots up to the walker's tile go underneath it.
            while let Some((x, y, name)) = overlays.next_if(|&(x, y, _)| (x + y, x) <= depth) {
                let (left, top) = sprite_origin(size, x, y);
                cell(canvas, gfx, left, top, name)?;
            }
            moves::draw_walker(canvas, gfx, viewer, &screen_f)?;
            walker = None;
        }
        let depth = (item.x + item.y, item.x);
        while let Some((x, y, name)) = overlays.next_if(|&(x, y, _)| {
            let at = (x + y, x);
            at < depth || (at == depth && item.layer == Layer::Unit)
        }) {
            let (left, top) = sprite_origin(size, x, y);
            cell(canvas, gfx, left, top, name)?;
        }
        let (left, top) = sprite_origin(size, item.x, item.y);
        let (x0, y0) = screen(left, top);
        let (x1, y1) = screen(left + 54, top + 56);
        if x1 < 0.0 || y1 < 0.0 || x0 > ow || y0 > oh {
            continue;
        }
        if let Some(texture) = gfx.sprite(&viewer.art, item.sheet, &item.sprite) {
            // Whole output pixels, so neighbouring tiles never leave seams.
            let (x0, y0, x1, y1) = (x0.floor(), y0.floor(), x1.floor(), y1.floor());
            canvas.copy(texture, None, Some(FRect::new(x0, y0, x1 - x0, y1 - y0)))?;
        }
    }
    if let Some(depth) = walker {
        while let Some((x, y, name)) = overlays.next_if(|&(x, y, _)| (x + y, x) <= depth) {
            let (left, top) = sprite_origin(size, x, y);
            cell(canvas, gfx, left, top, name)?;
        }
        moves::draw_walker(canvas, gfx, viewer, &screen_f)?;
    }
    for (x, y, name) in overlays {
        let (left, top) = sprite_origin(size, x, y);
        cell(canvas, gfx, left, top, name)?;
    }
    if let Some(((x, y), name)) = target_cursor {
        let (left, top) = sprite_origin(size, x, y);
        cell(canvas, gfx, left, top, name)?;
    }
    moves::draw_shot_effects(canvas, gfx, viewer, &screen_f)?;
    // Deployment cursor, on zone tiles only: a hazard-barred square, or a
    // red-barred one when a click would recall the unit already there.
    if matches!(viewer.screen, Screen::Deploy)
        && let Some((x, y)) = viewer.hover
        && viewer.zone(x, y)
    {
        let (left, top) = sprite_origin(size, x, y);
        let name = if viewer.recallable(x, y).is_some() {
            "CRECA"
        } else {
            "CDEPL"
        };
        cell(canvas, gfx, left, top, name)?;
    }
    let diamond = |canvas: &mut Canvas<Window>, tx: i64, ty: i64| -> Result {
        let corners = libday_view::compose::diamond(size, tx, ty);
        let points: Vec<FPoint> = corners
            .iter()
            .chain(corners.first())
            .map(|&p| point(p))
            .collect();
        if !points.iter().all(outside) {
            canvas.draw_lines(points.as_slice())?;
        }
        Ok(())
    };
    if viewer.grid {
        canvas.set_draw_color(Color::RGB(0x60, 0x70, 0x80));
        for ty in 0..size as i64 {
            for tx in 0..size as i64 {
                diamond(canvas, tx, ty)?;
            }
        }
    }
    // Arrows above the centre of a footprint: the selected unit's in its
    // owner's colour, a locked target's in red.
    let arrow = |canvas: &mut Canvas<Window>, gfx: &mut Gfx, tiles: &[(i64, i64)], name: &str| {
        let (Some(a), Some(b)) = (tiles.first(), tiles.last()) else {
            return Ok(());
        };
        let (ax, ay) = libday_view::tile_center(size, a.0, a.1);
        let (bx, by) = libday_view::tile_center(size, b.0, b.1);
        let (mx, my) = ((ax + bx) / 2, (ay + by) / 2);
        cell(
            canvas,
            gfx,
            mx - 27,
            my - libday_view::SPRITE_ANCHOR_Y,
            name,
        )
    };
    if !viewer.busy()
        && let Some(i) = viewer.selected
        && let Some(unit) = viewer.map.battle.scenario.units.get(i)
    {
        let name = format!("SU{:03}", unit.owner.clamp(0, 4));
        arrow(canvas, gfx, &footprint(i), &name)?;
    }
    if !viewer.busy() {
        arrow(canvas, gfx, &viewer.lock_tiles(), "SU000")?;
    }
    canvas.set_clip_rect(None);
    Ok(())
}

/// Draws a frame (without presenting it).
fn render(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    canvas.set_draw_color(Color::RGB(0, 0, 0));
    canvas.clear();
    if !viewer.screen.is_dialog() && !matches!(viewer.screen, Screen::Overview | Screen::Briefing) {
        draw_map(canvas, gfx, viewer)?;
    }
    hud::draw(canvas, gfx, viewer)?;
    Ok(())
}

fn main() -> Result {
    let args = parse_args()?;
    let script = match &args.script {
        Some(path) => {
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let lines = script::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            Some((path.clone(), lines))
        }
        None => None,
    };
    let game = match args.root {
        Some(root) => GameData::open(root)?,
        None => GameData::from_env()?,
    };
    let defs = game.definitions()?;
    let art = Art::load(&game, &defs)?;
    let ui = libday_view::load_ui(&game)?;
    let listbox = game
        .read(Dir::Imgs, "LISTBOX.DBI")
        .ok()
        .and_then(|bytes| Dbi::from_vec(bytes).ok());
    let mut maps: Vec<String> = game
        .skirmishes()?
        .into_iter()
        .map(|e| e.file_name)
        .collect();
    let loaded = match &args.load {
        Some(save) => {
            let path = match save.parse::<u32>() {
                Ok(n) => game.dir(Dir::SaveGame).join(format!("savegame.{n:02}")),
                Err(_) => PathBuf::from(save),
            };
            let battle = Battle::open(&game, &path).map_err(|e| format!("{save}: {e}"))?;
            Some((path.display().to_string(), battle))
        }
        None => None,
    };
    let scenario = match &loaded {
        Some((_, battle)) => match &battle.source {
            libday_data::battle::Source::Scenario(name) => Some(name.clone()),
            libday_data::battle::Source::File(_) => None,
        },
        None => args.scenario.clone(),
    };
    let index = match &scenario {
        Some(name) => {
            let name = if name.to_ascii_lowercase().ends_with(".mdb") {
                name.clone()
            } else {
                format!("{name}.mdb")
            };
            maps.iter()
                .position(|m| m.eq_ignore_ascii_case(&name))
                .unwrap_or_else(|| {
                    maps.insert(0, name);
                    0
                })
        }
        None => 0,
    };
    if maps.is_empty() {
        return Err("no skirmish maps found (SCENS\\SKIRMISH.MDB is empty)".into());
    }
    let (file, battle) = match loaded {
        Some(loaded) => loaded,
        None => (maps[index].clone(), Battle::start(&game, &maps[index])?),
    };

    // SDL's default `french_numbers` maps AZERTY's number row to digits, which
    // would turn the '-' (zoom out) key into '6'.
    sdl3::hint::set("SDL_KEYCODE_OPTIONS", "hide_numpad,latin_letters");
    // The click that focuses the window also acts.
    sdl3::hint::set("SDL_MOUSE_FOCUS_CLICKTHROUGH", "1");
    if args.headless {
        // No window on screen; software rendering so frames can be read back.
        sdl3::hint::set("SDL_VIDEO_DRIVER", "offscreen");
        sdl3::hint::set("SDL_RENDER_DRIVER", "software");
    }
    let sdl = sdl3::init()?;
    let video = sdl.video()?;
    // Headless runs ignore the user's options unless given, to be reproducible.
    let options_path = if args.headless {
        args.options
    } else {
        args.options.or_else(settings::default_path)
    };
    let settings = options_path
        .as_deref()
        .map(settings::Settings::load)
        .unwrap_or_default();
    // Fit to 90% of the screen unless a size is configured.
    let usable = video
        .get_primary_display()
        .and_then(|d| d.get_usable_bounds())
        .map(|r| (r.width(), r.height()))
        .unwrap_or((1280, 800));
    let fit = |set: u32, screen: u32| if set > 0 { set } else { screen * 9 / 10 };
    let (width, height) = match args.size {
        Some(size) => size,
        None if args.headless => (1024, 768),
        None => (
            fit(settings.width, usable.0),
            fit(settings.height, usable.1),
        ),
    };
    let mut builder = video.window("openlibday", width, height);
    if !args.headless {
        builder.position_centered().resizable().high_pixel_density();
        if settings.fullscreen {
            builder.fullscreen();
        }
    }
    let mut window = builder.build()?;
    if !args.headless {
        window.set_minimum_size(640, 520)?;
    }
    let density = window.pixel_density().max(1.0);
    let (pw, ph) = window.size_in_pixels();
    let mut canvas = window.into_canvas();
    let creator = canvas.texture_creator();
    let mut gfx = Gfx::new(&creator, &ui);
    gfx.load_icons(&game);
    drop(ui);

    let anims = game.read(Dir::AnmsUnit, "ANMSUNIT.FF").ok();
    let mut viewer = Viewer {
        game,
        defs,
        art,
        fonts: Fonts::new(),
        listbox,
        index,
        map: Map {
            file,
            battle,
            scene: Scene::default(),
        },
        maps,
        generation: 0,
        options: Options::default(),
        show_units: true,
        grid: false,
        zoom: density,
        camera: (0.0, 0.0),
        output: (pw as f32, ph as f32),
        density,
        scale: ((settings.scale as f32 * density).round() as i32).max(1),
        screen: Screen::Battle,
        mouse: (0, 0),
        pressed: None,
        drag: None,
        press: None,
        minimap_drag: false,
        info: None,
        hover: None,
        selected: None,
        preview: None,
        motion: None,
        lock: None,
        shot: None,
        fire_mode: None,
        anims,
        objectives: true,
        markers: Vec::new(),
        players: Vec::new(),
        player: 0,
        scroll: 0,
        entry: None,
        clock: if args.script.is_some() {
            Clock::manual()
        } else {
            Clock::real()
        },
        rng: Rand::new(if args.script.is_some() {
            1
        } else {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |d| d.as_secs() as u32)
        }),
        started: Duration::ZERO,
        saves: options_path
            .as_deref()
            .and_then(|p| p.parent())
            .map(|dir| dir.join("saves")),
        notice: None,
        error: None,
        trace: std::env::var_os("OPENLIBDAY_TRACE_EVENTS").is_some(),
        flow: Flow::new(),
        quit: false,
    };
    viewer.options.arrival = Some(0);
    viewer.reset();
    if args.scenario.is_none() && args.load.is_none() {
        viewer.open_selector()?;
    }

    let mut events = sdl.event_pump()?;
    if let Some((path, lines)) = &script {
        let mut runner = script::Runner {
            canvas: &mut canvas,
            gfx: &mut gfx,
            events: &mut events,
            visible: !args.headless,
            frame: Duration::ZERO,
        };
        if !runner.run(&mut viewer, path, lines)? || args.headless {
            return Ok(());
        }
        // A shown run stays playable once its script ends.
        viewer.clock.go_live();
    }
    let mut title = String::new();
    let mut dirty = true;
    let mut second = 0;
    let mut was_moving = false;
    loop {
        let mut running = true;
        let moving = viewer.tick();
        // The frame after a walk or shot ends shows the unit standing again.
        dirty |= moving != was_moving;
        was_moving = moving;
        dirty |= viewer.ai_tick();
        let wait = if moving {
            15
        } else if viewer.animating() {
            100
        } else {
            250
        };
        if let Some(event) = events.wait_event_timeout(Duration::from_millis(wait)) {
            running &= viewer.handle(event);
            dirty = true;
        }
        for event in events.poll_iter() {
            running &= viewer.handle(event);
            dirty = true;
        }
        if !running {
            break;
        }
        let now = viewer.clock.since(viewer.started).as_secs();
        if now != second || viewer.animating() || moving {
            second = now;
            dirty = true;
        }
        let next = viewer.title();
        if next != title {
            canvas.window_mut().set_title(&next)?;
            title = next;
        }
        if dirty {
            // Catch up with the time spent waiting for events, so the
            // hidden unit and its animation agree with the drawn instant.
            viewer.tick();
            render(&mut canvas, &mut gfx, &viewer)?;
            canvas.present();
            dirty = false;
        }
    }
    Ok(())
}
