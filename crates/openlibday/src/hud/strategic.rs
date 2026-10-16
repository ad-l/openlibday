//! The continent (strategic) overview: animated sea, provinces and sidebar.

use libday_view::continent::Look;
use libday_view::ui::{self, Button, Field, Rect, Screen, State, frame};
use sdl3::render::{Canvas, FRect};
use sdl3::video::Window;

use super::{Gfx, LINE, Result, TEXT_PX, WHITE, draw_sidebar, fill, frect, texture};
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
    draw_sidebar(canvas, gfx, &layout, frame::STRATEGIC_SIDEBAR)?;
    let screen = Screen::Strategic {
        victory: st.victory,
    };
    for button in layout.buttons(screen, 0) {
        if !matches!(button, Button::Chat | Button::Menu) {
            gfx.button(canvas, viewer, &layout, button, State::Normal, false)?;
        }
    }
    let state = &session.state;
    for (field, value) in [
        (Field::StrategicTech, state.tech),
        (Field::StrategicCredits, state.credits),
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
