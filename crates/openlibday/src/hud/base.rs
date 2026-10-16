//! The base editor's sidebar: credits, minimap, the BUILDINGS/DEFENSES list.

use libday_view::ui::{BASE_ROWS, Button, Screen, State, frame};
use sdl3::render::Canvas;
use sdl3::video::Window;

use super::{
    Gfx, Result, TEXT_PX, WHITE, draw_credits, draw_minimap, draw_sidebar, frect, portrait,
};
use crate::Viewer;
use crate::base_editor::Mode;

pub(super) fn draw(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let Some(e) = &viewer.base_editor else {
        return Ok(());
    };
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;
    draw_sidebar(canvas, gfx, &layout, frame::BASE_SIDEBAR)?;
    for button in layout.buttons(Screen::BaseEditor, 0) {
        let on = match button {
            Button::BaseBuildings => !e.defenses,
            Button::BaseDefenses => e.defenses,
            Button::Recycle => e.mode == Mode::Recycle,
            Button::BaseMove => e.mode == Mode::Move,
            Button::BaseArmy => viewer.screen == Screen::Army,
            Button::Chat | Button::Menu => continue,
            _ => false,
        };
        gfx.button(canvas, viewer, &layout, button, State::Normal, on)?;
    }
    draw_credits(canvas, gfx, viewer)?;
    let list = viewer.base_list();
    for (row, &item) in list.iter().skip(e.scroll).take(BASE_ROWS).enumerate() {
        let r = layout.base_row(row);
        let image = item.image(&viewer.defs);
        if let Some(t) = portrait(gfx, viewer.listbox.as_ref(), &image) {
            let p = libday_view::ui::Rect::new(r.x + 2 * s, r.y + s, 38 * s, 38 * s - s);
            canvas.copy(t, None, frect(p))?;
        }
        let name = item.name(&viewer.defs);
        gfx.text(canvas, fonts, &name, px, WHITE, r.x + 44 * s, r.y + 3 * s)?;
        let cost = item.cost(&viewer.defs).to_string();
        gfx.text(canvas, fonts, &cost, px, WHITE, r.x + 44 * s, r.y + 21 * s)?;
        if e.selected == Some(item) && e.mode == Mode::Build {
            canvas.set_draw_color(sdl3::pixels::Color::RGB(255, 255, 255));
            canvas.draw_rect(frect(r))?;
        }
    }
    let (scene, art) = (&viewer.map.scene, &viewer.art);
    draw_minimap(canvas, gfx, viewer, scene, art)
}
