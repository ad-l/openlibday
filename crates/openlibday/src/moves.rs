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

/// Where and how the walking (or shooting) unit is drawn.
struct Walker {
    unit: usize,
    /// Point the unit is centred on (a tile centre while standing), in map
    /// pixels.
    centre: (i32, i32),
    facing: u8,
    /// Animation (`None`: slide the standing sprite) and its frame counter.
    anim: Option<String>,
    frame: usize,
    /// Tile whose depth the unit is drawn at.
    depth: (i64, i64),
}

/// How a walk ends.
#[derive(Clone, Copy, PartialEq, Eq)]
enum End {
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
    /// Facing while vanishing.
    pub facing: u8,
}

/// Frames of the vanish and appear animations, shown one per position.
const TELEPORT_FRAMES: usize = 10;

/// Teleport queue: [`TELEPORT_FRAMES`] positions at `start`, then as many on
/// the destination.
fn teleport_queue(size: usize, start: (i64, i64), to: &Step) -> Vec<((i32, i32), usize)> {
    let from = tile_center(size, start.0, start.1);
    let to = tile_center(size, i64::from(to.x), i64::from(to.y));
    let mut out = vec![(from, 0); TELEPORT_FRAMES];
    out.extend(std::iter::repeat_n((to, 0), TELEPORT_FRAMES));
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
        match (self.selected, unit) {
            (Some(sel), Some(u)) if sel == u => self.turn_clockwise(u),
            // Only own units are selected; enemies are targeted.
            (_, Some(u)) if self.controllable(u) => {
                if self.selected != Some(u) {
                    self.preview = None;
                    self.fire_mode = None;
                }
                self.selected = Some(u);
                self.effect(Effect::Select);
            }
            (Some(sel), None) if self.options.arrival == Some(0) && self.controllable(sel) => {
                match &self.preview {
                    Some(p) if p.unit == sel && p.target == (x, y) => self.start_move(),
                    _ => self.preview = self.plan(sel, (x, y)),
                }
            }
            _ => {}
        }
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
        mut steps: Vec<Step>,
        final_ap: i64,
    ) -> bool {
        let Some(unit) = self.map.battle.scenario.units.get(unit_index) else {
            return false;
        };
        let Some(kind) = self.defs.unit_tech(unit.kind, unit.tech) else {
            return false;
        };
        if steps.is_empty() {
            return false;
        }
        // Already covered by overwatch: shot at without moving.
        if self.reactions && overwatch::spotted(&self.map.battle.scenario, &self.defs, unit_index) {
            self.start_reaction(unit_index);
            return true;
        }
        let now = self.clock.now();
        let size = self.map.battle.scenario.size;
        let start = (unit.x, unit.y);
        let pix_speed = kind.move_pix_speed.clamp(1, 1000) as i32;
        let colour = self.anims.as_deref().map_or(unit.owner, |ff| {
            shot::animation_owner(ff, &kind.image, unit.owner)
        });
        let teleport = Movement::from_id(kind.movement) == Movement::Teleport;
        let old_facing = unit.facing.rem_euclid(8) as u8;
        let positions = if teleport {
            // Only the destination counts (`0x42e1c1`); it appears facing a
            // random way (`0x42d9c9`).
            steps.drain(..steps.len() - 1);
            steps[0].facing = self.rng.rand().rem_euclid(8) as u8;
            teleport_queue(size, start, &steps[0])
        } else {
            walk_queue(size, start, &steps, pix_speed)
        };
        let facing = if teleport {
            i64::from(old_facing)
        } else {
            i64::from(steps[0].facing)
        };
        self.motion = Some(Motion {
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
        });
        self.map.battle.scenario.units[unit_index].facing = facing;
        self.options.hidden_unit = Some(unit_index);
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
        let mut out = Vec::new();
        if let Some(w) = self.walker()
            && let Some(name) = w.anim
        {
            let started = match (&self.shot, &self.motion) {
                (Some(s), _) => s.started,
                (None, Some(m)) => m.started,
                (None, None) => Duration::ZERO,
            };
            out.push(Playing {
                slot: 0,
                name,
                started,
                frame: w.frame,
            });
        }
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

    /// Advances the walking or shooting unit; returns whether one is busy.
    fn advance(&mut self) -> bool {
        let Some(m) = &self.motion else {
            return self.tick_shot();
        };
        let shown =
            (self.clock.since(m.started).as_millis() / u128::from(WALK_POSITION_MS)) as usize;
        // Steps whose positions have all been shown are walked.
        let walked = m.positions.get(shown).map_or(m.steps.len(), |&(_, s)| s);
        while let Some(m) = &mut self.motion
            && m.done < walked
        {
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
                return self.stop(i, ap, End::Spotted);
            }
            // A detector stops on sighting a hidden mine; else an armed
            // mine on the tile may go off. Either ends the walk there.
            let detected = mines::detected(scenario, &self.defs, i, to);
            let mine = if detected.is_empty() {
                mines::trigger(scenario, &self.defs, i, to, !last)
            } else {
                None
            };
            for &k in &detected {
                self.map.battle.scenario.mines[k].reveal_to(owner);
            }
            let end = match mine {
                Some(k) => Some(End::Mine(k)),
                None if !detected.is_empty() && !last => Some(End::Interrupted),
                None if last => Some(End::Arrived),
                None => None,
            };
            if let Some(end) = end {
                return self.stop(i, ap, end);
            }
            if let Some(facing) = next {
                self.map.battle.scenario.units[i].facing = facing;
            }
        }
        true
    }

    /// Ends the walk of unit `i` (with `ap` left, if given): it shoots its
    /// locked target if it arrived.
    fn stop(&mut self, i: usize, ap: Option<i64>, end: End) -> bool {
        self.motion = None;
        if let Some(ap) = ap {
            self.map.battle.scenario.units[i].action_points = Some(ap.max(0));
        }
        self.options.hidden_unit = None;
        self.rebuild();
        self.markers = libday_view::minimap::markers(&self.map.battle.scenario, &self.defs);
        if end != End::Arrived {
            self.lock = None;
        }
        match end {
            End::Mine(k) => self.start_mine(i, k),
            End::Spotted => self.start_reaction(i),
            End::Arrived | End::Interrupted => self.arrived(),
        }
        self.busy()
    }

    /// The walking unit's centre (map pixels), facing, animation frame
    /// counter and the tile whose depth it is drawn at.
    fn walker(&self) -> Option<Walker> {
        if let Some(s) = &self.shot {
            let (anim, frame) = s.shooter(s.frame(self.clock.now()))?;
            let unit = self.map.battle.scenario.units.get(s.unit)?;
            let kind = self.defs.unit(unit.kind)?;
            let facing = unit.facing.rem_euclid(8) as u8;
            let size = self.map.battle.scenario.size;
            return Some(Walker {
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
            });
        }
        let m = self.motion.as_ref()?;
        let size = self.map.battle.scenario.size;
        let step = m.steps.get(m.done)?;
        let to = (i64::from(step.x), i64::from(step.y));
        let shown =
            (self.clock.since(m.started).as_millis() / u128::from(WALK_POSITION_MS)) as usize;
        let centre = match m.positions.get(shown) {
            Some(&(p, s)) if s == m.done => p,
            _ => tile_center(size, to.0, to.1),
        };
        let unit = &self.map.battle.scenario.units[m.unit];
        if m.teleport {
            let kind = self.defs.unit(unit.kind)?;
            let (letter, facing, at, frame) = if shown < TELEPORT_FRAMES {
                ('L', m.facing, m.from, shown)
            } else {
                ('N', step.facing, to, shown - TELEPORT_FRAMES)
            };
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
                frame: frame.min(TELEPORT_FRAMES - 1),
                depth: at,
            });
        }
        let frame = shown;
        let depth = if (to.0 + to.1, to.0) > (m.from.0 + m.from.1, m.from.0) {
            to
        } else {
            m.from
        };
        let anim = self
            .defs
            .unit(unit.kind)
            .filter(|k| k.move_animation)
            .map(|k| movement::walk_animation_name(&k.image, m.colour, step.facing));
        Some(Walker {
            unit: m.unit,
            centre,
            facing: step.facing,
            anim,
            frame,
            depth,
        })
    }

    /// Tile the walking unit is drawn after (in back-to-front order).
    pub(crate) fn walker_depth(&self) -> Option<(i64, i64)> {
        self.walker().map(|w| w.depth)
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
    let creator = gfx.creator();
    let frames = gfx
        .anims
        .entry(name.to_string())
        .or_insert_with(|| {
            let ff = viewer.anims.as_deref()?;
            let frames = movement::animation_frames(ff, name)?;
            frames.iter().map(|f| hud::texture(creator, f)).collect()
        })
        .as_ref();
    let Some(frames) = frames.filter(|f| !f.is_empty()) else {
        return Ok(false);
    };
    let texture = &frames[frame % frames.len()];
    let (w, h) = (texture.width() as i32, texture.height() as i32);
    // Centred on the point (`0x468515`), halves rounded down.
    let (ox, oy) = (f64::from(centre.0 - w / 2), f64::from(centre.1 - h / 2));
    let (x0, y0) = screen(ox, oy);
    let (x1, y1) = screen(ox + f64::from(w), oy + f64::from(h));
    canvas.copy(texture, None, Some(FRect::new(x0, y0, x1 - x0, y1 - y0)))?;
    Ok(true)
}

/// Draws the walking or shooting unit; `screen` maps map pixels to output
/// pixels.
pub fn draw_walker(
    canvas: &mut Canvas<Window>,
    gfx: &mut Gfx,
    viewer: &Viewer,
    screen: &dyn Fn(f64, f64) -> (f32, f32),
) -> Result {
    let Some(w) = viewer.walker() else {
        return Ok(());
    };
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

/// Path dots of the preview, or of the rest of the walk.
pub fn path_dots(viewer: &Viewer) -> Vec<(i64, i64, &'static str)> {
    let (steps, affordable): (&[Step], usize) = match (&viewer.motion, &viewer.preview) {
        (Some(m), _) => (&m.steps[m.done..], usize::MAX),
        (None, Some(p)) => (&p.steps, p.affordable),
        _ => return Vec::new(),
    };
    steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let dot = movement::path_dot(s, i + 1 == steps.len(), i < affordable);
            (i64::from(s.x), i64::from(s.y), dot)
        })
        .collect()
}
