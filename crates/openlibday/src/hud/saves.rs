//! The LOAD GAME / SAVE GAME dialogs: the original frame and buttons, a
//! black scrolling list of saved games over its ten slots.

use libday_view::text::Fonts;
use libday_view::ui::{Button, Rect, SAVES_LIST, SAVES_ROWS, SAVES_TRACK, Screen, State, frame};
use sdl3::render::Canvas;
use sdl3::video::Window;

use super::{BLACK, Gfx, Result, TEXT_PX, WHITE, colour, fill, frect, texture};
use crate::Viewer;
use crate::saves::{self, Confirm, Entry, Kind, THUMB};

const GOLD: [u8; 3] = [232, 188, 80];
const GREY: [u8; 3] = [150, 150, 160];
const DIM: [u8; 3] = [60, 60, 66];
const HOVER: [u8; 3] = [34, 38, 52];
const SELECTED: [u8; 3] = [44, 60, 96];

/// `text` cut to fit `width` pixels.
fn fit(fonts: &Fonts, text: &str, px: f32, width: i32) -> String {
    if fonts.measure(text, px, false) <= width as f32 {
        return text.to_owned();
    }
    let mut cut: String = text.to_owned();
    while !cut.is_empty() && fonts.measure(&format!("{cut}..."), px, false) > width as f32 {
        cut.pop();
    }
    format!("{}...", cut.trim_end())
}

/// A row's three lines.
fn lines(entry: Option<&Entry>, confirm: Option<Confirm>, row: usize) -> [(String, [u8; 3]); 3] {
    let Some(e) = entry else {
        return [
            ("New save".into(), GOLD),
            ("Save the game in a new slot".into(), WHITE),
            (String::new(), GREY),
        ];
    };
    let s = &e.summary;
    let (first, second) = match s.kind {
        Kind::Campaign => {
            let mut first = if s.name.is_empty() {
                s.continent.clone()
            } else {
                format!("{} - {}", s.name, s.continent)
            };
            if !s.title.is_empty() {
                first = format!("{first}, turn {}", s.turn);
            }
            let second = format!(
                "{}/{} provinces, difficulty {}/10",
                s.captured, s.total, s.difficulty
            );
            (first, second)
        }
        Kind::Skirmish => (s.title.clone(), format!("Skirmish, turn {}", s.turn)),
        Kind::Map => (s.title.clone(), format!("Map, turn {}", s.turn)),
    };
    let players = if s.players == 1 {
        "1 player".to_owned()
    } else {
        format!("{} players", s.players)
    };
    let third = match confirm {
        Some(Confirm::Overwrite(r)) if r == row => ("SAVE again to overwrite".into(), GOLD),
        Some(Confirm::Delete(r)) if r == row => ("DELETE again to delete".into(), GOLD),
        _ if e.autosave => (
            format!("Autosave {}  {players}", saves::date(e.created)),
            GREY,
        ),
        _ => (format!("{}  {players}", saves::date(e.created)), GREY),
    };
    [(first, GOLD), (second, WHITE), third]
}

pub(super) fn draw(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;
    let saving = matches!(viewer.screen, Screen::Saves { saving: true });
    let dialog = layout.saves_dialog();
    let art = if saving {
        frame::SAVE_DIALOG
    } else {
        frame::LOAD_DIALOG
    };
    gfx.frame(canvas, art, dialog.x, dialog.y, s)?;
    let list = layout.in_saves(SAVES_LIST);
    fill(canvas, frect(list), BLACK)?;
    let ui = &viewer.saves_ui;
    let rows = viewer.saves_rows();
    if rows == 0 {
        gfx.text_in(canvas, fonts, "No saved games", px, GREY, list, 0.5, 0)?;
    }
    let (mx, my) = viewer.mouse;
    for i in 0..SAVES_ROWS {
        let index = ui.scroll + i;
        if index >= rows {
            break;
        }
        let row = layout.saves_row(i);
        if ui.selected == Some(index) {
            fill(canvas, frect(row), SELECTED)?;
        } else if row.contains(mx, my) {
            fill(canvas, frect(row), HOVER)?;
        }
        let entry = viewer.saves_entry(index);
        let side = THUMB as i32 * s;
        let thumb = Rect::new(row.x + 2 * s, row.y + 2 * s, side, side);
        match entry {
            Some(e) if e.thumbnail.is_some() => {
                let key = (e.path.clone(), e.created);
                if !gfx.saves.contains_key(&key)
                    && let Some(t) = e.thumbnail.as_ref().and_then(|i| texture(gfx.creator, i))
                {
                    gfx.saves.insert(key.clone(), t);
                }
                if let Some(t) = gfx.saves.get(&key) {
                    canvas.copy(t, None, frect(thumb))?;
                }
            }
            _ => {
                fill(canvas, frect(thumb), [16, 16, 20])?;
                let mark = if entry.is_none() { "+" } else { "?" };
                gfx.text_in(canvas, fonts, mark, px * 2.0, GREY, thumb, 0.5, 0)?;
            }
        }
        canvas.set_draw_color(colour(DIM));
        canvas.draw_rect(frect(thumb))?;
        let x = thumb.x + side + 6 * s;
        let w = row.x + row.w - x - 2 * s;
        for (k, (text, rgb)) in lines(entry, ui.confirm, index).iter().enumerate() {
            let line = Rect::new(x, row.y + 4 * s + k as i32 * 15 * s, w, 14 * s);
            let text = fit(fonts, text, px, w);
            gfx.text_in(canvas, fonts, &text, px, *rgb, line, 0.0, 0)?;
        }
    }
    if rows > SAVES_ROWS {
        let track = Rect::new(
            list.x + list.w - (SAVES_TRACK - 2) * s,
            list.y + 2 * s,
            2 * s,
            list.h - 4 * s,
        );
        fill(canvas, frect(track), DIM)?;
        let h = track.h * SAVES_ROWS as i32 / rows as i32;
        let y = track.y + (track.h - h) * ui.scroll as i32 / (rows - SAVES_ROWS) as i32;
        fill(canvas, frect(Rect::new(track.x, y, track.w, h)), GREY)?;
    }
    for button in layout.buttons(viewer.screen, 0) {
        if !matches!(button, Button::Chat | Button::Menu) {
            gfx.button(canvas, viewer, &layout, button, State::Normal, false)?;
        }
    }
    Ok(())
}
