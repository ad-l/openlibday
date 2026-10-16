//! Map drawing.

use libday_data::defs::AutomatedCategory;
use libday_view::base::{self, Item};
use libday_view::scene::DIM;
use libday_view::support;
use libday_view::ui::Screen;
use libday_view::{Art, Layer, Sheet, sprite_origin};
use sdl3::pixels::Color;
use sdl3::render::{Canvas, FPoint, FRect, Texture};
use sdl3::video::Window;

use crate::Viewer;
use crate::hud::{self, Gfx, Result};
use crate::moves;

impl<'r> Gfx<'r> {
    /// Sprite `name` of `sheet`; a [`RED`]-prefixed name is the sprite in
    /// red (from a yellow one).
    pub(crate) fn sprite(&mut self, art: &Art, sheet: Sheet, name: &str) -> Option<&Texture<'r>> {
        let creator = self.creator();
        self.sprites
            .entry((sheet, name.to_owned()))
            .or_insert_with(|| {
                let (name, odd) = match name.strip_prefix(ODD) {
                    Some(base) => (base, true),
                    None => (name, false),
                };
                let (name, dim) = match (name.strip_prefix(DIM_ODD), name.strip_prefix(DIM)) {
                    (Some(base), _) => (base, Some(1)),
                    (None, Some(base)) => (base, Some(0)),
                    _ => (name, None),
                };
                let (base, red) = match name.strip_prefix(RED) {
                    Some(base) => (base, true),
                    None => (name, false),
                };
                let (width, height, mut rgba) = art.rgba(sheet, base)?;
                if let Some(phase) = dim {
                    for (i, px) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                        if px[3] != 0 && (i % width + i / width + phase) % 2 == 0 {
                            *px = [0, 0, 0, 255];
                        }
                    }
                }
                if odd {
                    flip_stipple(width, &mut rgba);
                }
                if red {
                    for px in rgba.as_chunks_mut::<4>().0 {
                        (px[1], px[2]) = (0, 0);
                    }
                }
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

/// Sprite-name prefix for a sprite recoloured red.
pub(crate) const RED: &str = "red:";

/// Sprite-name prefix for a zone sprite with its checkerboard stipple moved
/// by one pixel. Neighbouring tiles sit (27, 14) apart, an odd distance, so
/// tiles with odd `x + y` take this one to keep the pattern seamless.
pub(crate) const ODD: &str = "odd:";

/// [`DIM`] for a sprite drawn at an odd `left + top`: the blackened pixels
/// always sit at even map `x + y`, so neighbouring tiles of a large unit
/// keep one seamless checkerboard.
const DIM_ODD: &str = "dim1:";

/// Name of zone sprite `name` for tile `(x, y)` (see [`ODD`]).
pub(crate) fn zone_sprite(name: &str, x: i64, y: i64) -> std::borrow::Cow<'_, str> {
    if (x + y).rem_euclid(2) == 1 {
        format!("{ODD}{name}").into()
    } else {
        name.into()
    }
}

/// Moves the checkerboard of the sprite's centre colour by one pixel: the
/// stipple area is its pixels plus the transparent gaps between them.
fn flip_stipple(width: usize, rgba: &mut [u8]) {
    let height = rgba.len() / 4 / width;
    let at = |x: usize, y: usize| 4 * (y * width + x);
    let (cx, cy) = (width / 2, height * 3 / 4);
    let Some(colour) = [(cx, cy), (cx - 1, cy), (cx, cy - 1)]
        .into_iter()
        .map(|(x, y)| at(x, y))
        .find(|&i| rgba[i + 3] != 0)
        .map(|i| [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]])
    else {
        return;
    };
    let src = rgba.to_vec();
    let dot = |x: usize, y: usize| src[at(x, y)..at(x, y) + 4] == colour;
    for y in 0..height {
        for x in 0..width {
            let i = at(x, y);
            let gap =
                src[i + 3] == 0 && ((x > 0 && dot(x - 1, y)) || (x + 1 < width && dot(x + 1, y)));
            if dot(x, y) || gap {
                let px = if dot(x, y) { [0; 4] } else { colour };
                rgba[i..i + 4].copy_from_slice(&px);
            }
        }
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
    let mut walkers = viewer.walkers();
    walkers.sort_by_key(|w| (w.depth.0 + w.depth.1, w.depth.0));
    let mut walkers = walkers.into_iter().peekable();
    // Tile overlays drawn in depth order, after their tile's ground and
    // buildings but before its units, so objects in front cover them: the
    // selection grid, path dots and the move cursor. Target cursors go on
    // top of everything, over the unit or building they aim at.
    let (move_cursor, target_cursor) = match viewer.battle_cursor() {
        Some(c) if c.1 == "CMOVE" => (Some(c), None),
        c => (None, c),
    };
    let watch: Vec<(i64, i64, std::borrow::Cow<str>)> = viewer
        .watch
        .iter()
        .map(|((x, y), name)| (*x, *y, zone_sprite(name, *x, *y)))
        .collect();
    let mut overlays: Vec<(i64, i64, &str)> = watch
        .iter()
        .map(|(x, y, name)| (*x, *y, name.as_ref()))
        .chain(hover_tiles.into_iter().map(|(x, y)| (x, y, "CSELC")))
        .chain(moves::path_dots(viewer))
        .chain(move_cursor.map(|((x, y), name)| (x, y, name)))
        .collect();
    overlays.sort_by_key(|&(x, y, _)| (x + y, x));
    let mut overlays = overlays.into_iter().peekable();
    let items = &viewer.map.scene.items;
    let flat =
        |item: &libday_view::scene::DrawItem| matches!(item.layer, Layer::Terrain | Layer::Road);
    let draw_item = |canvas: &mut Canvas<Window>,
                     gfx: &mut Gfx,
                     item: &libday_view::scene::DrawItem|
     -> Result {
        let (left, top) = sprite_origin(size, item.x, item.y);
        let (x0, y0) = screen(left, top);
        let (x1, y1) = screen(left + 54, top + 56);
        if x1 < 0.0 || y1 < 0.0 || x0 > ow || y0 > oh {
            return Ok(());
        }
        let sprite = if item.layer == Layer::Deployment {
            zone_sprite(&item.sprite, item.x, item.y)
        } else if let Some(base) = item.sprite.strip_prefix(DIM)
            && (left + top).rem_euclid(2) == 1
        {
            format!("{DIM_ODD}{base}").into()
        } else {
            item.sprite.as_str().into()
        };
        if let Some(texture) = gfx.sprite(&viewer.art, item.sheet, &sprite) {
            // Whole output pixels, so neighbouring tiles never leave seams.
            let (x0, y0, x1, y1) = (x0.floor(), y0.floor(), x1.floor(), y1.floor());
            canvas.copy(texture, None, Some(FRect::new(x0, y0, x1 - x0, y1 - y0)))?;
        }
        Ok(())
    };
    // Ground already drawn under a walker crossing its corner.
    let mut hoisted: Vec<(i64, i64)> = Vec::new();
    for item in items {
        while let Some(w) =
            walkers.next_if(|w| (item.x + item.y, item.x) > (w.depth.0 + w.depth.1, w.depth.0))
        {
            // Path dots up to the walker's tile go underneath it.
            let depth = (w.depth.0 + w.depth.1, w.depth.0);
            while let Some((x, y, name)) = overlays.next_if(|&(x, y, _)| (x + y, x) <= depth) {
                let (left, top) = sprite_origin(size, x, y);
                cell(canvas, gfx, left, top, name)?;
            }
            if let Some((ux, uy)) = w.under.filter(|t| !hoisted.contains(t)) {
                let key = (ux + uy, ux);
                let from = items.partition_point(|i| (i.x + i.y, i.x) < key);
                for i in items[from..].iter().take_while(|i| (i.x, i.y) == (ux, uy)) {
                    if flat(i) {
                        draw_item(canvas, gfx, i)?;
                    }
                }
                hoisted.push((ux, uy));
            }
            moves::draw_walker(canvas, gfx, viewer, &w, &screen_f)?;
        }
        let depth = (item.x + item.y, item.x);
        while let Some((x, y, name)) = overlays.next_if(|&(x, y, _)| {
            let at = (x + y, x);
            at < depth || (at == depth && item.layer == Layer::Unit)
        }) {
            let (left, top) = sprite_origin(size, x, y);
            cell(canvas, gfx, left, top, name)?;
        }
        if flat(item) && hoisted.contains(&(item.x, item.y)) {
            continue;
        }
        draw_item(canvas, gfx, item)?;
    }
    for w in walkers {
        let depth = (w.depth.0 + w.depth.1, w.depth.0);
        while let Some((x, y, name)) = overlays.next_if(|&(x, y, _)| (x + y, x) <= depth) {
            let (left, top) = sprite_origin(size, x, y);
            cell(canvas, gfx, left, top, name)?;
        }
        moves::draw_walker(canvas, gfx, viewer, &w, &screen_f)?;
    }
    for (x, y, name) in overlays {
        let (left, top) = sprite_origin(size, x, y);
        cell(canvas, gfx, left, top, name)?;
    }
    if let Some(((x, y), name)) = target_cursor {
        let (left, top) = sprite_origin(size, x, y);
        cell(canvas, gfx, left, top, name)?;
    }
    // A bulb over every technology building the player holds, on its
    // top-left tile (`CIsoAnimTech`, `0x46ac4d`).
    for b in &viewer.map.battle.scenario.buildings {
        if viewer.flow.tech.contains(&b.id) {
            let (left, top) = sprite_origin(size, b.x, b.y);
            cell(canvas, gfx, left, top, "LILBULB")?;
        }
    }
    moves::draw_shot_effects(canvas, gfx, viewer, &screen_f)?;
    // Deployment cursor, on zone tiles only: a hazard-barred square, or a
    // red-barred one when a click would recall the unit already there.
    if matches!(viewer.screen, Screen::Deploy)
        && let Some((x, y)) = viewer.hover
        && viewer.zone(x, y)
    {
        let (left, top) = sprite_origin(size, x, y);
        let name = if viewer.recallable(viewer.seats.local, x, y).is_some() {
            "CRECA"
        } else {
            "CDEPL"
        };
        cell(canvas, gfx, left, top, name)?;
    }
    // A bridge's sites without a road: those buildable now crossed in
    // red, the others as girders (`0x4722b3`); for the bridge support and
    // for the base editor's paved road.
    let paved = viewer
        .base_editor
        .as_ref()
        .is_some_and(|e| e.selected == Some(Item::Road(base::PAVED)));
    let armed = viewer.armed_support();
    if paved || armed.as_ref().and_then(|a| a.category()) == Some(AutomatedCategory::Build) {
        let (s, defs) = (&viewer.map.battle.scenario, &viewer.defs);
        let sites = support::bridge_sites(s, defs);
        for &(x, y) in &sites {
            if s.roads.iter().any(|r| (r.x, r.y) == (x, y)) {
                continue;
            }
            let name = if support::bridge_buildable(s, defs, &sites, (x, y)) {
                "BRIDGE"
            } else {
                "BRIDG001"
            };
            let (left, top) = sprite_origin(size, x, y);
            cell(canvas, gfx, left, top, name)?;
        }
    }
    // An armed support's cursor; a transport's pickup keeps its own.
    if let Some(a) = armed {
        if let Some((x, y)) = viewer.flow.pickup {
            let (left, top) = sprite_origin(size, x, y);
            cell(canvas, gfx, left, top, &a.cursor)?;
        }
        if let Some((x, y)) = viewer.hover {
            let (left, top) = sprite_origin(size, x, y);
            cell(canvas, gfx, left, top, &a.cursor)?;
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
    // Arrows above the centre of a footprint: the selected units' in their
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
    if !viewer.busy() {
        for i in viewer.selection() {
            if let Some(unit) = viewer.map.battle.scenario.units.get(i) {
                let name = format!("SU{:03}", unit.owner.clamp(0, 4));
                arrow(canvas, gfx, &footprint(i), &name)?;
            }
        }
    }
    if !viewer.busy() {
        arrow(canvas, gfx, &viewer.lock_tiles(), "SU000")?;
    }
    canvas.set_clip_rect(None);
    Ok(())
}

/// Draws a frame (without presenting it).
pub(crate) fn render(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    if viewer.cutscene.is_some() {
        return crate::cutscene::draw(canvas, gfx, viewer);
    }
    gfx.video = None;
    canvas.set_draw_color(Color::RGB(0, 0, 0));
    canvas.clear();
    let home_strategic = matches!(viewer.home(), Screen::Strategic { .. });
    if !viewer.screen.is_dialog()
        && !(viewer.menu.over_main && matches!(viewer.screen, Screen::Options { .. }))
        && !home_strategic
        && !matches!(viewer.screen, Screen::Overview | Screen::Briefing)
    {
        draw_map(canvas, gfx, viewer)?;
        if viewer.screen == Screen::Army {
            hud::grey_out(canvas, gfx, viewer.army_view)?;
        }
    }
    hud::draw(canvas, gfx, viewer)?;
    Ok(())
}
