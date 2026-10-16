//! openlibday application entry point.
//!
//! Without a scenario or save argument it starts at the main menu (NEW
//! CAMPAIGN, SKIRMISH, QUIT). A skirmish runs the selector,
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

mod ai;
mod army;
mod automated;
mod base_editor;
mod campaign;
mod clock;
mod combat;
mod front;
mod group;
mod hud;
mod input;
mod menu;
mod mixer;
mod moves;
mod render;
mod script;
mod settings;
mod skirmish;
mod sound;
mod strategic;

use std::path::PathBuf;
use std::time::Duration;

use libday_data::{Battle, Definitions, Dir, GameData};
use libday_formats::Dbi;
use libday_view::minimap::{self, Entry, Marker};
use libday_view::scatter::Rand;
use libday_view::text::Fonts;
use libday_view::ui::{Button, LIST_ROWS, Screen};
use libday_view::{Art, Options, Scene, map_pixels, tile_at};

use clock::Clock;
use hud::{Gfx, Result};
use skirmish::{Flow, Phase};

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
    /// Unit, building, wall or road whose info replaces the minimap while
    /// the right button is held.
    info: Option<libday_view::inspect::Target>,
    /// Overwatch zone of the inspected enemy (`info`): tiles and markers.
    watch: Vec<((i64, i64), String)>,
    hover: Option<(i64, i64)>,
    /// Index into `map.battle.scenario.units`: the selection's leader (the
    /// unit last added to it).
    selected: Option<usize>,
    /// The units selected together with GROUP, in the order added; it holds
    /// only while it ends with `selected`.
    group: Vec<usize>,
    /// The GROUP box is checked: unit clicks add to the selection.
    grouping: bool,
    /// A group order went out: the next unit click starts a new selection
    /// (`view+0x1c8`).
    stale: bool,
    preview: Option<moves::Preview>,
    /// Units walking, together when a batch was moved.
    motions: Vec<moves::Motion>,
    /// What finished walks ended in (a shot at a locked target, a mine,
    /// reaction fire), played in order once every walk is over.
    ends: Vec<(usize, moves::End)>,
    /// A group's shots still to fire, in order.
    volley: Vec<(usize, libday_view::fire::Target, bool)>,
    /// Enemy locked as the selected unit's target, and a shot playing.
    lock: Option<combat::Lock>,
    shot: Option<combat::Shot>,
    /// Earlier shots of a burst still in the air, in firing order.
    flights: Vec<combat::Shot>,
    /// Automated unit legs still to play (a transport's set-down).
    supports: Vec<combat::Shot>,
    /// Reaction fire at a unit spotted moving.
    reaction: Option<combat::Reaction>,
    /// Overwatch spots moving units (scripts may turn it off).
    reactions: bool,
    /// The AI turn in progress, and how the AI plays (`None`: it passes).
    brain: Option<ai::Brain>,
    ai_mode: Option<settings::AiMode>,
    /// The unit the FIRE button armed for a single shot at any tile.
    fire_mode: Option<usize>,
    /// `ANMSUNIT.FF` (unit animations).
    anims: Option<Vec<u8>>,
    /// Animation sound.
    sound: sound::Sound,
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
    /// The options as the OPTIONS dialog edits them, and the file they are
    /// saved to (none in headless runs unless given).
    settings: settings::Settings,
    settings_path: Option<PathBuf>,
    /// OPTIONS table scroll and focus.
    menu: menu::Menu,
    /// Fullscreen switch for the main loop to apply.
    fullscreen: Option<bool>,
    /// Main menu and NEW GAME dialog state.
    front: front::Front,
    /// The campaign being played, if any.
    campaign: Option<campaign::Session>,
    /// The continent overview, while it is open.
    strategic: Option<strategic::Strategic>,
    base_editor: Option<base_editor::BaseEditor>,
    /// The greyed map's snapshot serial for ARMY.
    army_view: u64,
}

impl Viewer {
    fn load(&mut self, index: usize) {
        let file = self.maps[index].clone();
        match Battle::start(&self.game, &file) {
            Ok(mut battle) => {
                battle.scenario.restore(&self.defs);
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

    /// Plays an interface sound effect.
    fn effect(&mut self, effect: sound::Effect) {
        self.sound.effect(&self.game, effect);
    }

    /// Per-map state after loading a map.
    fn reset(&mut self) {
        self.sound.stop();
        self.rebuild();
        self.recentre();
        self.markers = minimap::markers(&self.map.battle.scenario, &self.defs);
        self.players = minimap::players(&self.map.battle.scenario, &self.markers);
        self.player = 0;
        self.scroll = 0;
        self.entry = None;
        self.selected = None;
        self.info = None;
        self.watch.clear();
        self.preview = None;
        self.lock = None;
        self.fire_mode = None;
        self.motions.clear();
        self.ends.clear();
        self.volley.clear();
        self.group.clear();
        self.stale = false;
        self.shot = None;
        self.flights.clear();
        self.supports.clear();
        self.reaction = None;
        self.brain = None;
        self.options.hidden_units.clear();
        self.hover = None;
        self.started = self.clock.now();
    }

    fn rebuild(&mut self) {
        // In a battle the player sees only the mines they know about.
        self.options.viewer = (!matches!(self.flow.phase, Phase::Free)).then_some(skirmish::PLAYER);
        // The player's units that can do nothing more this turn are dimmed.
        self.options.dimmed_units = (0..self.map.battle.scenario.units.len())
            .filter(|&i| {
                self.map.battle.scenario.units[i].arrival == 0
                    && self.controllable(i)
                    && !matches!(self.flow.phase, Phase::Free)
                    && self.exhausted(i)
            })
            .collect();
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
        let zooms = settings::ZOOMS.map(|z| z as f32 / 100.0);
        let current = zooms
            .iter()
            .position(|&z| z >= self.zoom - 1e-3)
            .unwrap_or(zooms.len() - 1) as i32;
        let next = zooms[(current + step).clamp(0, zooms.len() as i32 - 1) as usize];
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

    /// What the right button inspects under output pixel `(x, y)`: the
    /// shown unit there, else the building, wall or road.
    fn inspect_under(&self, x: i32, y: i32) -> Option<libday_view::inspect::Target> {
        use libday_view::inspect::{Target, structure_at};
        let (mx, my) = self.to_map(x as f32, y as f32);
        let (tx, ty) = tile_at(self.map.battle.scenario.size, mx as f64, my as f64)?;
        if let Some(i) = self.unit_at(tx, ty) {
            return Some(Target::Unit(i));
        }
        if self.options.arrival.is_none() || self.options.terrain_only {
            return None;
        }
        structure_at(
            &self.map.battle.scenario,
            &self.defs,
            tx,
            ty,
            self.options.viewer,
        )
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
        // The arrows browse: the unit replaces the selection.
        self.stale = true;
        self.pick(unit);
        self.effect(sound::Effect::Select);
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
            Button::Menu
            | Button::OptionsRestart
            | Button::OptionsSave
            | Button::OptionsQuit
            | Button::OptionsBack
            | Button::OptionsUp
            | Button::OptionsDown => self.menu_enabled(button),
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
            Button::Reinforcement => self.can_reinforce(),
            Button::Automated => self.can_support(),
            Button::AutomatedUp => self.support_scroll_ok(-1),
            Button::AutomatedDown => self.support_scroll_ok(1),
            Button::NewCampaign
            | Button::LoadCampaign
            | Button::MainSkirmish
            | Button::MultiPlayer
            | Button::Intro
            | Button::Resolution
            | Button::Quit
            | Button::DifficultyLeft
            | Button::DifficultyRight
            | Button::NewGameBack
            | Button::NewGameContinue => self.front_enabled(button),
            Button::Intelligence
            | Button::Attack
            | Button::StrategicBack
            | Button::StrategicContinue => self.strategic_enabled(button),
            Button::BaseMap
            | Button::BaseBuildings
            | Button::BaseDefenses
            | Button::BaseUp
            | Button::BaseDown
            | Button::Recycle
            | Button::BaseMove
            | Button::Campaign
            | Button::BaseArmy => self.base_enabled(button),
            _ => true,
        }
    }

    fn activate(&mut self, button: Button) {
        match button {
            Button::Map | Button::DeployMap | Button::AutomatedMap => {
                self.close_supports();
                self.screen = Screen::Overview
            }
            Button::Goal | Button::DeployGoal | Button::AutomatedGoal => {
                self.close_supports();
                self.screen = Screen::Goal
            }
            Button::Back | Button::Continue => self.screen = self.home(),
            Button::Center => self.centre_on_selected(),
            Button::Left => self.cycle_selected(-1),
            Button::Right => self.cycle_selected(1),
            Button::Overwatch => self.toggle_overwatch(),
            Button::Group => self.toggle_group(),
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
            Button::Reinforcement => self.begin_reinforce(),
            Button::Automated => self.open_supports(),
            Button::AutomatedUp => self.support_scroll(-1),
            Button::AutomatedDown => self.support_scroll(1),
            Button::AutomatedBack => self.close_supports(),
            Button::SkirmishUp => self.pick_step(-1),
            Button::SkirmishDown => self.pick_step(1),
            Button::SkirmishBack => self.open_main_menu(),
            Button::SkirmishContinue => self.choose_skirmish(),
            Button::ReportContinue => self.close_report(),
            Button::TechLeft => self.tech_step(-1),
            Button::TechRight => self.tech_step(1),
            Button::TechUp => self.tech_step(-Self::TECH_ROW),
            Button::TechDown => self.tech_step(Self::TECH_ROW),
            Button::AutoLeft => self.auto_step(-1),
            Button::AutoRight => self.auto_step(1),
            Button::AutoUp => self.auto_step(-Self::AUTO_ROW),
            Button::AutoDown => self.auto_step(Self::AUTO_ROW),
            Button::TechUpgrade => self.upgrade(),
            Button::TechContinue => self.tech_continue(),
            Button::TechBack => self.tech_back(),
            Button::OverviewContinue => self.begin_deploy(),
            Button::DeployUp => self.deploy_scroll(-1),
            Button::DeployDown => self.deploy_scroll(1),
            Button::Army => self.screen = Screen::Tech { read_only: true },
            Button::DeployContinue => self.finish_deploy(),
            Button::Menu
            | Button::OptionsRestart
            | Button::OptionsSave
            | Button::OptionsQuit
            | Button::OptionsBack
            | Button::OptionsUp
            | Button::OptionsDown => self.menu_activate(button),
            Button::Yes | Button::No if matches!(self.screen, Screen::Options { .. }) => {
                self.menu_activate(button)
            }
            Button::Yes => self.start_battle(),
            Button::No => self.screen = Screen::Deploy,
            Button::NewCampaign
            | Button::LoadCampaign
            | Button::MainSkirmish
            | Button::MultiPlayer
            | Button::Intro
            | Button::Resolution
            | Button::Quit
            | Button::DifficultyLeft
            | Button::DifficultyRight
            | Button::NewGameBack
            | Button::NewGameContinue => self.front_activate(button),
            Button::Intelligence
            | Button::Attack
            | Button::StrategicBack
            | Button::StrategicContinue => self.strategic_activate(button),
            Button::IntelBack => self.close_intel(),
            Button::BaseMap
            | Button::BaseBuildings
            | Button::BaseDefenses
            | Button::BaseUp
            | Button::BaseDown
            | Button::Recycle
            | Button::BaseMove
            | Button::Campaign
            | Button::BaseArmy => self.base_activate(button),
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
        None => {
            let mut battle = Battle::start(&game, &maps[index])?;
            battle.scenario.restore(&defs);
            (maps[index].clone(), battle)
        }
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
        zoom: density * settings.zoom as f32 / 100.0,
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
        watch: Vec::new(),
        hover: None,
        selected: None,
        group: Vec::new(),
        grouping: false,
        stale: false,
        preview: None,
        motions: Vec::new(),
        ends: Vec::new(),
        volley: Vec::new(),
        lock: None,
        shot: None,
        flights: Vec::new(),
        supports: Vec::new(),
        reaction: None,
        reactions: true,
        brain: None,
        ai_mode: Some(settings.ai),
        fire_mode: None,
        anims,
        sound: sound::Sound::new(
            if args.headless {
                None
            } else {
                sdl.audio().ok()
            },
            settings.volume,
        ),
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
            .clone()
            .as_deref()
            .and_then(|p| p.parent())
            .map(|dir| dir.join("saves")),
        notice: None,
        error: None,
        trace: std::env::var_os("OPENLIBDAY_TRACE_EVENTS").is_some(),
        flow: Flow::new(),
        quit: false,
        settings,
        settings_path: options_path,
        menu: menu::Menu::default(),
        fullscreen: None,
        front: front::Front::default(),
        campaign: None,
        strategic: None,
        base_editor: None,
        army_view: 0,
    };
    if args.script.is_some() {
        // Scripted runs do not depend on who runs them.
        viewer.front.name.clear();
    }
    viewer.options.arrival = Some(0);
    viewer.reset();
    if args.scenario.is_none() && args.load.is_none() {
        viewer.open_main_menu();
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
        dirty |= viewer.battle_tick();
        let wait = if moving || viewer.flow.ai_until.is_some() {
            15
        } else if viewer.sea_animating() {
            // The sea's frame time (`frame_micros`), without drifting.
            viewer
                .strategic
                .as_ref()
                .map_or(100, |st| (st.map.frame_micros / 1000).clamp(15, 100))
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
        if let Some(on) = viewer.fullscreen.take() {
            canvas.window_mut().set_fullscreen(on)?;
            dirty = true;
        }
        // Typing goes to the NEW GAME dialog's name box.
        let typing = viewer.screen == Screen::NewGame;
        if typing != video.text_input().is_active(canvas.window()) {
            if typing {
                video.text_input().start(canvas.window());
            } else {
                video.text_input().stop(canvas.window());
            }
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
            render::render(&mut canvas, &mut gfx, &viewer)?;
            canvas.present();
            // After the frame that shows the view centred on its target.
            moves::preload_support(&mut gfx, &mut viewer);
            dirty = false;
        }
    }
    Ok(())
}
