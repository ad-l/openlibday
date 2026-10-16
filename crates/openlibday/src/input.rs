//! Keyboard and mouse handling.

use libday_view::tile_at;
use libday_view::ui::{AUTO_CELLS, LIST_ROWS, SKIRMISH_ROWS, Screen, TECH_CELLS};
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::Keycode;
use sdl3::mouse::MouseButton;

use crate::skirmish::{Flow, Phase};
use crate::{CLICK_SLOP, Viewer, settings};

impl Viewer {
    /// Keys of the full-window dialogs; `None` when not on one.
    fn dialog_key(&mut self, key: Keycode) -> Option<bool> {
        if let Some(running) = self.front_key(key) {
            return Some(running);
        }
        match (self.screen, key) {
            (Screen::Skirmishes, Keycode::Escape) => self.open_main_menu(),
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
                Keycode::Return | Keycode::KpEnter if !read_only => self.tech_continue(),
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
            (Screen::Report, Keycode::Return | Keycode::KpEnter | Keycode::Escape) => {
                self.close_report()
            }
            (Screen::Options { ask }, _) => self.menu_key(ask, key),
            (Screen::Strategic { .. }, _) => self.strategic_key(key),
            (Screen::Intelligence, Keycode::Return | Keycode::KpEnter | Keycode::Escape) => {
                self.close_intel()
            }
            (Screen::Intelligence, _) => {}
            (Screen::BaseEditor, Keycode::Escape) => self.base_escape(),
            (Screen::Army, _) => match key {
                Keycode::Left => self.tech_step(-1),
                Keycode::Right => self.tech_step(1),
                Keycode::Up => self.tech_step(-Self::TECH_ROW),
                Keycode::Down => self.tech_step(Self::TECH_ROW),
                Keycode::Space | Keycode::U => self.upgrade(),
                Keycode::Return | Keycode::KpEnter | Keycode::Escape => self.tech_back(),
                _ => {}
            },
            (Screen::Skirmishes | Screen::Briefing | Screen::Confirm | Screen::Report, _) => {}
            _ => return None,
        }
        Some(true)
    }

    pub(crate) fn key(&mut self, key: Keycode) -> bool {
        if let Some(running) = self.dialog_key(key) {
            return running;
        }
        let pan = self.settings.scroll.step() / self.zoom * self.density;
        let v = self.layout().viewport();
        let centre = (v.w as f32 / 2.0, v.h as f32 / 2.0);
        let count = self.maps.len();
        let free = self.flow.phase == Phase::Free;
        let home = self.home();
        match key {
            Keycode::Escape if self.screen == Screen::Automated => self.close_supports(),
            Keycode::Escape if self.screen != home => self.screen = home,
            Keycode::Escape if self.flow.reinforcing => self.finish_deploy(),
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
            Keycode::PageDown | Keycode::PageUp if self.campaign.is_none() => {
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

    pub(crate) fn mouse_down(&mut self, button: MouseButton, x: f32, y: f32) {
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
            Screen::Goal | Screen::Confirm | Screen::Report | Screen::MainMenu => {}
            Screen::NewGame if left => self.front_click(ox, oy),
            Screen::NewGame => {}
            Screen::Strategic { .. } if left => self.strategic_click(ox, oy),
            Screen::Strategic { .. } | Screen::Intelligence => {}
            Screen::Options { ask: None } if left => self.menu_click(ox, oy),
            Screen::Options { .. } => {}
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
            Screen::Tech { .. } | Screen::Army if left => {
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
            Screen::Skirmishes | Screen::Tech { .. } | Screen::Army => {}
            Screen::Overview | Screen::Briefing if left => {
                let entries = self.entries().len();
                if let Some(row) = (0..LIST_ROWS).find(|&i| layout.list_row(i).contains(ox, oy)) {
                    let index = self.scroll + row;
                    if index < entries {
                        self.entry = (self.entry != Some(index)).then_some(index);
                    }
                } else if let Some((tx, ty)) = self.picture_tile(ox, oy) {
                    self.centre_on_tile(tx, ty);
                    if self.screen == Screen::Briefing {
                        self.begin_deploy();
                    } else {
                        self.screen = self.home();
                    }
                }
            }
            Screen::Overview | Screen::Briefing => {}
            Screen::BaseEditor
                if left
                    && let Some(row) = (0..libday_view::ui::BASE_ROWS)
                        .find(|&i| layout.base_row(i).contains(ox, oy)) =>
            {
                self.base_select_row(row)
            }
            Screen::Battle
            | Screen::Deploy
            | Screen::AiTurn
            | Screen::Automated
            | Screen::BaseEditor => {
                let row = matches!(self.screen, Screen::Deploy | Screen::Automated)
                    .then(|| {
                        (0..libday_view::ui::DEPLOY_ROWS)
                            .find(|&i| layout.deploy_row(i).contains(ox, oy))
                    })
                    .flatten();
                if left && let Some(row) = row {
                    if self.screen == Screen::Automated {
                        self.support_select(self.flow.support_scroll + row);
                    } else {
                        let scroll = self.flow.setup.as_ref().map_or(0, |s| s.scroll);
                        self.deploy_select(scroll + row);
                    }
                } else if left && let Some((tx, ty)) = self.picture_tile(ox, oy) {
                    self.minimap_drag = true;
                    self.centre_on_tile(tx, ty);
                } else if !layout.on_panel(ox, oy) {
                    if button == MouseButton::Right
                        && let Some(target) = self.inspect_under(ox, oy)
                    {
                        match target.unit() {
                            Some(i) => self.inspect(i),
                            None => self.info = Some(target),
                        }
                        return;
                    }
                    self.drag = Some((x, y));
                    self.press = left.then_some((x, y));
                }
            }
        }
    }

    pub(crate) fn mouse_up(&mut self, x: f32, y: f32) {
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
                Some((tx, ty)) if self.screen == Screen::Automated => self.support_click(tx, ty),
                Some((tx, ty)) if self.screen == Screen::BaseEditor => self.base_click(tx, ty),
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
        self.watch.clear();
    }

    /// Shows unit `i`'s info while the right button is held and, for an
    /// enemy ready to fire on overwatch, the tiles it covers, joined for an
    /// anti-aircraft unit by those over which it fires at automated units
    /// (conveniences the original lacks).
    pub(crate) fn inspect(&mut self, i: usize) {
        self.info = Some(libday_view::inspect::Target::Unit(i));
        let scenario = &self.map.battle.scenario;
        let enemy = scenario
            .units
            .get(i)
            .is_some_and(|u| self.options.viewer.is_none_or(|v| u.owner != v));
        let mut tiles = libday_view::support::anti_air_zone(scenario, &self.defs, i);
        if enemy {
            tiles.extend(libday_view::overwatch::zone(scenario, &self.defs, i));
        }
        self.watch = libday_view::scene::zone_sprites(&tiles, 2)
            .into_iter()
            .map(|(at, name)| (at, format!("{}{name}", crate::render::RED)))
            .collect();
    }

    pub(crate) fn mouse_motion(&mut self, x: f32, y: f32) {
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
            Screen::Battle
                | Screen::Deploy
                | Screen::AiTurn
                | Screen::Automated
                | Screen::BaseEditor
        ) && !self.layout().on_panel(ox, oy);
        let was = self.hover;
        self.hover = on_map
            .then(|| {
                let (mx, my) = self.to_map(ox as f32, oy as f32);
                tile_at(self.map.battle.scenario.size, mx as f64, my as f64)
            })
            .flatten();
        if self.screen == Screen::BaseEditor && self.hover != was {
            self.base_refresh();
        }
    }

    /// Handles one event; returns `false` to quit.
    pub(crate) fn handle(&mut self, event: Event) -> bool {
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
                    Screen::Battle
                    | Screen::Deploy
                    | Screen::AiTurn
                    | Screen::Automated
                    | Screen::BaseEditor
                        if !self.layout().on_panel(ox, oy) =>
                    {
                        self.zoom_step(step, (ox as f32, oy as f32));
                    }
                    Screen::BaseEditor => self.base_scroll(-step as isize),
                    Screen::Deploy => self.deploy_scroll(-step as isize),
                    Screen::Automated => self.support_scroll(-step as isize),
                    Screen::Options { ask: None } => self.menu_scroll(-step as isize),
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
            Event::TextInput { text, .. } => self.text_input(&text),
            _ => {}
        }
        !self.quit
    }
}
