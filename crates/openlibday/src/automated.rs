//! Automated units (off-map supports): the AUTOMATED UNITS sidebar, where a
//! support bought with the player's credits is aimed at a tile, its
//! full-screen animation with the anti-aircraft flak, and the supports a
//! scenario has the AI launch on given turns. The rules are in
//! [`libday_view::support`].

use libday_data::defs::{AutomatedCategory, AutomatedType, auto_prop};
use libday_data::scenario::Scenario;
use libday_view::fire::Target;
use libday_view::shot::{self, Anim};
use libday_view::support::{self, Leg};
use libday_view::tile_center;
use libday_view::ui::{DEPLOY_ROWS, Screen};

use crate::Viewer;
use crate::combat::{Explosion, Shot};
use crate::skirmish::{AI, PLAYER, Phase};

/// Frames between two flak bursts (`0x41bc04` staggers the shooters).
const FLAK_STEP: usize = 3;

impl Viewer {
    /// Support types the player may still call in, in list order.
    pub(crate) fn support_list(&self) -> Vec<i64> {
        let Some(setup) = &self.flow.setup else {
            return Vec::new();
        };
        setup
            .automated
            .iter()
            .copied()
            .filter(|id| {
                self.defs
                    .automated
                    .get(id)
                    .is_some_and(|a| !(a.deploy_once && self.flow.used_once.contains(id)))
            })
            .collect()
    }

    /// The list row of the support with this id or (case-insensitive) name.
    pub(crate) fn support_row_named(&self, name: &str) -> Option<usize> {
        self.support_list().iter().position(|id| {
            id.to_string() == name
                || self
                    .defs
                    .automated
                    .get(id)
                    .is_some_and(|a| a.name.eq_ignore_ascii_case(name))
        })
    }

    /// Whether AUTOMATED UNITS opens the support sidebar.
    pub(crate) fn can_support(&self) -> bool {
        self.flow.phase == Phase::Player
            && !self.flow.reinforcing
            && self.flow.setup.is_some()
            && !self.busy()
    }

    pub(crate) fn open_supports(&mut self) {
        if !self.can_support() {
            return;
        }
        self.preview = None;
        self.lock = None;
        self.fire_mode = None;
        self.flow.support_row = None;
        self.flow.pickup = None;
        self.flow.message = None;
        self.screen = Screen::Automated;
    }

    /// Leaves the support sidebar, disarming the chosen support.
    pub(crate) fn close_supports(&mut self) {
        self.flow.support_row = None;
        self.flow.pickup = None;
        if self.screen == Screen::Automated {
            self.screen = self.home();
        }
    }

    /// The armed support, if any.
    pub(crate) fn armed_support(&self) -> Option<&AutomatedType> {
        let id = *self.support_list().get(self.flow.support_row?)?;
        self.defs.automated.get(&id)
    }

    /// Clicking row `i` of the list arms that support, if affordable.
    pub(crate) fn support_select(&mut self, i: usize) {
        let list = self.support_list();
        let Some(a) = list.get(i).and_then(|id| self.defs.automated.get(id)) else {
            return;
        };
        self.flow.pickup = None;
        if self.credits() < a.cost {
            self.flow.support_row = None;
            self.flow.message = Some("Need more tactical resource".into());
            return;
        }
        self.flow.message = Some(a.explain.clone());
        self.flow.support_row = Some(i);
    }

    pub(crate) fn support_scroll_ok(&self, step: isize) -> bool {
        let last = self.support_list().len().saturating_sub(DEPLOY_ROWS);
        let to = self.flow.support_scroll as isize + step;
        to >= 0 && to as usize <= last && step != 0
    }

    pub(crate) fn support_scroll(&mut self, step: isize) {
        if self.support_scroll_ok(step) {
            self.flow.support_scroll = self.flow.support_scroll.saturating_add_signed(step);
        }
    }

    /// A map click with a support armed: its target (for a transport, the
    /// unit to lift, then where to set it down).
    pub(crate) fn support_click(&mut self, x: i64, y: i64) {
        let Some(a) = self.armed_support().cloned() else {
            return;
        };
        let scenario = &self.map.battle.scenario;
        let transport = a.category() == Some(AutomatedCategory::Transport);
        let (at, dest) = match self.flow.pickup {
            Some(p) if transport => (p, Some((x, y))),
            _ => ((x, y), None),
        };
        if !support::valid_target(scenario, &self.defs, &a, PLAYER, at, dest) {
            self.flow.message = Some("Invalid target".into());
            return;
        }
        if transport && dest.is_none() {
            self.flow.pickup = Some(at);
            let prompt = a
                .infos
                .get(&auto_prop::SECOND_EXPLAIN)
                .cloned()
                .unwrap_or_default();
            self.flow.message = Some(prompt);
            return;
        }
        if self.credits() < a.cost {
            self.flow.message = Some("Need more tactical resource".into());
            return;
        }
        *self.credits_mut() -= a.cost;
        if a.deploy_once {
            self.flow.used_once.push(a.id);
        }
        self.close_supports();
        self.flow.message = None;
        self.launch_support(a.id, PLAYER, at, dest);
    }

    /// Calls in support `id` for `player` on `at` (and `dest`): its legs
    /// play one after the other, each swapping in its scenario.
    pub(crate) fn launch_support(
        &mut self,
        id: i64,
        player: i64,
        at: (i64, i64),
        dest: Option<(i64, i64)>,
    ) {
        let Some(a) = self.defs.automated.get(&id).cloned() else {
            return;
        };
        let legs = support::execute(
            &self.map.battle.scenario,
            &self.defs,
            &mut self.rng,
            &a,
            player,
            at,
            dest,
        );
        let mut before = self.map.battle.scenario.clone();
        let mut shots: Vec<Shot> = Vec::new();
        for leg in legs {
            let shot = self.support_shot(&a, &leg, &before, player);
            before = leg.scenario.clone();
            shots.push(shot);
        }
        if shots.is_empty() {
            return;
        }
        let first = shots.remove(0);
        self.supports = shots;
        self.selected = None;
        self.preview = None;
        self.lock = None;
        self.fire_mode = None;
        self.centre_on_tile(first.lands.0 as f64 + 0.5, first.lands.1 as f64 + 0.5);
        self.shot = Some(first);
        self.rebuild();
    }

    /// The animation of one leg: the support's SMK centred on the target,
    /// flak bursts from the anti-aircraft shooters (in `before`) and the
    /// explosions of what it destroyed.
    fn support_shot(&self, a: &AutomatedType, leg: &Leg, before: &Scenario, player: i64) -> Shot {
        let ff = self.anims.as_deref();
        let impact_name = leg.animation(a);
        let impact_len = ff
            .and_then(|ff| shot::animation_length(ff, &impact_name))
            .unwrap_or(0);
        let (aa, action) = leg.frames(a);
        let size = before.size;
        let centre = tile_center(size, leg.at.0, leg.at.1);
        let (px, py) = leg.aa_pos(a);
        let flak_at = (centre.0 - 512 + px as i32, centre.1 - 384 + py as i32);
        let mut explosions: Vec<Explosion> = leg
            .destroyed
            .iter()
            .filter_map(|d| self.explosion(d))
            .collect();
        let blast = explosions.iter().map(|e| e.len).max().unwrap_or(0);
        if let Some(ff) = ff {
            for (i, &s) in leg.shooters.iter().enumerate() {
                let Some(u) = before.units.get(s) else {
                    continue;
                };
                let Some(k) = self.defs.unit(u.kind) else {
                    continue;
                };
                let colour = shot::animation_owner(ff, &k.image, u.owner);
                let name = shot::animation_name(&k.image, colour, Anim::Impact, 0);
                let Some(len) = shot::animation_length(ff, &name).filter(|&n| n > 0) else {
                    continue;
                };
                explosions.push(Explosion {
                    name,
                    first: 0,
                    len,
                    centre: flak_at,
                    start: Some(aa + FLAK_STEP * i),
                });
            }
        }
        let flak_end = explosions
            .iter()
            .filter_map(|e| e.start.map(|s| s + e.len))
            .max()
            .unwrap_or(0);
        let hit = action.min(impact_len.saturating_sub(1)).max(aa);
        Shot {
            unit: usize::MAX,
            target: Target::Tile(leg.at.0, leg.at.1),
            aim: leg.at,
            lands: leg.at,
            started: self.clock.now(),
            begin: 0,
            shoot: 0,
            end: 0,
            launch: 0,
            path: Vec::new(),
            projectile_len: 0,
            impact_start: 0,
            impact_len,
            total: impact_len.max(hit + blast).max(flak_end),
            image: a.image.clone(),
            owner: player,
            attacker: player,
            impact_name,
            hit,
            damage: 0,
            result: Some(Box::new(leg.scenario.clone())),
            removed_units: leg.removed_units.clone(),
            removed_buildings: leg.removed_buildings.clone(),
            explosions,
            repeat: false,
            ruins: leg.ruins.clone(),
        }
    }

    /// Supports the scenario has the AI launch this turn (`0x46feb0`), one
    /// at a time once nothing plays; returns whether one started.
    pub(crate) fn launch_authored_support(&mut self) -> bool {
        if self.busy() {
            return false;
        }
        let turn = self.map.battle.clock.turn;
        let supports = &mut self.map.battle.scenario.info.supports;
        let Some(i) = supports.iter().position(|s| s.turn == turn) else {
            return false;
        };
        let s = supports.remove(i);
        let Some(a) = self.defs.automated.get(&s.kind) else {
            return true;
        };
        // `0x470acf`: the AI cannot fly Human transports, drops, repairs,
        // mine detectors or bridges.
        if a.race == 0 && !(4..=6).contains(&a.id) {
            return true;
        }
        let dest = (a.category() == Some(AutomatedCategory::Transport)).then_some(s.dest);
        self.launch_support(s.kind, AI, s.start, dest);
        true
    }
}
