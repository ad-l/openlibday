//! GROUP: up to five units selected together, moved and fired with in one
//! order (`view+0x134`). With the box checked a unit click adds the unit to
//! the selection (or takes it out); the unit last added leads. A group
//! order leaves the group selected, but the next unit click starts a new
//! selection (`view+0x1c8`).

use libday_view::fire::{self, Fire, Target};
use libday_view::movement::{self, Access, executable_steps, move_budget};

use crate::{Viewer, combat};

/// Most units selected at once.
pub(crate) const GROUP_MAX: usize = 5;

/// Tiles around a group's destination the others stop on, in the order the
/// original tries them.
const RING: [(i64, i64); 8] = [
    (-1, 0),
    (0, -1),
    (0, 1),
    (1, 0),
    (-1, -1),
    (-1, 1),
    (1, -1),
    (1, 1),
];

impl Viewer {
    /// The selected units, the leader last.
    pub(crate) fn selection(&self) -> Vec<usize> {
        match self.selected {
            Some(sel) if self.group.last() == Some(&sel) => self.group.clone(),
            Some(sel) => vec![sel],
            None => Vec::new(),
        }
    }

    /// A click on own unit `u` (`0x435934`): with GROUP it joins the
    /// selection, or leaves it if already in; without, it replaces it.
    /// Returns whether a unit was selected.
    pub(crate) fn pick(&mut self, u: usize) -> bool {
        let mut group = if self.grouping && !self.stale {
            self.selection()
        } else {
            Vec::new()
        };
        self.stale = false;
        if let Some(at) = group.iter().position(|&g| g == u) {
            group.remove(at);
        } else if group.len() >= GROUP_MAX {
            self.say(0x244, "Too many selected units");
            return false;
        } else {
            group.push(u);
        }
        let leader = group.last().copied();
        if self.selected != leader {
            self.preview = None;
            self.fire_mode = None;
        }
        self.lock = None;
        self.selected = leader;
        self.group = group;
        leader == Some(u)
    }

    /// Shows game string `id` in the message bar.
    pub(crate) fn say(&mut self, id: u32, fallback: &str) {
        let strings = self.game.strings().unwrap_or_default();
        self.flow.message = Some(strings.get(&id).cloned().unwrap_or_else(|| fallback.into()));
    }

    /// The GROUP box; unchecking it keeps only the leader.
    pub(crate) fn toggle_group(&mut self) {
        self.grouping ^= true;
        if !self.grouping {
            self.group = self.selected.into_iter().collect();
            self.preview = None;
            self.lock = None;
        }
    }

    /// The group's units able to take an order, the leader first.
    fn members(&self) -> Vec<usize> {
        let units = &self.map.battle.scenario.units;
        let mut out = self.selection();
        out.reverse();
        out.retain(|&u| self.controllable(u) && units[u].arrival == 0);
        out
    }

    /// Moves the selection to `target` (`0x452165`): the leader onto it,
    /// the others onto free tiles around it, each as far as its AP goes.
    /// Units are planned one after the other, each standing where the ones
    /// before end, then all walk together.
    pub(crate) fn move_group(&mut self, target: (i64, i64)) {
        self.preview = None;
        self.stale = true;
        let mut todo = self.members();
        let mut walks = Vec::new();
        let saved: Vec<(usize, (i64, i64))> = {
            let units = &self.map.battle.scenario.units;
            todo.iter()
                .map(|&u| (u, (units[u].x, units[u].y)))
                .collect()
        };
        // A unit blocked by another one still to move is tried again once
        // that one has.
        loop {
            let before = todo.len();
            todo.retain(|&u| match self.plan_member(u, target) {
                Some((steps, ap)) => {
                    let to = steps.last().map(|s| (i64::from(s.x), i64::from(s.y)));
                    let unit = &mut self.map.battle.scenario.units[u];
                    if let Some(to) = to {
                        (unit.x, unit.y) = to;
                    }
                    walks.push((u, steps, ap));
                    false
                }
                None => true,
            });
            if todo.is_empty() || todo.len() == before {
                break;
            }
        }
        for &(u, at) in &saved {
            let unit = &mut self.map.battle.scenario.units[u];
            (unit.x, unit.y) = at;
        }
        if walks.is_empty() {
            self.say(0x241, "Unit can't reach that position");
            return;
        }
        self.launch(walks);
    }

    /// Path and AP left of member `u` moving to `target` or next to it.
    fn plan_member(&self, u: usize, target: (i64, i64)) -> Option<(Vec<movement::Step>, i64)> {
        let scenario = &self.map.battle.scenario;
        let unit = scenario.units.get(u)?;
        let grid = movement::move_grid(scenario, &self.defs, u)?;
        let from = (unit.x as i32, unit.y as i32);
        let size = scenario.size as i64;
        let steps = std::iter::once((0, 0))
            .chain(RING)
            .map(|(dx, dy)| (target.0 + dx, target.1 + dy))
            .filter(|&(x, y)| (0..size).contains(&x) && (0..size).contains(&y))
            .filter(|&(x, y)| grid.get(x as i32, y as i32).0 == Access::Stop)
            .find_map(|(x, y)| grid.find_path(from, (x as i32, y as i32)))?;
        self.affordable_walk(u, steps)
    }

    /// `steps` cut to what unit `u` can pay for (keeping its overwatch
    /// shot), with the AP it has left; `None` if it cannot take a step.
    fn affordable_walk(
        &self,
        u: usize,
        mut steps: Vec<movement::Step>,
    ) -> Option<(Vec<movement::Step>, i64)> {
        let (ap, _) = self.action_points(u)?;
        let reserve = self
            .kind_of(u)
            .filter(|_| self.overwatch(u))
            .map(|k| k.shoot_action_points.max(0) as u32);
        let (n, spent) = executable_steps(&steps, move_budget(ap.max(0) as u32, reserve));
        steps.truncate(n);
        (!steps.is_empty()).then_some((steps, ap - i64::from(spent)))
    }

    /// Starts the walks of one group order together.
    fn launch(&mut self, walks: Vec<(usize, Vec<movement::Step>, i64)>) {
        for (nth, (u, steps, ap)) in walks.into_iter().enumerate() {
            self.start_batch_walk(u, steps, ap, nth, true);
            // Fired on before it set off: the rest stay put.
            if self
                .ends
                .iter()
                .any(|(_, e)| *e == crate::moves::End::Spotted)
            {
                break;
            }
        }
    }

    /// The selection attacks `target`: each member walks into range if it
    /// must, then all fire in turn, the leader first.
    pub(crate) fn fire_group(&mut self, target: Target, repeat: bool) {
        self.preview = None;
        self.lock = None;
        self.stale = true;
        let members = self.members();
        let saved: Vec<(usize, (i64, i64))> = {
            let units = &self.map.battle.scenario.units;
            members
                .iter()
                .map(|&u| (u, (units[u].x, units[u].y)))
                .collect()
        };
        let mut walks = Vec::new();
        let mut volley = Vec::new();
        for &u in &members {
            match fire::check(&self.map.battle.scenario, &self.defs, u, target) {
                Some(Fire::Clear(_)) => volley.push((u, target, repeat)),
                Some(Fire::OutOfRange | Fire::TooClose | Fire::Blocked) => {
                    let Some(p) = self.approach_avoiding(u, target, &[]) else {
                        continue;
                    };
                    let Some((steps, ap)) = self.affordable_walk(u, p.steps) else {
                        continue;
                    };
                    if let Some(s) = steps.last() {
                        let unit = &mut self.map.battle.scenario.units[u];
                        (unit.x, unit.y) = (i64::from(s.x), i64::from(s.y));
                    }
                    walks.push((u, steps, ap));
                    volley.push((u, target, repeat));
                }
                _ => {}
            }
        }
        for &(u, at) in &saved {
            let unit = &mut self.map.battle.scenario.units[u];
            (unit.x, unit.y) = at;
        }
        self.volley = volley;
        self.launch(walks);
    }

    /// Fires the group's next shot; returns whether one started.
    pub(crate) fn next_volley(&mut self) -> bool {
        while !self.volley.is_empty() {
            let (u, target, repeat) = self.volley.remove(0);
            let scenario = &self.map.battle.scenario;
            let clear = (!repeat || combat::still_hostile(scenario, u, target))
                && matches!(
                    fire::check(scenario, &self.defs, u, target),
                    Some(Fire::Clear(_))
                );
            if clear {
                self.start_shot(u, target, repeat, false);
                if self.shot.is_some() {
                    return true;
                }
            }
        }
        false
    }
}
