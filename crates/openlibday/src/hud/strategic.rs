//! The continent (strategic) overview: animated sea, provinces and sidebar.

use libday_view::continent::Look;
use libday_view::ui::{self, Button, Field, Rect, Screen, State, frame};
use sdl3::render::{Canvas, FRect};
use sdl3::video::Window;

use super::{Gfx, LINE, Result, TEXT_PX, WHITE, draw_sidebar, fill, frect, grey_out, texture};
use crate::Viewer;

pub(super) fn draw(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let (Some(st), Some(session)) = (&viewer.strategic, &viewer.campaign) else {
        return Ok(());
    };
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;
    let map = &st.map;
    let continent = session.state.continent;
    fill(canvas, frect(layout.viewport()), map.edge())?;
    if let (Some((x0, y0, _)), true) = (viewer.strategic_origin(), map.sea_frames() > 0) {
        let micros = viewer.clock.since(st.started).as_micros() as u64;
        let i = (micros / map.frame_micros.max(1)) as usize % map.sea_frames();
        if gfx.sea.as_ref().is_none_or(|(k, _)| *k != (continent, i)) {
            gfx.sea = texture(gfx.creator, &map.sea(i)).map(|t| ((continent, i), t));
        }
        let (w, h) = (map.width as i32 * s, map.height as i32 * s);
        if let Some((_, t)) = &gfx.sea {
            canvas.copy(t, None, frect(Rect::new(x0, y0, w, h)))?;
        }
        for n in map.provinces() {
            let captured = session.state.province(n).is_some_and(|p| p.captured);
            let look = if st.selected == Some(n) {
                Look::Selected
            } else if captured {
                Look::Captured
            } else {
                continue;
            };
            let Some(o) = map.overlay(n, look) else {
                continue;
            };
            let key = (continent, n, look as u8);
            if !gfx.provinces.contains_key(&key)
                && let Some(t) = texture(gfx.creator, &o.image)
            {
                gfx.provinces.insert(key, t);
            }
            if let Some(t) = gfx.provinces.get(&key) {
                let r = FRect::new(
                    (x0 + o.x * s) as f32,
                    (y0 + o.y * s) as f32,
                    (o.image.width as i32 * s) as f32,
                    (o.image.height as i32 * s) as f32,
                );
                canvas.copy(t, None, r)?;
            }
        }
    }
    if viewer.screen == Screen::Intelligence {
        grey_out(canvas, gfx, viewer.army_view)?;
    }
    draw_sidebar(canvas, gfx, &layout, frame::STRATEGIC_SIDEBAR)?;
    let screen = Screen::Strategic {
        victory: st.victory,
    };
    for button in layout.buttons(screen, 0) {
        if !matches!(button, Button::Chat | Button::Menu) {
            gfx.button(canvas, viewer, &layout, button, State::Normal, false)?;
        }
    }
    let me = session.state.member(viewer.seats.local);
    for (field, value) in [
        (Field::StrategicTech, me.map_or(0, |m| m.tech)),
        (Field::StrategicCredits, me.map_or(0, |m| m.credits)),
        (Field::StrategicZero, 0),
    ] {
        let r = layout.field(field);
        gfx.text_in(canvas, fonts, &value.to_string(), px, WHITE, r, 0.5, 0)?;
    }
    let r = layout.field(Field::StrategicText);
    let line = LINE * s;
    let mut y = r.y;
    for paragraph in &st.text {
        let lines = if paragraph.is_empty() {
            vec![String::new()]
        } else {
            ui::wrap(paragraph, r.w as f32, |t| fonts.measure(t, px, false))
        };
        for l in lines {
            if y + line > r.y + r.h {
                return Ok(());
            }
            gfx.text(canvas, fonts, &l, px, WHITE, r.x, y)?;
            y += line;
        }
    }
    Ok(())
}

/// The INTELLIGENCE dialog: map preview, province title and name, then the
/// objective and hint (or the need for a Satellite Uplink).
pub(super) fn draw_intel(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let Some(intel) = viewer.strategic.as_ref().and_then(|st| st.intel.as_ref()) else {
        return Ok(());
    };
    let Some(p) = viewer
        .campaign
        .as_ref()
        .and_then(|c| c.continent()?.province(intel.province))
    else {
        return Ok(());
    };
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;
    let line = LINE * s;
    let d = layout.intel_dialog();
    gfx.frame(canvas, frame::INTEL_DIALOG, d.x, d.y, s)?;
    if let Some(image) = &intel.preview {
        if gfx
            .intel
            .as_ref()
            .is_none_or(|(k, _)| *k != viewer.army_view)
        {
            gfx.intel = texture(gfx.creator, image).map(|t| (viewer.army_view, t));
        }
        if let Some((_, t)) = &gfx.intel {
            let b = layout.in_intel(ui::INTEL_MAP);
            let (w, h) = (image.width as i32 * s, image.height as i32 * s);
            let r = Rect::new(b.x + (b.w - w) / 2, b.y + (b.h - h) / 2, w, h);
            canvas.copy(t, None, frect(r))?;
        }
    }
    let t = layout.in_intel(ui::INTEL_TEXT);
    let title = Rect::new(t.x, t.y, t.w, line);
    gfx.text_in(canvas, fonts, &p.title.en, px, WHITE, title, 0.5, 0)?;
    let mut y = t.y + 30 * s;
    gfx.text(canvas, fonts, &p.name.en, px, WHITE, t.x, y)?;
    let text = viewer.intel_text(intel);
    if intel.uplink {
        y += 30 * s;
        for block in &text {
            for paragraph in block.split('\n') {
                for l in ui::wrap(paragraph, t.w as f32, |x| fonts.measure(x, px, false)) {
                    if y + line > t.bottom() {
                        break;
                    }
                    gfx.text(canvas, fonts, &l, px, WHITE, t.x, y)?;
                    y += line;
                }
            }
            y += 20 * s;
        }
    } else {
        y += 60 * s;
        let left = t.x - 200 * s;
        let w = t.right() - left;
        for paragraph in &text {
            for l in ui::wrap(paragraph, w as f32, |x| fonts.measure(x, px, false)) {
                gfx.text_in(
                    canvas,
                    fonts,
                    &l,
                    px,
                    WHITE,
                    Rect::new(left, y, w, line),
                    0.5,
                    0,
                )?;
                y += line;
            }
        }
    }
    gfx.button(
        canvas,
        viewer,
        &layout,
        Button::IntelBack,
        State::Normal,
        false,
    )
}
