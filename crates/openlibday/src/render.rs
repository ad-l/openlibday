//! Map drawing.

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
                let (base, red) = match name.strip_prefix(RED) {
                    Some(base) => (base, true),
                    None => (name, false),
                };
                let (width, height, mut rgba) = art.rgba(sheet, base)?;
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
    let mut overlays: Vec<(i64, i64, &str)> = viewer
        .watch
        .iter()
        .map(|((x, y), name)| (*x, *y, name.as_str()))
        .chain(hover_tiles.into_iter().map(|(x, y)| (x, y, "CSELC")))
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
    // An armed support's cursor; a transport's pickup keeps its own.
    if let Some(a) = viewer.armed_support() {
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
pub(crate) fn render(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    canvas.set_draw_color(Color::RGB(0, 0, 0));
    canvas.clear();
    if !viewer.screen.is_dialog() && !matches!(viewer.screen, Screen::Overview | Screen::Briefing) {
        draw_map(canvas, gfx, viewer)?;
    }
    hud::draw(canvas, gfx, viewer)?;
    Ok(())
}
