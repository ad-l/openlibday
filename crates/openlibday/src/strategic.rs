//! The continent (strategic) overview: the province map, choosing the
//! province to attack, and INTELLIGENCE / ATTACK / BACK.

use std::time::Duration;

use libday_data::Dir;
use libday_view::continent::ContinentMap;
use libday_view::ui::{Button, Screen};

use crate::Viewer;

/// String ids (`LIBDAYEN.DLL`).
const TOO_FAR: u32 = 802;
const NEED_UPLINK: u32 = 803;
const CREDITS: u32 = 865;
const VICTORIOUS: u32 = 883;
const CHOOSE: u32 = 884;
/// Satellite Uplink: INTELLIGENCE needs one in the base.
const UPLINK: i64 = 137;

pub struct Strategic {
    pub map: ContinentMap,
    pub selected: Option<i64>,
    /// Sidebar text paragraphs.
    pub text: Vec<String>,
    /// After a won province: CONTINUE replaces BACK.
    pub victory: bool,
    /// Clock time when the screen opened (the sea's animation start).
    pub started: Duration,
}

impl Viewer {
    fn string(&self, id: u32, fallback: &str) -> String {
        self.game
            .strings()
            .ok()
            .and_then(|s| s.get(&id).cloned())
            .unwrap_or_else(|| fallback.into())
    }

    /// Opens the continent overview of the campaign's continent.
    pub(crate) fn open_strategic(&mut self) {
        self.flow = crate::skirmish::Flow::new();
        let Some(session) = &self.campaign else {
            return;
        };
        let Some(c) = session.continent() else {
            return;
        };
        let anim = &c.map_animation;
        let provinces: Vec<i64> = c.provinces.iter().map(|p| p.number).collect();
        let map = self
            .game
            .read(Dir::ColAnms, &format!("{}.SMK", anim.file))
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                ContinentMap::from_smacker(
                    &bytes,
                    anim.first_frame.max(0) as usize,
                    anim.frames.max(1) as usize,
                    &provinces,
                )
                .map_err(|e| e.to_string())
            });
        let map = match map {
            Ok(map) => map,
            Err(e) => {
                self.error = Some(format!("{}.SMK: {e}", anim.file));
                return;
            }
        };
        self.strategic = Some(Strategic {
            map,
            selected: None,
            text: vec![self.string(CHOOSE, "Choose the province you wish to attack.")],
            victory: false,
            started: self.clock.now(),
        });
        self.screen = self.home();
    }

    /// After a won province: "You are victorious" and CONTINUE.
    pub(crate) fn strategic_victory(&mut self) {
        let text = self.string(VICTORIOUS, "You are victorious. Hit the continue button.");
        if let Some(st) = &mut self.strategic {
            st.victory = true;
            st.text = vec![text];
        }
        self.screen = self.home();
    }

    /// Top-left of the province map and the scale, in output pixels.
    pub(crate) fn strategic_origin(&self) -> Option<(i32, i32, i32)> {
        let st = self.strategic.as_ref()?;
        let layout = self.layout();
        let (v, s) = (layout.viewport(), layout.scale);
        let w = st.map.width as i32 * s;
        // Top-aligned as in the original (its first row is a grey rule).
        Some((v.x + (v.w - w) / 2, v.y, s))
    }

    /// A click on the map at output pixel `(x, y)` selects a province.
    pub(crate) fn strategic_click(&mut self, x: i32, y: i32) {
        let Some((x0, y0, s)) = self.strategic_origin() else {
            return;
        };
        if self.strategic.as_ref().is_some_and(|st| st.victory) {
            return;
        }
        let (mx, my) = ((x - x0).div_euclid(s), (y - y0).div_euclid(s));
        let hit = self.strategic.as_ref().and_then(|st| st.map.hit(mx, my));
        self.select_province(hit);
    }

    /// Selects province `n` and describes it.
    pub(crate) fn select_province(&mut self, n: Option<i64>) {
        let text = match n.and_then(|n| self.province_text(n)) {
            Some(text) => text,
            None => vec![self.string(CHOOSE, "Choose the province you wish to attack.")],
        };
        if let Some(st) = &mut self.strategic {
            st.selected = n;
            st.text = text;
        }
    }

    /// Name, story and objective, and the credits capturing it pays.
    fn province_text(&self, n: i64) -> Option<Vec<String>> {
        let session = self.campaign.as_ref()?;
        let p = session.continent()?.province(n)?;
        let mut text = vec![p.name.en.clone()];
        text.extend(
            p.description
                .en
                .split("##")
                .take(2)
                .map(|t| t.trim().to_owned()),
        );
        text.push(String::new());
        let credits = session.state.reward(p.credits);
        text.push(
            self.string(CREDITS, "Credits: %1!d!")
                .replace("%1!d!", &credits.to_string()),
        );
        Some(text)
    }

    /// Whether province `n` can be attacked: not captured, next to a
    /// captured one.
    fn attackable(&self, n: i64) -> bool {
        let Some(session) = &self.campaign else {
            return false;
        };
        let Some(c) = session.continent() else {
            return false;
        };
        let captured = |m: i64| session.state.province(m).is_some_and(|p| p.captured);
        !captured(n) && c.neighbours(n).into_iter().any(captured)
    }

    fn strategic_message(&mut self, text: String) {
        if let Some(st) = &mut self.strategic {
            st.text = vec![text];
        }
    }

    pub(crate) fn strategic_enabled(&self, button: Button) -> bool {
        let Some(st) = &self.strategic else {
            return false;
        };
        match button {
            Button::Intelligence | Button::Attack => !st.victory && st.selected.is_some(),
            _ => true,
        }
    }

    pub(crate) fn strategic_activate(&mut self, button: Button) {
        let Some(n) = self.strategic.as_ref().and_then(|st| st.selected) else {
            if matches!(button, Button::StrategicBack | Button::StrategicContinue) {
                self.strategic_back();
            }
            return;
        };
        match button {
            Button::Attack => self.strategic_attack(n),
            Button::Intelligence => self.intelligence(n),
            Button::StrategicBack | Button::StrategicContinue => self.strategic_back(),
            _ => {}
        }
    }

    pub(crate) fn strategic_attack(&mut self, n: i64) {
        if !self.attackable(n) {
            let text = self.string(TOO_FAR, "Province is too far.");
            self.strategic_message(text);
            return;
        }
        self.begin_tactic(n);
    }

    /// INTELLIGENCE: the province's hint, with a Satellite Uplink.
    fn intelligence(&mut self, n: i64) {
        let uplink = self.campaign.as_ref().is_some_and(|c| {
            c.state
                .base
                .as_ref()
                .is_some_and(|b| b.buildings.iter().any(|b| b.kind == UPLINK))
        });
        let text = if uplink {
            self.campaign
                .as_ref()
                .and_then(|c| c.continent()?.province(n))
                .map(|p| p.objective.en.clone())
                .unwrap_or_default()
        } else {
            self.string(
                NEED_UPLINK,
                "You must build a Satellite Uplink to see more information.",
            )
        };
        self.strategic_message(text);
    }

    /// BACK / CONTINUE lead to base building (not implemented yet).
    fn strategic_back(&mut self) {
        if let Some(st) = &mut self.strategic {
            st.victory = false;
            st.selected = None;
            st.text = vec!["Base building is not available yet.".into()];
        }
        self.screen = self.home();
    }

    /// Keys of the continent overview.
    pub(crate) fn strategic_key(&mut self, key: sdl3::keyboard::Keycode) {
        use sdl3::keyboard::Keycode;
        match key {
            Keycode::Escape => self.open_menu(),
            Keycode::Return | Keycode::KpEnter => {
                let victory = self.strategic.as_ref().is_some_and(|st| st.victory);
                let button = if victory {
                    Button::StrategicContinue
                } else {
                    Button::Attack
                };
                if self.strategic_enabled(button) {
                    self.strategic_activate(button);
                }
            }
            _ => {}
        }
    }

    /// The overview's screen.
    pub(crate) fn strategic_screen(&self) -> Option<Screen> {
        self.strategic.as_ref().map(|st| Screen::Strategic {
            victory: st.victory,
        })
    }
}
