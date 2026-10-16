//! Drawing of the battle interface: sidebar, bottom bar, minimap, MAP
//! overview and GOAL dialog. Layout and artwork come from `libday_view::ui`.

use std::collections::HashMap;
use std::error::Error;

use libday_formats::Dbi;
use libday_view::minimap::{self, Marker};
use libday_view::text::Fonts;
use libday_view::ui::{
    self, BOTTOM_H, BOTTOM_LEFT, BOTTOM_RIGHT, BOTTOM_TILE, BOTTOM_W, Button, Field, LIST_ROWS,
    Layout, Rect, SIDEBAR_H, SIDEBAR_TILE, SIDEBAR_W, Screen, State, UiArt, frame, unit_info,
};
use libday_view::{Art, Image, Scene, Sheet, map_point, tile_coords};
use sdl3::pixels::{Color, PixelFormat};
use sdl3::render::{BlendMode, Canvas, FPoint, FRect, ScaleMode, Texture, TextureCreator};
use sdl3::video::{Window, WindowContext};

use crate::Viewer;

mod base;
mod front;
mod options;
mod saves;
mod setup;
mod strategic;

/// Interface text size and line spacing, in original pixels.
const TEXT_PX: f32 = 11.5;
const LINE: i32 = 12;
const GOAL_LINE: i32 = 14;
const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [0, 0, 0];
/// `LBULB` bitmaps of `LIBDAY.EXE` (`0x45a029`): bright, mid, dark, mid.
const BULB: [u32; 4] = [305, 306, 307, 308];
/// The bulb's frame period (`0x459530` timer) and its offset from a
/// building's centre, in original pixels.
const BULB_MS: u128 = 175;
const BULB_OFFSET: (i32, i32) = (2, 8);

/// ARMY's snapshot: opening serial and output size.
type GreyKey = (u64, (u32, u32));

pub type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

pub fn texture<'r>(
    creator: &'r TextureCreator<WindowContext>,
    image: &Image,
) -> Option<Texture<'r>> {
    let mut texture = creator
        .create_texture_static(PixelFormat::RGBA32, image.width as u32, image.height as u32)
        .ok()?;
    texture.update(None, &image.rgba, image.width * 4).ok()?;
    texture.set_blend_mode(BlendMode::Blend);
    texture.set_scale_mode(ScaleMode::Nearest);
    Some(texture)
}

fn frect(r: Rect) -> FRect {
    FRect::new(r.x as f32, r.y as f32, r.w as f32, r.h as f32)
}

fn colour([r, g, b]: [u8; 3]) -> Color {
    Color::RGB(r, g, b)
}

/// A decoded `ANMSUNIT.FF` animation: each frame's opaque box (if any) and
/// its offset in the full `size`.
pub struct Anim<'r> {
    pub size: (i32, i32),
    pub frames: Vec<Option<(Texture<'r>, (i32, i32))>>,
}

/// Cached textures; all borrow the canvas' texture creator.
pub struct Gfx<'r> {
    creator: &'r TextureCreator<WindowContext>,
    pub sprites: HashMap<(Sheet, String), Option<Texture<'r>>>,
    /// Animation frames by `ANMSUNIT.FF` member.
    pub anims: HashMap<String, Option<Anim<'r>>>,
    ui: HashMap<u16, Texture<'r>>,
    text: HashMap<(String, u32, [u8; 3]), Option<Texture<'r>>>,
    portraits: HashMap<String, Option<Texture<'r>>>,
    /// Capability icons by `LIBDAY.EXE` bitmap id.
    icons: HashMap<u32, Texture<'r>>,
    minimap: Option<((u64, i32), Texture<'r>)>,
    overview: Option<((u64, usize), Texture<'r>)>,
    /// Continent overview: the (continent, sea frame) shown, and province
    /// overlays by (continent, province, look).
    sea: Option<((usize, usize), Texture<'r>)>,
    provinces: HashMap<(usize, i64, u8), Texture<'r>>,
    /// ARMY's greyed map snapshot, by opening serial and output size.
    grey: Option<(GreyKey, Texture<'r>)>,
    /// INTELLIGENCE map preview, by opening serial.
    intel: Option<(u64, Texture<'r>)>,
    /// Saved games' pictures, by file and creation time.
    saves: HashMap<(std::path::PathBuf, i64), Texture<'r>>,
    /// The cutscene frame shown, by (play serial, frame count).
    pub video: Option<((u64, usize), Texture<'r>)>,
}

impl<'r> Gfx<'r> {
    pub fn new(creator: &'r TextureCreator<WindowContext>, ui: &UiArt) -> Self {
        let ui = ui
            .frames()
            .filter_map(|(f, image)| Some((f, texture(creator, image)?)))
            .collect();
        Self {
            creator,
            sprites: HashMap::new(),
            anims: HashMap::new(),
            ui,
            text: HashMap::new(),
            portraits: HashMap::new(),
            icons: HashMap::new(),
            minimap: None,
            overview: None,
            sea: None,
            provinces: HashMap::new(),
            grey: None,
            intel: None,
            saves: HashMap::new(),
            video: None,
        }
    }

    /// Loads the unit-info capability icons and the overview's blinking
    /// technology bulb (`LBULB`) from the game executable.
    pub fn load_icons(&mut self, game: &libday_data::GameData) {
        for id in unit_info::ALL.into_iter().chain(BULB) {
            match game.exe_bitmap(id) {
                Ok(Some(b)) => {
                    let mut image = Image {
                        width: b.width,
                        height: b.height,
                        rgba: b.rgba,
                    };
                    if BULB.contains(&id) {
                        // Drawn with black transparent.
                        for p in image.rgba.as_chunks_mut::<4>().0 {
                            if p[..3] == [0, 0, 0] {
                                p[3] = 0;
                            }
                        }
                    }
                    if let Some(t) = texture(self.creator, &image) {
                        self.icons.insert(id, t);
                    }
                }
                Ok(None) => eprintln!("LIBDAY.EXE: missing bitmap {id}"),
                Err(e) => eprintln!("LIBDAY.EXE: bitmap {id}: {e}"),
            }
        }
    }

    pub fn creator(&self) -> &'r TextureCreator<WindowContext> {
        self.creator
    }

    /// Draws all of interface frame `f` at `(x, y)` (output pixels).
    fn frame(&self, canvas: &mut Canvas<Window>, f: u16, x: i32, y: i32, s: i32) -> Result {
        self.frame_part(canvas, f, Rect::new(0, 0, i32::MAX, i32::MAX), x, y, s)
    }

    /// Draws the `src` part (original pixels, clamped) of frame `f`.
    fn frame_part(
        &self,
        canvas: &mut Canvas<Window>,
        f: u16,
        src: Rect,
        x: i32,
        y: i32,
        s: i32,
    ) -> Result {
        let Some(texture) = self.ui.get(&f) else {
            return Ok(());
        };
        let (tw, th) = (texture.width() as i32, texture.height() as i32);
        let w = src.w.min(tw - src.x);
        let h = src.h.min(th - src.y);
        if w <= 0 || h <= 0 {
            return Ok(());
        }
        let src = Rect::new(src.x, src.y, w, h);
        canvas.copy(texture, frect(src), frect(Rect::new(x, y, w * s, h * s)))?;
        Ok(())
    }

    fn button(
        &self,
        canvas: &mut Canvas<Window>,
        viewer: &Viewer,
        layout: &Layout,
        button: Button,
        state: State,
        on: bool,
    ) -> Result {
        let state = if viewer.enabled(button) {
            state
        } else {
            State::Disabled
        };
        let state = viewer.button_state(button, state);
        let r = layout.button(button);
        let f = match button {
            Button::Swatch(slot) => {
                let owner = viewer.players.get(slot).copied().unwrap_or(0);
                // The magenta swatch is the key colour, hence transparent.
                if minimap::owner_colour(owner) == [255, 0, 255] {
                    fill(canvas, frect(r), [255, 0, 255])?;
                }
                // The shown player's swatch stays pressed.
                let state = if slot == viewer.player {
                    State::Pressed
                } else {
                    state
                };
                minimap::swatch_frame(owner) + state as u16
            }
            Button::SavesBack | Button::SavesOk
                if matches!(viewer.screen, Screen::Saves { saving: true }) =>
            {
                // SAVE GAME's buttons follow LOAD GAME's.
                button.frame_for(state, on) + (frame::SAVE_BACK - frame::LOAD_BACK)
            }
            _ => button.frame_for(state, on),
        };
        self.frame(canvas, f, r.x, r.y, layout.scale)
    }

    /// Draws one line of text with its top-left at `(x, y)`; returns its width.
    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        canvas: &mut Canvas<Window>,
        fonts: &Fonts,
        text: &str,
        px: f32,
        rgb: [u8; 3],
        x: i32,
        y: i32,
    ) -> Result<i32> {
        if text.is_empty() {
            return Ok(0);
        }
        let creator = self.creator;
        let texture = self
            .text
            .entry((text.to_owned(), (px * 10.0) as u32, rgb))
            .or_insert_with(|| texture(creator, &fonts.render(text, px, false, rgb)));
        let Some(texture) = texture else {
            return Ok(0);
        };
        let (w, h) = (texture.width() as f32, texture.height() as f32);
        canvas.copy(texture, None, FRect::new(x as f32, y as f32, w, h))?;
        Ok(w as i32)
    }

    /// Text centred (`align` 0.5) or right-aligned (1.0) in `r`, vertically centred.
    #[allow(clippy::too_many_arguments)]
    fn text_in(
        &mut self,
        canvas: &mut Canvas<Window>,
        fonts: &Fonts,
        text: &str,
        px: f32,
        rgb: [u8; 3],
        r: Rect,
        align: f32,
        pad: i32,
    ) -> Result {
        let w = fonts.measure(text, px, false).ceil() as i32;
        let h = fonts.line_height(px);
        let x = r.x + pad + ((r.w - 2 * pad - w) as f32 * align) as i32;
        self.text(canvas, fonts, text, px, rgb, x, r.y + (r.h - h) / 2)?;
        Ok(())
    }
}

/// Fills `r` with `c`.
fn fill(canvas: &mut Canvas<Window>, r: FRect, c: [u8; 3]) -> Result {
    canvas.set_draw_color(colour(c));
    canvas.fill_rect(r)?;
    Ok(())
}

/// Minimap/overview geometry: the square map picture inside `area`.
#[derive(Clone, Copy)]
struct Square {
    x: f32,
    y: f32,
    /// Output pixels per tile.
    tile: f32,
}

impl Square {
    fn point(&self, tx: f64, ty: f64) -> FPoint {
        FPoint::new(
            self.x + tx as f32 * self.tile,
            self.y + ty as f32 * self.tile,
        )
    }

    fn tile_rect(&self, x: i64, y: i64, w: i64, h: i64) -> FRect {
        let p = self.point(x as f64, y as f64);
        FRect::new(p.x, p.y, w as f32 * self.tile, h as f32 * self.tile)
    }

    fn tile_of(&self, px: i32, py: i32) -> (f64, f64) {
        (
            ((px as f32 - self.x) / self.tile) as f64,
            ((py as f32 - self.y) / self.tile) as f64,
        )
    }
}

impl Viewer {
    pub(crate) fn layout(&self) -> Layout {
        Layout::fitted(self.output.0 as i32, self.output.1 as i32, self.scale)
    }

    /// State a button is drawn in, given its default.
    fn button_state(&self, button: Button, default: State) -> State {
        if default == State::Disabled {
            return State::Disabled;
        }
        let layout = self.layout();
        let over = layout.button(button).contains(self.mouse.0, self.mouse.1);
        match self.pressed {
            Some(b) if b == button && over => State::Pressed,
            None if over => State::Hover,
            _ => State::Normal,
        }
    }

    /// The minimap's map square inside its black well.
    fn minimap_square(&self) -> (Rect, Square) {
        let layout = self.layout();
        let well = layout.field(Field::Minimap);
        let s = layout.scale;
        let side = (well.w.min(well.h) - 4 * s).max(1);
        let size = self.map.battle.scenario.size.max(1) as f32;
        let tile = (side as f32 / size).floor().max(1.0);
        let used = (tile * size) as i32;
        let x = well.x + (well.w - used) / 2;
        let y = well.y + (well.h - used) / 2;
        (
            Rect::new(x, y, used, used),
            Square {
                x: x as f32,
                y: y as f32,
                tile,
            },
        )
    }

    /// The overview's map square, centred in the viewport.
    fn overview_square(&self) -> (Rect, usize) {
        let v = self.layout().viewport();
        let size = self.map.battle.scenario.size.max(1) as i32;
        let cell = ((v.w.min(v.h) as f32 * 0.92) as i32 / size).max(1);
        let side = cell * size;
        (
            Rect::new(v.x + (v.w - side) / 2, v.y + (v.h - side) / 2, side, side),
            cell as usize,
        )
    }

    /// Tile under output pixel `(x, y)` of the minimap or overview picture.
    pub(crate) fn picture_tile(&self, x: i32, y: i32) -> Option<(f64, f64)> {
        let (rect, square) = match self.screen {
            Screen::Battle
            | Screen::Deploy
            | Screen::AiTurn
            | Screen::Automated
            | Screen::BaseEditor => self.minimap_square(),
            Screen::Overview | Screen::Briefing => {
                let (rect, cell) = self.overview_square();
                let tile = cell as f32;
                (
                    rect,
                    Square {
                        x: rect.x as f32,
                        y: rect.y as f32,
                        tile,
                    },
                )
            }
            _ => return None,
        };
        let margin = 2 * self.scale;
        let grown = Rect::new(
            rect.x - margin,
            rect.y - margin,
            rect.w + 2 * margin,
            rect.h + 2 * margin,
        );
        grown.contains(x, y).then(|| {
            let size = self.map.battle.scenario.size as f64;
            let (tx, ty) = square.tile_of(x, y);
            (tx.clamp(0.0, size), ty.clamp(0.0, size))
        })
    }

    /// Centres the camera on continuous tile coordinates.
    pub(crate) fn centre_on_tile(&mut self, tx: f64, ty: f64) {
        let (px, py) = map_point(self.map.battle.scenario.size, tx, ty);
        let v = self.layout().viewport();
        self.camera = (
            px as f32 - v.w as f32 / (2.0 * self.zoom),
            py as f32 - v.h as f32 / (2.0 * self.zoom),
        );
    }

    /// The camera's viewport corners in continuous tile coordinates.
    fn view_corners(&self) -> [(f64, f64); 4] {
        let v = self.layout().viewport();
        let size = self.map.battle.scenario.size;
        [(0, 0), (v.w, 0), (v.w, v.h), (0, v.h)].map(|(x, y)| {
            let (mx, my) = self.to_map(x as f32, y as f32);
            tile_coords(size, mx as f64, my as f64)
        })
    }

    /// Whether the overview's selected entry is in its "off" blink phase.
    fn blink_off(&self) -> bool {
        (self.clock.since(self.started).as_millis() / 300) % 2 == 1
    }

    fn marker_blinks(&self, marker: &Marker) -> bool {
        self.entry.is_some_and(|i| {
            self.entries().get(i).is_some_and(|e| {
                e.matches(marker) && Some(&marker.owner) == self.players.get(self.player)
            })
        })
    }

    /// Whether anything on screen animates (needs periodic redraws).
    pub(crate) fn animating(&self) -> bool {
        self.overview_shown() && (self.entry.is_some() || self.has_tech())
            || self.flow.ai_until.is_some()
            || self.sea_animating()
    }

    /// Whether the province map's sea animates on screen.
    pub(crate) fn sea_animating(&self) -> bool {
        matches!(self.screen, Screen::Strategic { .. })
            && self
                .strategic
                .as_ref()
                .is_some_and(|st| st.map.sea_frames() > 0)
    }

    /// Whether the map holds a technology building (its overview bulb blinks).
    fn has_tech(&self) -> bool {
        let s = &self.map.battle.scenario;
        s.buildings
            .iter()
            .any(|b| libday_view::tech::reward(&self.defs, b).is_some())
    }

    /// Whether the full-screen map overview (MAP or the briefing) is shown.
    fn overview_shown(&self) -> bool {
        matches!(self.screen, Screen::Overview | Screen::Briefing)
    }
}

fn draw_markers(
    canvas: &mut Canvas<Window>,
    viewer: &Viewer,
    square: Square,
    detailed: bool,
) -> Result {
    let off = viewer.blink_off();
    for building in [true, false] {
        for m in viewer.markers.iter().filter(|m| m.building == building) {
            let mut rect = square.tile_rect(m.x, m.y, m.width, m.height);
            let mut c = if building {
                minimap::building_colour(m.owner)
            } else {
                minimap::owner_colour(m.owner)
            };
            if viewer.overview_shown() && viewer.marker_blinks(m) && off {
                c = BLACK;
            }
            if detailed && !building {
                let inset = (rect.w * 0.18).floor();
                rect = FRect::new(
                    rect.x + inset,
                    rect.y + inset,
                    rect.w - 2.0 * inset,
                    rect.h - 2.0 * inset,
                );
            }
            fill(canvas, rect, c)?;
            if detailed {
                canvas.set_draw_color(Color::RGB(0, 0, 0));
                canvas.draw_rect(rect)?;
            }
        }
    }
    Ok(())
}

/// Outlines the camera's view on a map picture, clipped to the picture.
fn draw_view_outline(
    canvas: &mut Canvas<Window>,
    viewer: &Viewer,
    square: Square,
    picture: Rect,
) -> Result {
    let corners = viewer.view_corners().map(|(x, y)| square.point(x, y));
    canvas.set_clip_rect(sdl3::rect::Rect::new(
        picture.x,
        picture.y,
        picture.w as u32,
        picture.h as u32,
    ));
    canvas.set_draw_color(Color::RGB(255, 255, 255));
    for i in 0..4 {
        canvas.draw_line(corners[i], corners[(i + 1) % 4])?;
    }
    canvas.set_clip_rect(None);
    Ok(())
}

/// Draws the sidebar frame, tiling plain rows below 768.
fn draw_sidebar(canvas: &mut Canvas<Window>, gfx: &Gfx, layout: &Layout, f: u16) -> Result {
    let (s, x) = (layout.scale, layout.sidebar_x());
    if layout.height <= SIDEBAR_H * s {
        return gfx.frame(canvas, f, x, 0, s);
    }
    let (a, b) = SIDEBAR_TILE;
    gfx.frame_part(canvas, f, Rect::new(0, 0, SIDEBAR_W, b), x, 0, s)?;
    let bottom = layout.height - (SIDEBAR_H - b) * s;
    let mut y = b * s;
    while y < bottom {
        let rows = ((bottom - y) / s).min(b - a).max(1);
        gfx.frame_part(canvas, f, Rect::new(0, a, SIDEBAR_W, rows), x, y, s)?;
        y += rows * s;
    }
    gfx.frame_part(
        canvas,
        f,
        Rect::new(0, b, SIDEBAR_W, SIDEBAR_H - b),
        x,
        bottom,
        s,
    )
}

/// Draws the bottom bar stretched to the viewport width.
fn draw_bottom_bar(canvas: &mut Canvas<Window>, gfx: &Gfx, layout: &Layout) -> Result {
    let (s, bar) = (layout.scale, layout.bottom_bar());
    let f = frame::BOTTOM_BAR;
    let right_w = BOTTOM_W - BOTTOM_RIGHT;
    let right_x = bar.right() - right_w * s;
    let (a, b) = BOTTOM_TILE;
    let mut x = BOTTOM_LEFT * s;
    while x < right_x {
        let cols = ((right_x - x) / s).clamp(1, b - a);
        gfx.frame_part(canvas, f, Rect::new(a, 0, cols, BOTTOM_H), x, bar.y, s)?;
        x += cols * s;
    }
    gfx.frame_part(
        canvas,
        f,
        Rect::new(0, 0, BOTTOM_LEFT, BOTTOM_H),
        0,
        bar.y,
        s,
    )?;
    gfx.frame_part(
        canvas,
        f,
        Rect::new(BOTTOM_RIGHT, 0, right_w, BOTTOM_H),
        right_x,
        bar.y,
        s,
    )
}

fn portrait<'r, 'g>(
    gfx: &'g mut Gfx<'r>,
    listbox: Option<&Dbi<'static>>,
    image: &str,
) -> Option<&'g Texture<'r>> {
    let creator = gfx.creator;
    gfx.portraits
        .entry(image.to_owned())
        .or_insert_with(|| {
            let dbi = listbox?;
            let sprite = dbi.sprite(&format!("{image}999")).ok()?;
            let rgba = sprite.to_rgba(&dbi.palette);
            texture(
                creator,
                &Image {
                    width: sprite.width,
                    height: sprite.height,
                    rgba,
                },
            )
        })
        .as_ref()
}

fn draw_battle_sidebar(
    canvas: &mut Canvas<Window>,
    gfx: &mut Gfx,
    viewer: &Viewer,
    scene: &Scene,
    art: &Art,
) -> Result {
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;
    draw_sidebar(canvas, gfx, &layout, frame::SIDEBAR)?;
    let unit = viewer.selected_unit();
    for button in layout.buttons(Screen::Battle, 0) {
        let on = match button {
            Button::Overwatch => viewer.overwatch_on(),
            Button::AutoFire => viewer.map.battle.auto_fire,
            Button::Fire => viewer.fire_mode_on(),
            Button::Group => viewer.grouping,
            _ => false,
        };
        gfx.button(canvas, viewer, &layout, button, State::Normal, on)?;
    }
    draw_credits(canvas, gfx, viewer)?;
    draw_turns(canvas, gfx, viewer, Field::Turn)?;
    let group = viewer.selection().len();
    if group > 1 {
        // String 0x53 (`%1!d! units selected`) in place of the portrait.
        let text = format!("{group} units selected");
        let (p, t) = (layout.field(Field::Portrait), layout.field(Field::UnitText));
        let r = Rect::new(p.x, p.y, (t.x + t.w).max(p.x + p.w) - p.x, p.h);
        gfx.text_in(canvas, fonts, &text, px, WHITE, r, 0.5, 0)?;
    } else if let Some((name, image, ap, ar)) = unit {
        let r = layout.field(Field::Portrait);
        if let Some(t) = portrait(gfx, viewer.listbox.as_ref(), &image) {
            canvas.copy(t, None, frect(r))?;
        }
        let r = layout.field(Field::UnitText);
        for (i, line) in [name, ap, ar].iter().enumerate() {
            gfx.text(
                canvas,
                fonts,
                line,
                px,
                WHITE,
                r.x,
                r.y + i as i32 * LINE * s,
            )?;
        }
    }
    draw_minimap(canvas, gfx, viewer, scene, art)
}

/// The player's credits in the sidebar's top box.
fn draw_credits(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let layout = viewer.layout();
    let px = TEXT_PX * layout.scale as f32;
    let credits = viewer.credits();
    if credits > 0 || !viewer.map.battle.players.is_empty() {
        let r = layout.field(Field::Credits);
        let text = credits.to_string();
        gfx.text_in(canvas, &viewer.fonts, &text, px, WHITE, r, 0.5, 0)?;
    }
    Ok(())
}

/// Turns left in the sidebar's bottom box `field`.
fn draw_turns(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer, field: Field) -> Result {
    let layout = viewer.layout();
    let (s, px) = (layout.scale, TEXT_PX * layout.scale as f32);
    let turns = viewer.map.battle.scenario.info.turns;
    if turns > 0 {
        // Before the battle starts (turn 0) every turn is still to play.
        let turn = viewer.map.battle.clock.turn.max(1);
        let left = (turns - turn + 1).max(0);
        let r = layout.field(field);
        let text = left.to_string();
        gfx.text_in(canvas, &viewer.fonts, &text, px, WHITE, r, 1.0, 4 * s)?;
    }
    Ok(())
}

/// Minimap: box-filtered top-down picture, markers, camera outline. While the
/// right button is held on a unit, building, wall or road, its info replaces
/// the minimap.
fn draw_minimap(
    canvas: &mut Canvas<Window>,
    gfx: &mut Gfx,
    viewer: &Viewer,
    scene: &Scene,
    art: &Art,
) -> Result {
    if let Some(target) = viewer.info {
        return match target.unit() {
            Some(i) => draw_unit_info(canvas, gfx, viewer, i),
            None => draw_structure_info(canvas, gfx, viewer, target),
        };
    }
    let layout = viewer.layout();
    let (rect, square) = viewer.minimap_square();
    let key = (viewer.generation, rect.w);
    if gfx.minimap.as_ref().is_none_or(|(k, _)| *k != key) {
        let size = scene.size.max(1);
        let cell = (2 * rect.w as usize).div_ceil(size).max(2);
        let picture = minimap::top_down(scene, art, cell);
        let small = minimap::downscale(&picture, rect.w as usize, rect.h as usize);
        gfx.minimap = texture(gfx.creator, &small).map(|t| (key, t));
    }
    if let Some((_, t)) = &gfx.minimap {
        canvas.copy(t, None, frect(rect))?;
    }
    let well = layout.field(Field::Minimap);
    canvas.set_clip_rect(sdl3::rect::Rect::new(
        well.x,
        well.y,
        well.w as u32,
        well.h as u32,
    ));
    draw_markers(canvas, viewer, square, false)?;
    canvas.set_clip_rect(None);
    draw_view_outline(canvas, viewer, square, rect)
}

/// Unit info in the minimap well: portrait, name, AR, AP, range, whether it
/// can still fire on overwatch, owner and capability icons.
fn draw_unit_info(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer, i: usize) -> Result {
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;
    let well = layout.field(Field::Minimap);
    fill(canvas, frect(well), BLACK)?;
    let (Some(unit), Some(kind)) = (viewer.map.battle.scenario.units.get(i), viewer.kind_of(i))
    else {
        return Ok(());
    };
    let at =
        |x: i32, y: i32, w: i32, h: i32| Rect::new(well.x + x * s, well.y + y * s, w * s, h * s);
    let (x, y, side) = unit_info::PORTRAIT;
    if let Some(t) = portrait(gfx, viewer.listbox.as_ref(), &kind.image) {
        canvas.copy(t, None, frect(at(x, y, side, side)))?;
    }
    let ap = unit.action_points.unwrap_or(kind.action_points);
    let overwatch = if ap >= kind.shoot_action_points {
        "YES"
    } else {
        "NO"
    };
    let lines = [
        kind.name.clone(),
        format!("AR {}", unit.armor.unwrap_or(kind.armor)),
        format!("AP {ap}"),
        format!("RANGE {}", kind.range),
        format!("OVERWATCH {overwatch}"),
        viewer.player_name(unit.owner),
    ];
    let well_w = well.w / s;
    for (row, text) in lines.iter().enumerate() {
        let left = if row < 3 {
            unit_info::TEXT_BESIDE
        } else {
            unit_info::TEXT_LEFT
        };
        let top = unit_info::TEXT_TOP + row as i32 * unit_info::LINE;
        let r = at(left, top, well_w - left, unit_info::LINE);
        gfx.text_in(canvas, fonts, text, px, WHITE, r, 0.0, 0)?;
    }
    let (x, y) = unit_info::ICONS;
    for (n, id) in unit_info::icons(kind).into_iter().enumerate() {
        if let Some(t) = gfx.icons.get(&id) {
            let r = at(
                x + n as i32 * unit_info::ICON_STEP,
                y,
                unit_info::ICON,
                unit_info::ICON,
            );
            canvas.copy(t, None, frect(r))?;
        }
    }
    Ok(())
}

/// Building, wall or road info in the minimap well, laid out as a unit's:
/// portrait, name and AR beside it, owner (and technology) below.
fn draw_structure_info(
    canvas: &mut Canvas<Window>,
    gfx: &mut Gfx,
    viewer: &Viewer,
    target: libday_view::inspect::Target,
) -> Result {
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;
    let well = layout.field(Field::Minimap);
    fill(canvas, frect(well), BLACK)?;
    let Some(card) =
        libday_view::inspect::card_named(&viewer.map.battle.scenario, &viewer.defs, target, &|o| {
            viewer.player_name(o)
        })
    else {
        return Ok(());
    };
    let at =
        |x: i32, y: i32, w: i32, h: i32| Rect::new(well.x + x * s, well.y + y * s, w * s, h * s);
    let (x, y, side) = unit_info::PORTRAIT;
    if let Some(t) = portrait(gfx, viewer.listbox.as_ref(), &card.image) {
        canvas.copy(t, None, frect(at(x, y, side, side)))?;
    }
    let well_w = well.w / s;
    for (row, text) in card.lines.iter().enumerate() {
        let left = if row < 3 {
            unit_info::TEXT_BESIDE
        } else {
            unit_info::TEXT_LEFT
        };
        let top = unit_info::TEXT_TOP + row as i32 * unit_info::LINE;
        let r = at(left, top, well_w - left, unit_info::LINE);
        gfx.text_in(canvas, fonts, text, px, WHITE, r, 0.0, 0)?;
    }
    Ok(())
}

/// The overview's blinking bulb on every technology building, captured or
/// not (`0x459bce`, `0x45a162`): each at its footprint's centre less
/// [`BULB_OFFSET`].
fn draw_bulbs(
    canvas: &mut Canvas<Window>,
    gfx: &Gfx,
    viewer: &Viewer,
    square: Square,
    s: i32,
) -> Result {
    let frame = (viewer.clock.since(viewer.started).as_millis() / BULB_MS) as usize % BULB.len();
    let Some(t) = gfx.icons.get(&BULB[frame]) else {
        return Ok(());
    };
    let (w, h) = (t.width() as f32 * s as f32, t.height() as f32 * s as f32);
    let (s, defs) = (s as f32, &viewer.defs);
    for b in &viewer.map.battle.scenario.buildings {
        if libday_view::tech::reward(defs, b).is_none() {
            continue;
        }
        let (bw, bh) = defs
            .buildings
            .get(&b.kind)
            .map_or((1, 1), |k| (k.width, k.height));
        let c = square.point(b.x as f64 + bw as f64 / 2.0, b.y as f64 + bh as f64 / 2.0);
        let at = FRect::new(
            c.x - BULB_OFFSET.0 as f32 * s,
            c.y - BULB_OFFSET.1 as f32 * s,
            w,
            h,
        );
        canvas.copy(t, None, at)?;
    }
    Ok(())
}

fn draw_overview(
    canvas: &mut Canvas<Window>,
    gfx: &mut Gfx,
    viewer: &Viewer,
    scene: &Scene,
    art: &Art,
) -> Result {
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;

    let (rect, cell) = viewer.overview_square();
    let key = (viewer.generation, cell);
    if gfx.overview.as_ref().is_none_or(|(k, _)| *k != key) {
        let picture = minimap::top_down(scene, art, cell);
        gfx.overview = texture(gfx.creator, &picture).map(|t| (key, t));
    }
    if let Some((_, t)) = &gfx.overview {
        canvas.copy(t, None, frect(rect))?;
    }
    let square = Square {
        x: rect.x as f32,
        y: rect.y as f32,
        tile: cell as f32,
    };
    let owner = viewer.players.get(viewer.player).copied().unwrap_or(0);
    canvas.set_blend_mode(BlendMode::Blend);
    if viewer.screen == Screen::Briefing {
        let [r, g, b] = minimap::owner_colour(owner);
        canvas.set_draw_color(Color::RGBA(r, g, b, 110));
        for d in &viewer.map.battle.scenario.deployments {
            if viewer.zone(d.x, d.y) {
                canvas.fill_rect(square.tile_rect(d.x, d.y, 1, 1))?;
            }
        }
    }
    // The objective's area (capture, hold, defend...).
    canvas.set_draw_color(Color::RGBA(255, 0, 0, 110));
    if let Some(o) = &viewer.map.battle.scenario.objective {
        for &(x, y) in &o.tiles {
            canvas.fill_rect(square.tile_rect(x, y, 1, 1))?;
        }
    }
    canvas.set_blend_mode(BlendMode::None);
    draw_markers(canvas, viewer, square, true)?;
    draw_bulbs(canvas, gfx, viewer, square, s)?;

    draw_sidebar(canvas, gfx, &layout, frame::OVERVIEW_SIDEBAR)?;
    draw_credits(canvas, gfx, viewer)?;
    draw_turns(canvas, gfx, viewer, Field::OverviewBox)?;
    let entries = viewer.entries();
    for button in layout.buttons(viewer.screen, viewer.players.len()) {
        gfx.button(canvas, viewer, &layout, button, State::Normal, false)?;
    }
    let r = layout.field(Field::PlayerName);
    gfx.text_in(
        canvas,
        fonts,
        &viewer.player_name(owner),
        px,
        WHITE,
        r,
        0.0,
        2 * s,
    )?;
    for (row, entry) in entries
        .iter()
        .enumerate()
        .skip(viewer.scroll)
        .take(LIST_ROWS)
    {
        let r = layout.list_row(row - viewer.scroll);
        let selected = viewer.entry == Some(row);
        if selected {
            fill(canvas, frect(r), WHITE)?;
        }
        let rgb = if selected { BLACK } else { WHITE };
        gfx.text_in(canvas, fonts, &entry.label(), px, rgb, r, 0.0, 8 * s)?;
    }
    Ok(())
}

fn draw_goal(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;
    let dialog = layout.goal_dialog();
    gfx.frame(canvas, frame::GOAL_DIALOG, dialog.x, dialog.y, s)?;
    for button in layout.buttons(Screen::Goal, 0) {
        gfx.button(
            canvas,
            viewer,
            &layout,
            button,
            State::Normal,
            viewer.objectives,
        )?;
    }
    let r = layout.field(Field::GoalText);
    let info = &viewer.map.battle.scenario.info;
    let mut lines = vec![String::new()];
    if !info.province.is_empty() {
        lines.extend([info.province.clone(), String::new()]);
    }
    lines.extend(ui::wrap(&info.description, r.w as f32, |t| {
        fonts.measure(t, px, false)
    }));
    canvas.set_clip_rect(sdl3::rect::Rect::new(r.x, r.y, r.w as u32, r.h as u32));
    let title = if info.title.is_empty() {
        "untitled"
    } else {
        &info.title
    };
    let line = GOAL_LINE * s;
    gfx.text_in(
        canvas,
        fonts,
        title,
        px,
        WHITE,
        Rect::new(r.x, r.y, r.w, line),
        0.5,
        0,
    )?;
    for (i, text) in lines.iter().enumerate() {
        gfx.text(
            canvas,
            fonts,
            text,
            px,
            WHITE,
            r.x,
            r.y + (i as i32 + 1) * line,
        )?;
    }
    canvas.set_clip_rect(None);
    Ok(())
}

/// The battle report (`0x45ded0`): title and outcome centred, then the
/// player's statistics under the column headings.
fn draw_report(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;
    let dialog = layout.report_dialog();
    gfx.frame(canvas, frame::REPORT_DIALOG, dialog.x, dialog.y, s)?;
    gfx.button(
        canvas,
        viewer,
        &layout,
        Button::ReportContinue,
        State::Normal,
        false,
    )?;
    let Some(report) = &viewer.flow.report else {
        return Ok(());
    };
    let r = layout.in_report(ui::REPORT_TEXT);
    let line = GOAL_LINE * s;
    for (i, text) in [&report.title, &report.message].into_iter().enumerate() {
        let row = Rect::new(r.x, r.y + 2 * i as i32 * line, r.w, line);
        gfx.text_in(canvas, fonts, text, px, WHITE, row, 0.5, 0)?;
    }
    let top = dialog.y + ui::REPORT_HEADER_Y * s;
    let rows = std::iter::once((top, &report.headers)).chain(
        report
            .rows
            .iter()
            .enumerate()
            .map(|(i, row)| (top + (2 + i as i32) * line, row)),
    );
    for (y, row) in rows {
        for (text, x) in row.iter().zip(ui::REPORT_COLUMNS) {
            gfx.text(canvas, fonts, text, px, WHITE, r.x + x * s, y)?;
        }
    }
    let y = top + (2 + report.rows.len() as i32 + 1) * line;
    gfx.text(
        canvas,
        fonts,
        &report.tech,
        px,
        WHITE,
        r.x + ui::REPORT_COLUMNS[0] * s,
        y,
    )?;
    Ok(())
}

/// Greys the map drawn so far, as under the original's ARMY dialog. The
/// snapshot is taken once per opening (`serial`).
pub fn grey_out(canvas: &mut Canvas<Window>, gfx: &mut Gfx, serial: u64) -> Result {
    let size = canvas.output_size()?;
    let key = (serial, size);
    if gfx.grey.as_ref().is_none_or(|(k, _)| *k != key) {
        let mut surface = canvas
            .read_pixels(None)?
            .convert_format(PixelFormat::RGBA32)?;
        let (w, h) = surface.size();
        let pitch = surface.pitch() as usize;
        surface.with_lock_mut(|bytes| {
            for y in 0..h as usize {
                for px in bytes[y * pitch..y * pitch + w as usize * 4]
                    .as_chunks_mut::<4>()
                    .0
                {
                    let g =
                        (u32::from(px[0]) * 77 + u32::from(px[1]) * 150 + u32::from(px[2]) * 29)
                            >> 8;
                    px[..3].fill(g as u8);
                }
            }
        });
        let texture = gfx.creator.create_texture_from_surface(&surface)?;
        gfx.grey = Some((key, texture));
    }
    if let Some((_, texture)) = &gfx.grey {
        canvas.copy(texture, None, None)?;
    }
    Ok(())
}

/// Draws the whole interface over the (already drawn) map.
pub fn draw(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let (scene, art) = (&viewer.map.scene, &viewer.art);
    match viewer.screen {
        Screen::Skirmishes => return setup::draw_selector(canvas, gfx, viewer),
        Screen::MainMenu => return front::draw_main_menu(canvas, gfx, viewer),
        // RESOLUTION: the OPTIONS dialog over the main menu.
        Screen::Options { ask } if viewer.menu.over_main => {
            front::draw_main_menu(canvas, gfx, viewer)?;
            options::draw_options(canvas, gfx, viewer)?;
            if ask.is_some() {
                options::draw_question(canvas, gfx, viewer)?;
            }
            return Ok(());
        }
        Screen::NewGame => return front::draw_new_game(canvas, gfx, viewer),
        Screen::Saves { saving: false } => {
            front::draw_main_menu(canvas, gfx, viewer)?;
            return saves::draw(canvas, gfx, viewer);
        }
        Screen::Sessions => return front::draw_sessions(canvas, gfx, viewer),
        Screen::Lobby => return front::draw_lobby(canvas, gfx, viewer),
        Screen::Tech { .. } => return setup::draw_tech(canvas, gfx, viewer),
        Screen::Overview | Screen::Briefing => draw_overview(canvas, gfx, viewer, scene, art)?,
        Screen::Battle => draw_battle_sidebar(canvas, gfx, viewer, scene, art)?,
        Screen::Deploy | Screen::Confirm => setup::draw_deploy_sidebar(canvas, gfx, viewer)?,
        Screen::AiTurn => setup::draw_ai_sidebar(canvas, gfx, viewer)?,
        Screen::Automated => setup::draw_automated_sidebar(canvas, gfx, viewer)?,
        Screen::Strategic { .. } => strategic::draw(canvas, gfx, viewer)?,
        Screen::Intelligence => {
            strategic::draw(canvas, gfx, viewer)?;
            strategic::draw_intel(canvas, gfx, viewer)?;
        }
        Screen::BaseEditor => base::draw(canvas, gfx, viewer)?,
        Screen::Army => {
            base::draw(canvas, gfx, viewer)?;
            setup::draw_tech(canvas, gfx, viewer)?;
        }
        Screen::Goal => {
            match viewer.home() {
                Screen::Deploy => setup::draw_deploy_sidebar(canvas, gfx, viewer)?,
                Screen::AiTurn => setup::draw_ai_sidebar(canvas, gfx, viewer)?,
                _ => draw_battle_sidebar(canvas, gfx, viewer, scene, art)?,
            }
            draw_goal(canvas, gfx, viewer)?;
        }
        Screen::Report => {
            draw_battle_sidebar(canvas, gfx, viewer, scene, art)?;
            draw_report(canvas, gfx, viewer)?;
        }
        Screen::Options { .. } | Screen::Saves { .. } => {
            match viewer.home() {
                Screen::Deploy => setup::draw_deploy_sidebar(canvas, gfx, viewer)?,
                Screen::AiTurn => setup::draw_ai_sidebar(canvas, gfx, viewer)?,
                Screen::Strategic { .. } => strategic::draw(canvas, gfx, viewer)?,
                Screen::BaseEditor => base::draw(canvas, gfx, viewer)?,
                _ => draw_battle_sidebar(canvas, gfx, viewer, scene, art)?,
            }
            if matches!(viewer.screen, Screen::Saves { .. }) {
                saves::draw(canvas, gfx, viewer)?;
            } else {
                options::draw_options(canvas, gfx, viewer)?;
            }
        }
    }
    draw_bottom_bar(canvas, gfx, &layout)?;
    for button in [Button::Chat, Button::Menu] {
        gfx.button(canvas, viewer, &layout, button, State::Normal, false)?;
    }
    // The turn's time left under a turn limit (red in its last half
    // minute), otherwise its time so far.
    let (secs, rgb) = match viewer.turn_left() {
        Some(left) => {
            let secs = left.as_millis().div_ceil(1000) as u64;
            (secs, if secs <= 30 { [255, 80, 64] } else { WHITE })
        }
        None => (viewer.clock.since(viewer.started).as_secs(), WHITE),
    };
    let timer = format!("{:02}:{:02}", secs / 60 % 100, secs % 60);
    let r = layout.field(Field::Timer);
    gfx.text_in(canvas, fonts, &timer, TEXT_PX * s as f32, rgb, r, 0.5, 0)?;
    if let Some(message) = &viewer.flow.message {
        let bar = layout.bottom_bar();
        let r = Rect::new(bar.x + 62 * s, r.y, r.x - bar.x - 70 * s, r.h);
        gfx.text_in(canvas, fonts, message, TEXT_PX * s as f32, WHITE, r, 0.0, 0)?;
    }
    front::draw_chat_overlay(canvas, gfx, viewer)?;
    if viewer.screen == Screen::Confirm {
        setup::draw_confirm(canvas, gfx, viewer)?;
    }
    if let Screen::Options { ask: Some(_) } = viewer.screen {
        options::draw_question(canvas, gfx, viewer)?;
    }
    Ok(())
}
