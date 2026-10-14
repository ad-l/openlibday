//! Movement rules and path finding, as recovered from `LIBDAY.EXE`.
//!
//! The original stores a cost table indexed by terrain kind and movement
//! type. Every entry says whether a unit may stop on the tile, may only pass
//! through it (flyers over mountains), and what entering it costs in action
//! points. Roads make ground and foot units cheaper; blockers (units,
//! buildings, walls, solid ruins, visible mines) turn stop tiles into
//! fly-over tiles for flyers and into walls for everybody else.
//!
//! The search minimises AP, not distance: Dijkstra over 8 neighbours where
//! a diagonal step adds 1 to the search cost (but not to the AP charged) so
//! straight lines win ties.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Longest path the original accepts, in steps.
pub const MAX_STEPS: usize = 150;

/// `Unit Types.Movement`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Movement {
    NoneGround,
    Ground,
    Trooper,
    Grav,
    Boat,
    Air,
    Underwater,
    Teleport,
    NoneWater,
    GravFly,
}

impl Movement {
    pub fn from_id(id: i64) -> Self {
        match id {
            1 => Self::Ground,
            2 => Self::Trooper,
            3 => Self::Grav,
            4 => Self::Boat,
            5 => Self::Air,
            6 => Self::Underwater,
            7 => Self::Teleport,
            8 => Self::NoneWater,
            9 => Self::GravFly,
            _ => Self::NoneGround,
        }
    }

    /// Can fly over tiles it cannot stop on.
    pub fn flies(self) -> bool {
        matches!(self, Self::Air | Self::GravFly)
    }

    /// Ignores normal (non-grav) mines.
    fn floats(self) -> bool {
        matches!(
            self,
            Self::Grav | Self::Air | Self::GravFly | Self::Teleport
        )
    }

    fn uses_roads(self) -> bool {
        matches!(self, Self::Ground | Self::Trooper)
    }
}

/// Terrain kinds (`Terrain Types.TerrainType`).
pub mod kind {
    pub const SHALLOW_WATER: u8 = 0;
    pub const DEEP_WATER: u8 = 1;
    pub const RIVERBANK: u8 = 2;
    pub const PLAIN: u8 = 3;
    pub const MOUNTAIN: u8 = 4;
    pub const FOREST: u8 = 5;
    pub const CRACK: u8 = 6;
}

/// Default AP cost of entering a tile.
pub const BASE_COST: u32 = 10;
pub const FOREST_COST: u32 = 13;
/// Basic (`RoadType` 0) and paved (`RoadType` 1) road costs.
pub const ROAD_COSTS: [u32; 2] = [7, 4];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    None,
    /// May cross but not end a move here.
    Pass,
    Stop,
}

/// What occupies a map tile, as far as movement cares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tile {
    pub kind: u8,
    /// Road type (0 basic, 1 paved). A road on water is a bridge.
    pub road: Option<u8>,
    /// A unit, building, wall or solid ruin.
    pub occupied: bool,
    /// Mine type (0 normal, 1 grav) of an armed mine the mover can see.
    pub mine: Option<u8>,
}

fn water(kind: u8) -> bool {
    matches!(
        kind,
        kind::SHALLOW_WATER | kind::DEEP_WATER | kind::RIVERBANK
    )
}

/// Access and AP cost of entering `tile` for a unit of type `m`.
pub fn tile_rule(m: Movement, tile: Tile) -> (Access, u32) {
    use Movement::*;
    let road_cost = tile.road.map(|r| ROAD_COSTS[usize::from(r.min(1))]);
    let (mut access, mut cost) = match (tile.kind, m) {
        (_, NoneGround | NoneWater) => (Access::None, BASE_COST),
        (kind::SHALLOW_WATER | kind::RIVERBANK, Grav | Teleport | GravFly) => {
            (Access::Stop, BASE_COST)
        }
        (kind::DEEP_WATER, Grav | Boat | Underwater | Teleport | GravFly) => {
            (Access::Stop, BASE_COST)
        }
        (kind::PLAIN, Ground | Trooper | Grav | Air | Teleport | GravFly) => {
            (Access::Stop, BASE_COST)
        }
        (kind::FOREST, Trooper) => (Access::Stop, FOREST_COST),
        (_, Air) => (Access::Pass, BASE_COST),
        (kind::FOREST | kind::MOUNTAIN | kind::CRACK, GravFly) => (Access::Pass, BASE_COST),
        _ => (Access::None, BASE_COST),
    };
    if let Some(road) = road_cost {
        if water(tile.kind) {
            match m {
                Ground | Trooper => (access, cost) = (Access::Stop, road),
                Grav | Air | GravFly | Teleport => access = Access::Stop,
                Boat => access = Access::None,
                Underwater => access = Access::Pass,
                _ => {}
            }
        } else if m.uses_roads() && access == Access::Stop {
            cost = road;
        }
    }
    let mined = match tile.mine {
        Some(1) => true,
        Some(_) => !m.floats(),
        None => false,
    };
    if (tile.occupied || mined) && access != Access::None {
        access = if m.flies() {
            Access::Pass
        } else {
            Access::None
        };
    }
    (access, cost)
}

/// Map directions in facing order: facing `i` moves by `DIRS[i]`.
pub const DIRS: [(i32, i32); 8] = [
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
];

/// Facing index for a one-tile step, if `(dx, dy)` is one.
pub fn facing(dx: i32, dy: i32) -> Option<u8> {
    DIRS.iter().position(|&d| d == (dx, dy)).map(|i| i as u8)
}

/// Per-tile access and cost for one mover.
#[derive(Clone, Debug)]
pub struct MoveGrid {
    width: i32,
    height: i32,
    cells: Vec<(Access, u32)>,
}

impl MoveGrid {
    /// Builds the grid from a tile description; `tile` is called once per
    /// map position, row by row.
    pub fn new(
        width: i32,
        height: i32,
        m: Movement,
        mut tile: impl FnMut(i32, i32) -> Tile,
    ) -> Self {
        let mut cells = Vec::with_capacity((width.max(0) * height.max(0)) as usize);
        for y in 0..height {
            for x in 0..width {
                cells.push(tile_rule(m, tile(x, y)));
            }
        }
        Self {
            width,
            height,
            cells,
        }
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && x < self.width && y < self.height)
            .then(|| (y * self.width + x) as usize)
    }

    pub fn get(&self, x: i32, y: i32) -> (Access, u32) {
        self.index(x, y)
            .map_or((Access::None, BASE_COST), |i| self.cells[i])
    }

    /// Cheapest path from `from` to `to` (excluding `from`), or `None`
    /// when `to` is unreachable or not a tile the unit may stop on.
    pub fn find_path(&self, from: (i32, i32), to: (i32, i32)) -> Option<Vec<Step>> {
        let start = self.index(from.0, from.1)?;
        let goal = self.index(to.0, to.1)?;
        if start == goal || self.cells[goal].0 != Access::Stop {
            return None;
        }
        let n = self.cells.len();
        let mut dist = vec![u32::MAX; n];
        let mut steps = vec![0u16; n];
        let mut prev = vec![usize::MAX; n];
        let mut heap = BinaryHeap::new();
        let mut order = 0u32;
        dist[start] = 0;
        heap.push(Reverse((0u32, 0u16, order, start)));
        while let Some(Reverse((d, s, _, i))) = heap.pop() {
            if d > dist[i] {
                continue;
            }
            if i == goal {
                break;
            }
            if usize::from(s) >= MAX_STEPS {
                continue;
            }
            let (x, y) = ((i as i32) % self.width, (i as i32) / self.width);
            for (dx, dy) in DIRS {
                let Some(j) = self.index(x + dx, y + dy) else {
                    continue;
                };
                let (access, cost) = self.cells[j];
                if access == Access::None {
                    continue;
                }
                let nd = d + cost + u32::from(dx != 0 && dy != 0);
                if nd < dist[j] {
                    dist[j] = nd;
                    steps[j] = s + 1;
                    prev[j] = i;
                    order += 1;
                    heap.push(Reverse((nd, s + 1, order, j)));
                }
            }
        }
        if prev[goal] == usize::MAX {
            return None;
        }
        let mut path = Vec::with_capacity(usize::from(steps[goal]));
        let mut at = goal;
        while at != start {
            let p = prev[at];
            let (x, y) = ((at as i32) % self.width, (at as i32) / self.width);
            let (px, py) = ((p as i32) % self.width, (p as i32) / self.width);
            let (access, cost) = self.cells[at];
            path.push(Step {
                x,
                y,
                facing: facing(x - px, y - py).unwrap_or(0),
                cost,
                stop: access == Access::Stop,
            });
            at = p;
        }
        path.reverse();
        Some(path)
    }
}

/// One tile of a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub x: i32,
    pub y: i32,
    /// Facing while walking onto this tile.
    pub facing: u8,
    /// AP charged for entering it.
    pub cost: u32,
    /// The unit may end its move here.
    pub stop: bool,
}

/// AP usable for moving: overwatch keeps one shot in reserve when the unit
/// can still afford it.
pub fn move_budget(action_points: u32, overwatch_reserve: Option<u32>) -> u32 {
    match overwatch_reserve {
        Some(r) if action_points >= r => action_points - r,
        _ => action_points,
    }
}

/// Number of leading steps whose cumulative cost fits in `budget`: the
/// first unaffordable step is where the preview starts drawing crossed dots.
pub fn affordable_steps(path: &[Step], budget: u32) -> usize {
    let mut spent = 0;
    path.iter()
        .take_while(|s| {
            spent += s.cost;
            spent <= budget
        })
        .count()
}

/// How far a unit with `budget` AP actually moves along `path`: up to the
/// last affordable tile it may stop on. Flyers that run out of AP over a
/// pass-only tile overspend to reach the next stop tile. Returns the step
/// count and the AP spent.
pub fn executable_steps(path: &[Step], budget: u32) -> (usize, u32) {
    let spent_to = |n: usize| path[..n].iter().map(|s| s.cost).sum::<u32>();
    let n = affordable_steps(path, budget);
    if n == 0 || path[n - 1].stop {
        return (n, spent_to(n));
    }
    if let Some(next) = path[n..].iter().position(|s| s.stop) {
        return (n + next + 1, budget);
    }
    match path[..n].iter().rposition(|s| s.stop) {
        Some(last) => (last + 1, spent_to(last + 1)),
        None => (0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&str], m: Movement) -> MoveGrid {
        let rows: Vec<&[u8]> = rows.iter().map(|r| r.as_bytes()).collect();
        MoveGrid::new(rows[0].len() as i32, rows.len() as i32, m, |x, y| {
            let c = rows[y as usize][x as usize];
            let mut t = Tile {
                kind: kind::PLAIN,
                ..Tile::default()
            };
            match c {
                b'~' => t.kind = kind::SHALLOW_WATER,
                b'w' => t.kind = kind::DEEP_WATER,
                b'T' => t.kind = kind::FOREST,
                b'^' => t.kind = kind::MOUNTAIN,
                b'r' => t.road = Some(0),
                b'p' => t.road = Some(1),
                b'=' => {
                    t.kind = kind::DEEP_WATER;
                    t.road = Some(0);
                }
                b'U' => t.occupied = true,
                b'm' => t.mine = Some(0),
                b'g' => t.mine = Some(1),
                _ => {}
            }
            t
        })
    }

    fn cells(path: &[Step]) -> Vec<(i32, i32)> {
        path.iter().map(|s| (s.x, s.y)).collect()
    }

    fn cost(path: &[Step]) -> u32 {
        path.iter().map(|s| s.cost).sum()
    }

    #[test]
    fn rule_table() {
        let t = |kind, road| Tile {
            kind,
            road,
            ..Tile::default()
        };
        use Movement::*;
        assert_eq!(
            tile_rule(Trooper, t(kind::FOREST, None)),
            (Access::Stop, 13)
        );
        assert_eq!(tile_rule(Ground, t(kind::FOREST, None)).0, Access::None);
        assert_eq!(tile_rule(Grav, t(kind::FOREST, None)).0, Access::None);
        assert_eq!(tile_rule(Air, t(kind::FOREST, None)).0, Access::Pass);
        assert_eq!(tile_rule(Grav, t(kind::DEEP_WATER, None)).0, Access::Stop);
        assert_eq!(tile_rule(Grav, t(kind::PLAIN, None)).0, Access::Stop);
        assert_eq!(tile_rule(Boat, t(kind::PLAIN, None)).0, Access::None);
        assert_eq!(
            tile_rule(Boat, t(kind::SHALLOW_WATER, None)).0,
            Access::None
        );
        assert_eq!(
            tile_rule(Ground, t(kind::PLAIN, Some(1))),
            (Access::Stop, 4)
        );
        assert_eq!(tile_rule(Grav, t(kind::PLAIN, Some(1))), (Access::Stop, 10));
        assert_eq!(
            tile_rule(Ground, t(kind::DEEP_WATER, Some(0))),
            (Access::Stop, 7)
        );
        assert_eq!(
            tile_rule(Boat, t(kind::DEEP_WATER, Some(0))).0,
            Access::None
        );
        assert_eq!(
            tile_rule(Underwater, t(kind::DEEP_WATER, Some(0))).0,
            Access::Pass
        );
        assert_eq!(tile_rule(NoneGround, t(kind::PLAIN, None)).0, Access::None);
    }

    #[test]
    fn blockers() {
        use Movement::*;
        let unit = Tile {
            kind: kind::PLAIN,
            occupied: true,
            ..Tile::default()
        };
        assert_eq!(tile_rule(Ground, unit).0, Access::None);
        assert_eq!(tile_rule(Air, unit).0, Access::Pass);
        let mine = Tile {
            kind: kind::PLAIN,
            mine: Some(0),
            ..Tile::default()
        };
        assert_eq!(tile_rule(Trooper, mine).0, Access::None);
        assert_eq!(tile_rule(Grav, mine).0, Access::Stop);
        let grav_mine = Tile {
            mine: Some(1),
            ..mine
        };
        assert_eq!(tile_rule(Grav, grav_mine).0, Access::None);
    }

    #[test]
    fn straight_beats_diagonal_on_ties() {
        let g = grid(&[".....", ".....", "....."], Movement::Ground);
        let p = g.find_path((0, 1), (4, 1)).unwrap();
        assert_eq!(cells(&p), [(1, 1), (2, 1), (3, 1), (4, 1)]);
        assert!(p.iter().all(|s| s.facing == 6));
        assert_eq!(cost(&p), 40);
    }

    #[test]
    fn prefers_road_over_shorter_route() {
        // Direct: 4 plain steps = 40 AP. Detour over paved road: 2 diagonal
        // plain steps + 2 road steps = 28 AP.
        let g = grid(&[".....", ".ppp.", "....."], Movement::Ground);
        let p = g.find_path((0, 0), (4, 0)).unwrap();
        assert!(cost(&p) < 40, "{:?}", cells(&p));
        assert!(cells(&p).contains(&(2, 1)));
    }

    #[test]
    fn forest_only_for_foot() {
        let rows = ["TTT", ".T.", "TTT"];
        assert!(
            grid(&rows, Movement::Ground)
                .find_path((0, 1), (2, 1))
                .is_none()
        );
        let p = grid(&rows, Movement::Trooper)
            .find_path((0, 1), (2, 1))
            .unwrap();
        assert_eq!(cost(&p), 23);
    }

    #[test]
    fn grav_crosses_water_ground_uses_bridge() {
        let rows = ["..w..", "..w..", "..=.."];
        let grav = grid(&rows, Movement::Grav)
            .find_path((0, 0), (4, 0))
            .unwrap();
        assert_eq!(grav.len(), 4);
        let tank = grid(&rows, Movement::Ground)
            .find_path((0, 0), (4, 0))
            .unwrap();
        assert!(cells(&tank).contains(&(2, 2)));
        assert!(
            grid(&rows, Movement::Boat)
                .find_path((2, 0), (2, 1))
                .is_some()
        );
        assert!(
            grid(&rows, Movement::Boat)
                .find_path((2, 0), (2, 2))
                .is_none()
        );
    }

    #[test]
    fn units_block_ground_but_not_flyers() {
        let rows = ["UUU", ".U.", "UUU"];
        assert!(
            grid(&rows, Movement::Ground)
                .find_path((0, 1), (2, 1))
                .is_none()
        );
        let p = grid(&rows, Movement::Air)
            .find_path((0, 1), (2, 1))
            .unwrap();
        assert!(!p[0].stop && p[1].stop);
        assert!(
            grid(&rows, Movement::Air)
                .find_path((0, 1), (1, 1))
                .is_none()
        );
    }

    #[test]
    fn cap_and_unreachable() {
        let row = ".".repeat(200);
        let g = grid(&[&row], Movement::Ground);
        assert_eq!(g.find_path((0, 0), (150, 0)).unwrap().len(), 150);
        assert!(g.find_path((0, 0), (151, 0)).is_none());
        assert!(g.find_path((0, 0), (0, 0)).is_none());
    }

    #[test]
    fn partial_moves_and_overwatch() {
        let g = grid(&["......"], Movement::Ground);
        let p = g.find_path((0, 0), (5, 0)).unwrap();
        assert_eq!(affordable_steps(&p, 35), 3);
        assert_eq!(executable_steps(&p, 35), (3, 30));
        assert_eq!(executable_steps(&p, 50), (5, 50));
        assert_eq!(executable_steps(&p, 9), (0, 0));
        assert_eq!(move_budget(120, Some(80)), 40);
        assert_eq!(move_budget(60, Some(80)), 60);
        assert_eq!(move_budget(60, None), 60);
    }

    #[test]
    fn flyers_overspend_to_next_landing() {
        let g = grid(&[".^^."], Movement::Air);
        let p = g.find_path((0, 0), (3, 0)).unwrap();
        assert_eq!(executable_steps(&p, 15), (3, 15));
        assert_eq!(executable_steps(&p, 5), (0, 0));
    }
}
