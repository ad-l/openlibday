//! Unit movement: path preview on a single click, the move itself on a
//! second click on the same tile (or a double click), animated along the
//! path with the unit's walk animation and charged in action points.

use std::time::Duration;

use libday_view::movement::{
    self, ANIMATION_FPS, ANIMATION_OFFSET, Step, affordable_steps, executable_steps, move_budget,
};
use libday_view::{Sheet, sprite_origin, tile_center};
use sdl3::render::{Canvas, FRect};
use sdl3::video::Window;

use crate::Viewer;
use crate::hud::{self, Gfx, Result};

/// Path shown after a single click.
pub struct Preview {
    pub unit: usize,
    pub target: (i64, i64),
    pub steps: Vec<Step>,
    /// Leading steps the unit can pay for; later dots are crossed.
    pub affordable: usize,
}

/// Where and how the walking unit is drawn.
struct Walker {
    /// Sprite cell origin in map pixels.
    origin: (f64, f64),
    facing: u8,
    /// Animation frame counter.
    frame: usize,
    /// Tile whose depth the unit is drawn at.
    depth: (i64, i64),
}

/// A unit walking along a path.
pub struct Motion {
    pub unit: usize,
    pub steps: Vec<Step>,
    /// Steps completed; the unit walks onto `steps[done]`.
    pub done: usize,
    /// Tile the current step starts from.
    pub from: (i64, i64),
    /// Clock times the current step and the walk started.
    pub step_started: Duration,
    pub started: Duration,
    /// Map pixels per second.
    pub speed: f64,
    pub final_ap: i64,
}

/// Map pixels between two tile centres.
fn step_length(size: usize, a: (i64, i64), b: (i64, i64)) -> f64 {
    let (ax, ay) = tile_center(size, a.0, a.1);
    let (bx, by) = tile_center(size, b.0, b.1);
    f64::from(bx - ax).hypot(f64::from(by - ay))
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
        if self.motion.is_some() {
            return;
        }
        let unit = self.unit_at(x, y);
        match (self.selected, unit) {
            (_, Some(u)) => {
                if self.selected != Some(u) {
                    self.preview = None;
                }
                self.selected = Some(u);
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

    fn start_move(&mut self) {
        let Some(p) = self.preview.take() else {
            return;
        };
        let Some((ap, _)) = self.action_points(p.unit) else {
            return;
        };
        let budget = move_budget(ap.max(0) as u32, self.reserve(p.unit));
        let (n, spent) = executable_steps(&p.steps, budget);
        let unit = &self.map.battle.scenario.units[p.unit];
        let Some(kind) = self.defs.unit_tech(unit.kind, unit.tech) else {
            return;
        };
        if n == 0 {
            return;
        }
        let now = self.clock.now();
        self.motion = Some(Motion {
            unit: p.unit,
            steps: p.steps[..n].to_vec(),
            done: 0,
            from: (unit.x, unit.y),
            step_started: now,
            started: now,
            speed: kind.move_pix_speed.max(1) as f64 * ANIMATION_FPS,
            final_ap: ap - i64::from(spent),
        });
        self.map.battle.scenario.units[p.unit].facing = i64::from(p.steps[0].facing);
        self.options.hidden_unit = Some(p.unit);
        self.rebuild();
    }

    /// Advances the walking unit; returns whether one is moving.
    pub(crate) fn tick(&mut self) -> bool {
        let size = self.map.battle.scenario.size;
        let Some(m) = &mut self.motion else {
            return false;
        };
        let now = self.clock.now();
        loop {
            let step = m.steps[m.done];
            let to = (i64::from(step.x), i64::from(step.y));
            let length = step_length(size, m.from, to);
            let elapsed = now.saturating_sub(m.step_started).as_secs_f64();
            if elapsed * m.speed < length {
                break;
            }
            let unit = &mut self.map.battle.scenario.units[m.unit];
            (unit.x, unit.y) = to;
            if let Some(ap) = &mut unit.action_points {
                *ap -= i64::from(step.cost);
            } else if let Some(k) = self.defs.unit_tech(unit.kind, unit.tech) {
                unit.action_points = Some(k.action_points - i64::from(step.cost));
            }
            m.step_started += Duration::from_secs_f64(length / m.speed);
            m.from = to;
            m.done += 1;
            if m.done == m.steps.len() {
                let (i, ap) = (m.unit, m.final_ap);
                self.motion = None;
                self.map.battle.scenario.units[i].action_points = Some(ap.max(0));
                self.options.hidden_unit = None;
                self.rebuild();
                self.markers = libday_view::minimap::markers(&self.map.battle.scenario, &self.defs);
                return false;
            }
            unit.facing = i64::from(m.steps[m.done].facing);
        }
        true
    }

    /// The walking unit's sprite cell origin (map pixels), facing, animation
    /// frame counter and the tile whose depth it is drawn at.
    fn walker(&self) -> Option<Walker> {
        let m = self.motion.as_ref()?;
        let size = self.map.battle.scenario.size;
        let step = m.steps.get(m.done)?;
        let to = (i64::from(step.x), i64::from(step.y));
        let length = step_length(size, m.from, to);
        let elapsed = self.clock.since(m.step_started).as_secs_f64();
        let t = (elapsed * m.speed / length).clamp(0.0, 1.0);
        let (ax, ay) = sprite_origin(size, m.from.0, m.from.1);
        let (bx, by) = sprite_origin(size, to.0, to.1);
        let lerp = |a: i32, b: i32| f64::from(a) + (f64::from(b) - f64::from(a)) * t;
        let frame = (self.clock.since(m.started).as_secs_f64() * ANIMATION_FPS) as usize;
        let depth = if (to.0 + to.1, to.0) > (m.from.0 + m.from.1, m.from.0) {
            to
        } else {
            m.from
        };
        Some(Walker {
            origin: (lerp(ax, bx), lerp(ay, by)),
            facing: step.facing,
            frame,
            depth,
        })
    }

    /// Tile the walking unit is drawn after (in back-to-front order).
    pub(crate) fn walker_depth(&self) -> Option<(i64, i64)> {
        self.walker().map(|w| w.depth)
    }
}

/// Draws the walking unit; `screen` maps map pixels to output pixels.
pub fn draw_walker(
    canvas: &mut Canvas<Window>,
    gfx: &mut Gfx,
    viewer: &Viewer,
    screen: &dyn Fn(f64, f64) -> (f32, f32),
) -> Result {
    let (Some(m), Some(w)) = (&viewer.motion, viewer.walker()) else {
        return Ok(());
    };
    let ((x, y), facing, frame) = (w.origin, w.facing, w.frame);
    let unit = &viewer.map.battle.scenario.units[m.unit];
    let Some(kind) = viewer.defs.unit(unit.kind) else {
        return Ok(());
    };
    let name = movement::walk_animation_name(&kind.image, unit.owner, facing);
    let creator = gfx.creator();
    let frames = gfx
        .anims
        .entry(name.clone())
        .or_insert_with(|| {
            let ff = viewer.anims.as_deref()?;
            let frames = movement::animation_frames(ff, &name)?;
            frames.iter().map(|f| hud::texture(creator, f)).collect()
        })
        .as_ref();
    if kind.move_animation
        && let Some(frames) = frames
        && !frames.is_empty()
    {
        let texture = &frames[frame % frames.len()];
        let (w, h) = (texture.width() as f64, texture.height() as f64);
        let (ox, oy) = (
            x - f64::from(ANIMATION_OFFSET.0),
            y - f64::from(ANIMATION_OFFSET.1),
        );
        let (x0, y0) = screen(ox, oy);
        let (x1, y1) = screen(ox + w, oy + h);
        canvas.copy(texture, None, Some(FRect::new(x0, y0, x1 - x0, y1 - y0)))?;
        return Ok(());
    }
    // No animation: slide the standing sprite.
    let still = viewer.art.unit_frame(unit.kind, unit.owner).unwrap_or(0);
    let sprite = kind.sprite(0, 0, i64::from(facing), still);
    if let Some(texture) = gfx.sprite(&viewer.art, Sheet::Ub, &sprite) {
        let (x0, y0) = screen(x, y);
        let (x1, y1) = screen(x + 54.0, y + 56.0);
        canvas.copy(texture, None, Some(FRect::new(x0, y0, x1 - x0, y1 - y0)))?;
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
