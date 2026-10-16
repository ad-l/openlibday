//! Skirmish screens: selector, technology, deployment and AI-turn sidebars,
//! and the "resources left" question.

use libday_data::defs::UnitType;
use libday_data::scenario::difficulty_label;
use libday_view::minimap;
use libday_view::ui::{
    self, AUTO_CELLS, Button, DEPLOY_ROWS, Field, QUESTION_TEXT, Rect, SKIRMISH_ROWS,
    SKIRMISH_TEXT, Screen, State, TECH_CELLS, TECH_COLUMNS, TECH_EMBLEM, TECH_NAME, TECH_POINTS,
    TECH_STAT_ROW, TECH_STATS, frame,
};
use sdl3::pixels::Color;
use sdl3::render::{BlendMode, Canvas};
use sdl3::video::Window;

use super::{
    BLACK, Gfx, LINE, Result, TEXT_PX, WHITE, draw_credits, draw_minimap, draw_sidebar, draw_turns,
    fill, frect, portrait,
};
use crate::Viewer;

const RED: [u8; 3] = [255, 0, 0];
const GREY: [u8; 3] = [160, 160, 160];

/// Draws `screen`'s buttons other than CHAT and MENU (drawn with the bar).
fn buttons(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer, screen: Screen) -> Result {
    let layout = viewer.layout();
    for button in layout.buttons(screen, 0) {
        if !matches!(button, Button::Chat | Button::Menu) {
            gfx.button(canvas, viewer, &layout, button, State::Normal, false)?;
        }
    }
    Ok(())
}

pub(super) fn draw_selector(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;
    let dialog = layout.skirmish_dialog();
    gfx.frame(canvas, frame::SKIRMISH_DIALOG, dialog.x, dialog.y, s)?;
    buttons(canvas, gfx, viewer, Screen::Skirmishes)?;
    let flow = &viewer.flow;
    for (i, entry) in flow
        .catalog
        .iter()
        .enumerate()
        .skip(flow.scroll)
        .take(SKIRMISH_ROWS)
    {
        let r = layout.skirmish_row(i - flow.scroll);
        let selected = i == flow.pick;
        if selected {
            fill(canvas, frect(r), WHITE)?;
        }
        let rgb = if selected { BLACK } else { WHITE };
        let label = format!("{} ( {} )", entry.title, difficulty_label(entry.points));
        gfx.text_in(canvas, fonts, &label, px, rgb, r, 0.0, 4 * s)?;
    }
    if let Some(entry) = flow.catalog.get(flow.pick) {
        let r = layout.in_skirmish(SKIRMISH_TEXT);
        let lines = ui::wrap(&entry.description, r.w as f32, |t| {
            fonts.measure(t, px, false)
        });
        canvas.set_clip_rect(sdl3::rect::Rect::new(r.x, r.y, r.w as u32, r.h as u32));
        for (i, line) in lines.iter().enumerate() {
            let y = r.y + i as i32 * LINE * s;
            gfx.text(canvas, fonts, line, px, WHITE, r.x, y)?;
        }
        canvas.set_clip_rect(None);
    }
    Ok(())
}

/// How a technology grid cell is shown.
struct Look {
    /// Outlined.
    selected: bool,
    /// Portrait at half brightness (fully upgraded).
    dim: bool,
}

/// A technology grid cell: portrait, then name and more lines beside it.
fn cell(
    canvas: &mut Canvas<Window>,
    gfx: &mut Gfx,
    viewer: &Viewer,
    r: Rect,
    image: &str,
    lines: &[String],
    look: Look,
) -> Result {
    let s = viewer.layout().scale;
    let px = TEXT_PX * s as f32;
    if let Some(t) = portrait(gfx, viewer.listbox.as_ref(), image) {
        let side = 38 * s;
        let to = Rect::new(r.x + 2 * s, r.y + 2 * s, side, side);
        canvas.copy(t, None, frect(to))?;
        if look.dim {
            // Fully upgraded units' portraits are shown at half brightness.
            canvas.set_blend_mode(BlendMode::Blend);
            canvas.set_draw_color(Color::RGBA(0, 0, 0, 128));
            canvas.fill_rect(frect(to))?;
            canvas.set_blend_mode(BlendMode::None);
        }
    }
    for (i, line) in lines.iter().enumerate() {
        let y = r.y + s + i as i32 * 14 * s;
        gfx.text(canvas, &viewer.fonts, line, px, WHITE, r.x + 42 * s, y)?;
    }
    if look.selected {
        canvas.set_draw_color(Color::RGB(255, 255, 255));
        canvas.draw_rect(frect(Rect::new(r.x, r.y, r.w - 2 * s, r.h - s)))?;
    }
    Ok(())
}

fn avg_damage(k: &UnitType) -> i64 {
    k.damage_base + k.damage_random / 2
}

fn rate_of_fire(k: &UnitType) -> i64 {
    k.action_points / k.shoot_action_points.max(1)
}

pub(super) fn draw_tech(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let read_only = viewer.screen == (Screen::Tech { read_only: true });
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;
    let dialog = layout.tech_dialog();
    gfx.frame(canvas, frame::TECH_DIALOG, dialog.x, dialog.y, s)?;
    let (f, ex, ey) = TECH_EMBLEM;
    let e = layout.in_tech(Rect::new(ex, ey, 0, 0));
    gfx.frame(canvas, f, e.x, e.y, s)?;
    buttons(canvas, gfx, viewer, viewer.screen)?;
    let Some(setup) = &viewer.flow.setup else {
        return Ok(());
    };
    let first = setup.tech_scroll * TECH_COLUMNS;
    for i in first..setup.units.len().min(first + TECH_CELLS) {
        let Some((kind, tl)) = viewer.tech_unit(i) else {
            continue;
        };
        let mut lines = vec![kind.name.clone()];
        match viewer.next_tech_cost(i) {
            Some(cost) => lines.extend([format!("Next tech: {}", tl + 2), format!("Cost: {cost}")]),
            None => lines.push("Maximum level".into()),
        }
        let maxed = lines.len() == 2;
        let r = layout.tech_cell(i - first);
        cell(
            canvas,
            gfx,
            viewer,
            r,
            &kind.image,
            &lines,
            Look {
                selected: i == setup.cell,
                dim: maxed,
            },
        )?;
    }
    let first = setup.auto_scroll * ui::AUTO_COLUMNS;
    for i in first..setup.automated.len().min(first + AUTO_CELLS) {
        if let Some(a) = viewer.defs.automated.get(&setup.automated[i]) {
            let lines = [a.name.clone(), "Maximum level".into()];
            let r = layout.tech_automated_cell(i - first);
            let selected = i == setup.auto_cell;
            // Automated units are always at their maximum level.
            let look = Look {
                selected,
                dim: true,
            };
            cell(canvas, gfx, viewer, r, &a.image, &lines, look)?;
        }
    }
    if let Some((kind, tl)) = viewer.tech_unit(setup.cell) {
        // The next level's values, red where they improve on the current one.
        let shown = match viewer.next_tech_cost(setup.cell) {
            Some(_) => viewer.defs.unit_tech(kind.id, tl + 1).unwrap_or(kind),
            None => kind,
        };
        let r = layout.in_tech(Rect::new(TECH_NAME.0, TECH_NAME.1, 0, 0));
        gfx.text(canvas, fonts, &kind.name, px, WHITE, r.x, r.y)?;
        let (kind, base) = (shown, kind);
        let stats = [
            ("ARMOR", kind.armor, base.armor),
            ("ACTION POINTS", kind.action_points, base.action_points),
            if kind.anti_air {
                ("DAMAGE AA", kind.damage_aa, base.damage_aa)
            } else {
                ("AVG DAMAGE", avg_damage(kind), avg_damage(base))
            },
            ("RANGE", kind.range, base.range),
            ("ROF", rate_of_fire(kind), rate_of_fire(base)),
        ];
        let (lx, vx, y) = TECH_STATS;
        for (row, (label, value, was)) in stats.into_iter().enumerate() {
            let y = y + row as i32 * TECH_STAT_ROW;
            let l = layout.in_tech(Rect::new(lx, y, 0, 0));
            let v = layout.in_tech(Rect::new(vx, y, 0, 0));
            gfx.text(canvas, fonts, label, px, WHITE, l.x, l.y)?;
            let rgb = if value == was { WHITE } else { RED };
            gfx.text(canvas, fonts, &value.to_string(), px, rgb, v.x, v.y)?;
        }
    }
    let points = if read_only { 0 } else { setup.points };
    let r = layout.in_tech(TECH_POINTS);
    gfx.text_in(canvas, fonts, &points.to_string(), px, WHITE, r, 0.5, 0)
}

pub(super) fn draw_deploy_sidebar(
    canvas: &mut Canvas<Window>,
    gfx: &mut Gfx,
    viewer: &Viewer,
) -> Result {
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;
    draw_sidebar(canvas, gfx, &layout, frame::DEPLOY_SIDEBAR)?;
    buttons(canvas, gfx, viewer, Screen::Deploy)?;
    draw_credits(canvas, gfx, viewer)?;
    if let Some(setup) = &viewer.flow.setup {
        let x = layout.in_side(Rect::new(58, 0, 0, 0)).x;
        for (row, &i) in viewer
            .deploy_list()
            .iter()
            .enumerate()
            .skip(setup.scroll)
            .take(DEPLOY_ROWS)
        {
            let Some((kind, _)) = viewer.tech_unit(i) else {
                continue;
            };
            let r = layout.deploy_row(row - setup.scroll);
            if let Some(t) = portrait(gfx, viewer.listbox.as_ref(), &kind.image) {
                let side = 38 * s;
                canvas.copy(t, None, frect(Rect::new(r.x + 2 * s, r.y, side, side)))?;
            }
            gfx.text(canvas, fonts, &kind.name, px, WHITE, x, r.y + 2 * s)?;
            let cost = viewer.price(kind.cost).to_string();
            gfx.text(canvas, fonts, &cost, px, WHITE, x, r.y + 21 * s)?;
            if row == setup.row {
                canvas.set_draw_color(Color::RGB(255, 255, 255));
                canvas.draw_rect(frect(Rect::new(r.x, r.y - s, r.w, r.h)))?;
            }
        }
    }
    draw_minimap(canvas, gfx, viewer, &viewer.map.scene, &viewer.art)
}

/// The AUTOMATED UNITS sidebar: the supports with their prices, the armed
/// one outlined, those the credits cannot pay greyed.
pub(super) fn draw_automated_sidebar(
    canvas: &mut Canvas<Window>,
    gfx: &mut Gfx,
    viewer: &Viewer,
) -> Result {
    const GREY: [u8; 3] = [128, 128, 128];
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;
    draw_sidebar(canvas, gfx, &layout, frame::AUTOMATED_SIDEBAR)?;
    buttons(canvas, gfx, viewer, Screen::Automated)?;
    draw_credits(canvas, gfx, viewer)?;
    draw_turns(canvas, gfx, viewer, Field::Turn)?;
    let x = layout.in_side(Rect::new(58, 0, 0, 0)).x;
    let first = viewer.flow.support_scroll;
    let credits = viewer.credits();
    for (row, id) in viewer
        .support_list()
        .into_iter()
        .enumerate()
        .skip(first)
        .take(DEPLOY_ROWS)
    {
        let Some(a) = viewer.defs.automated.get(&id) else {
            continue;
        };
        let r = layout.deploy_row(row - first);
        if let Some(t) = portrait(gfx, viewer.listbox.as_ref(), &a.image) {
            let side = 38 * s;
            canvas.copy(t, None, frect(Rect::new(r.x + 2 * s, r.y, side, side)))?;
        }
        let rgb = if a.cost <= credits { WHITE } else { GREY };
        gfx.text(canvas, fonts, &a.name, px, rgb, x, r.y + 2 * s)?;
        gfx.text(canvas, fonts, &a.cost.to_string(), px, rgb, x, r.y + 21 * s)?;
        if viewer.flow.support_row == Some(row) {
            canvas.set_draw_color(Color::RGB(255, 255, 255));
            canvas.draw_rect(frect(Rect::new(r.x, r.y - s, r.w, r.h)))?;
        }
    }
    draw_minimap(canvas, gfx, viewer, &viewer.map.scene, &viewer.art)
}

pub(super) fn draw_ai_sidebar(
    canvas: &mut Canvas<Window>,
    gfx: &mut Gfx,
    viewer: &Viewer,
) -> Result {
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;
    draw_sidebar(canvas, gfx, &layout, frame::AI_SIDEBAR)?;
    buttons(canvas, gfx, viewer, Screen::AiTurn)?;
    draw_credits(canvas, gfx, viewer)?;
    draw_turns(canvas, gfx, viewer, Field::Turn)?;
    for (row, owner) in viewer.seats.overview().into_iter().enumerate() {
        let r = layout.ai_row(row);
        gfx.frame(canvas, minimap::swatch_frame(owner), r.x, r.y, s)?;
        let name = Rect::new(r.x + 22 * s, r.y, 100 * s, r.h);
        let rgb = if owner == viewer.map.battle.clock.player {
            WHITE
        } else {
            GREY
        };
        let text = viewer.player_name(owner);
        gfx.text_in(canvas, fonts, &text, px, rgb, name, 0.0, 0)?;
    }
    draw_minimap(canvas, gfx, viewer, &viewer.map.scene, &viewer.art)
}

/// "Resources left" question over the deployment screen.
pub(super) fn draw_confirm(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let layout = viewer.layout();
    let (s, fonts) = (layout.scale, &viewer.fonts);
    let px = TEXT_PX * s as f32;
    canvas.set_blend_mode(BlendMode::Blend);
    canvas.set_draw_color(Color::RGBA(0, 0, 0, 120));
    canvas.fill_rect(frect(layout.viewport()))?;
    canvas.set_blend_mode(BlendMode::None);
    let dialog = layout.question_dialog();
    gfx.frame(canvas, frame::QUESTION, dialog.x, dialog.y, s)?;
    buttons(canvas, gfx, viewer, Screen::Confirm)?;
    let r = layout.in_question(QUESTION_TEXT);
    let text = "You still have some resources left. Do you want to continue anyway?";
    let lines = ui::wrap(text, r.w as f32, |t| fonts.measure(t, px, false));
    let line = LINE * s;
    let top = r.y + (r.h - line * lines.len() as i32) / 2;
    for (i, l) in lines.iter().enumerate() {
        let row = Rect::new(r.x, top + i as i32 * line, r.w, line);
        gfx.text_in(canvas, fonts, l, px, WHITE, row, 0.5, 0)?;
    }
    Ok(())
}
