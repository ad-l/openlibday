//! Unit movement: path preview on a single click, the move itself on a
//! second click on the same tile (or a double click), animated along the
//! path with the unit's walk animation and charged in action points.

use std::time::Duration;

use libday_view::fire::{self, Fire};
use libday_view::mines;
use libday_view::movement::{
    self, Movement, Step, WALK_POSITION_MS, affordable_steps, executable_steps, move_budget,
};
use libday_view::overwatch;
use libday_view::shot::{self, Anim};
use libday_view::{SPRITE_ANCHOR_Y, SPRITE_W, Sheet, tile_center};
use sdl3::render::{Canvas, FRect};
use sdl3::video::Window;

use crate::Viewer;
use crate::hud::{self, Gfx, Result};
use crate::sound::{Effect, Playing};

/// Path shown after a single click.
pub struct Preview {
    pub unit: usize,
    pub target: (i64, i64),
    pub steps: Vec<Step>,
    /// Leading steps the unit can pay for; later dots are crossed.
    pub affordable: usize,
}

/// Where and how a walking (or shooting) unit is drawn.
pub(crate) struct Walker {
    unit: usize,
    /// Point the unit is centred on (a tile centre while standing), in map
    /// pixels.
    centre: (i32, i32),
    facing: u8,
    /// Animation (`None`: slide the standing sprite) and its frame counter.
    anim: Option<String>,
    frame: usize,
    /// Tile whose depth the unit is drawn at.
    pub(crate) depth: (i64, i64),
    /// Tile in the row in front whose top corner a sideways (grid-diagonal)
    /// step crosses: its flat ground is drawn before the unit, not over it.
    pub(crate) under: Option<(i64, i64)>,
    /// Sound slot of its animation.
    slot: usize,
    started: Duration,
}

/// How a walk ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum End {
    /// At the destination: a locked target is shot.
    Arrived,
    /// Short of it, on sighting a mine.
    Interrupted,
    /// On mine `k`, which goes off.
    Mine(usize),
    /// Spotted by overwatch: reaction fire.
    Spotted,
}

/// A unit walking along a path.
pub struct Motion {
    pub unit: usize,
    pub steps: Vec<Step>,
    /// Steps completed; the unit walks onto `steps[done]`.
    pub done: usize,
    /// Tile the current step starts from.
    pub from: (i64, i64),
    /// Queued positions (map pixel centre, step index), each shown for
    /// [`WALK_POSITION_MS`].
    pub positions: Vec<((i32, i32), usize)>,
    /// Clock time the walk started.
    pub started: Duration,
    pub final_ap: i64,
    /// Owner colour of the walk animation ([`shot::animation_owner`]).
    pub colour: i64,
    /// A teleport: the unit vanishes (`L` animation) where it stands, then
    /// appears (`N`) on its single step, without crossing the map.
    pub teleport: bool,
    /// Facing during the start (`L`) animation: the one stood in.
    pub facing: u8,
    /// Leading positions of the start (take-off / vanish) animation, one
    /// per frame, before the walk itself.
    pub lead: usize,
    /// Positions of the walk proper; any after them play the end (landing /
    /// appear, `N`) animation on the last step.
    pub walk: usize,
    /// A walk cut short still lands where it stopped before its end plays
    /// out (reaction fire, a mine).
    pub landing: Option<Landing>,
    /// One of a player's group moves: a unit fired on or hitting a mine
    /// stops the whole group (`0x416cf1` breaks out of the batch).
    pub halts: bool,
}

/// Animation ticks between the starts of a batch's moves (`0x41b25e`).
const BATCH_STAGGER: u64 = 3;

/// End animation of a walk cut short.
pub struct Landing {
    started: Duration,
    at: (i64, i64),
    facing: u8,
    frames: usize,
    ap: Option<i64>,
    end: End,
}

/// The original queues a move as three animation objects (`0x41b25e`): the
/// start animation in the facing stood in (`0x46aef9`), the walk, and the
/// end animation in the last step's facing (`0x46bf41`). `lead` and `tail`
/// are the frame counts of the start and end animations (0: none).
fn move_queue(
    size: usize,
    start: (i64, i64),
    steps: &[Step],
    walk: Vec<((i32, i32), usize)>,
    lead: usize,
    tail: usize,
) -> Vec<((i32, i32), usize)> {
    let last = steps.len().saturating_sub(1);
    let to = steps
        .last()
        .map_or(start, |s| (i64::from(s.x), i64::from(s.y)));
    let mut out = vec![(tile_center(size, start.0, start.1), 0); lead];
    out.extend(walk);
    out.extend(std::iter::repeat_n(
        (tile_center(size, to.0, to.1), last),
        tail,
    ));
    out
}

/// The original's walk queue for `steps` from tile `start`.
fn walk_queue(
    size: usize,
    start: (i64, i64),
    steps: &[Step],
    pix_speed: i32,
) -> Vec<((i32, i32), usize)> {
    let mut from = tile_center(size, start.0, start.1);
    let mut out = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        let to = tile_center(size, i64::from(s.x), i64::from(s.y));
        let points = movement::walk_positions(from, to, pix_speed);
        if points.is_empty() {
            out.push((to, i));
        }
        out.extend(points.into_iter().map(|p| (p, i)));
        from = to;
    }
    out
}

impl Viewer {
    /// Current and maximum action points of unit `i`.
    pub(crate) fn action_points(&self, i: usize) -> Option<(i64, i64)> {
        let unit = self.map.battle.scenario.units.get(i)?;
        let kind = self.kind_of(i)?;
        Some((
            unit.action_points.unwrap_or(kind.action_points),
            kind.action_points,
        ))
    }

    /// Overwatch state of the selected unit.
    pub(crate) fn overwatch_on(&self) -> bool {
        self.selected.is_some_and(|i| self.overwatch(i))
    }

    pub(crate) fn overwatch(&self, i: usize) -> bool {
        let can = self.kind_of(i).is_some_and(|k| k.can_overwatch);
        can && self.map.battle.scenario.units[i].overwatch
    }

    pub(crate) fn toggle_overwatch(&mut self) {
        if !self.selected.is_some_and(|i| self.controllable(i)) {
            return;
        }
        if let Some(unit) = self
            .selected
            .and_then(|i| self.map.battle.scenario.units.get_mut(i))
        {
            unit.overwatch ^= true;
        }
        self.refresh_preview();
    }

    /// AP of one shot reserved by overwatch, if on.
    fn reserve(&self, i: usize) -> Option<u32> {
        let kind = self.kind_of(i)?;
        self.overwatch(i)
            .then_some(kind.shoot_action_points.max(0) as u32)
    }

    pub(crate) fn plan(&self, unit: usize, target: (i64, i64)) -> Option<Preview> {
        let u = self.map.battle.scenario.units.get(unit)?;
        let grid = movement::move_grid(&self.map.battle.scenario, &self.defs, unit)?;
        let steps = grid.find_path((u.x as i32, u.y as i32), (target.0 as i32, target.1 as i32))?;
        let ap = self.action_points(unit)?.0.max(0) as u32;
        // The preview always keeps the overwatch shot back.
        let budget = ap.saturating_sub(self.reserve(unit).unwrap_or(0));
        Some(Preview {
            unit,
            target,
            affordable: affordable_steps(&steps, budget),
            steps,
        })
    }

    fn refresh_preview(&mut self) {
        if let Some(p) = &self.preview {
            self.preview = self.plan(p.unit, p.target);
        }
    }

    /// A left click on map tile `(x, y)`.
    pub(crate) fn click_tile(&mut self, x: i64, y: i64) {
        if self.busy() {
            return;
        }
        if let Some(target) = self.target_at(x, y) {
            self.click_target(target);
            return;
        }
        self.lock = None;
        let unit = self.unit_at(x, y);
        let sole = self.selection().len() == 1;
        match (self.selected, unit) {
            (Some(sel), Some(u)) if sel == u && sole && !self.stale => self.turn_clockwise(u),
            // Only own units are selected; enemies are targeted.
            (_, Some(u)) if self.controllable(u) => {
                if self.pick(u) {
                    self.effect(Effect::Select);
                }
            }
            (Some(sel), None) if self.options.arrival == Some(0) && self.controllable(sel) => {
                match &self.preview {
                    Some(p) if p.target == (x, y) && self.selection().contains(&p.unit) => {
                        self.start_move()
                    }
                    _ => self.preview = self.plan_selection((x, y)),
                }
            }
            _ => {}
        }
    }

    /// The path previewed for the selection's move to `target`: the
    /// leader's, or the first other member's that has one (`0x452165`).
    fn plan_selection(&mut self, target: (i64, i64)) -> Option<Preview> {
        let units = self.selection();
        let preview = units
            .iter()
            .rev()
            .filter_map(|&u| self.plan(u, target))
            .find(|p| !p.steps.is_empty());
        if preview.is_none() && units.len() > 1 {
            self.say(0x241, "Unit can't reach that position");
        }
        preview
    }

    /// Clicking the selected unit turns it one facing clockwise (facings
    /// run SW, W, NW, N, ... on screen), for free.
    fn turn_clockwise(&mut self, u: usize) {
        if self.options.arrival != Some(0) || !self.controllable(u) {
            return;
        }
        let unit = &self.map.battle.scenario.units[u];
        if unit.arrival != 0 || !self.defs.unit(unit.kind).is_some_and(|k| k.facings) {
            return;
        }
        self.preview = None;
        let unit = &mut self.map.battle.scenario.units[u];
        unit.facing = (unit.facing + 1).rem_euclid(8);
        self.rebuild();
    }

    pub(crate) fn start_move(&mut self) {
        let Some(p) = self.preview.take() else {
            return;
        };
        if self.selection().len() > 1 {
            self.move_group(p.target);
            return;
        }
        let Some((ap, _)) = self.action_points(p.unit) else {
            return;
        };
        let budget = move_budget(ap.max(0) as u32, self.reserve(p.unit));
        let (n, spent) = executable_steps(&p.steps, budget);
        let mut steps = p.steps;
        steps.truncate(n);
        self.start_walk(p.unit, steps, ap - i64::from(spent));
    }

    /// Walks unit `unit` along `steps`, ending with `final_ap` AP; returns
    /// whether anything started (a walk, or reaction fire at its start).
    pub(crate) fn start_walk(
        &mut self,
        unit_index: usize,
        steps: Vec<Step>,
        final_ap: i64,
    ) -> bool {
        self.start_batch_walk(unit_index, steps, final_ap, 0, false)
    }

    /// [`Self::start_walk`] as move `nth` of a batch, all of whose walks
    /// play together, each starting [`BATCH_STAGGER`] ticks after the one
    /// before; `halts` for a player's group.
    pub(crate) fn start_batch_walk(
        &mut self,
        unit_index: usize,
        mut steps: Vec<Step>,
        final_ap: i64,
        nth: usize,
        halts: bool,
    ) -> bool {
        let Some(unit) = self.map.battle.scenario.units.get(unit_index) else {
            return false;
        };
        let Some(kind) = self.defs.unit_tech(unit.kind, unit.tech) else {
            return false;
        };
        if steps.is_empty() || self.motions.iter().any(|m| m.unit == unit_index) {
            return false;
        }
        // Already covered by overwatch: shot at without moving, once the
        // rest of the batch has walked.
        if self.reactions && overwatch::spotted(&self.map.battle.scenario, &self.defs, unit_index) {
            self.ends.push((unit_index, End::Spotted));
            if halts {
                self.halt_group();
            }
            return true;
        }
        let now =
            self.clock.now() + Duration::from_millis(nth as u64 * BATCH_STAGGER * WALK_POSITION_MS);
        let size = self.map.battle.scenario.size;
        let start = (unit.x, unit.y);
        let pix_speed = kind.move_pix_speed.clamp(1, 1000) as i32;
        let colour = self.anims.as_deref().map_or(unit.owner, |ff| {
            shot::animation_owner(ff, &kind.image, unit.owner)
        });
        let teleport = Movement::from_id(kind.movement) == Movement::Teleport;
        let old_facing = unit.facing.rem_euclid(8) as u8;
        if teleport {
            // Only the destination counts (`0x42e1c1`); it appears facing a
            // random way (`0x42d9c9`).
            steps.drain(..steps.len() - 1);
            steps[0].facing = self.rng.rand().rem_euclid(8) as u8;
        }
        let length = |has: bool, letter: char, facing: u8| {
            let ff = self.anims.as_deref().filter(|_| has)?;
            let name = movement::move_animation_name(&kind.image, colour, letter, facing);
            shot::animation_length(ff, &name)
        };
        let last_facing = steps.last().map_or(old_facing, |s| s.facing);
        let lead = length(kind.move_start_animation, 'L', old_facing).unwrap_or(0);
        let tail = length(kind.move_end_animation, 'N', last_facing).unwrap_or(0);
        let walk = if teleport {
            Vec::new()
        } else {
            walk_queue(size, start, &steps, pix_speed)
        };
        let walk_len = walk.len();
        let positions = move_queue(size, start, &steps, walk, lead, tail);
        let facing = if teleport {
            i64::from(old_facing)
        } else {
            i64::from(steps[0].facing)
        };
        self.motions.push(Motion {
            unit: unit_index,
            positions,
            steps,
            done: 0,
            from: start,
            started: now,
            final_ap,
            colour,
            teleport,
            facing: old_facing,
            lead,
            walk: walk_len,
            landing: None,
            halts,
        });
        self.map.battle.scenario.units[unit_index].facing = facing;
        self.options.hidden_units.push(unit_index);
        self.rebuild();
        true
    }

    /// Advances the walking or shooting unit and plays the sound of what is
    /// shown; returns whether a unit is busy.
    pub(crate) fn tick(&mut self) -> bool {
        let busy = self.advance();
        let playing = self.playing();
        self.sound.update(self.anims.as_deref(), playing);
        busy
    }

    /// Animations on screen, for their sound.
    fn playing(&self) -> Vec<Playing> {
        let mut out: Vec<Playing> = self
            .walkers()
            .into_iter()
            .filter_map(|w| {
                Some(Playing {
                    slot: w.slot,
                    name: w.anim?,
                    started: w.started,
                    frame: w.frame,
                })
            })
            .collect();
        for s in self.flights.iter().chain(&self.shot) {
            let f = s.frame(self.clock.now());
            if s.projectile(f).is_some() {
                out.push(Playing {
                    slot: 1,
                    name: shot::animation_name(&s.image, s.owner, Anim::Projectile, 0),
                    started: s.started,
                    frame: f - s.launch,
                });
            }
            if let Some(frame) = s.impact(f) {
                out.push(Playing {
                    slot: 2,
                    name: s.impact_name.clone(),
                    started: s.started,
                    frame,
                });
            }
            for (i, e, frame) in s.explosions(f) {
                out.push(Playing {
                    slot: 3 + i,
                    name: e.name.clone(),
                    started: s.started,
                    frame,
                });
            }
        }
        out
    }

    /// Advances the walking units, then the shots, then what the walks
    /// ended in, one at a time; returns whether anything is busy.
    fn advance(&mut self) -> bool {
        // A walk ending reshuffles the list: go over it again.
        'walks: loop {
            for k in 0..self.motions.len() {
                if !self.advance_motion(k) {
                    continue 'walks;
                }
            }
            break;
        }
        if !self.motions.is_empty() || self.tick_shot() {
            return true;
        }
        while !self.ends.is_empty() {
            let (i, end) = self.ends.remove(0);
            if end != End::Arrived && self.lock.as_ref().is_some_and(|l| l.unit == i) {
                self.lock = None;
            }
            match end {
                End::Mine(k) => {
                    // An earlier blast may have shifted the list: the mine
                    // is the one under the unit.
                    let u = &self.map.battle.scenario.units[i];
                    let at = (u.x, u.y);
                    let mines = &self.map.battle.scenario.mines;
                    let k = match mines.get(k) {
                        Some(m) if (m.x, m.y) == at => Some(k),
                        _ => mines.iter().position(|m| (m.x, m.y) == at),
                    };
                    if let Some(k) = k {
                        self.start_mine(i, k);
                    }
                }
                End::Spotted => self.start_reaction(i),
                End::Arrived | End::Interrupted => self.arrived(i),
            }
            if self.shot.is_some() || self.reaction.is_some() || !self.flights.is_empty() {
                return true;
            }
        }
        self.next_volley()
    }

    /// Advances walk `k`; returns whether it is still walking.
    fn advance_motion(&mut self, k: usize) -> bool {
        let m = &self.motions[k];
        if let Some(l) = &m.landing {
            let ms = self.clock.since(l.started).as_millis();
            if ms >= (l.frames as u128) * u128::from(WALK_POSITION_MS) {
                let (ap, end) = (l.ap, l.end);
                self.stop(k, ap, end);
                return false;
            }
            return true;
        }
        let shown =
            (self.clock.since(m.started).as_millis() / u128::from(WALK_POSITION_MS)) as usize;
        // Steps whose positions have all been shown are walked.
        let walked = m.positions.get(shown).map_or(m.steps.len(), |&(_, s)| s);
        while self.motions[k].done < walked {
            let m = &mut self.motions[k];
            let step = m.steps[m.done];
            let to = (i64::from(step.x), i64::from(step.y));
            m.from = to;
            m.done += 1;
            let (i, done, final_ap) = (m.unit, m.done, m.final_ap);
            let last = done == m.steps.len();
            let next = m.steps.get(done).map(|s| i64::from(s.facing));
            let teleport = m.teleport;
            let unit = &mut self.map.battle.scenario.units[i];
            (unit.x, unit.y) = to;
            if teleport {
                unit.facing = i64::from(step.facing);
            }
            if let Some(ap) = &mut unit.action_points {
                *ap -= i64::from(step.cost);
            } else if let Some(k) = self.defs.unit_tech(unit.kind, unit.tech) {
                unit.action_points = Some(k.action_points - i64::from(step.cost));
            }
            let owner = unit.owner;
            let ap = last.then_some(final_ap);
            // Spotted by a hostile unit on overwatch, the walk ends there
            // under reaction fire, before mines are looked at.
            let scenario = &self.map.battle.scenario;
            if self.reactions && overwatch::spotted(scenario, &self.defs, i) {
                return self.cut_short(k, ap, End::Spotted, to, step.facing);
            }
            // A detector stops on sighting a hidden mine; else an armed
            // mine on the tile may go off. Either ends the walk there.
            let detected = mines::detected(scenario, &self.defs, i, to);
            let mine = if detected.is_empty() {
                mines::trigger(scenario, &self.defs, i, to, !last)
            } else {
                None
            };
            for &d in &detected {
                self.map.battle.scenario.mines[d].reveal_to(owner);
            }
            let end = match mine {
                Some(d) => Some(End::Mine(d)),
                None if !detected.is_empty() && !last => Some(End::Interrupted),
                None if last => Some(End::Arrived),
                None => None,
            };
            if let Some(end) = end {
                return self.cut_short(k, ap, end, to, step.facing);
            }
            if let Some(facing) = next {
                self.map.battle.scenario.units[i].facing = facing;
            }
        }
        true
    }

    /// Ends walk `k` on tile `at`; returns whether it is still playing. On
    /// the last step the end animation has already played; short of it, a
    /// unit with one plays it there first (facing `facing`), then the walk
    /// stops.
    fn cut_short(
        &mut self,
        k: usize,
        ap: Option<i64>,
        end: End,
        at: (i64, i64),
        facing: u8,
    ) -> bool {
        let now = self.clock.now();
        let m = &mut self.motions[k];
        let frames = m.positions.len().saturating_sub(m.lead + m.walk);
        if frames > 0 && m.done < m.steps.len() {
            m.steps.truncate(m.done);
            m.landing = Some(Landing {
                started: now,
                at,
                facing,
                frames,
                ap,
                end,
            });
            return true;
        }
        self.stop(k, ap, end);
        false
    }

    /// Ends walk `k` (with `ap` left, if given); what it ended in (a shot
    /// at its locked target, a mine, reaction fire) plays once every walk
    /// is over. A group member fired on or blown up stops the group.
    fn stop(&mut self, k: usize, ap: Option<i64>, end: End) {
        let m = self.motions.remove(k);
        let i = m.unit;
        if let Some(ap) = ap {
            self.map.battle.scenario.units[i].action_points = Some(ap.max(0));
        }
        self.options.hidden_units.retain(|&u| u != i);
        if m.halts && matches!(end, End::Mine(_) | End::Spotted) {
            self.halt_group();
        }
        self.ends.push((i, end));
        self.rebuild();
        self.markers = libday_view::minimap::markers(&self.map.battle.scenario, &self.defs);
    }

    /// Stops every group walk where it stands (on the last tile it
    /// reached) and calls off the group's shots.
    pub(crate) fn halt_group(&mut self) {
        self.volley.clear();
        let (halted, walking): (Vec<Motion>, Vec<Motion>) = std::mem::take(&mut self.motions)
            .into_iter()
            .partition(|m| m.halts);
        self.motions = walking;
        for m in halted {
            self.options.hidden_units.retain(|&u| u != m.unit);
        }
        self.rebuild();
    }

    /// The walking and shooting units: centre (map pixels), facing,
    /// animation frame counter and the tile whose depth each is drawn at.
    pub(crate) fn walkers(&self) -> Vec<Walker> {
        let mut out: Vec<Walker> = self.shooter().into_iter().collect();
        for (k, m) in self.motions.iter().enumerate() {
            let slot = if k == 0 { 0 } else { 100 + k };
            out.extend(self.motion_walker(m, slot));
        }
        out
    }

    fn shooter(&self) -> Option<Walker> {
        let s = self.shot.as_ref()?;
        let (anim, frame) = s.shooter(s.frame(self.clock.now()))?;
        let unit = self.map.battle.scenario.units.get(s.unit)?;
        let kind = self.defs.unit(unit.kind)?;
        let facing = unit.facing.rem_euclid(8) as u8;
        let size = self.map.battle.scenario.size;
        Some(Walker {
            unit: s.unit,
            centre: tile_center(size, unit.x, unit.y),
            facing,
            anim: Some(shot::animation_name(&kind.image, s.owner, anim, facing)),
            frame,
            // In front of the whole footprint.
            depth: (
                unit.x + kind.width.max(1) - 1,
                unit.y + kind.height.max(1) - 1,
            ),
            under: None,
            slot: 0,
            started: s.started,
        })
    }

    fn motion_walker(&self, m: &Motion, slot: usize) -> Option<Walker> {
        let size = self.map.battle.scenario.size;
        let unit = self.map.battle.scenario.units.get(m.unit)?;
        if let Some(l) = &m.landing {
            let kind = self.defs.unit(unit.kind)?;
            let frame =
                (self.clock.since(l.started).as_millis() / u128::from(WALK_POSITION_MS)) as usize;
            return Some(Walker {
                unit: m.unit,
                centre: tile_center(size, l.at.0, l.at.1),
                facing: l.facing,
                anim: Some(movement::move_animation_name(
                    &kind.image,
                    m.colour,
                    'N',
                    l.facing,
                )),
                frame: frame.min(l.frames - 1),
                depth: l.at,
                under: None,
                slot,
                started: l.started,
            });
        }
        let step = m.steps.get(m.done)?;
        let to = (i64::from(step.x), i64::from(step.y));
        let shown =
            (self.clock.since(m.started).as_millis() / u128::from(WALK_POSITION_MS)) as usize;
        let centre = match m.positions.get(shown) {
            Some(&(p, s)) if s == m.done => p,
            _ => tile_center(size, to.0, to.1),
        };
        let walk_end = m.lead + m.walk;
        let phase = if shown < m.lead {
            Some(('L', m.facing, m.from, shown, m.lead))
        } else if m.positions.len() > walk_end && shown >= walk_end {
            let tail = m.positions.len() - walk_end;
            Some(('N', step.facing, to, shown - walk_end, tail))
        } else {
            None
        };
        if let Some((letter, facing, at, frame, frames)) = phase {
            let kind = self.defs.unit(unit.kind)?;
            return Some(Walker {
                unit: m.unit,
                centre: tile_center(size, at.0, at.1),
                facing,
                anim: Some(movement::move_animation_name(
                    &kind.image,
                    m.colour,
                    letter,
                    facing,
                )),
                frame: frame.min(frames - 1),
                depth: at,
                under: None,
                slot,
                started: m.started,
            });
        }
        let frame = shown - m.lead;
        let depth = if (to.0 + to.1, to.0) > (m.from.0 + m.from.1, m.from.0) {
            to
        } else {
            m.from
        };
        let kind = self.defs.unit(unit.kind);
        // A NE/SW step runs along a screen row, over the top corner of the
        // tile below the front of the footprint.
        let under = (to.0 + to.1 == m.from.0 + m.from.1 && to != m.from).then(|| {
            let (w, h) = kind.map_or((1, 1), |k| (k.width.max(1), k.height.max(1)));
            (to.0.max(m.from.0) + w - 1, to.1.max(m.from.1) + h - 1)
        });
        let anim = kind
            .filter(|k| k.move_animation)
            .map(|k| movement::walk_animation_name(&k.image, m.colour, step.facing));
        Some(Walker {
            unit: m.unit,
            centre,
            facing: step.facing,
            anim,
            frame,
            depth,
            under,
            slot,
            started: m.started,
        })
    }

    /// Building on tile `(x, y)`.
    pub(crate) fn building_at(&self, x: i64, y: i64) -> Option<usize> {
        if self.options.terrain_only {
            return None;
        }
        self.map.battle.scenario.buildings.iter().position(|b| {
            let (w, h) = self
                .defs
                .buildings
                .get(&b.kind)
                .map_or((1, 1), |k| (k.width, k.height));
            (b.x..b.x + w).contains(&x) && (b.y..b.y + h).contains(&y)
        })
    }

    /// Battle cursor on the hovered tile for the selected unit's orders, as
    /// the original picks it (`0x4519cf`): `CMOVE` where it can walk this
    /// turn, and over an enemy unit or building `CTARG` when a shot is
    /// possible, `CINVA` when the unit lacks the AP to shoot and `CMVSH`
    /// otherwise (out of range or line of sight: move closer). In fire
    /// mode any tile but the shooter's is a target. Own units otherwise
    /// get the selection grid, drawn separately.
    pub(crate) fn battle_cursor(&self) -> Option<((i64, i64), &'static str)> {
        if self.busy() || !matches!(self.screen, libday_view::ui::Screen::Battle) {
            return None;
        }
        let (x, y) = self.hover?;
        let sel = self.selected.filter(|&s| self.controllable(s))?;
        let scenario = &self.map.battle.scenario;
        if scenario.units[sel].arrival != 0 {
            return None;
        }
        let target = self.target_at(x, y);
        let name = match target {
            Some(_) => match self.hover_fire()? {
                Fire::Clear(_) => "CTARG",
                Fire::NoActionPoints => "CINVA",
                _ => "CMVSH",
            },
            None if self.unit_at(x, y).is_some() || self.building_at(x, y).is_some() => {
                return None;
            }
            None => {
                let p = self.plan(sel, (x, y))?;
                (!p.steps.is_empty() && p.affordable == p.steps.len()).then_some("CMOVE")?
            }
        };
        Some(((x, y), name))
    }

    /// Result of the selected unit firing at the hovered target, if any.
    pub(crate) fn hover_fire(&self) -> Option<Fire> {
        let (x, y) = self.hover?;
        let sel = self.selected.filter(|&s| self.controllable(s))?;
        let target = self.target_at(x, y)?;
        fire::check(&self.map.battle.scenario, &self.defs, sel, target)
    }
}

/// The decoded frames of `ANMSUNIT.FF` animation `name`, decoding them on
/// first use. Full-screen support animations are large: only the latest is
/// kept.
fn load_animation<'g, 'r>(
    gfx: &'g mut Gfx<'r>,
    viewer: &Viewer,
    name: &str,
) -> Option<&'g hud::Anim<'r>> {
    let creator = gfx.creator();
    if name.starts_with("AUTOUNIT\\") && !gfx.anims.contains_key(name) {
        gfx.anims
            .retain(|k, _| !k.starts_with("AUTOUNIT\\") || k == name);
    }
    gfx.anims
        .entry(name.to_string())
        .or_insert_with(|| {
            let ff = viewer.anims.as_deref()?;
            let frames = movement::animation_frames(ff, name)?;
            let first = frames.first()?;
            let size = (first.width as i32, first.height as i32);
            let frames = frames
                .iter()
                .map(|f| {
                    let (img, (x, y)) = movement::crop(f)?;
                    Some((hud::texture(creator, &img)?, (x as i32, y as i32)))
                })
                .collect();
            Some(hud::Anim { size, frames })
        })
        .as_ref()
}

/// Decodes the playing support's animation if it is not ready yet, and
/// restarts the support's clock so it plays from its first frame: the
/// decoding takes a noticeable time, which the animation would otherwise
/// skip, and the frame before it already shows the view centred on the
/// target.
pub fn preload_support(gfx: &mut Gfx, viewer: &mut Viewer) {
    let Some(name) = viewer
        .shot
        .as_ref()
        .filter(|s| s.impact_name.starts_with("AUTOUNIT\\"))
        .map(|s| s.impact_name.clone())
    else {
        return;
    };
    if gfx.anims.contains_key(&name) {
        return;
    }
    load_animation(gfx, viewer, &name);
    let now = viewer.clock.now();
    if let Some(s) = &mut viewer.shot {
        s.started = now;
    }
}

/// Draws frame `frame` (looping) of `ANMSUNIT.FF` animation `name`
/// centred on map pixel `centre`; `false` when it has no frames.
pub fn draw_animation(
    canvas: &mut Canvas<Window>,
    gfx: &mut Gfx,
    viewer: &Viewer,
    name: &str,
    frame: usize,
    centre: (i32, i32),
    screen: &dyn Fn(f64, f64) -> (f32, f32),
) -> Result<bool> {
    // A support's full-screen animation is decoded between frames
    // (`preload_support`), never while drawing one.
    if name.starts_with("AUTOUNIT\\") && !gfx.anims.contains_key(name) {
        return Ok(false);
    }
    let anim = load_animation(gfx, viewer, name);
    let Some(anim) = anim.filter(|a| !a.frames.is_empty()) else {
        return Ok(false);
    };
    let Some((texture, (dx, dy))) = &anim.frames[frame % anim.frames.len()] else {
        return Ok(true);
    };
    let (w, h) = anim.size;
    let (tw, th) = (texture.width() as i32, texture.height() as i32);
    // Centred on the point (`0x468515`), halves rounded down.
    let (ox, oy) = (
        f64::from(centre.0 - w / 2 + dx),
        f64::from(centre.1 - h / 2 + dy),
    );
    let (x0, y0) = screen(ox, oy);
    let (x1, y1) = screen(ox + f64::from(tw), oy + f64::from(th));
    canvas.copy(texture, None, Some(FRect::new(x0, y0, x1 - x0, y1 - y0)))?;
    Ok(true)
}

/// Draws a walking or shooting unit; `screen` maps map pixels to output
/// pixels.
pub(crate) fn draw_walker(
    canvas: &mut Canvas<Window>,
    gfx: &mut Gfx,
    viewer: &Viewer,
    w: &Walker,
    screen: &dyn Fn(f64, f64) -> (f32, f32),
) -> Result {
    let ((cx, cy), facing) = (w.centre, w.facing);
    if let Some(name) = &w.anim
        && draw_animation(canvas, gfx, viewer, name, w.frame, w.centre, screen)?
    {
        return Ok(());
    }
    let unit = &viewer.map.battle.scenario.units[w.unit];
    let Some(kind) = viewer.defs.unit(unit.kind) else {
        return Ok(());
    };
    // No animation: slide the standing sprite.
    let (x, y) = (
        f64::from(cx - SPRITE_W / 2),
        f64::from(cy - SPRITE_ANCHOR_Y),
    );
    let still = viewer.art.unit_frame(unit.kind, unit.owner).unwrap_or(0);
    let sprite = kind.sprite(0, 0, i64::from(facing), still);
    if let Some(texture) = gfx.sprite(&viewer.art, Sheet::Ub, &sprite) {
        let (x0, y0) = screen(x, y);
        let (x1, y1) = screen(x + 54.0, y + 56.0);
        canvas.copy(texture, None, Some(FRect::new(x0, y0, x1 - x0, y1 - y0)))?;
    }
    Ok(())
}

/// Draws the shot's projectile and impact, over everything.
pub fn draw_shot_effects(
    canvas: &mut Canvas<Window>,
    gfx: &mut Gfx,
    viewer: &Viewer,
    screen: &dyn Fn(f64, f64) -> (f32, f32),
) -> Result {
    let now = viewer.clock.now();
    for s in viewer.flights.iter().chain(&viewer.shot) {
        let f = s.frame(now);
        if let Some((at, frame)) = s.projectile(f) {
            let name = shot::animation_name(&s.image, s.owner, Anim::Projectile, 0);
            draw_animation(canvas, gfx, viewer, &name, frame, at, screen)?;
        }
        if let Some(frame) = s.impact(f) {
            let size = viewer.map.battle.scenario.size;
            let at = tile_center(size, s.lands.0, s.lands.1);
            draw_animation(canvas, gfx, viewer, &s.impact_name, frame, at, screen)?;
        }
        for (_, e, frame) in s.explosions(f) {
            draw_animation(canvas, gfx, viewer, &e.name, frame, e.centre, screen)?;
        }
    }
    Ok(())
}

/// Path dots of the preview, or of the rest of the walks.
pub fn path_dots(viewer: &Viewer) -> Vec<(i64, i64, &'static str)> {
    let dots = |steps: &[Step], affordable: usize| -> Vec<(i64, i64, &'static str)> {
        steps
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let dot = movement::path_dot(s, i + 1 == steps.len(), i < affordable);
                (i64::from(s.x), i64::from(s.y), dot)
            })
            .collect()
    };
    if !viewer.motions.is_empty() {
        return viewer
            .motions
            .iter()
            .flat_map(|m| dots(&m.steps[m.done.min(m.steps.len())..], usize::MAX))
            .collect();
    }
    viewer
        .preview
        .as_ref()
        .map(|p| dots(&p.steps, p.affordable))
        .unwrap_or_default()
}
