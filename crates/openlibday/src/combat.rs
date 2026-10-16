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

/// A mine type's explosion animation.
fn mine_animation(image: &str) -> String {
    format!("MINE\\{}.SMK", image.to_ascii_uppercase())
}

/// Frames the next lock-and-fire shot leaves before the fire animation
/// ends.
const REPEAT_LEAD: usize = 1;

/// Reaction fire at a unit spotted moving ([`overwatch`]): rounds in which
/// every hostile unit that can fire at it on overwatch shoots once, while
/// one did and it stands. As in the original (`0x41b6b5`), each shooter's
/// shots play as one burst, the weapon raised once and lowered after the
/// last, and the shooters' bursts play at the same time.
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
    /// The animation colour (see [`shot::animation_owner`]).
    pub owner: i64,
    /// The player the damage is credited to: the shooter's or the mine's
    /// owner.
    pub attacker: i64,
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
    /// Anti-aircraft units firing at an automated unit.
    pub gunners: Vec<Gunner>,
}

/// An anti-aircraft unit firing at an automated unit (`0x41be62`): turned
/// toward the flak, it plays its fire (`T`) animation once.
pub struct Gunner {
    pub unit: usize,
    pub facing: u8,
    pub name: String,
    /// Timeline frame it starts at, and its frame count.
    pub start: usize,
    pub len: usize,
}

/// An explosion of something destroyed, played from the shot's `hit`.
pub struct Explosion {
    pub name: String,
    /// First frame and frame count of the animation.
    pub first: usize,
    pub len: usize,
    /// Centre (map pixels).
    pub centre: (i32, i32),
    /// Timeline frame it starts at, if not the shot's `hit` (flak bursts
    /// around an automated unit).
    pub start: Option<usize>,
}

impl Shot {
    /// Frames of the timeline played so far.
    pub fn frame(&self, now: Duration) -> usize {
        (now.saturating_sub(self.started).as_millis() / u128::from(WALK_POSITION_MS)) as usize
    }

    /// Whether the shot is scheduled to start later (a reaction burst's
    /// next shot).
    pub fn waiting(&self, now: Duration) -> bool {
        now < self.started
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
    pub fn explosions(&self, f: usize) -> impl Iterator<Item = (usize, &Explosion, usize)> {
        self.explosions
            .iter()
            .enumerate()
            .filter_map(move |(i, e)| {
                let k = f
                    .checked_sub(e.start.unwrap_or(self.hit))
                    .filter(|&k| k < e.len)?;
                Some((i, e, e.first + k))
            })
    }

    /// Gunners firing at timeline frame `f`, with their frames.
    pub fn gunners(&self, f: usize) -> impl Iterator<Item = (&Gunner, usize)> {
        self.gunners.iter().filter_map(move |g| {
            let k = f.checked_sub(g.start).filter(|&k| k < g.len)?;
            Some((g, k))
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

/// Whether lock and fire by `unit` still has something to shoot at in
/// `target`: a tile only while an enemy mine or wall is left on it.
pub(crate) fn still_hostile(scenario: &Scenario, unit: usize, target: Target) -> bool {
    let Target::Tile(x, y) = target else {
        return true;
    };
    let Some(owner) = scenario.units.get(unit).map(|u| u.owner) else {
        return false;
    };
    scenario
        .mines
        .iter()
        .any(|m| (m.x, m.y) == (x, y) && m.owner != owner)
        || scenario
            .walls
            .iter()
            .any(|w| (w.x, w.y) == (x, y) && w.owner != owner)
}

/// Notes that someone fired in the reaction round.
fn r_fired(reaction: &mut Option<Reaction>) {
    if let Some(r) = reaction {
        r.fired = true;
    }
}

/// Chebyshev distance between two tiles.
fn distance(a: (i64, i64), b: (i64, i64)) -> i64 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

impl Viewer {
    /// A unit is walking or shooting: orders and selection wait.
    pub(crate) fn busy(&self) -> bool {
        !self.motions.is_empty()
            || !self.ends.is_empty()
            || !self.volley.is_empty()
            || self.shot.is_some()
            || !self.flights.is_empty()
            || self.reaction.is_some()
    }

    /// The scenario once every shot in the air has landed: the original's
    /// server resolves a shot's damage as it fires (`0x41a7a3`), so a
    /// burst's next shot is decided on it.
    fn projected<'a>(flights: &'a [Shot], live: &'a Scenario) -> &'a Scenario {
        flights
            .iter()
            .rev()
            .find_map(|f| f.result.as_deref())
            .unwrap_or(live)
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

    /// Hostile thing on tile `(x, y)` for the selected unit, in the
    /// original cursor's order (`0x4519cf`): an enemy unit, an enemy
    /// destructible building, an enemy mine its owner can see, then an
    /// enemy wall.
    fn enemy_at(&self, x: i64, y: i64) -> Option<Target> {
        let sel = self.selected.filter(|&s| self.controllable(s))?;
        let scenario = &self.map.battle.scenario;
        let owner = scenario.units[sel].owner;
        if let Some(u) = self.unit_at(x, y) {
            return (scenario.units[u].owner != owner && !self.controllable(u))
                .then_some(Target::Unit(u));
        }
        if let Some(b) = self.building_at(x, y) {
            let b2 = &scenario.buildings[b];
            let destructible = self
                .defs
                .buildings
                .get(&b2.kind)
                .is_some_and(|t| t.destructible);
            return (b2.owner != owner && destructible).then_some(Target::Building(b));
        }
        if self.options.terrain_only {
            return None;
        }
        let mine = scenario
            .mines
            .iter()
            .any(|m| (m.x, m.y) == (x, y) && m.owner != owner && !m.hidden_to(owner));
        let wall = scenario
            .walls
            .iter()
            .any(|w| (w.x, w.y) == (x, y) && w.owner != owner);
        (mine || wall).then_some(Target::Tile(x, y))
    }

    /// A click on an enemy: locks it, or confirms the lock.
    pub(crate) fn click_target(&mut self, target: Target) {
        let Some(sel) = self.selected.filter(|&s| self.controllable(s)) else {
            return;
        };
        if self.options.arrival != Some(0) || self.map.battle.scenario.units[sel].arrival != 0 {
            return;
        }
        let group = self.selection().len() > 1;
        if let Some(lock) = &mut self.lock
            && lock.unit == sel
            && lock.target == target
        {
            // Confirming a fire-mode lock spends the fire mode.
            let once = lock.once;
            if once {
                self.fire_mode = None;
            }
            if group {
                self.lock = None;
                self.preview = None;
                self.fire_group(target, !once);
                return;
            }
            match &self.preview {
                Some(p) if p.unit == sel => {
                    lock.pending = true;
                    self.start_move();
                    if self.motions.is_empty() && self.ends.is_empty() {
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
        self.approach_avoiding(unit, target, &[])
    }

    /// [`Self::approach`] ending on none of the `claimed` tiles.
    pub(crate) fn approach_avoiding(
        &self,
        unit: usize,
        target: Target,
        claimed: &[(i64, i64)],
    ) -> Option<Preview> {
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
            if claimed.contains(&at) || !matches!(checker.check(at, i64::MAX / 2), Fire::Clear(_)) {
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
        let scenario = Self::projected(&self.flights, &self.map.battle.scenario);
        let Some(Fire::Clear(aim)) = fire::check(scenario, &self.defs, unit, target) else {
            return;
        };
        let aim = (i64::from(aim.0), i64::from(aim.1));
        let u = &scenario.units[unit];
        let Some(kind) = self.defs.unit_tech(u.kind, u.tech) else {
            return;
        };
        let at = (u.x, u.y);
        let attacker = u.owner;
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
        // Shots in the air share the shooter's indices (they removed
        // nothing): their scenarios spend the AP too.
        for f in &mut self.flights {
            if let Some(u) = f.result.as_mut().and_then(|r| r.units.get_mut(unit)) {
                u.facing = facing;
                u.action_points = Some((ap - cost).max(0));
            }
        }
        let mut result = Self::projected(&self.flights, &self.map.battle.scenario).clone();
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
            attacker,
            impact_name,
            hit,
            damage,
            result: Some(Box::new(result)),
            removed_units: outcome.removed_units,
            removed_buildings: outcome.removed_buildings,
            explosions,
            repeat,
            ruins: outcome.ruins,
            gunners: Vec::new(),
        };
        if shot.animating_shooter() {
            self.options.hidden_units.push(unit);
        }
        self.shot = Some(shot);
        self.rebuild();
    }

    /// After unit `unit` moved: its confirmed attack shoots if it now can.
    pub(crate) fn arrived(&mut self, unit: usize) {
        let Some(lock) = self.lock.take_if(|l| l.unit == unit) else {
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
        let impact_name = mine_animation(&kind.image);
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
            // The mine's own explosion is the impact.
            .filter(|d| !matches!(d, Destroyed::Mine { at: m, .. } if *m == at))
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
            attacker: owner,
            impact_name,
            hit: impact_len,
            damage,
            result: Some(Box::new(result)),
            removed_units: outcome.removed_units,
            removed_buildings: outcome.removed_buildings,
            explosions,
            repeat: false,
            ruins: outcome.ruins,
            gunners: Vec::new(),
        });
    }

    /// Advances the shot; returns whether one is playing.
    pub(crate) fn tick_shot(&mut self) -> bool {
        let now = self.clock.now();
        // Shots in the air land in firing order.
        while let Some(i) = self.flights.iter().position(|f| f.result.is_some()) {
            let f = &mut self.flights[i];
            if f.frame(now) < f.hit {
                break;
            }
            let attacker = f.attacker;
            if let Some(result) = f.result.take() {
                self.swap_in(*result, attacker);
                self.rebuild();
            }
        }
        let mut done = Vec::new();
        let mut finished = Vec::new();
        self.flights.retain_mut(|f| {
            let over = f.result.is_none() && f.frame(now) >= f.total;
            if over {
                done.append(&mut f.ruins);
                finished.push(f.unit);
            }
            !over
        });
        if !done.is_empty() {
            self.map.battle.scenario.ruins.extend(done);
            self.rebuild();
        }
        self.unhide_flight_shooters(now, finished);
        let landed = self.flights.iter().all(|f| f.result.is_none());
        let Some(s) = &self.shot else {
            if self.flights.is_empty() && self.reaction.is_some() {
                return self.next_reaction();
            }
            return !self.flights.is_empty();
        };
        let f = s.frame(now);
        if landed && f >= s.hit && s.result.is_some() {
            self.apply_hit();
        }
        let Some(s) = &self.shot else {
            return false;
        };
        let (unit, total) = (s.unit, s.total);
        let shooter_done = s.shooter(f).is_none();
        // Anti-aircraft gunners show their animation instead while firing.
        let firing: Vec<usize> = s.gunners(f).map(|(g, _)| g.unit).collect();
        let gunners: Vec<usize> = s.gunners.iter().map(|g| g.unit).collect();
        let hidden = self.options.hidden_units.clone();
        self.options
            .hidden_units
            .retain(|u| !gunners.contains(u) || firing.contains(u));
        for u in firing {
            if !self.options.hidden_units.contains(&u) {
                self.options.hidden_units.push(u);
            }
        }
        if self.options.hidden_units != hidden {
            self.rebuild();
        }
        if shooter_done && self.options.hidden_units.contains(&unit) {
            self.options.hidden_units.retain(|&u| u != unit);
            self.rebuild();
        }
        let Some(s) = &self.shot else {
            return false;
        };
        // Lock and fire: as in the original, the next shot leaves once the
        // fire animation is over, while this one is still in the air, its
        // damage already decided. Shots that destroy something end the
        // burst the slow way, keeping the unit indices still. The loop
        // skips the fire animation's last frame: the original fires a
        // LAV every ~0.73 s, 11 of its 12 frames.
        if s.repeat
            && f < total
            && f + REPEAT_LEAD >= s.begin + s.shoot
            && still_hostile(
                s.result.as_deref().unwrap_or(&self.map.battle.scenario),
                s.unit,
                s.target,
            )
            && s.removed_units.is_empty()
            && s.removed_buildings.is_empty()
            && s.explosions.is_empty()
        {
            if let Some(s) = self.shot.take() {
                let (unit, target) = (s.unit, s.target);
                self.flights.push(s);
                self.start_shot(unit, target, true, true);
            }
            return true;
        }
        if f >= total && landed {
            let Some(s) = self.shot.take() else {
                return false;
            };
            if !s.gunners.is_empty() {
                self.options
                    .hidden_units
                    .retain(|u| !s.gunners.iter().any(|g| g.unit == *u));
                self.rebuild();
            }
            if !s.ruins.is_empty() {
                self.map.battle.scenario.ruins.extend(s.ruins);
                self.rebuild();
            }
            // A transport's second leg follows its first.
            if !self.supports.is_empty() {
                let mut next = self.supports.remove(0);
                next.started = self.clock.now();
                self.centre_on_tile(next.lands.0 as f64 + 0.5, next.lands.1 as f64 + 0.5);
                self.shot = Some(next);
                self.rebuild();
                return true;
            }
            // Lock and fire keeps shooting while the shot is clear.
            if s.repeat
                && still_hostile(&self.map.battle.scenario, s.unit, s.target)
                && matches!(
                    fire::check(&self.map.battle.scenario, &self.defs, s.unit, s.target),
                    Some(Fire::Clear(_))
                )
            {
                self.start_shot(s.unit, s.target, true, true);
                return self.shot.is_some() || !self.flights.is_empty();
            }
            if !self.flights.is_empty() {
                return true;
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

    /// Resolves the reaction rounds (`0x41a46b`) in firing order and
    /// schedules their shots: a unit's first shot leaves straight away,
    /// its next ones as its previous fire animation ends (the original
    /// adds them to the same fire message, `0x41b6b5`, played back to back
    /// by `0x41b7d2`). Returns whether any is playing, ending the reaction
    /// fire otherwise. A shot that destroys something is the last to
    /// leave (the unit indices shift when it lands); the rest fires after.
    fn next_reaction(&mut self) -> bool {
        loop {
            let units = self.map.battle.scenario.units.len();
            let Some(r) = &mut self.reaction else {
                return !self.flights.is_empty();
            };
            let target = r.target;
            let Some(shooter) = r.queue.pop() else {
                // The next round is resolved straight away: its shots
                // follow the bursts in the air.
                if r.fired && target < units {
                    r.fired = false;
                    r.queue = (0..units).rev().collect();
                    continue;
                }
                self.reaction = None;
                return !self.flights.is_empty();
            };
            let scenario = Self::projected(&self.flights, &self.map.battle.scenario);
            if !overwatch::reacts(scenario, &self.defs, shooter, target) {
                continue;
            }
            // The unit's previous shot, if its fire animation is still to
            // end: this one follows it, the weapon kept raised.
            let now = self.clock.now();
            let previous = self
                .flights
                .iter_mut()
                .filter(|f| f.unit == shooter)
                .max_by_key(|f| f.started)
                .map(|f| {
                    let fire = (f.begin + f.shoot).saturating_sub(REPEAT_LEAD) as u64;
                    let at = f.started + Duration::from_millis(fire * WALK_POSITION_MS);
                    (f, at)
                })
                .filter(|(_, at)| *at > now);
            let after = previous.map(|(f, at)| {
                f.end = 0;
                f.total = (f.begin + f.shoot).max(f.hit);
                at
            });
            self.start_shot(shooter, Target::Unit(target), false, after.is_some());
            let Some(mut s) = self.shot.take() else {
                continue;
            };
            if let Some(at) = after {
                s.started = at;
            }
            r_fired(&mut self.reaction);
            if s.removed_units.is_empty()
                && s.removed_buildings.is_empty()
                && s.explosions.is_empty()
            {
                self.flights.push(s);
            } else {
                self.shot = Some(s);
                return true;
            }
        }
    }

    /// Shows the units of shots in the air (or just over, `finished`)
    /// again once their fire animations are over, unless another shot
    /// animates them.
    fn unhide_flight_shooters(&mut self, now: Duration, finished: Vec<usize>) {
        let animating = |u: usize| {
            self.flights
                .iter()
                .chain(&self.shot)
                .any(|s| s.unit == u && s.shooter(s.frame(now)).is_some())
        };
        let done: Vec<usize> = self
            .flights
            .iter()
            .filter(|f| f.shooter(f.frame(now)).is_none())
            .map(|f| f.unit)
            .chain(finished)
            .filter(|&u| !animating(u))
            .filter(|u| self.options.hidden_units.contains(u))
            .filter(|u| !self.motions.iter().any(|m| m.unit == *u))
            .collect();
        if !done.is_empty() {
            self.options.hidden_units.retain(|u| !done.contains(u));
            self.rebuild();
        }
    }

    /// Makes `result` the current scenario, counting what `attacker`
    /// destroyed in the battle statistics.
    fn swap_in(&mut self, result: Scenario, attacker: i64) {
        let before = std::mem::replace(&mut self.map.battle.scenario, result);
        // A mine gone shifts the others: stop inspecting mines.
        if before.mines.len() != self.map.battle.scenario.mines.len()
            && matches!(self.info, Some(libday_view::inspect::Target::Mine(_)))
        {
            self.info = None;
        }
        self.map.battle.tally(&before, attacker);
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
        let attacker = s.attacker;
        self.swap_in(*result, attacker);
        let Some(s) = &mut self.shot else {
            return;
        };
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
        s.gunners = std::mem::take(&mut s.gunners)
            .into_iter()
            .filter_map(|mut g| {
                g.unit = damage::remap(g.unit, &removed)?;
                Some(g)
            })
            .collect();
        // Shots of the round that already landed: their shooters may still
        // be lowering their weapons.
        for f in &mut self.flights {
            match damage::remap(f.unit, &removed) {
                Some(u) => f.unit = u,
                None => (f.begin, f.shoot, f.end) = (0, 0, 0),
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
        self.group = self.group.iter().filter_map(|&i| remap(Some(i))).collect();
        self.ends = std::mem::take(&mut self.ends)
            .into_iter()
            .filter_map(|(i, e)| Some((remap(Some(i))?, e)))
            .collect();
        self.volley = std::mem::take(&mut self.volley)
            .into_iter()
            .filter_map(|(i, t, r)| {
                Some((
                    remap(Some(i))?,
                    remap_target(t, &removed, &removed_buildings)?,
                    r,
                ))
            })
            .collect();
        self.fire_mode = remap(self.fire_mode);
        // Buildings, walls and roads may have gone too: stop inspecting them.
        self.info = remap(self.info.and_then(|t| t.unit())).map(libday_view::inspect::Target::Unit);
        self.options.hidden_units = std::mem::take(&mut self.options.hidden_units)
            .into_iter()
            .filter_map(|i| remap(Some(i)))
            .collect();
        self.markers = minimap::markers(&self.map.battle.scenario, &self.defs);
        self.entry = None;
        self.rebuild();
    }

    /// The explosion of something destroyed, if it has one.
    pub(crate) fn explosion(&self, d: &Destroyed) -> Option<Explosion> {
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
                    start: None,
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
                    start: None,
                })
            }
            Destroyed::Mine { kind, at } => {
                let name = mine_animation(&self.defs.mines.get(kind)?.image);
                let len = shot::animation_length(ff, &name).filter(|&n| n > 0)?;
                Some(Explosion {
                    name,
                    first: 0,
                    len,
                    centre: tile_center(size, at.0, at.1),
                    start: None,
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
            attacker: 1,
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
                start: None,
            }],
            ruins: Vec::new(),
            gunners: Vec::new(),
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
        let frames = |f| s.explosions(f).map(|(_, _, k)| k).collect::<Vec<_>>();
        assert_eq!(frames(12), Vec::<usize>::new());
        assert_eq!(frames(13), vec![1]);
        assert_eq!(frames(15), vec![3]);
        assert_eq!(frames(16), Vec::<usize>::new());
    }
}
