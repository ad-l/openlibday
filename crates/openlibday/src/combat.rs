//! Attacks: clicking an enemy locks it as the target (with a path into
//! firing position when the unit cannot hit it from where it stands), a
//! second click moves there and shoots. The shot plays the shooter's
//! raise/fire/lower animations, the projectile's flight when it has one,
//! and the impact at the target.

use std::time::Duration;

use libday_data::scenario::facing_toward;
use libday_view::fire::{self, Checker, Fire, Target};
use libday_view::movement::{self, WALK_POSITION_MS, affordable_steps};
use libday_view::shot::{self, Anim};
use libday_view::tile_center;

use crate::Viewer;
use crate::moves::Preview;

/// A target locked by a first click.
pub struct Lock {
    pub unit: usize,
    pub target: Target,
    /// Set once confirmed: the unit is moving into position, then shoots.
    pub pending: bool,
}

/// A shot being animated, timed in frames of [`WALK_POSITION_MS`].
pub struct Shot {
    pub unit: usize,
    pub target: Target,
    pub aim: (i64, i64),
    pub started: Duration,
    /// Lengths of the raise (`S`), fire (`T`) and lower (`U`) animations.
    pub begin: usize,
    pub shoot: usize,
    pub end: usize,
    /// Frame the projectile leaves (or the impact starts without one).
    pub launch: usize,
    /// Projectile positions (map pixels), one per frame from `launch`.
    pub path: Vec<(i32, i32)>,
    pub projectile_len: usize,
    pub impact_start: usize,
    pub impact_len: usize,
    pub total: usize,
}

impl Shot {
    /// Frames of the timeline played so far.
    pub fn frame(&self, now: Duration) -> usize {
        (now.saturating_sub(self.started).as_millis() / u128::from(WALK_POSITION_MS)) as usize
    }

    /// The shooter's animation and its frame at timeline frame `f`, while
    /// it plays.
    pub fn shooter(&self, f: usize) -> Option<(Anim, usize)> {
        if f < self.begin {
            Some((Anim::Begin, f))
        } else if f < self.begin + self.shoot {
            Some((Anim::Shoot, f - self.begin))
        } else if f < self.begin + self.shoot + self.end {
            Some((Anim::End, f - self.begin - self.shoot))
        } else {
            None
        }
    }

    /// The projectile's position and frame at timeline frame `f`.
    pub fn projectile(&self, f: usize) -> Option<((i32, i32), usize)> {
        let k = f.checked_sub(self.launch)?;
        let at = *self.path.get(k)?;
        Some((at, k % self.projectile_len.max(1)))
    }

    /// The impact frame at timeline frame `f`.
    pub fn impact(&self, f: usize) -> Option<usize> {
        let k = f.checked_sub(self.impact_start)?;
        (k < self.impact_len).then_some(k)
    }

    fn animating_shooter(&self) -> bool {
        self.begin + self.shoot + self.end > 0
    }
}

/// Chebyshev distance between two tiles.
fn distance(a: (i64, i64), b: (i64, i64)) -> i64 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

impl Viewer {
    /// A unit is walking or shooting: orders and selection wait.
    pub(crate) fn busy(&self) -> bool {
        self.motion.is_some() || self.shot.is_some()
    }

    /// Enemy unit or building on tile `(x, y)` for the selected unit.
    pub(crate) fn enemy_at(&self, x: i64, y: i64) -> Option<Target> {
        let sel = self.selected.filter(|&s| self.controllable(s))?;
        let scenario = &self.map.battle.scenario;
        let owner = scenario.units[sel].owner;
        match self.unit_at(x, y) {
            Some(u) if scenario.units[u].owner != owner && !self.controllable(u) => {
                Some(Target::Unit(u))
            }
            Some(_) => None,
            None => {
                let b = self.building_at(x, y)?;
                let o = scenario.buildings[b].owner;
                (o != 0 && o != owner).then_some(Target::Building(b))
            }
        }
    }

    /// A click on an enemy: locks it, or confirms the lock.
    pub(crate) fn click_target(&mut self, target: Target) {
        let Some(sel) = self.selected.filter(|&s| self.controllable(s)) else {
            return;
        };
        if self.options.arrival != Some(0) || self.map.battle.scenario.units[sel].arrival != 0 {
            return;
        }
        if let Some(lock) = &mut self.lock
            && lock.unit == sel
            && lock.target == target
        {
            match &self.preview {
                Some(p) if p.unit == sel => {
                    lock.pending = true;
                    self.start_move();
                    if self.motion.is_none() {
                        self.lock = None;
                    }
                }
                _ => self.start_shot(sel, target),
            }
            return;
        }
        self.lock = None;
        self.preview = None;
        match fire::check(&self.map.battle.scenario, &self.defs, sel, target) {
            Some(Fire::Clear(_)) => {}
            Some(Fire::OutOfRange | Fire::TooClose | Fire::Blocked) => {
                let Some(p) = self.approach(sel, target) else {
                    return;
                };
                self.preview = Some(p);
            }
            _ => return,
        }
        self.lock = Some(Lock {
            unit: sel,
            target,
            pending: false,
        });
    }

    /// Path to the cheapest tile the unit can hit `target` from with the AP
    /// left after walking there, or failing that the cheapest with a clear
    /// shot (walked as far as it can afford).
    fn approach(&self, unit: usize, target: Target) -> Option<Preview> {
        let scenario = &self.map.battle.scenario;
        let u = scenario.units.get(unit)?;
        let grid = movement::move_grid(scenario, &self.defs, unit)?;
        let checker = Checker::new(scenario, &self.defs, unit, target)?;
        let ap = self.action_points(unit)?.0.max(0);
        let cost = checker.shot_cost().max(0);
        let from = (u.x as i32, u.y as i32);
        let mut fallback = None;
        for (tile, _) in grid.reachable(from) {
            let at = (i64::from(tile.0), i64::from(tile.1));
            if !matches!(checker.check(at, i64::MAX / 2), Fire::Clear(_)) {
                continue;
            }
            let Some(steps) = grid.find_path(from, tile) else {
                continue;
            };
            let walk: i64 = steps.iter().map(|s| i64::from(s.cost)).sum();
            let preview = Preview {
                unit,
                target: at,
                affordable: affordable_steps(&steps, (ap - cost).max(0) as u32),
                steps,
            };
            if walk + cost <= ap {
                return Some(preview);
            }
            fallback.get_or_insert(preview);
        }
        fallback
    }

    /// Starts unit `unit` shooting at `target` if it can from where it is.
    pub(crate) fn start_shot(&mut self, unit: usize, target: Target) {
        self.lock = None;
        self.preview = None;
        let scenario = &self.map.battle.scenario;
        let Some(Fire::Clear(aim)) = fire::check(scenario, &self.defs, unit, target) else {
            return;
        };
        let aim = (i64::from(aim.0), i64::from(aim.1));
        let u = &scenario.units[unit];
        let Some(kind) = self.defs.unit_tech(u.kind, u.tech) else {
            return;
        };
        let at = (u.x, u.y);
        let facing = if kind.facings {
            facing_toward(at, aim)
        } else {
            u.facing
        };
        let anim = kind.shot.clone();
        let size = scenario.size;
        let len = |a: Anim, present: bool| -> usize {
            if !present {
                return 0;
            }
            let name = shot::animation_name(&kind.image, u.owner, a, facing as u8);
            self.anims
                .as_deref()
                .and_then(|ff| shot::animation_length(ff, &name))
                .unwrap_or(0)
        };
        let begin = len(Anim::Begin, anim.begin);
        let shoot = len(Anim::Shoot, anim.shoot);
        let end = len(Anim::End, anim.end);
        let projectile_len = len(Anim::Projectile, anim.projectile);
        let impact_len = len(Anim::Impact, anim.impact);
        let launch = begin + (anim.proj_begin.max(0) as usize).min(shoot.saturating_sub(1));
        let path = if projectile_len > 0 && distance(at, aim) > 1 {
            shot::projectile_path(
                tile_center(size, at.0, at.1),
                tile_center(size, aim.0, aim.1),
                anim.pix_offset,
                anim.pix_speed.max(1),
                anim.curve,
            )
        } else {
            Vec::new()
        };
        let impact_start = launch + path.len();
        let total = (begin + shoot + end).max(impact_start + impact_len);
        let cost = kind.shoot_action_points.max(0);
        let ap = u.action_points.unwrap_or(kind.action_points);
        let u = &mut self.map.battle.scenario.units[unit];
        u.facing = facing;
        u.action_points = Some((ap - cost).max(0));
        let shot = Shot {
            unit,
            target,
            aim,
            started: self.clock.now(),
            begin,
            shoot,
            end,
            launch,
            path,
            projectile_len,
            impact_start,
            impact_len,
            total,
        };
        if shot.animating_shooter() {
            self.options.hidden_unit = Some(unit);
        }
        self.shot = Some(shot);
        self.rebuild();
    }

    /// After a move: the confirmed attack shoots if it now can.
    pub(crate) fn arrived(&mut self) {
        let Some(lock) = self.lock.take() else {
            return;
        };
        if lock.pending {
            self.start_shot(lock.unit, lock.target);
        }
    }

    /// Advances the shot; returns whether one is playing.
    pub(crate) fn tick_shot(&mut self) -> bool {
        let Some(s) = &self.shot else {
            return false;
        };
        let f = s.frame(self.clock.now());
        let (unit, total) = (s.unit, s.total);
        if s.shooter(f).is_none() && self.options.hidden_unit == Some(unit) {
            self.options.hidden_unit = None;
            self.rebuild();
        }
        if f >= total {
            self.shot = None;
            return false;
        }
        true
    }

    /// Tiles of the locked target's footprint.
    pub(crate) fn lock_tiles(&self) -> Vec<(i64, i64)> {
        let Some(lock) = &self.lock else {
            return Vec::new();
        };
        let scenario = &self.map.battle.scenario;
        let (x, y, (w, h)) = match lock.target {
            Target::Unit(i) => {
                let Some(u) = scenario.units.get(i) else {
                    return Vec::new();
                };
                let wh = self
                    .defs
                    .unit(u.kind)
                    .map_or((1, 1), |k| (k.width, k.height));
                (u.x, u.y, wh)
            }
            Target::Building(i) => {
                let Some(b) = scenario.buildings.get(i) else {
                    return Vec::new();
                };
                let wh = self
                    .defs
                    .buildings
                    .get(&b.kind)
                    .map_or((1, 1), |k| (k.width, k.height));
                (b.x, b.y, wh)
            }
        };
        (0..h)
            .flat_map(|dy| (0..w).map(move |dx| (x + dx, y + dy)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot() -> Shot {
        Shot {
            unit: 0,
            target: Target::Unit(1),
            aim: (0, 0),
            started: Duration::ZERO,
            begin: 2,
            shoot: 5,
            end: 3,
            launch: 4,
            path: vec![(0, 0), (10, 0), (20, 0)],
            projectile_len: 1,
            impact_start: 7,
            impact_len: 6,
            total: 13,
        }
    }

    #[test]
    fn timeline_phases() {
        let s = shot();
        assert_eq!(s.shooter(0), Some((Anim::Begin, 0)));
        assert_eq!(s.shooter(2), Some((Anim::Shoot, 0)));
        assert_eq!(s.shooter(9), Some((Anim::End, 2)));
        assert_eq!(s.shooter(10), None);
        assert_eq!(s.projectile(3), None);
        assert_eq!(s.projectile(5), Some(((10, 0), 0)));
        assert_eq!(s.projectile(7), None);
        assert_eq!(s.impact(6), None);
        assert_eq!(s.impact(12), Some(5));
        assert_eq!(s.impact(13), None);
    }
}
