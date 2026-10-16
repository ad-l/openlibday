//! Front end: the main menu, the NEW GAME dialog, the network game
//! dialogs (SESSIONS and the WAITING AREA) and the battle's chat.

use libday_view::minimap;
use libday_view::ui::{self, Rect, Screen, State, frame};
use sdl3::pixels::Color;
use sdl3::render::{BlendMode, Canvas};
use sdl3::video::Window;

use super::{Gfx, LINE, Result, TEXT_PX, WHITE, fill, frect};
use crate::Viewer;
use crate::front::{
    DIFFICULTY_MAX, InfoRow, NAME_BOX, SESSION_ADDRESS, SESSION_NAME, SLIDER, info_address,
    info_arrows, info_copy, info_row, info_rows, info_value,
};
use crate::lobby::{Chat, Mode, Setting};

/// Labels and notes in the dialogs' khaki.
const KHAKI: [u8; 3] = [214, 196, 132];
const GREY: [u8; 3] = [160, 160, 160];
/// SESSIONS: its black area; WAITING AREA: players, chat and chat line
/// boxes (dialog coordinates).
const SESSIONS_AREA: Rect = Rect::new(14, 40, 336, 217);
const LOBBY_PLAYERS: Rect = Rect::new(17, 70, 255, 123);
const LOBBY_CHAT: Rect = Rect::new(17, 224, 527, 129);
const LOBBY_LINE: Rect = Rect::new(15, 390, 427, 26);
/// How long chat lines stay over the battle.
const CHAT_SHOWN_MS: u128 = 10_000;
const CHAT_OVERLAY_LINES: usize = 6;

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

/// `text` with a blinking caret.
fn caret(viewer: &Viewer, text: &str) -> String {
    if (viewer.clock.now().as_millis() / 500).is_multiple_of(2) {
        format!("{text}_")
    } else {
        text.to_owned()
    }
}

/// A text box's dark fill and rim.
fn draw_box(canvas: &mut Canvas<Window>, r: Rect, s: i32, focused: bool) -> Result {
    fill(canvas, frect(r), [0, 0, 0])?;
    let rim = if focused { KHAKI } else { [90, 84, 60] };
    for edge in [
        Rect::new(r.x, r.y, r.w, s),
        Rect::new(r.x, r.bottom() - s, r.w, s),
        Rect::new(r.x, r.y, s, r.h),
        Rect::new(r.right() - s, r.y, s, r.h),
    ] {
        fill(canvas, frect(edge), rim)?;
    }
    Ok(())
}

/// Wrapped lines of `text` from `y` down in `r`; returns the next `y`.
#[allow(clippy::too_many_arguments)]
fn paragraph(
    canvas: &mut Canvas<Window>,
    gfx: &mut Gfx,
    viewer: &Viewer,
    text: &str,
    rgb: [u8; 3],
    r: Rect,
    y: i32,
    s: i32,
) -> Result<i32> {
    let px = TEXT_PX * s as f32;
    let fonts = &viewer.fonts;
    let mut y = y;
    for line in ui::wrap(text, r.w as f32, |t| fonts.measure(t, px, false)) {
        gfx.text_in(
            canvas,
            fonts,
            &line,
            px,
            rgb,
            Rect::new(r.x, y, r.w, LINE * s),
            0.0,
            0,
        )?;
        y += LINE * s;
    }
    Ok(y)
}

/// A chat line as shown: "name: text", or a notice alone.
fn chat_text(viewer: &Viewer, chat: &Chat) -> (String, [u8; 3]) {
    match chat.slot {
        Some(slot) => (
            format!("{}: {}", viewer.player_name(slot), chat.text),
            WHITE,
        ),
        None => (chat.text.clone(), KHAKI),
    }
}

pub(super) fn draw_sessions(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let layout = viewer.layout();
    let s = layout.scale;
    let px = TEXT_PX * s as f32;
    let dialog = layout.sessions_dialog();
    gfx.frame(canvas, frame::SESSIONS, dialog.x, dialog.y, s)?;
    buttons(canvas, gfx, viewer, Screen::Sessions)?;
    let area = layout.in_sessions(SESSIONS_AREA);
    let inner = Rect::new(area.x + 8 * s, area.y, area.w - 16 * s, area.h);
    paragraph(
        canvas,
        gfx,
        viewer,
        "HOST a game for others to join, or JOIN the game at an address.",
        KHAKI,
        inner,
        area.y + 8 * s,
        s,
    )?;
    let front = &viewer.front;
    for (label, field, text, focused) in [
        ("Your name", SESSION_NAME, &front.name, !front.focus_address),
        (
            "Host address",
            SESSION_ADDRESS,
            &front.address,
            front.focus_address,
        ),
    ] {
        let r = layout.in_sessions(field);
        let l = Rect::new(inner.x, r.y, r.x - inner.x, r.h);
        gfx.text_in(canvas, &viewer.fonts, label, px, KHAKI, l, 0.0, 0)?;
        draw_box(canvas, r, s, focused)?;
        let shown = if focused {
            caret(viewer, text)
        } else {
            text.clone()
        };
        gfx.text_in(canvas, &viewer.fonts, &shown, px, WHITE, r, 0.0, 4 * s)?;
    }
    let below = layout.in_sessions(SESSION_ADDRESS).bottom() + 10 * s;
    let note = format!(
        "Games are hosted on TCP port {}. For players outside your home network to join, forward that port on your router to this computer.",
        front.port
    );
    let y = paragraph(canvas, gfx, viewer, &note, GREY, inner, below, s)?;
    if let Some(status) = &front.status {
        paragraph(canvas, gfx, viewer, status, WHITE, inner, y + 8 * s, s)?;
    }
    Ok(())
}

pub(super) fn draw_lobby(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let layout = viewer.layout();
    let s = layout.scale;
    let px = TEXT_PX * s as f32;
    let fonts = &viewer.fonts;
    let dialog = layout.lobby_dialog();
    gfx.frame(canvas, frame::LOBBY, dialog.x, dialog.y, s)?;
    buttons(canvas, gfx, viewer, Screen::Lobby)?;
    let Some(online) = &viewer.net.online else {
        return Ok(());
    };

    // Players, with their colours.
    let players = layout.in_lobby(LOBBY_PLAYERS);
    for (row, (&slot, name)) in online.names.iter().enumerate() {
        let y = players.y + (6 + row as i32 * 20) * s;
        let swatch = Rect::new(players.x + 8 * s, y + 2 * s, 12 * s, 12 * s);
        fill(canvas, frect(swatch), minimap::owner_colour(slot))?;
        let mut text = name.clone();
        if slot == 1 {
            text.push_str(" (host)");
        }
        if slot == online.slot {
            text.push_str(" (you)");
        }
        let r = Rect::new(players.x + 28 * s, y, players.w - 34 * s, 16 * s);
        gfx.text_in(canvas, fonts, &text, px, WHITE, r, 0.0, 0)?;
    }

    // Game information: the addresses to give the others, then the
    // settings (the host steps them with the arrows).
    let campaign = online.settings.mode == Mode::Campaign;
    let (lan, wan) = viewer.game_addresses();
    for (i, row) in info_rows(campaign).into_iter().enumerate() {
        let r = layout.in_lobby(info_row(i));
        let value = layout.in_lobby(info_value(i));
        let label = Rect::new(r.x, r.y, value.x - r.x, r.h);
        let name = match row {
            InfoRow::Lan => "LAN",
            InfoRow::Wan => "WAN",
            InfoRow::Setting(Setting::Mode) => "Mode",
            InfoRow::Setting(Setting::Difficulty) => "Difficulty",
            InfoRow::Setting(Setting::TurnLimit) => "Turn limit",
        };
        gfx.text_in(canvas, fonts, name, px, KHAKI, label, 0.0, 0)?;
        match row {
            InfoRow::Lan | InfoRow::Wan => {
                let address = if row == InfoRow::Lan { &lan } else { &wan };
                let field = layout.in_lobby(info_address(i));
                draw_box(canvas, field, s, false)?;
                let (text, rgb) = if address.is_empty() {
                    ("Not available yet", GREY)
                } else {
                    (address.as_str(), WHITE)
                };
                gfx.text_in(canvas, fonts, text, px, rgb, field, 0.0, 4 * s)?;
                let copy = layout.in_lobby(info_copy(i));
                draw_box(canvas, copy, s, !address.is_empty())?;
                let rgb = if address.is_empty() { GREY } else { KHAKI };
                gfx.text_in(canvas, fonts, "Copy", px, rgb, copy, 0.5, 0)?;
            }
            InfoRow::Setting(setting) => {
                let settings = &online.settings;
                let text = match setting {
                    Setting::Mode => match settings.mode {
                        Mode::Skirmish => "Skirmish".to_owned(),
                        Mode::Campaign => "Campaign (soon)".to_owned(),
                    },
                    Setting::Difficulty => {
                        format!("{} of {DIFFICULTY_MAX}", settings.difficulty)
                    }
                    Setting::TurnLimit => match settings.turn_minutes {
                        0 => "Unlimited".to_owned(),
                        m => format!("{m} min"),
                    },
                };
                draw_box(canvas, value, s, false)?;
                if online.is_host() {
                    let (left, right) = info_arrows(i);
                    for (arrow, glyph) in [(left, "<"), (right, ">")] {
                        let a = layout.in_lobby(arrow);
                        draw_box(canvas, a, s, true)?;
                        gfx.text_in(canvas, fonts, glyph, px, KHAKI, a, 0.5, 0)?;
                    }
                }
                gfx.text_in(canvas, fonts, &text, px, WHITE, value, 0.5, 0)?;
            }
        }
    }

    // Chat, newest at the bottom.
    let chat = layout.in_lobby(LOBBY_CHAT);
    let inner = Rect::new(chat.x + 6 * s, chat.y, chat.w - 12 * s, chat.h);
    let mut lines = Vec::new();
    for c in &online.chat {
        let (text, rgb) = chat_text(viewer, c);
        for line in ui::wrap(&text, inner.w as f32, |t| fonts.measure(t, px, false)) {
            lines.push((line, rgb));
        }
    }
    let rows = ((chat.h - 8 * s) / (LINE * s)).max(1) as usize;
    let first = lines.len().saturating_sub(rows);
    for (i, (line, rgb)) in lines[first..].iter().enumerate() {
        let r = Rect::new(
            inner.x,
            chat.y + (4 + i as i32 * LINE) * s,
            inner.w,
            LINE * s,
        );
        gfx.text_in(canvas, fonts, line, px, *rgb, r, 0.0, 0)?;
    }
    let line = layout.in_lobby(LOBBY_LINE);
    gfx.text_in(
        canvas,
        fonts,
        &caret(viewer, &viewer.front.chat),
        px,
        WHITE,
        line,
        0.0,
        6 * s,
    )?;
    Ok(())
}

/// Recent chat over the battle's top-left, and the line being typed.
pub(super) fn draw_chat_overlay(
    canvas: &mut Canvas<Window>,
    gfx: &mut Gfx,
    viewer: &Viewer,
) -> Result {
    let Some(online) = &viewer.net.online else {
        return Ok(());
    };
    let layout = viewer.layout();
    let s = layout.scale;
    let px = TEXT_PX * s as f32;
    let now = viewer.clock.now();
    let chatting = viewer.front.chatting;
    let recent: Vec<&Chat> = online
        .chat
        .iter()
        .filter(|c| chatting || now.saturating_sub(c.at).as_millis() < CHAT_SHOWN_MS)
        .collect();
    let recent = &recent[recent.len().saturating_sub(CHAT_OVERLAY_LINES)..];
    if recent.is_empty() && !chatting {
        return Ok(());
    }
    let view = layout.viewport();
    let width = (view.w / 2).max(200 * s);
    let rows = recent.len() as i32 + i32::from(chatting);
    let back = Rect::new(view.x + 4 * s, view.y + 4 * s, width, (rows * LINE + 6) * s);
    canvas.set_blend_mode(BlendMode::Blend);
    canvas.set_draw_color(Color::RGBA(0, 0, 0, 150));
    canvas.fill_rect(frect(back))?;
    canvas.set_blend_mode(BlendMode::None);
    let mut y = back.y + 3 * s;
    for c in recent {
        let (text, rgb) = chat_text(viewer, c);
        let r = Rect::new(back.x, y, back.w, LINE * s);
        gfx.text_in(canvas, &viewer.fonts, &text, px, rgb, r, 0.0, 4 * s)?;
        y += LINE * s;
    }
    if chatting {
        let text = caret(viewer, &format!("Say: {}", viewer.front.chat));
        let r = Rect::new(back.x, y, back.w, LINE * s);
        gfx.text_in(canvas, &viewer.fonts, &text, px, KHAKI, r, 0.0, 4 * s)?;
    }
    Ok(())
}
