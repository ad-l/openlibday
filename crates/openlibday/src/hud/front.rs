//! Front end: the main menu and the NEW GAME dialog.

use libday_view::ui::{Rect, Screen, State, frame};
use sdl3::pixels::Color;
use sdl3::render::{BlendMode, Canvas};
use sdl3::video::Window;

use super::{Gfx, Result, TEXT_PX, WHITE, fill, frect};
use crate::Viewer;
use crate::front::{DIFFICULTY_MAX, NAME_BOX, SLIDER};

fn buttons(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer, screen: Screen) -> Result {
    let layout = viewer.layout();
    for button in layout.buttons(screen, 0) {
        gfx.button(canvas, viewer, &layout, button, State::Normal, false)?;
    }
    Ok(())
}

pub(super) fn draw_main_menu(
    canvas: &mut Canvas<Window>,
    gfx: &mut Gfx,
    viewer: &Viewer,
) -> Result {
    let layout = viewer.layout();
    let panel = layout.main_menu();
    gfx.frame(canvas, frame::MAIN_MENU, panel.x, panel.y, layout.scale)?;
    buttons(canvas, gfx, viewer, Screen::MainMenu)
}

/// Left edge of the slider thumb's bright face at difficulty `d`.
pub(crate) fn thumb_x(d: u8) -> i32 {
    // Measured on the original: 44 at 0, 119 at 4, steps of ~18.9.
    44 + (d as i32 * 189) / 10
}

pub(super) fn draw_new_game(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let layout = viewer.layout();
    let s = layout.scale;
    let dialog = layout.new_game_dialog();
    gfx.frame(canvas, frame::NEW_GAME, dialog.x, dialog.y, s)?;
    buttons(canvas, gfx, viewer, Screen::NewGame)?;

    // The player's name, with a caret while typing.
    let field = layout.in_new_game(NAME_BOX);
    let blink = (viewer.clock.now().as_millis() / 500).is_multiple_of(2);
    let name = if blink {
        format!("{}_", viewer.front.name)
    } else {
        viewer.front.name.clone()
    };
    gfx.text_in(
        canvas,
        &viewer.fonts,
        &name,
        TEXT_PX * s as f32,
        WHITE,
        field,
        0.0,
        4 * s,
    )?;

    // Slider track: a dark translucent band with a dark rim.
    let track = layout.in_new_game(SLIDER);
    canvas.set_blend_mode(BlendMode::Blend);
    canvas.set_draw_color(Color::RGBA(72, 72, 70, 185));
    canvas.fill_rect(frect(track))?;
    canvas.set_blend_mode(BlendMode::None);
    let rim = [56, 56, 55];
    fill(canvas, frect(Rect::new(track.x, track.y, track.w, s)), rim)?;
    fill(
        canvas,
        frect(Rect::new(track.x, track.bottom() - s, track.w, s)),
        rim,
    )?;
    fill(canvas, frect(Rect::new(track.x, track.y, s, track.h)), rim)?;

    // Thumb: khaki bevel, dark brown ring, light rim, bright face.
    let x = thumb_x(viewer.front.difficulty.min(DIFFICULTY_MAX));
    for (dx, dy, w, h, rgb) in [
        (-4, 115, 17, 15, [164, 140, 110]),
        (-2, 117, 13, 11, [113, 83, 55]),
        (-1, 118, 11, 9, [208, 198, 139]),
        (0, 119, 9, 7, [247, 255, 183]),
    ] {
        let r = layout.in_new_game(Rect::new(x + dx, dy, w, h));
        fill(canvas, frect(r), rgb)?;
    }
    Ok(())
}
