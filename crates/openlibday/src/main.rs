//! openlibday application entry point.
//!
//! Milestone 2: a map viewer. Loads any skirmish (or other scenario) map and
//! draws terrain, roads, walls, mines, buildings, units and starting zones in
//! the original stacking order, with pan and zoom.
//!
//! Usage: `openlibday [--root DIR] [SCENARIO]`, where `SCENARIO` is a
//! scenario database name such as `s1-2-1` or `s1-2-1.mdb` (default: the first skirmish).

use std::collections::HashMap;
use std::error::Error;
use std::time::Duration;

use libday_data::{Definitions, GameData, Scenario};
use libday_view::{Art, Options, Scene, Sheet, map_pixels, sprite_origin, tile_at};
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::Keycode;
use sdl3::mouse::MouseButton;
use sdl3::pixels::{Color, PixelFormat};
use sdl3::render::{BlendMode, Canvas, FPoint, FRect, ScaleMode, Texture, TextureCreator};
use sdl3::video::{Window, WindowContext};

const ZOOMS: [f32; 9] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0];
const HELP: &str = "PgUp/PgDn map  wheel/+/- zoom  drag/arrows pan  Home recentre  \
                    G grid  D zones  U units  T terrain  [ ] reinforcements";

struct Args {
    root: Option<String>,
    scenario: Option<String>,
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut args = Args {
        root: None,
        scenario: None,
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--root" => args.root = Some(iter.next().ok_or("--root needs a directory")?),
            "-h" | "--help" => {
                println!("usage: openlibday [--root DIR] [SCENARIO]\n{HELP}");
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
    /// Scenario database names, in PgUp/PgDn order.
    maps: Vec<String>,
    index: usize,
    map: Map,
    options: Options,
    show_units: bool,
    grid: bool,
    /// Output pixels per map pixel.
    zoom: f32,
    /// Map pixel at the window's top-left corner.
    camera: (f32, f32),
    /// Output size in pixels.
    output: (f32, f32),
    /// Output pixels per window coordinate (mouse events use the latter).
    density: f32,
    drag: Option<(f32, f32)>,
    hover: Option<(i64, i64)>,
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
                self.rebuild();
                self.recentre();
                self.error = None;
                self.hover = None;
            }
            Err(err) => self.error = Some(format!("{file}: {err}")),
        }
    }

    fn rebuild(&mut self) {
        self.map.scene =
            libday_view::build(&self.map.scenario, &self.defs, &self.art, &self.options);
        for name in &self.map.scene.missing {
            eprintln!("{}: missing {name}", self.map.file);
        }
    }

    fn recentre(&mut self) {
        let (w, h) = map_pixels(self.map.scenario.size);
        self.camera = (
            w as f32 / 2.0 - self.output.0 / (2.0 * self.zoom),
            h as f32 / 2.0 - self.output.1 / (2.0 * self.zoom),
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
                for u in s
                    .units
                    .iter()
                    .filter(|u| Some(u.arrival) == self.options.arrival)
                {
                    if let Some(k) = self.defs.unit(u.kind)
                        && covers(u.x, u.y, k.width, k.height)
                    {
                        title += &format!(" - {} #{}", k.name, u.id);
                        if !u.reinforcement {
                            title += &format!(" (P{})", u.owner);
                        }
                        break;
                    }
                }
            }
        }
        if let Some(err) = &self.error {
            title += &format!("  ERROR: {err}");
        }
        title
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
            } => self.output = (w as f32, h as f32),
            Event::KeyDown {
                keycode: Some(key), ..
            } => {
                let pan = 64.0 / self.zoom * self.density;
                let centre = (self.output.0 / 2.0, self.output.1 / 2.0);
                let count = self.maps.len();
                match key {
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
                        self.rebuild();
                    }
                    Keycode::LeftBracket | Keycode::RightBracket => {
                        let step = if key == Keycode::RightBracket { 1 } else { -1 };
                        let turn =
                            (self.options.arrival.unwrap_or(0) + step).clamp(0, self.max_arrival());
                        self.show_units = true;
                        self.options.arrival = Some(turn);
                        self.rebuild();
                    }
                    _ => {}
                }
            }
            Event::MouseWheel {
                y,
                mouse_x,
                mouse_y,
                ..
            } if y != 0.0 => {
                let step = if y > 0.0 { 1 } else { -1 };
                self.zoom_step(step, (mouse_x * self.density, mouse_y * self.density));
            }
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Left | MouseButton::Right | MouseButton::Middle,
                x,
                y,
                ..
            } => self.drag = Some((x, y)),
            Event::MouseButtonUp { .. } => self.drag = None,
            Event::MouseMotion { x, y, .. } => {
                if let Some((px, py)) = self.drag {
                    let scale = self.density / self.zoom;
                    self.camera.0 -= (x - px) * scale;
                    self.camera.1 -= (y - py) * scale;
                    self.drag = Some((x, y));
                }
                let (mx, my) = self.to_map(x * self.density, y * self.density);
                self.hover = tile_at(self.map.scenario.size, mx as f64, my as f64);
            }
            _ => {}
        }
        true
    }
}

type Textures<'r> = HashMap<(Sheet, String), Option<Texture<'r>>>;

fn texture<'r, 't>(
    cache: &'t mut Textures<'r>,
    creator: &'r TextureCreator<WindowContext>,
    art: &Art,
    sheet: Sheet,
    name: &str,
) -> Option<&'t Texture<'r>> {
    cache
        .entry((sheet, name.to_owned()))
        .or_insert_with(|| {
            let (w, h, rgba) = art.rgba(sheet, name)?;
            let mut texture = creator
                .create_texture_static(PixelFormat::RGBA32, w as u32, h as u32)
                .ok()?;
            texture.update(None, &rgba, w * 4).ok()?;
            texture.set_blend_mode(BlendMode::Blend);
            texture.set_scale_mode(ScaleMode::Nearest);
            Some(texture)
        })
        .as_ref()
}

fn draw<'r>(
    canvas: &mut Canvas<Window>,
    creator: &'r TextureCreator<WindowContext>,
    cache: &mut Textures<'r>,
    viewer: &Viewer,
) -> Result<(), Box<dyn Error>> {
    canvas.set_draw_color(Color::RGB(0, 0, 0));
    canvas.clear();
    let (zoom, (cx, cy), (ow, oh)) = (viewer.zoom, viewer.camera, viewer.output);
    let size = viewer.map.scenario.size;
    let screen = |mx: i32, my: i32| ((mx as f32 - cx) * zoom, (my as f32 - cy) * zoom);
    for item in &viewer.map.scene.items {
        let (left, top) = sprite_origin(size, item.x, item.y);
        let (x0, y0) = screen(left, top);
        let (x1, y1) = screen(left + 54, top + 56);
        if x1 < 0.0 || y1 < 0.0 || x0 > ow || y0 > oh {
            continue;
        }
        if let Some(texture) = texture(cache, creator, &viewer.art, item.sheet, &item.sprite) {
            // Whole output pixels, so neighbouring tiles never leave seams.
            let (x0, y0, x1, y1) = (x0.floor(), y0.floor(), x1.floor(), y1.floor());
            canvas.copy(texture, None, Some(FRect::new(x0, y0, x1 - x0, y1 - y0)))?;
        }
    }
    if viewer.grid {
        canvas.set_draw_color(Color::RGB(0x60, 0x70, 0x80));
        for ty in 0..size as i64 {
            for tx in 0..size as i64 {
                let corners = libday_view::compose::diamond(size, tx, ty);
                let points: Vec<FPoint> = corners
                    .iter()
                    .chain(corners.first())
                    .map(|&(px, py)| {
                        let (x, y) = screen(px, py);
                        FPoint::new(x, y)
                    })
                    .collect();
                let outside = |p: &FPoint| p.x < 0.0 || p.y < 0.0 || p.x > ow || p.y > oh;
                if !points.iter().all(outside) {
                    canvas.draw_lines(points.as_slice())?;
                }
            }
        }
    }
    canvas.present();
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let game = match args.root {
        Some(root) => GameData::open(root)?,
        None => GameData::from_env()?,
    };
    let defs = game.definitions()?;
    let art = Art::load(&game, &defs)?;
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
    let sdl = sdl3::init()?;
    let video = sdl.video()?;
    let window = video
        .window("openlibday", 1280, 800)
        .position_centered()
        .resizable()
        .high_pixel_density()
        .build()?;
    let density = window.pixel_density().max(1.0);
    let (pw, ph) = window.size_in_pixels();
    let mut canvas = window.into_canvas();
    let creator = canvas.texture_creator();
    let mut cache: Textures = HashMap::new();

    let mut viewer = Viewer {
        game,
        defs,
        art,
        index,
        map: Map {
            file: maps[index].clone(),
            scenario,
            scene: Scene::default(),
        },
        maps,
        options: Options::default(),
        show_units: true,
        grid: false,
        zoom: density,
        camera: (0.0, 0.0),
        output: (pw as f32, ph as f32),
        density,
        drag: None,
        hover: None,
        error: None,
        trace: std::env::var_os("OPENLIBDAY_TRACE_EVENTS").is_some(),
    };
    viewer.rebuild();
    viewer.recentre();

    let mut events = sdl.event_pump()?;
    let mut title = String::new();
    let mut dirty = true;
    loop {
        let mut running = true;
        if let Some(event) = events.wait_event_timeout(Duration::from_millis(500)) {
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
        let next = viewer.title();
        if next != title {
            canvas.window_mut().set_title(&next)?;
            title = next;
        }
        if dirty {
            draw(&mut canvas, &creator, &mut cache, &viewer)?;
            dirty = false;
        }
    }
    Ok(())
}
