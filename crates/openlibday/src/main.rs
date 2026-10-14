//! openlibday application entry point.
//!
//! A map viewer with the battle interface: loads any skirmish (or other
//! scenario) map and draws terrain, roads, walls, mines, buildings, units and
//! starting zones in the original stacking order, with pan and zoom, inside
//! the original sidebar and bottom bar. The minimap, the MAP overview and the
//! GOAL dialog work; the other buttons are not wired to game logic yet.
//!
//! Usage: `openlibday [--root DIR] [SCENARIO]`, where `SCENARIO` is a
//! scenario database name such as `s1-2-1` or `s1-2-1.mdb` (default: the first skirmish).

mod hud;
mod settings;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use libday_data::{Definitions, Dir, GameData, Scenario};
use libday_formats::Dbi;
use libday_view::minimap::{self, Entry, Marker};
use libday_view::text::Fonts;
use libday_view::ui::{Button, LIST_ROWS, Screen};
use libday_view::{Art, Options, Scene, Sheet, map_pixels, sprite_origin, tile_at};
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::Keycode;
use sdl3::mouse::MouseButton;
use sdl3::pixels::Color;
use sdl3::render::{Canvas, FPoint, FRect, Texture};
use sdl3::video::Window;

use hud::{Gfx, Result};

const ZOOMS: [f32; 9] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0];
const HELP: &str = "PgUp/PgDn map  wheel/+/- zoom  drag/arrows pan  Home recentre  \
                    G grid  D zones  U units  T terrain  [ ] reinforcements  \
                    M map overview  O goal  I interface scale  Esc close/quit";
/// Window-coordinate movement below which a press counts as a click.
const CLICK_SLOP: f32 = 4.0;

struct Args {
    root: Option<String>,
    scenario: Option<String>,
    options: Option<PathBuf>,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        root: None,
        scenario: None,
        options: None,
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--root" => args.root = Some(iter.next().ok_or("--root needs a directory")?),
            "--options" => args.options = Some(iter.next().ok_or("--options needs a file")?.into()),
            "-h" | "--help" => {
                let file = settings::default_path()
                    .map_or_else(|| settings::FILE_NAME.into(), |p| p.display().to_string());
                println!(
                    "usage: openlibday [--root DIR] [--options FILE] [SCENARIO]\n\
                     options file: {file}\n{HELP}"
                );
                std::process::exit(0);
            }
            // M0 compatibility: a bare directory is the install root.
            _ if std::path::Path::new(&arg).is_dir() => args.root = Some(arg),
            _ => args.scenario = Some(arg),
        }
    }
    Ok(args)
}

/// The loaded map and its draw list.
struct Map {
    file: String,
    scenario: Scenario,
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
    hover: Option<(i64, i64)>,
    /// Index into `map.scenario.units`.
    selected: Option<usize>,
    overwatch: bool,
    auto_fire: bool,
    objectives: bool,
    /// Units and buildings on the minimap; owners with an overview list.
    markers: Vec<Marker>,
    players: Vec<i64>,
    /// Overview: shown player slot, list scroll and selected (blinking) entry.
    player: usize,
    scroll: usize,
    entry: Option<usize>,
    started: Instant,
    error: Option<String>,
    /// Log events to stderr (`OPENLIBDAY_TRACE_EVENTS`).
    trace: bool,
}

impl Viewer {
    fn load(&mut self, index: usize) {
        let file = self.maps[index].clone();
        match self.game.load_scenario(&file) {
            Ok(scenario) => {
                self.index = index;
                self.map = Map {
                    file,
                    scene: Scene::default(),
                    scenario,
                };
                self.options.arrival = self.show_units.then_some(0);
                self.reset();
                self.error = None;
            }
            Err(err) => self.error = Some(format!("{file}: {err}")),
        }
    }

    /// Per-map state after loading a map.
    fn reset(&mut self) {
        self.rebuild();
        self.recentre();
        self.markers = minimap::markers(&self.map.scenario, &self.defs);
        self.players = minimap::players(&self.map.scenario, &self.markers);
        self.player = 0;
        self.scroll = 0;
        self.entry = None;
        self.selected = None;
        self.hover = None;
        self.started = Instant::now();
    }

    fn rebuild(&mut self) {
        self.map.scene =
            libday_view::build(&self.map.scenario, &self.defs, &self.art, &self.options);
        self.generation += 1;
        for name in &self.map.scene.missing {
            eprintln!("{}: missing {name}", self.map.file);
        }
    }

    fn recentre(&mut self) {
        let (w, h) = map_pixels(self.map.scenario.size);
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
        let units = &self.map.scenario.units;
        units.iter().map(|u| u.arrival).max().unwrap_or(0)
    }

    /// The shown player's overview list.
    fn entries(&self) -> Vec<Entry> {
        let owner = self.players.get(self.player).copied().unwrap_or(0);
        minimap::entries(&self.markers, &self.defs, owner)
    }

    /// Unit on tile `(x, y)` among the shown units.
    fn unit_at(&self, x: i64, y: i64) -> Option<usize> {
        let arrival = self.options.arrival?;
        if self.options.terrain_only {
            return None;
        }
        self.map.scenario.units.iter().position(|u| {
            let (w, h) = self
                .defs
                .unit(u.kind)
                .map_or((1, 1), |k| (k.width, k.height));
            u.arrival == arrival && (u.x..u.x + w).contains(&x) && (u.y..u.y + h).contains(&y)
        })
    }

    /// Name, portrait image and the AP/AR lines of the selected unit.
    fn selected_unit(&self) -> Option<(String, String, String, String)> {
        let unit = self.map.scenario.units.get(self.selected?)?;
        let kind = self
            .defs
            .units
            .get(&(unit.kind, unit.tech))
            .or_else(|| self.defs.unit(unit.kind))?;
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
        let Some(unit) = self.selected.and_then(|i| self.map.scenario.units.get(i)) else {
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

    /// Whether a button reacts to clicks (otherwise drawn greyed out).
    fn enabled(&self, button: Button) -> bool {
        match button {
            Button::Chat => false,
            Button::Overwatch | Button::AutoFire => self.selected.is_some(),
            Button::Up => self.scroll > 0,
            Button::Down => self.scroll + LIST_ROWS < self.entries().len(),
            Button::Menu => self.screen != Screen::Goal,
            _ => true,
        }
    }

    fn activate(&mut self, button: Button) {
        match button {
            Button::Map => self.screen = Screen::Overview,
            Button::Goal => self.screen = Screen::Goal,
            Button::Back | Button::Continue => self.screen = Screen::Battle,
            Button::Center => self.centre_on_selected(),
            Button::Overwatch => self.overwatch = !self.overwatch,
            Button::AutoFire => self.auto_fire = !self.auto_fire,
            Button::Objectives => self.objectives = !self.objectives,
            Button::Swatch(slot) => {
                self.player = slot;
                self.scroll = 0;
                self.entry = None;
            }
            Button::Up => self.scroll = self.scroll.saturating_sub(1),
            Button::Down => self.scroll += 1,
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
        let info = &self.map.scenario.info;
        let size = self.map.scenario.size;
        let name = if info.title.is_empty() {
            "untitled"
        } else {
            &info.title
        };
        let mut title = format!(
            "openlibday - {name} ({}, {size}x{size}, {}/{})  zoom {}%",
            self.map.file,
            self.index + 1,
            self.maps.len(),
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
            && let Some(cell) = self.map.scenario.cell(x, y)
        {
            title += &format!("  [{x},{y}] {}", cell.terrain);
            let covers = |ox: i64, oy: i64, w: i64, h: i64| {
                (ox..ox + w).contains(&x) && (oy..oy + h).contains(&y)
            };
            let s = &self.map.scenario;
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
        if let Some(err) = &self.error {
            title += &format!("  ERROR: {err}");
        }
        title
    }

    fn key(&mut self, key: Keycode) -> bool {
        let pan = 64.0 / self.zoom * self.density;
        let v = self.layout().viewport();
        let centre = (v.w as f32 / 2.0, v.h as f32 / 2.0);
        let count = self.maps.len();
        match key {
            Keycode::Escape if self.screen != Screen::Battle => self.screen = Screen::Battle,
            Keycode::Escape => return false,
            Keycode::Left => self.camera.0 -= pan,
            Keycode::Right => self.camera.0 += pan,
            Keycode::Up => self.camera.1 -= pan,
            Keycode::Down => self.camera.1 += pan,
            Keycode::Home => self.recentre(),
            Keycode::Equals | Keycode::Plus | Keycode::KpPlus => self.zoom_step(1, centre),
            Keycode::Minus | Keycode::KpMinus => self.zoom_step(-1, centre),
            Keycode::PageDown => self.load((self.index + 1) % count),
            Keycode::PageUp => self.load((self.index + count - 1) % count),
            Keycode::G => self.grid = !self.grid,
            Keycode::M => {
                self.screen = match self.screen {
                    Screen::Overview => Screen::Battle,
                    _ => Screen::Overview,
                }
            }
            Keycode::O => {
                self.screen = match self.screen {
                    Screen::Goal => Screen::Battle,
                    _ => Screen::Goal,
                }
            }
            Keycode::I => {
                let max = (settings::MAX_SCALE as f32 * self.density).round() as i32;
                // Start from the scale in use, which may have been fitted down.
                self.set_scale(self.layout().scale % max + 1)
            }
            Keycode::D => {
                self.options.deployment = !self.options.deployment;
                self.rebuild();
            }
            Keycode::T => {
                self.options.terrain_only = !self.options.terrain_only;
                self.rebuild();
            }
            Keycode::U => {
                self.show_units = !self.show_units;
                self.options.arrival = self.show_units.then_some(0);
                self.selected = None;
                self.rebuild();
            }
            Keycode::LeftBracket | Keycode::RightBracket => {
                let step = if key == Keycode::RightBracket { 1 } else { -1 };
                let turn = (self.options.arrival.unwrap_or(0) + step).clamp(0, self.max_arrival());
                self.show_units = true;
                self.options.arrival = Some(turn);
                self.selected = None;
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
            Screen::Goal => {}
            Screen::Overview if left => {
                let entries = self.entries().len();
                if let Some(row) = (0..LIST_ROWS).find(|&i| layout.list_row(i).contains(ox, oy)) {
                    let index = self.scroll + row;
                    if index < entries {
                        self.entry = (self.entry != Some(index)).then_some(index);
                    }
                } else if let Some((tx, ty)) = self.picture_tile(ox, oy) {
                    self.centre_on_tile(tx, ty);
                    self.screen = Screen::Battle;
                }
            }
            Screen::Overview => {}
            Screen::Battle => {
                if left && let Some((tx, ty)) = self.picture_tile(ox, oy) {
                    self.minimap_drag = true;
                    self.centre_on_tile(tx, ty);
                } else if !layout.on_panel(ox, oy) {
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
            self.selected = tile_at(self.map.scenario.size, mx as f64, my as f64)
                .and_then(|(tx, ty)| self.unit_at(tx, ty));
        }
        self.drag = None;
        self.minimap_drag = false;
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
        let on_map = self.screen != Screen::Overview && !self.layout().on_panel(ox, oy);
        self.hover = on_map
            .then(|| {
                let (mx, my) = self.to_map(ox as f32, oy as f32);
                tile_at(self.map.scenario.size, mx as f64, my as f64)
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
                    Screen::Battle if !self.layout().on_panel(ox, oy) => {
                        self.zoom_step(step, (ox as f32, oy as f32));
                    }
                    Screen::Overview => {
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
        true
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
    let size = viewer.map.scenario.size;
    let screen = |mx: i32, my: i32| ((mx as f32 - cx) * zoom, (my as f32 - cy) * zoom);
    let point = |(px, py): (i32, i32)| {
        let (x, y) = screen(px, py);
        FPoint::new(x, y)
    };
    let outside = |p: &FPoint| p.x < 0.0 || p.y < 0.0 || p.x > ow || p.y > oh;
    for item in &viewer.map.scene.items {
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
    if let Some(unit) = viewer
        .selected
        .and_then(|i| viewer.map.scenario.units.get(i))
    {
        let (w, h) = viewer
            .defs
            .unit(unit.kind)
            .map_or((1, 1), |k| (k.width, k.height));
        canvas.set_draw_color(Color::RGB(255, 255, 0));
        for dy in 0..h {
            for dx in 0..w {
                diamond(canvas, unit.x + dx, unit.y + dy)?;
            }
        }
    }
    canvas.set_clip_rect(None);
    Ok(())
}

fn draw(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    canvas.set_draw_color(Color::RGB(0, 0, 0));
    canvas.clear();
    if viewer.screen != Screen::Overview {
        draw_map(canvas, gfx, viewer)?;
    }
    hud::draw(canvas, gfx, viewer)?;
    canvas.present();
    Ok(())
}

fn main() -> Result {
    let args = parse_args()?;
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
    let index = match &args.scenario {
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
    let scenario = game.load_scenario(&maps[index])?;

    // SDL's default `french_numbers` maps AZERTY's number row to digits, which
    // would turn the '-' (zoom out) key into '6'.
    sdl3::hint::set("SDL_KEYCODE_OPTIONS", "hide_numpad,latin_letters");
    // The click that focuses the window also acts.
    sdl3::hint::set("SDL_MOUSE_FOCUS_CLICKTHROUGH", "1");
    let sdl = sdl3::init()?;
    let video = sdl.video()?;
    let options_path = args.options.or_else(settings::default_path);
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
    let mut builder = video.window(
        "openlibday",
        fit(settings.width, usable.0),
        fit(settings.height, usable.1),
    );
    builder.position_centered().resizable().high_pixel_density();
    if settings.fullscreen {
        builder.fullscreen();
    }
    let mut window = builder.build()?;
    window.set_minimum_size(640, 520)?;
    let density = window.pixel_density().max(1.0);
    let (pw, ph) = window.size_in_pixels();
    let mut canvas = window.into_canvas();
    let creator = canvas.texture_creator();
    let mut gfx = Gfx::new(&creator, &ui);
    drop(ui);

    let mut viewer = Viewer {
        game,
        defs,
        art,
        fonts: Fonts::new(),
        listbox,
        index,
        map: Map {
            file: maps[index].clone(),
            scenario,
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
        hover: None,
        selected: None,
        overwatch: true,
        auto_fire: true,
        objectives: true,
        markers: Vec::new(),
        players: Vec::new(),
        player: 0,
        scroll: 0,
        entry: None,
        started: Instant::now(),
        error: None,
        trace: std::env::var_os("OPENLIBDAY_TRACE_EVENTS").is_some(),
    };
    viewer.options.arrival = Some(0);
    viewer.reset();

    let mut events = sdl.event_pump()?;
    let mut title = String::new();
    let mut dirty = true;
    let mut second = 0;
    loop {
        let mut running = true;
        let wait = if viewer.animating() { 100 } else { 250 };
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
        let now = viewer.started.elapsed().as_secs();
        if now != second || viewer.animating() {
            second = now;
            dirty = true;
        }
        let next = viewer.title();
        if next != title {
            canvas.window_mut().set_title(&next)?;
            title = next;
        }
        if dirty {
            draw(&mut canvas, &mut gfx, &viewer)?;
            dirty = false;
        }
    }
    Ok(())
}
