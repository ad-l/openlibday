//! The OPTIONS dialog: the original frame and buttons around a black,
//! scrolling table of options with check boxes and selectors.

use libday_view::ui::{
    self, Button, OPTIONS_ROWS, OPTIONS_TABLE, QUESTION_TEXT, Rect, Screen, State, frame,
};
use sdl3::pixels::Color;
use sdl3::render::{BlendMode, Canvas, FPoint};
use sdl3::video::Window;

use super::{BLACK, Gfx, LINE, Result, TEXT_PX, WHITE, colour, fill, frect};
use crate::Viewer;
use crate::menu::{ITEMS, Item, Value, control};

const GOLD: [u8; 3] = [232, 188, 80];
const GREY: [u8; 3] = [120, 120, 130];
const DIM: [u8; 3] = [60, 60, 66];
const HOVER: [u8; 3] = [34, 38, 52];
const ACCENT: [u8; 3] = [64, 150, 230];

pub(super) fn draw_options(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;
    let dialog = layout.options_dialog();
    gfx.frame(canvas, frame::OPTIONS_DIALOG, dialog.x, dialog.y, s)?;
    fill(canvas, frect(layout.in_options(OPTIONS_TABLE)), BLACK)?;
    let menu = &viewer.menu;
    let asking = !matches!(viewer.screen, Screen::Options { ask: None });
    for i in 0..OPTIONS_ROWS {
        let index = menu.scroll + i;
        let Some(&item) = ITEMS.get(index) else {
            break;
        };
        let row = layout.options_row(i);
        if let Item::Heading(text) = item {
            gfx.text_in(canvas, fonts, text, px, GOLD, row, 0.0, 8 * s)?;
            let y = (row.y + row.h - 2 * s) as f32;
            canvas.set_draw_color(colour(DIM));
            canvas.draw_line(
                FPoint::new((row.x + 8 * s) as f32, y),
                FPoint::new((row.x + row.w - 8 * s) as f32, y),
            )?;
            continue;
        }
        let (mx, my) = viewer.mouse;
        if !asking && (menu.focus == Some(index) || row.contains(mx, my)) {
            fill(canvas, frect(row), HOVER)?;
        }
        gfx.text_in(canvas, fonts, item.label(), px, WHITE, row, 0.0, 20 * s)?;
        let c = control(row, s);
        match viewer.option_value(item) {
            Some(Value::Check(on)) => check_box(canvas, c, s, on)?,
            Some(Value::Choice(text, down, up)) => {
                canvas.set_draw_color(colour(DIM));
                canvas.draw_rect(frect(c))?;
                let arrow = Rect::new(c.x, c.y, 16 * s, c.h);
                let rgb = |on: bool| if on { WHITE } else { DIM };
                gfx.text_in(canvas, fonts, "<", px, rgb(down), arrow, 0.5, 0)?;
                let arrow = Rect::new(c.x + c.w - 16 * s, c.y, 16 * s, c.h);
                gfx.text_in(canvas, fonts, ">", px, rgb(up), arrow, 0.5, 0)?;
                gfx.text_in(canvas, fonts, &text, px, WHITE, c, 0.5, 0)?;
            }
            None => {}
        }
    }
    if ITEMS.len() > OPTIONS_ROWS {
        let t = layout.in_options(OPTIONS_TABLE);
        let track = Rect::new(t.x + t.w - 4 * s, t.y + 2 * s, 2 * s, t.h - 4 * s);
        fill(canvas, frect(track), DIM)?;
        let h = track.h * OPTIONS_ROWS as i32 / ITEMS.len() as i32;
        let y = track.y + (track.h - h) * menu.scroll as i32 / (ITEMS.len() - OPTIONS_ROWS) as i32;
        fill(canvas, frect(Rect::new(track.x, y, track.w, h)), GREY)?;
    }
    for button in layout.buttons(Screen::Options { ask: None }, 0) {
        if !matches!(button, Button::Chat | Button::Menu) {
            gfx.button(canvas, viewer, &layout, button, State::Normal, false)?;
        }
    }
    Ok(())
}

/// A rounded-looking check box at the right end of `c`, ticked when `on`.
fn check_box(canvas: &mut Canvas<Window>, c: Rect, s: i32, on: bool) -> Result {
    let size = 12 * s;
    let b = Rect::new(c.x + c.w - size - 2 * s, c.y + (c.h - size) / 2, size, size);
    if on {
        fill(canvas, frect(b), ACCENT)?;
        canvas.set_draw_color(colour(WHITE));
        let p = |x: i32, y: i32| FPoint::new((b.x + x * s) as f32, (b.y + y * s) as f32);
        for d in 0..s.max(1) {
            let o = d as f32;
            let off = |q: FPoint| FPoint::new(q.x, q.y + o);
            canvas.draw_line(off(p(3, 6)), off(p(5, 8)))?;
            canvas.draw_line(off(p(5, 8)), off(p(9, 3)))?;
        }
    } else {
        canvas.set_draw_color(colour(GREY));
        canvas.draw_rect(frect(b))?;
    }
    Ok(())
}

/// The OPTIONS dialog's YES / NO question, text at the top as the original.
pub(super) fn draw_question(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;
    canvas.set_blend_mode(BlendMode::Blend);
    canvas.set_draw_color(Color::RGBA(0, 0, 0, 120));
    canvas.fill_rect(frect(layout.viewport()))?;
    canvas.set_blend_mode(BlendMode::None);
    let dialog = layout.question_dialog();
    gfx.frame(canvas, frame::QUESTION, dialog.x, dialog.y, s)?;
    for button in layout.buttons(viewer.screen, 0) {
        if !matches!(button, Button::Chat | Button::Menu) {
            gfx.button(canvas, viewer, &layout, button, State::Normal, false)?;
        }
    }
    let r = layout.in_question(QUESTION_TEXT);
    let lines = ui::wrap(&viewer.menu.question, r.w as f32, |t| {
        fonts.measure(t, px, false)
    });
    let line = LINE * s;
    for (i, l) in lines.iter().enumerate() {
        let row = Rect::new(r.x, r.y + 4 * s + i as i32 * line, r.w, line);
        gfx.text_in(canvas, fonts, l, px, WHITE, row, 0.5, 0)?;
    }
    Ok(())
}
