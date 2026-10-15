//! Attacks: clicking an enemy locks it as the target (with a path into
//! firing position when the unit cannot hit it from where it stands), a
//! second click moves there and shoots, again and again while it can. In
//! fire mode (the FIRE button) any tile can be locked, for a single shot.
//! The shot plays the shooter's
//! raise/fire/lower animations, the projectile's flight when it has one,
//! and the impact at the target.

use std::time::Duration;

use libday_data::scenario::{Ruin, Scenario, facing_toward};
use libday_view::damage::{self, Destroyed};
use libday_view::fire::{self, Checker, Fire, Target};
use libday_view::mines;
use libday_view::minimap;
use libday_view::movement::{self, WALK_POSITION_MS, affordable_steps};
use libday_view::overwatch;
use libday_view::scatter;
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
    /// Locked in fire mode: a single shot. Otherwise the unit keeps
    /// shooting while it can.
    pub once: bool,
}

/// Reaction fire at a unit spotted moving ([`overwatch`]): rounds in which
/// every hostile unit that can fire at it on overwatch shoots once, while
/// one did and it stands.
pub struct Reaction {
    pub target: usize,
    /// Units still to consider this round, last first.
    pub queue: Vec<usize>,
    /// Someone fired this round.
    pub fired: bool,
}

/// A shot being animated, timed in frames of [`WALK_POSITION_MS`].
pub struct Shot {
    pub unit: usize,
    pub target: Target,
    pub aim: (i64, i64),
    /// Where the shot lands: `aim`, or a neighbour on a miss.
    pub lands: (i64, i64),
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
    /// The shooter's artwork, for its projectile.
    pub image: String,
    pub owner: i64,
    /// `ANMSUNIT.FF` animation of the impact.
    pub impact_name: String,
    /// Frame the damage shows: once the impact has played.
    pub hit: usize,
    pub damage: i64,
    /// The scenario after the damage, swapped in at `hit`.
    pub result: Option<Box<Scenario>>,
    pub removed_units: Vec<usize>,
    pub removed_buildings: Vec<usize>,
    pub explosions: Vec<Explosion>,
    /// Shoot again once over, while the target stands and AP lasts.
    pub repeat: bool,
    /// Wreckage, added once the shot is over.
    pub ruins: Vec<Ruin>,
}

/// An explosion of something destroyed, played from the shot's `hit`.
pub struct Explosion {
    pub name: String,
    /// First frame and frame count of the animation.
    pub first: usize,
    pub len: usize,
    /// Centre (map pixels).
    pub centre: (i32, i32),
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

    /// Explosions playing at timeline frame `f`, with their frames.
    pub fn explosions(&self, f: usize) -> impl Iterator<Item = (&Explosion, usize)> {
        let k = f.checked_sub(self.hit);
        self.explosions.iter().filter_map(move |e| {
            let k = k.filter(|&k| k < e.len)?;
            Some((e, e.first + k))
        })
    }

    fn animating_shooter(&self) -> bool {
        self.begin + self.shoot + self.end > 0
    }
}

/// `target` once `units` and `buildings` (ascending) are removed, or `None`
/// if it is one of them.
fn remap_target(target: Target, units: &[usize], buildings: &[usize]) -> Option<Target> {
    match target {
        Target::Unit(i) => damage::remap(i, units).map(Target::Unit),
        Target::Building(i) => damage::remap(i, buildings).map(Target::Building),
        t @ Target::Tile(..) => Some(t),
    }
}

/// Chebyshev distance between two tiles.
fn distance(a: (i64, i64), b: (i64, i64)) -> i64 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

impl Viewer {
    /// A unit is walking or shooting: orders and selection wait.
    pub(crate) fn busy(&self) -> bool {
        self.motion.is_some() || self.shot.is_some() || self.reaction.is_some()
    }

    /// Fire mode (the FIRE button) is on for the selected unit.
    pub(crate) fn fire_mode_on(&self) -> bool {
        self.fire_mode.is_some() && self.fire_mode == self.selected
    }

    /// Whether the FIRE button can be used: the selected unit is ours, on
    /// the map and armed.
    pub(crate) fn can_fire_mode(&self) -> bool {
        self.selected.is_some_and(|i| {
            self.controllable(i)
                && self.map.battle.scenario.units[i].arrival == 0
                && self.kind_of(i).is_some_and(|k| k.has_weapon)
        })
    }

    pub(crate) fn toggle_fire_mode(&mut self) {
        if self.fire_mode_on() {
            self.fire_mode = None;
        } else if self.can_fire_mode() {
            self.fire_mode = self.selected;
        }
        self.lock = None;
        self.preview = None;
    }

    /// What the selected unit targets on tile `(x, y)`: an enemy unit or
    /// building, or in fire mode anything there but itself (the bare
    /// ground included).
    pub(crate) fn target_at(&self, x: i64, y: i64) -> Option<Target> {
        let sel = self.selected.filter(|&s| self.controllable(s))?;
        if !self.fire_mode_on() {
            return self.enemy_at(x, y);
        }
        let size = self.map.battle.scenario.size as i64;
        if !(0..size).contains(&x) || !(0..size).contains(&y) {
            return None;
        }
        match self.unit_at(x, y) {
            Some(u) if u == sel => None,
            Some(u) => Some(Target::Unit(u)),
            None => Some(
                self.building_at(x, y)
                    .map_or(Target::Tile(x, y), Target::Building),
            ),
        }
    }

    /// Enemy unit or building on tile `(x, y)` for the selected unit.
    fn enemy_at(&self, x: i64, y: i64) -> Option<Target> {
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
            // Confirming a fire-mode lock spends the fire mode.
            let once = lock.once;
            if once {
                self.fire_mode = None;
            }
            match &self.preview {
                Some(p) if p.unit == sel => {
                    lock.pending = true;
                    self.start_move();
                    if self.motion.is_none() {
                        self.lock = None;
                    }
                }
                _ => self.start_shot(sel, target, !once, false),
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
            once: self.fire_mode_on(),
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

    /// Starts unit `unit` shooting at `target` if it can from where it is;
    /// with `repeat` it shoots again after, while it can.
    pub(crate) fn start_shot(&mut self, unit: usize, target: Target, repeat: bool, raised: bool) {
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
        // Units without facings still have eight fire animations (the
        // OutPost's turret turns), so every shooter faces its target.
        let facing = facing_toward(at, aim);
        let colour = self.anims.as_deref().map_or(u.owner, |ff| {
            shot::animation_owner(ff, &kind.image, u.owner)
        });
        let anim = kind.shot.clone();
        let size = scenario.size;
        let len = |a: Anim, present: bool| -> usize {
            if !present {
                return 0;
            }
            let name = shot::animation_name(&kind.image, colour, a, facing as u8);
            self.anims
                .as_deref()
                .and_then(|ff| shot::animation_length(ff, &name))
                .unwrap_or(0)
        };
        // A follow-up shot fires straight away, the weapon already raised.
        let begin = if raised {
            0
        } else {
            len(Anim::Begin, anim.begin)
        };
        let shoot = len(Anim::Shoot, anim.shoot);
        let mut end = len(Anim::End, anim.end);
        let projectile_len = len(Anim::Projectile, anim.projectile);
        let impact_len = len(Anim::Impact, anim.impact);
        let launch = begin + (anim.proj_begin.max(0) as usize).min(shoot.saturating_sub(1));
        // The side whose turn it is: the shooter's, or under reaction fire
        // the moving unit's.
        let side = match &self.reaction {
            Some(r) => scenario.units.get(r.target).map_or(u.owner, |t| t.owner),
            None => u.owner,
        };
        let evader = self
            .unit_at(aim.0, aim.1)
            .map(|v| &scenario.units[v])
            .filter(|v| v.owner == side)
            .and_then(|v| self.defs.unit_tech(v.kind, v.tech));
        let lands = scatter::land(
            &mut self.rng,
            kind,
            &self.defs.ranges,
            at,
            aim,
            evader,
            size as i64,
        );
        let path = if projectile_len > 0 && distance(at, lands) > 1 {
            shot::projectile_path(
                tile_center(size, at.0, at.1),
                tile_center(size, lands.0, lands.1),
                anim.pix_offset,
                anim.pix_speed.max(1),
                anim.curve,
            )
        } else {
            Vec::new()
        };
        let impact_start = launch + path.len();
        let hit = impact_start + impact_len;
        let damage = damage::roll(&mut self.rng, kind.damage_base, kind.damage_random);
        let (damage_type, area) = (kind.damage_type, kind.damage_area);
        let (image, owner) = (kind.image.clone(), colour);
        let impact_name = shot::animation_name(&image, owner, Anim::Impact, 0);
        let cost = kind.shoot_action_points.max(0);
        let ap = u.action_points.unwrap_or(kind.action_points);
        let u = &mut self.map.battle.scenario.units[unit];
        u.facing = facing;
        u.action_points = Some((ap - cost).max(0));
        let mut result = self.map.battle.scenario.clone();
        let outcome = damage::resolve(
            &mut result,
            &self.defs,
            &mut self.rng,
            lands,
            damage,
            damage_type,
            area,
        );
        let explosions: Vec<Explosion> = outcome
            .destroyed
            .iter()
            .filter_map(|d| self.explosion(d))
            .collect();
        // Another shot follows (the target stands, the AP lasts): the
        // weapon stays raised, as in the original, which loops the fire
        // animation and lowers it after the last shot only.
        let removed = &outcome.removed_units;
        let repeat = repeat
            && damage::remap(unit, removed).is_some_and(|u| {
                remap_target(target, removed, &outcome.removed_buildings).is_some_and(|t| {
                    matches!(fire::check(&result, &self.defs, u, t), Some(Fire::Clear(_)))
                })
            });
        if repeat {
            end = 0;
        }
        let blast = explosions.iter().map(|e| e.len).max().unwrap_or(0);
        let total = (begin + shoot + end).max(hit + blast);
        let shot = Shot {
            unit,
            target,
            aim,
            lands,
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
            image,
            owner,
            impact_name,
            hit,
            damage,
            result: Some(Box::new(result)),
            removed_units: outcome.removed_units,
            removed_buildings: outcome.removed_buildings,
            explosions,
            repeat,
            ruins: outcome.ruins,
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
            self.start_shot(lock.unit, lock.target, !lock.once, false);
        }
    }

    /// Mine `mine` goes off under unit `unit` (`0x41aca5`): its explosion
    /// plays on the tile, which takes the mine type's damage like a shot
    /// (destroying the mine), and the mines around are revealed to the
    /// unit's owner.
    pub(crate) fn start_mine(&mut self, unit: usize, mine: usize) {
        let scenario = &self.map.battle.scenario;
        let (Some(m), Some(u)) = (scenario.mines.get(mine), scenario.units.get(unit)) else {
            return;
        };
        let Some(kind) = self.defs.mines.get(&m.kind) else {
            return;
        };
        let (at, owner, mover) = ((m.x, m.y), m.owner, u.owner);
        let impact_name = format!("MINE\\{}.SMK", kind.image.to_ascii_uppercase());
        let impact_len = self
            .anims
            .as_deref()
            .and_then(|ff| shot::animation_length(ff, &impact_name))
            .unwrap_or(0);
        let damage = damage::roll(&mut self.rng, kind.damage_base, kind.damage_random);
        let mut result = scenario.clone();
        let outcome = damage::resolve(
            &mut result,
            &self.defs,
            &mut self.rng,
            at,
            damage,
            kind.damage_type,
            1,
        );
        mines::reveal_around(&mut result.mines, at, mover);
        let explosions: Vec<Explosion> = outcome
            .destroyed
            .iter()
            .filter_map(|d| self.explosion(d))
            .collect();
        let blast = explosions.iter().map(|e| e.len).max().unwrap_or(0);
        self.shot = Some(Shot {
            unit,
            target: Target::Tile(at.0, at.1),
            aim: at,
            lands: at,
            started: self.clock.now(),
            begin: 0,
            shoot: 0,
            end: 0,
            launch: 0,
            path: Vec::new(),
            projectile_len: 0,
            impact_start: 0,
            impact_len,
            total: impact_len + blast,
            image: kind.image.clone(),
            owner,
            impact_name,
            hit: impact_len,
            damage,
            result: Some(Box::new(result)),
            removed_units: outcome.removed_units,
            removed_buildings: outcome.removed_buildings,
            explosions,
            repeat: false,
            ruins: outcome.ruins,
        });
    }

    /// Advances the shot; returns whether one is playing.
    pub(crate) fn tick_shot(&mut self) -> bool {
        let Some(s) = &self.shot else {
            return false;
        };
        let f = s.frame(self.clock.now());
        if f >= s.hit && s.result.is_some() {
            self.apply_hit();
        }
        let Some(s) = &self.shot else {
            return false;
        };
        let (unit, total) = (s.unit, s.total);
        if s.shooter(f).is_none() && self.options.hidden_unit == Some(unit) {
            self.options.hidden_unit = None;
            self.rebuild();
        }
        if f >= total {
            let Some(s) = self.shot.take() else {
                return false;
            };
            if !s.ruins.is_empty() {
                self.map.battle.scenario.ruins.extend(s.ruins);
                self.rebuild();
            }
            // Lock and fire keeps shooting while the shot is clear.
            if s.repeat
                && matches!(
                    fire::check(&self.map.battle.scenario, &self.defs, s.unit, s.target),
                    Some(Fire::Clear(_))
                )
            {
                self.start_shot(s.unit, s.target, true, true);
                return self.shot.is_some();
            }
            return self.next_reaction();
        }
        true
    }

    /// Unit `target` was spotted moving: reaction fire starts.
    pub(crate) fn start_reaction(&mut self, target: usize) {
        self.lock = None;
        self.reaction = Some(Reaction {
            target,
            queue: Vec::new(),
            fired: true,
        });
        self.next_reaction();
    }

    /// Starts the next reaction shot (`0x41a46b`); returns whether one is
    /// playing, ending the reaction fire otherwise.
    fn next_reaction(&mut self) -> bool {
        let units = self.map.battle.scenario.units.len();
        loop {
            let Some(r) = &mut self.reaction else {
                return false;
            };
            let target = r.target;
            let Some(shooter) = r.queue.pop() else {
                if !r.fired || target >= units {
                    self.reaction = None;
                    return false;
                }
                r.fired = false;
                r.queue = (0..units).rev().collect();
                continue;
            };
            if overwatch::reacts(&self.map.battle.scenario, &self.defs, shooter, target) {
                self.start_shot(shooter, Target::Unit(target), false, false);
                if self.shot.is_some() {
                    if let Some(r) = &mut self.reaction {
                        r.fired = true;
                    }
                    return true;
                }
            }
        }
    }

    /// Shows the damage: swaps in the damaged scenario, keeping the unit
    /// indices pointing at the same (surviving) units.
    fn apply_hit(&mut self) {
        let Some(s) = &mut self.shot else {
            return;
        };
        let Some(result) = s.result.take() else {
            return;
        };
        self.map.battle.scenario = *result;
        let removed = std::mem::take(&mut s.removed_units);
        let removed_buildings = std::mem::take(&mut s.removed_buildings);
        match remap_target(s.target, &removed, &removed_buildings) {
            Some(t) => s.target = t,
            // Destroyed: nothing left to shoot at.
            None => s.repeat = false,
        }
        if removed.is_empty() && s.explosions.is_empty() {
            self.rebuild();
            return;
        }
        let remap = |i: Option<usize>| i.and_then(|i| damage::remap(i, &removed));
        match damage::remap(s.unit, &removed) {
            Some(u) => s.unit = u,
            // Caught in its own blast: no more shooter to animate.
            None => {
                (s.begin, s.shoot, s.end) = (0, 0, 0);
                s.repeat = false;
            }
        }
        if let Some(r) = &mut self.reaction {
            r.queue = r
                .queue
                .iter()
                .filter_map(|&i| damage::remap(i, &removed))
                .collect();
            match damage::remap(r.target, &removed) {
                Some(t) => r.target = t,
                None => self.reaction = None,
            }
        }
        self.selected = remap(self.selected);
        self.fire_mode = remap(self.fire_mode);
        self.info = remap(self.info);
        self.options.hidden_unit = remap(self.options.hidden_unit);
        self.markers = minimap::markers(&self.map.battle.scenario, &self.defs);
        self.entry = None;
        self.rebuild();
    }

    /// The explosion of something destroyed, if it has one.
    fn explosion(&self, d: &Destroyed) -> Option<Explosion> {
        let ff = self.anims.as_deref()?;
        let size = self.map.battle.scenario.size;
        match d {
            Destroyed::Unit { image, owner, at } => {
                let owner = shot::animation_owner(ff, image, *owner);
                let name = shot::animation_name(image, owner, Anim::Explosion, 0);
                let len = shot::animation_length(ff, &name).filter(|&n| n > 0)?;
                Some(Explosion {
                    name,
                    first: 0,
                    len,
                    centre: tile_center(size, at.0, at.1),
                })
            }
            Destroyed::Building { kind, at } => {
                let b = self.defs.buildings.get(kind)?;
                let (first, len) = b.explosion?;
                let name = format!("BUILDING\\{}.SMK", b.image.to_ascii_uppercase());
                let frames = shot::animation_length(ff, &name)?;
                let len = len.min(frames.saturating_sub(first));
                // Centred on the footprint.
                let a = tile_center(size, at.0, at.1);
                let z = tile_center(size, at.0 + b.width - 1, at.1 + b.height - 1);
                (len > 0).then(|| Explosion {
                    name,
                    first,
                    len,
                    centre: ((a.0 + z.0) / 2, (a.1 + z.1) / 2),
                })
            }
        }
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
            Target::Tile(x, y) => (x, y, (1, 1)),
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
            lands: (0, 0),
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
            image: String::new(),
            owner: 1,
            impact_name: String::new(),
            hit: 13,
            damage: 0,
            result: None,
            removed_units: Vec::new(),
            removed_buildings: Vec::new(),
            repeat: false,
            explosions: vec![Explosion {
                name: String::new(),
                first: 1,
                len: 3,
                centre: (0, 0),
            }],
            ruins: Vec::new(),
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
        let frames = |f| s.explosions(f).map(|(_, k)| k).collect::<Vec<_>>();
        assert_eq!(frames(12), Vec::<usize>::new());
        assert_eq!(frames(13), vec![1]);
        assert_eq!(frames(15), vec![3]);
        assert_eq!(frames(16), Vec::<usize>::new());
    }
}
