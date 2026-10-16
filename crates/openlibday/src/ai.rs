//! The computer player (`docs/ai.md`): the brain (`0x46fcbd`) runs its
//! stages one action step at a time, between the walks and shots it starts,
//! so the AI turn plays out on screen.
//!
//! [`AiMode::Legacy`] reproduces the original's quirks; [`AiMode::Modern`]
//! fixes those listed in `docs/ai.md`.

use libday_data::defs::UnitType;
use libday_data::scenario::{Target as Goal, Unit};
use libday_data::{Definitions, Scenario};
use libday_view::fire::{self, Checker, Fire, Target};
use libday_view::movement::{self, Access, DIRS, Flood, MoveGrid, Step, executable_steps};
use libday_view::support::placeable;

use crate::Viewer;
use crate::settings::AiMode;

/// The record score meaning "no target" (`0x4f4330`).
const SENTINEL: i64 = -65535;
/// Execute calls an action gets per turn.
const MAX_CALLS: u32 = 10;
/// Brain steps per frame at most.
const STEPS_PER_TICK: usize = 200;
/// The computer player.
const AI: i64 = 0;
/// AI priority categories of walls and mines.
const WALL_CATEGORY: i64 = 7;
const MINE_CATEGORY: i64 = 8;
/// AP an action needs to start.
const MIN_AP: i64 = 4;
/// Human types stage 3 counts (`UNIT TYPE ID LOOKUP`): Trooper,
/// Grenadier, Elite (Assault) Trooper, LAV and Light Tank.
const HORDE_TYPES: [i64; 5] = [2, 56, 58, 8, 9];
/// Automated units: the Shubblar Pyro-Maniac drop and the alien transport.
const PYRO_DROP: i64 = 19;
const TRANSPORT: i64 = 7;

/// What a target record points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    None,
    Unit(i64),
    Building(i64),
    Wall,
    Mine,
}

/// The target record (`0x471b65`): the target, its position, the score of
/// the best firing tile and the path there, starting with the unit's tile.
#[derive(Clone, Debug)]
struct Record {
    kind: Kind,
    pos: (i64, i64),
    score: i64,
    path: Vec<Step>,
}

impl Record {
    fn none() -> Self {
        Self {
            kind: Kind::None,
            pos: (-1, -1),
            score: SENTINEL,
            path: Vec::new(),
        }
    }

    fn reset(&mut self) {
        *self = Self::none();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Order {
    /// No order: attack whatever scores best.
    Base,
    /// `shoot_overwatch`: fire from where the unit stands.
    Overwatch,
    Goto,
    GotoFast,
    Escort,
    Defend,
    Patrol,
    Attack,
}

impl Order {
    fn from_id(id: i64) -> Option<Self> {
        Some(match id {
            1 => Self::Goto,
            2 => Self::GotoFast,
            3 => Self::Escort,
            4 => Self::Defend,
            5 => Self::Patrol,
            6 => Self::Attack,
            _ => return None,
        })
    }

    fn id(self) -> i64 {
        match self {
            Self::Base | Self::Overwatch => 0,
            Self::Goto => 1,
            Self::GotoFast => 2,
            Self::Escort => 3,
            Self::Defend => 4,
            Self::Patrol => 5,
            Self::Attack => 6,
        }
    }
}

/// A walk or shot an action started; its outcome is looked at on the next
/// call, once it has played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    /// `0x46e7a5` after the walk, or a forced next state.
    Move {
        then: Option<u8>,
    },
    Fire,
}

#[derive(Clone, Debug)]
struct Action {
    unit: i64,
    order: Order,
    counter: u32,
    state: u8,
    /// Where the last move was headed.
    dest: (i64, i64),
    pending: Option<Pending>,
    /// Escort inside its zone: runs the base action.
    delegate: bool,
    /// Patrol: attacking from the route rather than walking to it.
    attack_mode: bool,
    /// Defend/escort zone `(l, t, r, b)`, right and bottom exclusive.
    zone: (i64, i64, i64, i64),
}

impl Action {
    fn new(unit: i64, order: Order) -> Self {
        Self {
            unit,
            order,
            counter: 0,
            state: 0,
            dest: (-1, -1),
            pending: None,
            delegate: false,
            attack_mode: false,
            zone: (0, 0, 0, 0),
        }
    }
}

/// The AI turn in progress.
pub(crate) struct Brain {
    mode: AiMode,
    stage: u8,
    /// Authored orders (`brain+0xfc`).
    orders: Vec<Action>,
    shooters: Vec<Action>,
    base: Vec<Action>,
    /// The shared best-target record (`brain+0x30`).
    best: Record,
    /// Goto moves to make: unit id and destination.
    batch: Vec<(i64, (i64, i64))>,
    /// The goto batch is walking (`0x47142f`).
    flushing: bool,
    /// Stage 3 Pyro-Maniac drops still to launch.
    drops: u8,
}

impl Brain {
    pub(crate) fn new(mode: AiMode) -> Self {
        Self {
            mode,
            stage: 0,
            orders: Vec::new(),
            shooters: Vec::new(),
            base: Vec::new(),
            best: Record::none(),
            batch: Vec::new(),
            flushing: false,
            drops: 0,
        }
    }
}

/// What an attempt to move did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Issued {
    /// No path left to walk.
    Nothing,
    /// The path is the unit's own tile: it is already there.
    Stay,
    Walk,
    /// Modern: no tile keeps the shot in reserve; give up.
    Abort,
}

fn is_static(k: &UnitType) -> bool {
    matches!(k.movement, 0 | 8)
}

/// Stage 3's Pyro-Maniac drops against hostile units of types `kinds`
/// with `money` (the map's `ReinfRes`): 3, 2 or 1 over 1200, 700 or 300
/// when over 30 units are mostly (over 60%) of one horde type. Their cost
/// is added to the support budget and taken off it again.
fn drop_count(kinds: &[i64], money: i64) -> u8 {
    let n = kinds.len();
    let horde = HORDE_TYPES
        .iter()
        .any(|&t| n > 30 && kinds.iter().filter(|&&k| k == t).count() * 100 > n * 60);
    match money {
        _ if !horde => 0,
        r if r > 1200 => 3,
        r if r > 700 => 2,
        r if r > 300 => 1,
        _ => 0,
    }
}

fn ap_of(defs: &Definitions, u: &Unit) -> i64 {
    defs.unit_tech(u.kind, u.tech)
        .map_or(0, |k| u.action_points.unwrap_or(k.action_points))
        .max(0)
}

/// `0x43422d`: armed with the AP for a shot.
fn can_fire_now(defs: &Definitions, u: &Unit) -> bool {
    defs.unit_tech(u.kind, u.tech)
        .is_some_and(|k| k.has_weapon && ap_of(defs, u) >= k.shoot_action_points)
}

/// `relation(0, u) == 2`: units the AI attacks.
fn hostile(defs: &Definitions, u: &Unit) -> bool {
    if u.arrival != 0 {
        return false;
    }
    u.owner != AI
        || defs
            .unit(u.kind)
            .is_some_and(|k| k.race == 0 && k.is_convoy != k.is_ai_target)
}

/// An AI unit that moves (`0x4348fb`).
fn mobile(defs: &Definitions, u: &Unit) -> bool {
    u.owner == AI
        && u.arrival == 0
        && defs
            .unit_tech(u.kind, u.tech)
            .is_some_and(|k| k.race != 0 && !is_static(k))
}

/// An AI tower (`0x43487f`).
fn stationary(defs: &Definitions, u: &Unit) -> bool {
    u.owner == AI && u.arrival == 0 && defs.unit_tech(u.kind, u.tech).is_some_and(is_static)
}

fn priority(defs: &Definitions, attacker: i64, target: i64) -> i64 {
    let (Ok(a), Ok(t)) = (usize::try_from(attacker), usize::try_from(target)) else {
        return 0;
    };
    defs.ai_priority
        .get(a)
        .and_then(|row| row.get(t))
        .copied()
        .unwrap_or(0)
}

/// Expected damage of the attacker's shots (`0x46cd73`, `0x46cdaf`).
fn damage(k: &UnitType, ap: i64) -> f64 {
    let shots = ap / k.shoot_action_points.max(1);
    (shots * (k.damage_base + k.damage_random / 2)) as f64
}

/// `0x46ce87`: `prio` percent of the kill chance scaled by `factor`, less
/// the path length; a likely kill of a unit adds its `cost`.
fn score(prio: i64, dmg: f64, armour: f64, len: usize, factor: f64, cost: Option<i64>) -> i64 {
    let kill = if armour <= 0.0 {
        100.0
    } else {
        (100.0 * dmg / armour).min(100.0)
    };
    let pen = (10.0 * len as f64).min(500.0);
    let mut base = kill * 0.01 * factor - pen;
    if let Some(cost) = cost
        && kill > 99.0
    {
        base += cost as f64;
    }
    (prio as f64 * 0.01 * base) as i64
}

/// `0x42deef`.
fn closer(p: (i64, i64), q: (i64, i64), r: (i64, i64)) -> bool {
    q == (-1, -1) || (p.0 - r.0).abs() < (q.0 - r.0).abs() || (p.1 - r.1).abs() < (q.1 - r.1).abs()
}

/// `0x46d09e`: keeps the better record; near-ties go to the later one
/// unless the best is closer.
fn keep_best(best: &mut Record, cand: Record, unit: (i64, i64), target: (i64, i64)) {
    if best.score > cand.score + 1 {
        return;
    }
    if best.score < cand.score {
        *best = cand;
        return;
    }
    let r = if best.pos == cand.pos { target } else { unit };
    if !closer(best.pos, cand.pos, r) {
        *best = cand;
    }
}

fn inflate((l, t, r, b): (i64, i64, i64, i64), n: i64) -> (i64, i64, i64, i64) {
    (l - n, t - n, r + n, b + n)
}

fn inside((l, t, r, b): (i64, i64, i64, i64), (x, y): (i64, i64)) -> bool {
    (l..r).contains(&x) && (t..b).contains(&y)
}

/// The border of `(l, t, r, b)` (right and bottom exclusive) in the order
/// of `0x46df6b`, corners repeated.
fn ring((l, t, r, b): (i64, i64, i64, i64)) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    out.extend((l..r).map(|x| (x, t)));
    out.extend((t..b).map(|y| (r - 1, y)));
    out.extend((l + 1..r).rev().map(|x| (x, b - 1)));
    out.extend((t + 1..b).rev().map(|y| (l, y)));
    out
}

/// Entries of `path` (from the unit's tile) a unit with `ap` walks.
fn cut(mode: AiMode, path: &[Step], ap: i64) -> usize {
    match mode {
        AiMode::Legacy => legacy_cut(path, ap),
        AiMode::Modern if path.is_empty() => 0,
        AiMode::Modern => 1 + executable_steps(&path[1..], ap.max(0) as u32).0,
    }
}

/// `0x42d73c`: charges the origin too, and walks on past the AP to a tile
/// it may stop on.
fn legacy_cut(path: &[Step], ap: i64) -> usize {
    let Some(first) = path.first() else {
        return 0;
    };
    if ap < i64::from(first.cost) {
        return 0;
    }
    let (mut ap, mut prev) = (ap, false);
    for (i, s) in path.iter().enumerate() {
        ap -= i64::from(s.cost);
        if prev && ap < 0 {
            return i;
        }
        prev = s.stop;
    }
    path.len()
}

/// Inserts `item` before the first element it is `less` than
/// (`0x4711ae`; the comparators are not orders).
fn insert_sorted<T>(list: &mut Vec<T>, item: T, less: impl Fn(&T, &T) -> bool) {
    let at = list
        .iter()
        .position(|e| less(&item, e))
        .unwrap_or(list.len());
    list.insert(at, item);
}

/// `0x471030`: the opportunity-fire order.
fn less_shooter(a: &UnitType, b: &UnitType) -> bool {
    if is_static(a) {
        return true;
    }
    if is_static(b) {
        return false;
    }
    if a.damage_type == 1 {
        return true;
    }
    if b.damage_type == 1 {
        return false;
    }
    if a.jammer <= 0 {
        return false;
    }
    a.range < b.range
}

/// `0x4710b9`: the unordered units' order.
fn less_base(mode: AiMode, a: &UnitType, b: &UnitType) -> bool {
    if a.damage_type == 1 {
        return true;
    }
    if b.damage_type == 1 {
        return false;
    }
    if a.jammer <= 0 {
        return true;
    }
    if b.jammer <= 0 {
        return false;
    }
    if a.jammer < b.jammer {
        return true;
    }
    // The original compares b with itself here.
    if mode == AiMode::Modern && b.jammer < a.jammer {
        return false;
    }
    a.range > b.range
}

/// Paths from a unit's tile, each starting with that tile.
struct Paths<'a> {
    grid: &'a MoveGrid,
    flood: Flood<'a>,
    origin: Step,
}

impl<'a> Paths<'a> {
    fn new(grid: &'a MoveGrid, (x, y): (i64, i64)) -> Self {
        let (x, y) = (x as i32, y as i32);
        Self {
            grid,
            flood: grid.flood((x, y)),
            origin: Step {
                x,
                y,
                facing: 0,
                cost: grid.get(x, y).1,
                stop: true,
            },
        }
    }

    fn to(&self, (x, y): (i64, i64)) -> Option<Vec<Step>> {
        let o = self.origin;
        if (x, y) == (i64::from(o.x), i64::from(o.y)) {
            return (self.grid.get(o.x, o.y).0 == Access::Stop).then(|| vec![o]);
        }
        let (x, y) = (i32::try_from(x).ok()?, i32::try_from(y).ok()?);
        let mut path = self.flood.path((x, y))?;
        path.insert(0, o);
        Some(path)
    }
}

/// Target evaluation for one attacker (`0x46de6e` and siblings).
struct Eval<'a> {
    s: &'a Scenario,
    defs: &'a Definitions,
    mode: AiMode,
    idx: usize,
    unit: &'a Unit,
    kind: &'a UnitType,
    ap: i64,
    /// None: only scoring targets in range.
    paths: Option<&'a Paths<'a>>,
}

impl Eval<'_> {
    fn pos(&self) -> (i64, i64) {
        (self.unit.x, self.unit.y)
    }

    /// Priority of shooting unit `e` (`0x46ce87`, kind 0).
    fn unit_priority(&self, e: &Unit) -> i64 {
        let Some(ek) = self.defs.unit_tech(e.kind, e.tech) else {
            return 0;
        };
        let mut p = priority(self.defs, self.kind.ai_category, ek.ai_category);
        if e.kind == self.unit.order.enemy_type {
            p *= 2;
        }
        if self.kind.damage_type == 1 && !can_fire_now(self.defs, e) {
            p = 0;
        }
        p
    }

    /// Scores the firing tiles from which `checker` is clear.
    fn consider(
        &self,
        best: &mut Record,
        checker: &Checker,
        tiles: &[(i64, i64)],
        kind: Kind,
        pos: (i64, i64),
        score: impl Fn(usize) -> i64,
    ) {
        let Some(paths) = self.paths else {
            return;
        };
        let size = self.s.size as i64;
        for &t in tiles {
            if !(0..size).contains(&t.0) || !(0..size).contains(&t.1) {
                continue;
            }
            let Some(path) = paths.to(t) else {
                continue;
            };
            if !matches!(checker.check(t, self.ap), Fire::Clear(_)) {
                continue;
            }
            let cand = Record {
                kind,
                pos,
                score: score(path.len()),
                path,
            };
            keep_best(best, cand, self.pos(), pos);
        }
    }

    fn unit(&self, best: &mut Record, j: usize) {
        let e = &self.s.units[j];
        let (Some(ek), Some(ek0)) = (self.defs.unit_tech(e.kind, e.tech), self.defs.unit(e.kind))
        else {
            return;
        };
        let Some(checker) = Checker::new(self.s, self.defs, self.idx, Target::Unit(j)) else {
            return;
        };
        let r = self.kind.range;
        let m = self.kind.min_range.max(1);
        let rect = (e.x, e.y, e.x + ek0.width, e.y + ek0.height);
        let mut tiles = ring(inflate(rect, r));
        if r > 1 {
            tiles.extend(ring(inflate(rect, m)));
            tiles.extend(ring(inflate(rect, (r + m) / 2)));
        }
        let armour = e.armor.unwrap_or(ek.armor) as f64;
        let (p, dmg, cost) = (self.unit_priority(e), damage(self.kind, self.ap), ek0.cost);
        self.consider(
            best,
            &checker,
            &tiles,
            Kind::Unit(e.id),
            (e.x, e.y),
            |len| score(p, dmg, armour, len, 500.0, Some(cost)),
        );
    }

    fn building(&self, best: &mut Record, j: usize) {
        let b = &self.s.buildings[j];
        let Some(bk) = self.defs.buildings.get(&b.kind) else {
            return;
        };
        let Some(checker) = Checker::new(self.s, self.defs, self.idx, Target::Building(j)) else {
            return;
        };
        let (l, t, r, bt) = (b.x, b.y, b.x + bk.width, b.y + bk.height);
        let tiles = match self.mode {
            // x and y mixed up in the original.
            AiMode::Legacy => [
                (l - 1, t - 1),
                (r + 1, bt + 1),
                (t + 1, r - 1),
                (bt - 1, l + 1),
            ],
            AiMode::Modern => [(l - 1, t - 1), (r, bt), (r, t - 1), (l - 1, bt)],
        };
        let mut p = priority(self.defs, self.kind.ai_category, bk.ai_category);
        if self.kind.damage_type == 1 {
            p = 0;
        }
        let (armour, dmg) = (b.armor as f64, damage(self.kind, self.ap));
        self.consider(
            best,
            &checker,
            &tiles,
            Kind::Building(b.id),
            (l, t),
            |len| score(p, dmg, armour, len, 1000.0, None),
        );
    }

    /// A wall or mine on `pos`: the tiles around it.
    fn tile(&self, best: &mut Record, kind: Kind, pos: (i64, i64), armour: i64) {
        let Some(checker) = Checker::new(self.s, self.defs, self.idx, Target::Tile(pos.0, pos.1))
        else {
            return;
        };
        let (category, factor) = match kind {
            Kind::Wall => (WALL_CATEGORY, 5.0),
            _ => (MINE_CATEGORY, 500.0),
        };
        // The original's mine loop misses the last row and column.
        let high = i64::from(kind == Kind::Wall || self.mode == AiMode::Modern);
        let tiles: Vec<(i64, i64)> = (-1..=high)
            .flat_map(|dx| (-1..=high).map(move |dy| (pos.0 + dx, pos.1 + dy)))
            .collect();
        let mut p = priority(self.defs, self.kind.ai_category, category);
        if self.kind.damage_type == 1 {
            p = 0;
        }
        let dmg = damage(self.kind, self.ap);
        self.consider(best, &checker, &tiles, kind, pos, |len| {
            score(p, dmg, armour as f64, len, factor, None)
        });
    }
}

impl Viewer {
    /// Runs the AI turn while nothing plays; returns whether anything
    /// happened.
    pub(crate) fn run_brain(&mut self) -> Option<bool> {
        let mut brain = self.brain.take()?;
        let mut acted = false;
        for _ in 0..STEPS_PER_TICK {
            if self.busy() {
                self.brain = Some(brain);
                return Some(acted);
            }
            acted = true;
            if self.ai_step(&mut brain) {
                return Some(true);
            }
            // The objective is judged after every AI step.
            if self.judge() {
                return Some(true);
            }
        }
        self.brain = Some(brain);
        Some(acted)
    }

    /// One brain step; returns whether the turn is over.
    fn ai_step(&mut self, b: &mut Brain) -> bool {
        match b.stage {
            0 => {
                self.build_orders(b);
                b.drops = self.pyro_drops();
                b.stage = 3;
            }
            3 => {
                if b.drops == 0 {
                    b.stage = 4;
                } else {
                    b.drops -= 1;
                    self.pyro_drop();
                }
            }
            4 => {
                self.build_shooters(b);
                b.stage = 5;
            }
            5 => {
                if !self.run_first(b, |b| &mut b.shooters) {
                    b.stage = 8;
                }
            }
            8 => {
                self.build_base(b);
                b.stage = 9;
            }
            9 => {
                if !self.run_first(b, |b| &mut b.base) {
                    b.stage = 10;
                }
            }
            10 | 11 => {
                let (order, keep) = if b.stage == 10 {
                    (Order::Goto, true)
                } else {
                    (Order::GotoFast, false)
                };
                if !self.run_order(b, Some(order)) && self.flush_step(b, keep) {
                    b.stage += 1;
                }
            }
            12 => {
                if !self.run_order(b, None) {
                    b.stage = 13;
                }
            }
            _ => return true,
        }
        false
    }

    /// One call of the first action of a list; false when it is empty.
    fn run_first(&mut self, b: &mut Brain, list: fn(&mut Brain) -> &mut Vec<Action>) -> bool {
        if list(b).is_empty() {
            return false;
        }
        let mut a = list(b).remove(0);
        if !self.execute(b, &mut a) {
            list(b).insert(0, a);
        }
        true
    }

    /// One call of the first authored order (of kind `order`).
    fn run_order(&mut self, b: &mut Brain, order: Option<Order>) -> bool {
        let Some(i) = b
            .orders
            .iter()
            .position(|a| order.is_none_or(|o| a.order == o))
        else {
            return false;
        };
        let mut a = b.orders.remove(i);
        if !self.execute(b, &mut a) {
            b.orders.insert(i, a);
        }
        true
    }

    fn units(&self) -> &[Unit] {
        &self.map.battle.scenario.units
    }

    fn unit_index(&self, id: i64) -> Option<usize> {
        self.units()
            .iter()
            .position(|u| u.id == id && u.arrival == 0)
    }

    fn ap(&self, idx: usize) -> i64 {
        ap_of(&self.defs, &self.units()[idx])
    }

    /// Stage 0 (`0x470d71`): orders are kept with their probability and
    /// until their turn, and each ordered unit gets its action.
    fn build_orders(&mut self, b: &mut Brain) {
        let turn = self.map.battle.clock.turn;
        for i in 0..self.units().len() {
            let u = &self.units()[i];
            if u.owner != AI || u.arrival != 0 || u.order.id == 0 {
                continue;
            }
            let p = u.order.probability;
            if self.rng.rand() % 100 > p {
                self.map.battle.scenario.units[i].order.id = 0;
            }
        }
        for u in &mut self.map.battle.scenario.units {
            if u.order.id != 0 && u.order.expiry < turn {
                u.order.id = 0;
            }
        }
        let defs = &self.defs;
        b.orders = self
            .units()
            .iter()
            .filter(|u| mobile(defs, u) && ap_of(defs, u) > MIN_AP)
            .filter_map(|u| Order::from_id(u.order.id).map(|o| Action::new(u.id, o)))
            .collect();
        b.orders.sort_by_key(|a| a.order.id());
    }

    /// Stage 3 (`0x46ff88`): on the first four turns, two times in three,
    /// Pyro-Maniac drops against a horde of light human units: over 30
    /// hostile units, over 60% of them of one type.
    fn pyro_drops(&mut self) -> u8 {
        if self.rng.rand() % 100 > 66 || self.map.battle.clock.turn > 4 {
            return 0;
        }
        let defs = &self.defs;
        let hostiles: Vec<i64> = self
            .units()
            .iter()
            .filter(|u| hostile(defs, u))
            .map(|u| u.kind)
            .collect();
        drop_count(&hostiles, self.map.battle.scenario.info.reinf_res)
    }

    /// `0x47084a`: one drop on the first hostile soft land or hover unit
    /// (armour ≤ 30) from a random place in the unit list, wrapping not.
    fn pyro_drop(&mut self) {
        let len = self.units().len() as i64;
        if len == 0 {
            return;
        }
        let start = (self.rng.rand() % len) as usize;
        let defs = &self.defs;
        let target = self.units()[start..].iter().find(|u| {
            hostile(defs, u)
                && defs.unit_tech(u.kind, u.tech).is_some_and(|k| {
                    u.armor.unwrap_or(k.armor) <= 30 && matches!(k.category, 0 | 1)
                })
        });
        if let Some(u) = target {
            let at = (u.x, u.y);
            self.launch_support(PYRO_DROP, AI, at, None);
        }
    }

    /// `0x46d3e0`: in a battle on the base, a unit with nothing to shoot
    /// is flown by an alien transport next to the human units.
    fn counter_transport(&mut self, idx: usize) {
        let unit = self.units()[idx].clone();
        let Some(k) = self.defs.unit_tech(unit.kind, unit.tech) else {
            return;
        };
        // Boats and submarines.
        if matches!(k.movement, 4 | 6) || self.ap(idx) < 10 {
            return;
        }
        let Some(cost) = self.defs.automated.get(&TRANSPORT).map(|a| a.cost) else {
            return;
        };
        if !self.ai_support_available(TRANSPORT) || cost > self.flow.ai_budget {
            return;
        }
        self.flow.ai_budget -= cost;
        let s = &self.map.battle.scenario;
        let targets: Vec<(i64, i64, i64, i64)> = s
            .units
            .iter()
            .filter(|u| hostile(&self.defs, u))
            .map(|u| {
                let (w, h) = self
                    .defs
                    .unit_tech(u.kind, u.tech)
                    .map_or((1, 1), |k| (k.width.max(1), k.height.max(1)));
                (u.x - 10, u.y - 10, u.x + w + 10, u.y + h + 10)
            })
            .collect();
        for (x0, y0, x1, y1) in targets {
            let mut tiles: Vec<(i64, i64)> = (x0..x1)
                .flat_map(|x| (y0..y1).map(move |y| (x, y)))
                .collect();
            // `0x4412f3` shuffles the rectangle's tiles.
            for i in 0..tiles.len() {
                let j = (self.rng.rand() % tiles.len() as i64) as usize;
                tiles.swap(i, j);
            }
            let s = &self.map.battle.scenario;
            let dest = tiles
                .into_iter()
                .find(|&t| placeable(s, &self.defs, unit.kind, unit.tech, AI, t));
            if let Some(dest) = dest {
                self.launch_support(TRANSPORT, AI, (unit.x, unit.y), Some(dest));
                return;
            }
        }
    }

    /// Stage 4: every unit that could fire, towers first.
    fn build_shooters(&mut self, b: &mut Brain) {
        let defs = &self.defs;
        let units = self.units();
        let kind = |i: usize| defs.unit_tech(units[i].kind, units[i].tech);
        let mut list: Vec<usize> = Vec::new();
        let movers = (0..units.len()).filter(|&i| {
            mobile(defs, &units[i]) && ap_of(defs, &units[i]) > MIN_AP && kind(i).is_some()
        });
        let towers = (0..units.len()).filter(|&i| stationary(defs, &units[i]) && kind(i).is_some());
        for i in movers.chain(towers) {
            insert_sorted(&mut list, i, |&a, &e| match (kind(a), kind(e)) {
                (Some(a), Some(e)) => less_shooter(a, e),
                _ => false,
            });
        }
        b.shooters = list
            .into_iter()
            .map(|i| Action::new(units[i].id, Order::Overwatch))
            .collect();
    }

    /// Stage 8: the units without orders, when there is anything to attack.
    fn build_base(&mut self, b: &mut Brain) {
        let defs = &self.defs;
        let units = self.units();
        b.base.clear();
        if !units.iter().any(|u| hostile(defs, u)) {
            return;
        }
        let kind = |i: usize| defs.unit_tech(units[i].kind, units[i].tech);
        let mut list: Vec<usize> = Vec::new();
        for (i, u) in units.iter().enumerate() {
            if !mobile(defs, u) || u.order.id != 0 || kind(i).is_none() {
                continue;
            }
            insert_sorted(&mut list, i, |&a, &e| match (kind(a), kind(e)) {
                (Some(a), Some(e)) => less_base(b.mode, a, e),
                _ => false,
            });
        }
        b.base = list
            .into_iter()
            .map(|i| Action::new(units[i].id, Order::Base))
            .collect();
    }

    /// One execute call; returns whether the action is finished.
    fn execute(&mut self, b: &mut Brain, a: &mut Action) -> bool {
        self.resolve(b, a);
        let Some(idx) = self.unit_index(a.unit) else {
            return true;
        };
        a.counter += 1;
        if a.counter > MAX_CALLS {
            return true;
        }
        match a.order {
            Order::Base => self.exec_base(b, a, idx),
            Order::Overwatch => self.exec_overwatch(b, a, idx),
            Order::Goto | Order::GotoFast => self.exec_goto(b, a, idx),
            Order::Escort | Order::Defend if a.delegate => {
                // The original counts the call twice.
                if b.mode == AiMode::Legacy {
                    a.counter += 1;
                    if a.counter > MAX_CALLS {
                        return true;
                    }
                }
                self.exec_base(b, a, idx)
            }
            Order::Escort | Order::Defend => self.exec_guard(b, a, idx),
            Order::Patrol => self.exec_patrol(b, a, idx),
            Order::Attack => self.exec_attack(b, a, idx),
        }
    }

    /// The outcome of the walk or shot started by the last call.
    fn resolve(&self, b: &Brain, a: &mut Action) {
        match a.pending.take() {
            None => {}
            Some(Pending::Move { then }) => {
                a.state = match self.unit_index(a.unit) {
                    None => 3,
                    Some(_) if then.is_some() => then.unwrap_or(0),
                    Some(i) => {
                        let u = &self.units()[i];
                        if (u.x, u.y) == a.dest { 2 } else { 0 }
                    }
                };
            }
            // Destroyed: look for another target.
            Some(Pending::Fire) => a.state = if self.valid(&b.best) { 3 } else { 0 },
        }
    }

    /// The target still exists (`0x471a92`).
    fn valid(&self, r: &Record) -> bool {
        let s = &self.map.battle.scenario;
        match r.kind {
            Kind::None => false,
            Kind::Unit(id) => s
                .units
                .iter()
                .any(|u| u.id == id && u.arrival == 0 && (u.x, u.y) == r.pos),
            Kind::Building(id) => s.buildings.iter().any(|b| b.id == id),
            Kind::Wall => s.walls.iter().any(|w| (w.x, w.y) == r.pos),
            Kind::Mine => s.mines.iter().any(|m| (m.x, m.y) == r.pos),
        }
    }

    fn record_target(&self, r: &Record) -> Option<Target> {
        let s = &self.map.battle.scenario;
        match r.kind {
            Kind::None => None,
            Kind::Unit(id) => self.unit_index(id).map(Target::Unit),
            Kind::Building(id) => s
                .buildings
                .iter()
                .position(|b| b.id == id)
                .map(Target::Building),
            Kind::Wall | Kind::Mine => Some(Target::Tile(r.pos.0, r.pos.1)),
        }
    }

    fn clear(&self, idx: usize, target: Target) -> bool {
        matches!(
            fire::check(&self.map.battle.scenario, &self.defs, idx, target),
            Some(Fire::Clear(_))
        )
    }

    /// The movement grid an action plans on (`0x42da30`): legacy limits
    /// it to 20 tiles around the unit unless it is in the centre.
    fn ai_grid(&mut self, mode: AiMode, idx: usize) -> Option<MoveGrid> {
        let s = &self.map.battle.scenario;
        let mut grid = movement::move_grid(s, &self.defs, idx)?;
        let u = &s.units[idx];
        let boat = self.defs.unit_tech(u.kind, u.tech)?.movement == 4;
        if mode == AiMode::Legacy && !boat {
            let size = s.size as i64;
            let (x, y) = (u.x, u.y);
            if size > 56 {
                self.rng.rand();
            }
            let centre = (size - 12) / 2..(size - 12) / 2 + 12;
            let (lo, hi) = if centre.contains(&x) && centre.contains(&y) {
                ((1, 1), (size - 2, size - 2))
            } else {
                (
                    ((x - 20).max(1), (y - 20).max(1)),
                    ((x + 20).min(size - 2), (y + 20).min(size - 2)),
                )
            };
            grid.restrict((lo.0 as i32, lo.1 as i32), (hi.0 as i32, hi.1 as i32));
        }
        Some(grid)
    }

    /// Centres the view on a tile the AI is about to act on: a walk's
    /// destination, or a unit that fires without moving.
    fn watch_tile(&mut self, (x, y): (i64, i64)) {
        self.centre_on_tile(x as f64 + 0.5, y as f64 + 0.5);
    }

    /// Cuts `path` to the unit's AP and walks it (`0x46e7e7`).
    fn issue_move(&mut self, mode: AiMode, a: &mut Action, idx: usize, path: &[Step]) -> Issued {
        let ap = self.ap(idx);
        let n = cut(mode, path, ap);
        if n == 0 {
            return Issued::Nothing;
        }
        let last = path[n - 1];
        a.dest = (i64::from(last.x), i64::from(last.y));
        if n == 1 {
            return Issued::Stay;
        }
        let steps = path[1..n].to_vec();
        let spent: i64 = steps.iter().map(|s| i64::from(s.cost)).sum();
        self.watch_tile(a.dest);
        if self.start_walk(idx, steps, (ap - spent).max(0)) {
            Issued::Walk
        } else {
            Issued::Nothing
        }
    }

    /// After a move (`0x46e7a5`), now or once the walk has played.
    fn moved(&self, a: &mut Action, issued: Issued, then: Option<u8>) {
        match issued {
            Issued::Walk => a.pending = Some(Pending::Move { then }),
            Issued::Abort => a.state = then.unwrap_or(3),
            Issued::Stay | Issued::Nothing => {
                a.state = match then {
                    Some(s) => s,
                    None => {
                        let at = self
                            .unit_index(a.unit)
                            .map(|i| (self.units()[i].x, self.units()[i].y));
                        match at {
                            None => 3,
                            Some(p) if p == a.dest => 2,
                            Some(_) => 0,
                        }
                    }
                }
            }
        }
    }

    // --- the base action (`0x46d172`) ---

    fn exec_base(&mut self, b: &mut Brain, a: &mut Action, idx: usize) -> bool {
        match a.state {
            0 => {
                let grid = self.plan_base(b, a, idx);
                a.state += 1;
                if b.best.score == SENTINEL {
                    a.state = 3;
                    if self.defending_base() {
                        if self.ap(idx) > 10 {
                            self.counter_transport(idx);
                        }
                    } else {
                        self.walk_to_centre(b.mode, a, idx, grid);
                    }
                }
            }
            1 => {
                if let Some(issued) = self.base_move(b, a, idx) {
                    self.moved(a, issued, None);
                }
            }
            2 => self.fire_step(b, a, idx, 3),
            3 => return true,
            _ => {}
        }
        false
    }

    /// `0x46dcc6`: the best enemy unit, revealed mine or, failing those,
    /// wall to shoot, and the tile to shoot it from.
    fn plan_base(&mut self, b: &mut Brain, a: &mut Action, idx: usize) -> Option<MoveGrid> {
        b.best.reset();
        if self.ap(idx) < MIN_AP {
            a.state = 3;
            return None;
        }
        let grid = self.ai_grid(b.mode, idx)?;
        let s = &self.map.battle.scenario;
        let unit = &s.units[idx];
        let kind = self.defs.unit_tech(unit.kind, unit.tech)?;
        let paths = Paths::new(&grid, (unit.x, unit.y));
        let ev = Eval {
            s,
            defs: &self.defs,
            mode: b.mode,
            idx,
            unit,
            kind,
            ap: self.ap(idx),
            paths: Some(&paths),
        };
        for (j, e) in s.units.iter().enumerate() {
            if j != idx && hostile(&self.defs, e) {
                ev.unit(&mut b.best, j);
            }
        }
        for m in &s.mines {
            let shown = !m.hidden_to(AI);
            if shown && (b.mode == AiMode::Legacy || (m.armed && m.owner != AI)) {
                ev.tile(&mut b.best, Kind::Mine, (m.x, m.y), 1);
            }
        }
        if b.best.score == SENTINEL || (b.best.path.len() > 25 && b.best.score < 0) {
            b.best.reset();
            for w in &s.walls {
                ev.tile(&mut b.best, Kind::Wall, (w.x, w.y), w.armor);
            }
        }
        drop(paths);
        Some(grid)
    }

    /// `0x46d5dc`: nothing to attack, so head for the map's centre.
    fn walk_to_centre(&mut self, mode: AiMode, a: &mut Action, idx: usize, grid: Option<MoveGrid>) {
        let Some(grid) = grid else {
            return;
        };
        let u = &self.units()[idx];
        let size = self.map.battle.scenario.size as i64;
        let c0 = (size - 12) / 2;
        let path = {
            let paths = Paths::new(&grid, (u.x, u.y));
            (c0..c0 + 12)
                .flat_map(|x| (c0..c0 + 12).map(move |y| (x, y)))
                .find_map(|t| paths.to(t))
        };
        if let Some(path) = path {
            self.issue_move(mode, a, idx, &path);
        }
    }

    /// `0x46e86c`: towards the firing tile, stopping early where it can
    /// already fire, before the target can fire back, or keeping a shot
    /// for overwatch. `None` when it did not try to move.
    fn base_move(&mut self, b: &mut Brain, a: &mut Action, idx: usize) -> Option<Issued> {
        if b.best.path.is_empty() {
            return None;
        }
        let mode = b.mode;
        let ap = self.ap(idx);
        let path = b.best.path[..cut(mode, &b.best.path, ap)].to_vec();
        let target = self.record_target(&b.best);
        if path.len() <= 1 || target.is_some_and(|t| self.clear(idx, t)) {
            a.state = 2;
            return None;
        }
        if let Some(t) = target
            && let Some(i) = self.first_firing_tile(mode, idx, &path, t)
        {
            return Some(self.issue_move(mode, a, idx, &path[..=i]));
        }
        if let Some(Target::Unit(tu)) = target
            && let Some(keep) = self.before_enemy_fire(mode, idx, &path, tu)
        {
            return Some(match keep {
                Some(i) => self.issue_move(mode, a, idx, &path[..=i]),
                None => Issued::Abort,
            });
        }
        if let Some(keep) = self.keeping_shot(mode, idx, &path) {
            return Some(match keep {
                Some(n) => self.issue_move(mode, a, idx, &path[..n]),
                None => Issued::Abort,
            });
        }
        Some(self.issue_move(mode, a, idx, &path))
    }

    /// `0x46e96e`: the first stop tile on `path` to fire from with more
    /// than a shot's AP left.
    fn first_firing_tile(
        &self,
        mode: AiMode,
        idx: usize,
        path: &[Step],
        target: Target,
    ) -> Option<usize> {
        let checker = Checker::new(&self.map.battle.scenario, &self.defs, idx, target)?;
        let (ap, shot) = (self.ap(idx), checker.shot_cost());
        let mut sum = 0;
        for (i, s) in path.iter().enumerate() {
            if i > 0 || mode == AiMode::Legacy {
                sum += i64::from(s.cost);
            }
            if s.stop
                && ap - sum > shot
                && matches!(
                    checker.check((i64::from(s.x), i64::from(s.y)), ap),
                    Fire::Clear(_)
                )
            {
                return Some(i);
            }
        }
        None
    }

    /// `0x46ea5d`: against a unit that can fire back, stop on the last
    /// tile before its fire reaches the path. `None` when it does not
    /// apply, `Some(None)` to not move.
    fn before_enemy_fire(
        &self,
        mode: AiMode,
        idx: usize,
        path: &[Step],
        tu: usize,
    ) -> Option<Option<usize>> {
        let s = &self.map.battle.scenario;
        let e = s.units.get(tu)?;
        if !can_fire_now(&self.defs, e) {
            return None;
        }
        let (at, ap) = ((e.x, e.y), ap_of(&self.defs, e));
        let hits = |t: Target| {
            Checker::new(s, &self.defs, tu, t)
                .is_some_and(|c| matches!(c.check(at, ap), Fire::Clear(_)))
        };
        if hits(Target::Unit(idx)) {
            return None;
        }
        let tile = |s: &Step| Target::Tile(i64::from(s.x), i64::from(s.y));
        match mode {
            AiMode::Legacy => {
                for i in 0..path.len() {
                    let prev = i.saturating_sub(1);
                    if hits(tile(&path[i])) {
                        if prev == 0 {
                            return None;
                        }
                        if path[prev].stop {
                            return Some(Some(prev));
                        }
                    }
                }
                None
            }
            AiMode::Modern => {
                let i = path.iter().position(|s| hits(tile(s)))?;
                Some(Some((1..i).rev().find(|&j| path[j].stop)?))
            }
        }
    }

    /// `0x46eb78`: overwatch units keep a shot's AP. Returns the entries to
    /// walk, `Some(None)` to give up (modern), `None` when it does not
    /// apply.
    fn keeping_shot(&self, mode: AiMode, idx: usize, path: &[Step]) -> Option<Option<usize>> {
        let k = self.kind_of(idx)?;
        let (ap, shot) = (self.ap(idx), k.shoot_action_points);
        if !k.can_overwatch || ap < shot {
            return None;
        }
        let mut sum = 0;
        match mode {
            AiMode::Legacy => {
                for (i, s) in path.iter().enumerate() {
                    sum += i64::from(s.cost);
                    // Stops on the first tile that breaks the reserve.
                    if s.stop && ap - sum < shot {
                        return Some(Some(i + 1));
                    }
                }
                (ap - sum >= shot).then_some(Some(path.len()))
            }
            AiMode::Modern => {
                let mut last = None;
                for (i, s) in path.iter().enumerate().skip(1) {
                    sum += i64::from(s.cost);
                    if ap - sum < shot {
                        break;
                    }
                    if s.stop {
                        last = Some(i + 1);
                    }
                }
                Some(last)
            }
        }
    }

    /// `0x46d268`: empties the AP into the target if it can be hit.
    fn fire_step(&mut self, b: &Brain, a: &mut Action, idx: usize, invalid: u8) {
        if !self.valid(&b.best) {
            a.state = invalid;
            return;
        }
        if let Some(t) = self.record_target(&b.best)
            && self.clear(idx, t)
        {
            let u = &self.units()[idx];
            self.watch_tile((u.x, u.y));
            self.start_shot(idx, t, true, false);
            if self.shot.is_some() {
                a.pending = Some(Pending::Fire);
                return;
            }
        }
        a.state = 3;
    }

    // --- shoot_overwatch (`0x46f11e`) ---

    fn exec_overwatch(&mut self, b: &mut Brain, a: &mut Action, idx: usize) -> bool {
        match a.state {
            0 => {
                b.best.reset();
                self.select_in_range(b, idx);
                if b.best.score == SENTINEL {
                    return true;
                }
                a.state = 1;
            }
            1 => a.state = 2,
            2 => self.fire_step(b, a, idx, 0),
            _ => return true,
        }
        false
    }

    /// `0x46f1c0`: the best unit to shoot from where the unit stands.
    fn select_in_range(&self, b: &mut Brain, idx: usize) {
        let s = &self.map.battle.scenario;
        let unit = &s.units[idx];
        let Some(kind) = self.kind_of(idx) else {
            return;
        };
        let ev = Eval {
            s,
            defs: &self.defs,
            mode: b.mode,
            idx,
            unit,
            kind,
            ap: self.ap(idx),
            paths: None,
        };
        let dmg = damage(kind, ev.ap);
        for (j, e) in s.units.iter().enumerate() {
            if j == idx || !hostile(&self.defs, e) || !self.clear(idx, Target::Unit(j)) {
                continue;
            }
            let (Some(ek), Some(ek0)) =
                (self.defs.unit_tech(e.kind, e.tech), self.defs.unit(e.kind))
            else {
                continue;
            };
            let factor = if can_fire_now(&self.defs, e) {
                1000.0
            } else {
                500.0
            };
            let armour = e.armor.unwrap_or(ek.armor) as f64;
            let cand = Record {
                kind: Kind::Unit(e.id),
                pos: (e.x, e.y),
                score: score(ev.unit_priority(e), dmg, armour, 0, factor, Some(ek0.cost)),
                path: Vec::new(),
            };
            keep_best(&mut b.best, cand, (unit.x, unit.y), (e.x, e.y));
        }
    }

    // --- orders ---

    fn order_targets(&self, id: i64) -> impl Iterator<Item = &Goal> {
        self.map
            .battle
            .scenario
            .order_targets
            .iter()
            .filter(move |t| t.unit == id)
            .map(|t| &t.target)
    }

    fn waypoints(&self, id: i64) -> Vec<(i64, i64)> {
        self.order_targets(id)
            .filter_map(|t| match *t {
                Goal::Position { x, y } => Some((x, y)),
                Goal::Id(_) => None,
            })
            .collect()
    }

    fn first_target(&self, id: i64) -> Option<i64> {
        self.order_targets(id).find_map(|t| match *t {
            Goal::Id(i) => Some(i),
            Goal::Position { .. } => None,
        })
    }

    /// Drops the first target (`ids`) or waypoint of unit `id`'s order.
    fn pop_target(&mut self, id: i64, ids: bool) {
        let list = &mut self.map.battle.scenario.order_targets;
        if let Some(i) = list
            .iter()
            .position(|t| t.unit == id && matches!(t.target, Goal::Id(_)) == ids)
        {
            list.remove(i);
        }
    }

    fn clear_order(&mut self, id: i64) {
        if let Some(i) = self.unit_index(id) {
            self.map.battle.scenario.units[i].order.id = 0;
        }
    }

    /// `0x46d2d5`: the first target id, clearing the order when none is
    /// left.
    fn order_target(&mut self, a: &mut Action) -> Option<i64> {
        let id = self.first_target(a.unit);
        if id.is_none() {
            self.clear_order(a.unit);
            a.state = 3;
        }
        id
    }

    // attack (`0x46d6c2`)

    fn exec_attack(&mut self, b: &mut Brain, a: &mut Action, idx: usize) -> bool {
        match a.state {
            0 => self.attack_select(b, a, idx),
            1 => {
                if let Some(issued) = self.base_move(b, a, idx) {
                    self.moved(a, issued, None);
                }
            }
            2 => self.fire_step(b, a, idx, 0),
            _ => return true,
        }
        false
    }

    /// `0x46d745`: the best tile to fire at the first target from.
    fn attack_select(&mut self, b: &mut Brain, a: &mut Action, idx: usize) {
        let Some(id) = self.order_target(a) else {
            return;
        };
        if self.ap(idx) < MIN_AP {
            a.state = 3;
            return;
        }
        b.best.reset();
        let Some(grid) = self.ai_grid(b.mode, idx) else {
            a.state = 3;
            return;
        };
        let s = &self.map.battle.scenario;
        let tu = s.units.iter().position(|u| u.id == id && u.arrival == 0);
        let tb = s.buildings.iter().position(|b| b.id == id);
        if tu.is_none() && tb.is_none() {
            self.pop_target(a.unit, true);
            a.state = 0;
            return;
        }
        let unit = &s.units[idx];
        let Some(kind) = self.kind_of(idx) else {
            a.state = 3;
            return;
        };
        let paths = Paths::new(&grid, (unit.x, unit.y));
        let ev = Eval {
            s,
            defs: &self.defs,
            mode: b.mode,
            idx,
            unit,
            kind,
            ap: self.ap(idx),
            paths: Some(&paths),
        };
        match (tu, tb) {
            (Some(j), _) => ev.unit(&mut b.best, j),
            (None, Some(j)) => ev.building(&mut b.best, j),
            (None, None) => {}
        }
        a.state = if b.best.path.is_empty() { 3 } else { 1 };
    }

    // defend (`0x46d897`) and escort (`0x46dad6`)

    fn exec_guard(&mut self, b: &mut Brain, a: &mut Action, idx: usize) -> bool {
        match a.state {
            0 => self.guard_select(b, a, idx),
            1 => self.guard_move(b, a, idx),
            2 => a.state = 3,
            _ => return true,
        }
        false
    }

    /// `0x46d934` / `0x46db73`: done (or, escorting, attacking) near the
    /// target, else the path to beside it.
    fn guard_select(&mut self, b: &mut Brain, a: &mut Action, idx: usize) {
        b.best.reset();
        let Some(id) = self.order_target(a) else {
            return;
        };
        if self.ap(idx) < MIN_AP {
            a.state = 3;
            return;
        }
        let s = &self.map.battle.scenario;
        let rect = if let Some(u) = s.units.iter().find(|u| u.id == id && u.arrival == 0) {
            let (w, h) = self
                .defs
                .unit(u.kind)
                .map_or((1, 1), |k| (k.width, k.height));
            b.best.kind = Kind::Unit(id);
            (u.x, u.y, u.x + w, u.y + h)
        } else if let Some(bd) = s.buildings.iter().find(|b| b.id == id) {
            let (w, h) = self
                .defs
                .buildings
                .get(&bd.kind)
                .map_or((1, 1), |k| (k.width, k.height));
            b.best.kind = Kind::Building(id);
            (bd.x, bd.y, bd.x + w, bd.y + h)
        } else {
            self.pop_target(a.unit, true);
            a.state = 0;
            return;
        };
        b.best.pos = (rect.0, rect.1);
        let mut zone = inflate(rect, if a.order == Order::Defend { 2 } else { 3 });
        if b.mode == AiMode::Legacy {
            zone.2 += 1;
            zone.3 += 1;
        }
        a.zone = zone;
        let u = &s.units[idx];
        if inside(zone, (u.x, u.y)) {
            if a.order == Order::Defend {
                a.state = 3;
            } else {
                a.delegate = true;
            }
            return;
        }
        let Some(grid) = self.ai_grid(b.mode, idx) else {
            a.state = 3;
            return;
        };
        let u = &self.units()[idx];
        let paths = Paths::new(&grid, (u.x, u.y));
        // `0x46d2f8`: the neighbour of the top-left with the fewest steps.
        let mut found: Option<Vec<Step>> = None;
        for (dx, dy) in DIRS {
            let t = (rect.0 + i64::from(dx), rect.1 + i64::from(dy));
            if let Some(p) = paths.to(t)
                && found.as_ref().is_none_or(|f| p.len() < f.len())
            {
                found = Some(p);
            }
        }
        match found {
            Some(p) => {
                b.best.path = p;
                a.state = 1;
            }
            None => a.state = 3,
        }
    }

    /// `0x46ec63`: towards the target, stopping on entering the zone.
    fn guard_move(&mut self, b: &mut Brain, a: &mut Action, idx: usize) {
        let u = &self.units()[idx];
        if inside(a.zone, (u.x, u.y)) {
            a.dest = (u.x, u.y);
            self.moved(a, Issued::Stay, None);
            return;
        }
        let path = b.best.path.clone();
        let issued = match path
            .iter()
            .position(|s| s.stop && inside(a.zone, (i64::from(s.x), i64::from(s.y))))
        {
            Some(i) => {
                let last = path[i];
                a.dest = (i64::from(last.x), i64::from(last.y));
                self.issue_move(b.mode, a, idx, &path[..=i])
            }
            None => Issued::Nothing,
        };
        self.moved(a, issued, None);
    }

    // patrol (`0x46ee2e`)

    fn exec_patrol(&mut self, b: &mut Brain, a: &mut Action, idx: usize) -> bool {
        match a.state {
            0 => {
                self.patrol_plan(b, a, idx);
                if a.attack_mode {
                    a.state += 1;
                } else {
                    a.state = 1;
                }
            }
            1 if a.attack_mode => {
                if b.best.score == SENTINEL {
                    return true;
                }
                if self.clamp_to_route(b, a) {
                    let issued = self.base_move(b, a, idx).unwrap_or(Issued::Nothing);
                    self.moved(a, issued, Some(2));
                } else {
                    a.state = 3;
                }
            }
            1 => {
                if !b.best.path.is_empty() {
                    let path = b.best.path.clone();
                    let issued = self.issue_move(b.mode, a, idx, &path);
                    self.moved(a, issued, Some(0));
                }
            }
            2 => self.fire_step(b, a, idx, 0),
            _ => return true,
        }
        false
    }

    /// `0x46ef66`: on a waypoint, the best attack; else the path to the
    /// nearest waypoint.
    fn patrol_plan(&mut self, b: &mut Brain, a: &mut Action, idx: usize) {
        a.attack_mode = true;
        b.best.reset();
        let route = self.waypoints(a.unit);
        if self.ap(idx) < MIN_AP || route.is_empty() {
            a.state = 2;
            return;
        }
        let Some(grid) = self.ai_grid(b.mode, idx) else {
            a.state = 2;
            return;
        };
        let s = &self.map.battle.scenario;
        let unit = &s.units[idx];
        let Some(kind) = self.kind_of(idx) else {
            a.state = 2;
            return;
        };
        let paths = Paths::new(&grid, (unit.x, unit.y));
        if route.contains(&(unit.x, unit.y)) {
            let ev = Eval {
                s,
                defs: &self.defs,
                mode: b.mode,
                idx,
                unit,
                kind,
                ap: self.ap(idx),
                paths: Some(&paths),
            };
            for (j, e) in s.units.iter().enumerate() {
                if j != idx && hostile(&self.defs, e) {
                    ev.unit(&mut b.best, j);
                }
            }
            return;
        }
        let mut found: Option<Vec<Step>> = None;
        for &wp in &route {
            if let Some(p) = paths.to(wp)
                && found.as_ref().is_none_or(|f| p.len() < f.len())
            {
                found = Some(p);
            }
        }
        b.best.path = found.unwrap_or_default();
        a.attack_mode = false;
    }

    /// `0x46ed14`: an attack from the route may step one tile off it.
    fn clamp_to_route(&self, b: &mut Brain, a: &Action) -> bool {
        let route = self.waypoints(a.unit);
        let on = |s: &Step| route.contains(&(i64::from(s.x), i64::from(s.y)));
        let path = &mut b.best.path;
        if !path.first().is_some_and(on) {
            path.clear();
            return false;
        }
        if let Some(i) = path.iter().position(|s| s.stop && !on(s)) {
            path.truncate(i + 1);
        }
        true
    }

    // goto (`0x46e427`) and goto fast (`0x46e641`)

    fn exec_goto(&mut self, b: &mut Brain, a: &mut Action, idx: usize) -> bool {
        match a.state {
            0 => {
                let route = self.waypoints(a.unit);
                let u = &self.units()[idx];
                let at = (u.x, u.y);
                match route.first() {
                    None => {
                        self.clear_order(a.unit);
                        a.state = 3;
                    }
                    Some(_) if self.ap(idx) < MIN_AP => a.state = 3,
                    Some(&head) if head == at => {
                        self.pop_target(a.unit, false);
                        a.state = 3;
                    }
                    Some(&head) => {
                        a.dest = head;
                        a.state = 1;
                    }
                }
            }
            1 => {
                b.batch.push((a.unit, a.dest));
                a.state = 3;
            }
            2 => a.state = 3,
            _ => return true,
        }
        false
    }

    /// `0x47153d`: the path of a batched goto, AP-cut, from the unit's
    /// tile; `claimed` tiles are taken and `vacated` ones freed.
    fn goto_path(
        &self,
        mode: AiMode,
        idx: usize,
        dest: (i64, i64),
        keep: bool,
        claimed: &[(i64, i64)],
        vacated: &[(i64, i64)],
    ) -> Option<Vec<Step>> {
        let s = &self.map.battle.scenario;
        let mut grid = movement::move_grid(s, &self.defs, idx)?;
        let k = self.kind_of(idx)?;
        let free = |g: &MoveGrid, t: (i64, i64)| {
            g.get(t.0 as i32, t.1 as i32).0 == Access::Stop && !claimed.contains(&t)
        };
        // `0x47170e`: the destination, else the nearest free neighbour.
        let (x, y) = dest;
        let goal = if free(&grid, dest) {
            dest
        } else {
            [
                (x - 1, y),
                (x, y - 1),
                (x, y + 1),
                (x + 1, y),
                (x - 1, y - 1),
                (x - 1, y + 1),
                (x + 1, y - 1),
                (x + 1, y + 1),
            ]
            .into_iter()
            .find(|&t| free(&grid, t))?
        };
        for &(x, y) in claimed {
            if grid.get(x as i32, y as i32).0 != Access::None {
                grid.set(x as i32, y as i32, Access::Pass);
            }
        }
        for &(x, y) in vacated {
            grid.set(x as i32, y as i32, Access::Stop);
        }
        let u = &s.units[idx];
        let paths = Paths::new(&grid, (u.x, u.y));
        let path = paths.to(goal)?;
        let reserve = if keep && k.can_overwatch {
            k.shoot_action_points
        } else {
            0
        };
        let n = cut(mode, &path, (self.ap(idx) - reserve).max(0));
        (n > 1).then(|| path[..n].to_vec())
    }

    /// The goto batch's moves, all walking together (`0x47142f`), then
    /// the waypoints reached are done; returns whether it is finished.
    fn flush_step(&mut self, b: &mut Brain, keep: bool) -> bool {
        if !b.flushing {
            if b.batch.is_empty() {
                return true;
            }
            // Each path ends clear of where those before it end, and may
            // cross the tiles they leave.
            let (mut claimed, mut vacated) = (Vec::new(), Vec::new());
            let mut walks = Vec::new();
            for &(id, dest) in &b.batch {
                let Some(idx) = self.unit_index(id) else {
                    continue;
                };
                let Some(p) = self.goto_path(b.mode, idx, dest, keep, &claimed, &vacated) else {
                    continue;
                };
                let u = &self.units()[idx];
                if !p
                    .first()
                    .is_some_and(|o| (i64::from(o.x), i64::from(o.y)) == (u.x, u.y))
                {
                    continue;
                }
                let last = p[p.len() - 1];
                vacated.push((u.x, u.y));
                claimed.push((i64::from(last.x), i64::from(last.y)));
                walks.push((idx, p));
            }
            if let Some((_, p)) = walks.first() {
                let last = p[p.len() - 1];
                self.watch_tile((i64::from(last.x), i64::from(last.y)));
            }
            for (nth, (idx, p)) in walks.into_iter().enumerate() {
                let ap = self.ap(idx);
                let spent: i64 = p[1..].iter().map(|s| i64::from(s.cost)).sum();
                self.start_batch_walk(idx, p[1..].to_vec(), (ap - spent).max(0), nth, false);
            }
            b.flushing = true;
            return false;
        }
        for (id, _) in std::mem::take(&mut b.batch) {
            let Some(idx) = self.unit_index(id) else {
                continue;
            };
            let u = &self.units()[idx];
            let at = (u.x, u.y);
            if let Some(&head) = self.waypoints(id).first()
                && (head.0 - at.0).abs().max((head.1 - at.1).abs()) <= 1
            {
                self.pop_target(id, false);
            }
            if self.waypoints(id).is_empty() {
                self.clear_order(id);
            }
        }
        b.flushing = false;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(x: i32, cost: u32, stop: bool) -> Step {
        Step {
            x,
            y: 0,
            facing: 0,
            cost,
            stop,
        }
    }

    #[test]
    fn pyro_drops_need_a_horde_and_money() {
        let mut kinds = vec![2; 19];
        kinds.extend([9; 12]);
        // 31 units, 61% troopers.
        assert_eq!(drop_count(&kinds, 1201), 3);
        assert_eq!(drop_count(&kinds, 1200), 2);
        assert_eq!(drop_count(&kinds, 701), 2);
        assert_eq!(drop_count(&kinds, 301), 1);
        assert_eq!(drop_count(&kinds, 300), 0);
        // Exactly 60% is not enough, nor are 30 units.
        assert_eq!(
            drop_count(&[[2; 21].as_slice(), &[9; 14]].concat(), 5000),
            0
        );
        assert_eq!(drop_count(&[56; 30], 5000), 0);
        assert_eq!(drop_count(&[56; 31], 5000), 3);
        // Other types do not count.
        assert_eq!(drop_count(&[1; 40], 5000), 0);
    }

    #[test]
    fn ring_walks_the_border_with_repeated_corners() {
        assert_eq!(
            ring((0, 0, 3, 3)),
            vec![
                (0, 0),
                (1, 0),
                (2, 0),
                (2, 0),
                (2, 1),
                (2, 2),
                (2, 2),
                (1, 2),
                (0, 2),
                (0, 1),
            ]
        );
    }

    #[test]
    fn score_matches_the_formula() {
        // 2 shots of 30 against 100 armour: 60% kill chance.
        assert_eq!(score(100, 60.0, 100.0, 3, 500.0, Some(50)), 270);
        // A sure kill adds the cost; the path penalty caps at 500.
        assert_eq!(score(50, 300.0, 100.0, 80, 500.0, Some(200)), 100);
        assert_eq!(score(100, 10.0, 0.0, 1, 5.0, None), -5);
    }

    #[test]
    fn near_ties_go_to_the_later_record_unless_the_best_is_closer() {
        let rec = |pos, score| Record {
            kind: Kind::Wall,
            pos,
            score,
            path: Vec::new(),
        };
        let mut best = rec((5, 5), 10);
        keep_best(&mut best, rec((9, 9), 9), (0, 0), (9, 9));
        assert_eq!(best.pos, (5, 5));
        keep_best(&mut best, rec((2, 9), 11), (0, 0), (2, 9));
        assert_eq!(best.pos, (2, 9));
        keep_best(&mut best, rec((1, 1), 13), (0, 0), (1, 1));
        assert_eq!(best.pos, (1, 1));
        keep_best(&mut best, rec((4, 4), 20), (0, 0), (4, 4));
        assert_eq!(best.score, 20);
    }

    #[test]
    fn legacy_cut_charges_the_origin_and_overshoots_to_a_stop() {
        let path = [step(0, 10, true), step(1, 10, false), step(2, 10, true)];
        assert_eq!(legacy_cut(&path, 5), 0);
        assert_eq!(legacy_cut(&path, 15), 1);
        assert_eq!(legacy_cut(&path, 20), 3);
        // The engine's rule: an unaffordable pass tile overspends to a stop.
        assert_eq!(cut(AiMode::Modern, &path, 5), 1);
        assert_eq!(cut(AiMode::Modern, &path, 15), 3);
        assert_eq!(cut(AiMode::Modern, &path, 20), 3);
    }

    #[test]
    fn insertion_follows_the_comparator() {
        let mut list = Vec::new();
        for i in [3, 1, 2] {
            insert_sorted(&mut list, i, |a, b| a < b);
        }
        assert_eq!(list, vec![1, 2, 3]);
    }
}
