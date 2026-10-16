//! The campaign base editor's rules: what the build list offers and where
//! things may be built (`CCanBuild`, `0x4038b0`).
//!
//! Every footprint tile must be on the map away from its border, of the
//! item's terrain and free; something of the player's must stand within
//! [`RANGE`] tiles (water buildings excepted). Roads must touch the
//! headquarters or one of the player's roads, and may cross water as paved
//! bridges. Nothing else may touch the headquarters; buildings and towers
//! need a road beside them (towers may stand by a wall instead) and
//! buildings need a free ring around them.

use libday_data::Definitions;
use libday_data::defs::terrain_kind::{DEEP_WATER, PLAIN, RIVERBANK};
use libday_data::scenario::Scenario;

use crate::support::{building_at, kind_at, unit_at};

/// Headquarters building type.
pub const HQ: i64 = 130;
/// "No prerequisite".
pub const NONE: i64 = 65535;
/// Paved road type, the only one bridges are built of.
pub const PAVED: i64 = 1;
/// Deployment centre types (the HQ also deploys but is not built).
pub const DEPLOY_CENTRES: [i64; 4] = [131, 173, 174, 175];
/// Most deployment centres a base may hold.
pub const MAX_DEPLOY_CENTRES: usize = 10;
/// Tiles along each map edge where nothing may be built.
const BORDER: i64 = 13;
/// Something of the player's must be this close.
const RANGE: i64 = 4;
/// Distance kept from other players' headquarters.
const HQ_DISTANCE: i64 = 6;
/// `position` of buildings that stand on water.
const WATER: i64 = DEEP_WATER as i64;

/// Something the build list offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    Building(i64),
    Road(i64),
    Wall(i64),
    Mine(i64),
}

/// Why an item cannot be built: string `384 + n` of `LIBDAYEN.DLL`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Credits = 1,
    Terrain = 2,
    Occupied = 3,
    Crowded = 4,
    ForeignHq = 5,
    TooFar = 6,
    Border = 10,
    Bridge = 11,
    BridgeRoad = 12,
    AroundHq = 13,
    DeployCentres = 14,
    Roads = 15,
}

impl Refusal {
    pub fn string(self) -> u32 {
        384 + self as u32
    }

    pub fn fallback(self) -> &'static str {
        match self {
            Refusal::Credits => "Not enough resources to build",
            Refusal::Terrain => "Can't build on that type of terrain",
            Refusal::Occupied => "The area is already occupied",
            Refusal::Crowded => "Too close to other buildings",
            Refusal::ForeignHq => "Too close to other players' HQ",
            Refusal::TooFar => "Too far from your other constructions",
            Refusal::Border => "Can't build on province's borders",
            Refusal::Bridge => "Can't build bridge",
            Refusal::BridgeRoad => "Bridges must be built with hi tech road",
            Refusal::AroundHq => "Can't build around HQ",
            Refusal::DeployCentres => "Too many owned deployment centers",
            Refusal::Roads => "Must build adjacent to your roads",
        }
    }
}

impl Item {
    /// Footprint size.
    pub fn size(self, defs: &Definitions) -> (i64, i64) {
        match self {
            Item::Building(k) => defs
                .buildings
                .get(&k)
                .map_or((1, 1), |b| (b.width.max(1), b.height.max(1))),
            _ => (1, 1),
        }
    }

    pub fn name(self, defs: &Definitions) -> String {
        match self {
            Item::Building(k) => defs.buildings.get(&k).map(|b| b.name.clone()),
            Item::Road(k) => defs.roads.get(&k).map(|r| r.name.clone()),
            Item::Wall(k) => defs.walls.get(&k).map(|w| w.name.clone()),
            Item::Mine(k) => defs.mines.get(&k).map(|m| m.name.clone()),
        }
        .unwrap_or_default()
    }

    /// `LISTBOX.DBI` picture prefix.
    pub fn image(self, defs: &Definitions) -> String {
        match self {
            Item::Building(k) => defs.buildings.get(&k).map(|b| b.image.clone()),
            Item::Road(k) => defs.roads.get(&k).map(|r| r.image.clone()),
            Item::Wall(k) => defs.walls.get(&k).map(|w| w.image.clone()),
            Item::Mine(k) => defs.mines.get(&k).map(|m| m.image.clone()),
        }
        .unwrap_or_default()
        .to_ascii_uppercase()
    }

    pub fn cost(self, defs: &Definitions) -> i64 {
        match self {
            Item::Building(k) => defs.buildings.get(&k).map_or(0, |b| b.cost),
            Item::Road(k) => defs.roads.get(&k).map_or(0, |r| r.cost),
            Item::Wall(k) => defs.walls.get(&k).map_or(0, |w| w.cost),
            Item::Mine(k) => defs.mines.get(&k).map_or(0, |m| m.cost),
        }
    }

    /// Credits recycling one built in an earlier session gives back.
    pub fn recycle(self, defs: &Definitions) -> i64 {
        match self {
            Item::Building(k) => defs.buildings.get(&k).map_or(0, |b| b.recycle),
            Item::Road(k) => defs.roads.get(&k).map_or(0, |r| r.recycle),
            Item::Wall(k) => defs.walls.get(&k).map_or(0, |w| w.recycle),
            Item::Mine(k) => defs.mines.get(&k).map_or(0, |m| m.recycle),
        }
    }

    /// Roads and walls are laid as segments between two clicks.
    pub fn segment(self) -> bool {
        matches!(self, Item::Road(_) | Item::Wall(_))
    }

    fn tower(self, defs: &Definitions) -> bool {
        matches!(self, Item::Building(k) if is_tower(defs, k))
    }
}

/// A defence tower: a building that becomes a unit in battle.
pub fn is_tower(defs: &Definitions, kind: i64) -> bool {
    defs.buildings
        .get(&kind)
        .is_some_and(|b| b.transform_into.is_some())
}

fn ready(prerequisite: i64, owned: &[i64]) -> bool {
    prerequisite == NONE || owned.contains(&prerequisite)
}

/// The BUILDINGS list (`defenses` false) or the DEFENSES list: of the
/// continent's `available` buildings those whose prerequisite is `owned`
/// (unique ones not yet owned); the defences are the roads, the towers, the
/// walls and the mines.
pub fn offered(defs: &Definitions, available: &[i64], owned: &[i64], defenses: bool) -> Vec<Item> {
    let buildings = available.iter().filter(|&&k| {
        defs.buildings.get(&k).is_some_and(|b| {
            b.can_build
                && is_tower(defs, k) == defenses
                && ready(b.prerequisite, owned)
                && !(b.unique && owned.contains(&k))
        })
    });
    if !defenses {
        return buildings.map(|&k| Item::Building(k)).collect();
    }
    let mut roads: Vec<i64> = defs
        .roads
        .iter()
        .filter(|(_, r)| r.can_build)
        .map(|(&k, _)| k)
        .collect();
    // Basic road first.
    roads.sort_by_key(|&k| k != 0);
    roads
        .into_iter()
        .map(Item::Road)
        .chain(buildings.map(|&k| Item::Building(k)))
        .chain(
            defs.walls
                .iter()
                .filter(|(_, w)| w.can_build && ready(w.prerequisite, owned))
                .map(|(&k, _)| Item::Wall(k)),
        )
        .chain(
            defs.mines
                .iter()
                .filter(|(_, m)| m.can_build && ready(m.prerequisite, owned))
                .map(|(&k, _)| Item::Mine(k)),
        )
        .collect()
}

/// Footprint of building `i`: `(x, y, w, h)`.
fn rect_of(s: &Scenario, defs: &Definitions, i: usize) -> (i64, i64, i64, i64) {
    let b = &s.buildings[i];
    let (w, h) = Item::Building(b.kind).size(defs);
    (b.x, b.y, w, h)
}

/// Whether rectangles `a` and `b` (`x, y, w, h`) overlap once `a` is grown
/// by `by` tiles on every side.
fn near(a: (i64, i64, i64, i64), b: (i64, i64, i64, i64), by: i64) -> bool {
    a.0 - by < b.0 + b.2 && b.0 < a.0 + a.2 + by && a.1 - by < b.1 + b.3 && b.1 < a.1 + a.3 + by
}

/// Whether `player` may build `item` with its top-left corner on `at`.
pub fn check(
    s: &Scenario,
    defs: &Definitions,
    player: i64,
    item: Item,
    at: (i64, i64),
) -> Result<(), Refusal> {
    let (w, h) = item.size(defs);
    let (x, y) = at;
    let size = s.size as i64;
    if x < BORDER || y < BORDER || x + w + BORDER > size || y + h + BORDER > size {
        return Err(Refusal::Border);
    }
    let rect = (x, y, w, h);
    let tiles: Vec<(i64, i64)> = (y..y + h)
        .flat_map(|ty| (x..x + w).map(move |tx| (tx, ty)))
        .collect();
    let position = match item {
        Item::Building(k) => defs.buildings.get(&k).map_or(3, |b| b.position),
        _ => i64::from(PLAIN),
    };
    let road_at = |p: (i64, i64)| s.roads.iter().any(|r| (r.x, r.y) == p);
    let own_road = |p: (i64, i64)| s.roads.iter().any(|r| (r.x, r.y) == p && r.owner == player);
    let own_wall = |p: (i64, i64)| s.walls.iter().any(|r| (r.x, r.y) == p && r.owner == player);
    for &t in &tiles {
        let kind = kind_at(s, defs, t);
        let bridge = matches!(item, Item::Road(_)) && matches!(kind, Some(DEEP_WATER | RIVERBANK));
        if bridge {
            if item != Item::Road(PAVED) {
                return Err(Refusal::BridgeRoad);
            }
            if !crate::support::bridge_sites(s, defs).contains(&t) {
                return Err(Refusal::Bridge);
            }
        } else if kind.map(i64::from) != Some(position) {
            return Err(Refusal::Terrain);
        }
        if unit_at(s, defs, t).is_some()
            || building_at(s, defs, t).is_some()
            || road_at(t)
            || s.walls.iter().any(|r| (r.x, r.y) == t)
            || s.mines.iter().any(|m| (m.x, m.y) == t)
        {
            return Err(Refusal::Occupied);
        }
    }
    let water = matches!(item, Item::Building(_)) && position == WATER;
    let mine = |o: i64| o == player;
    if !water {
        let close = (0..s.buildings.len())
            .any(|i| mine(s.buildings[i].owner) && near(rect, rect_of(s, defs, i), RANGE))
            || s.roads
                .iter()
                .chain(&s.walls)
                .any(|r| mine(r.owner) && near(rect, (r.x, r.y, 1, 1), RANGE));
        if !close {
            return Err(Refusal::TooFar);
        }
    }
    let hq = (0..s.buildings.len())
        .filter(|&i| s.buildings[i].kind == HQ && mine(s.buildings[i].owner))
        .map(|i| rect_of(s, defs, i))
        .collect::<Vec<_>>();
    let edges: Vec<(i64, i64)> = (x..x + w)
        .flat_map(|tx| [(tx, y - 1), (tx, y + h)])
        .chain((y..y + h).flat_map(|ty| [(x - 1, ty), (x + w, ty)]))
        .collect();
    if let Item::Road(_) = item {
        let by_hq = hq.iter().any(|&r| near(rect, r, 1));
        if !by_hq && !edges.iter().any(|&p| own_road(p)) {
            return Err(Refusal::Roads);
        }
        return Ok(());
    }
    if hq.iter().any(|&r| near(rect, r, 1)) {
        return Err(Refusal::AroundHq);
    }
    let Item::Building(_) = item else {
        return Ok(());
    };
    let tower = item.tower(defs);
    if !water && !edges.iter().any(|&p| own_road(p) || (tower && own_wall(p))) {
        return Err(Refusal::Roads);
    }
    if !tower
        && (0..s.buildings.len())
            .any(|i| !is_tower(defs, s.buildings[i].kind) && near(rect, rect_of(s, defs, i), 1))
    {
        return Err(Refusal::Crowded);
    }
    if (0..s.buildings.len()).any(|i| {
        let b = &s.buildings[i];
        b.kind == HQ && !mine(b.owner) && near(rect, rect_of(s, defs, i), HQ_DISTANCE)
    }) {
        return Err(Refusal::ForeignHq);
    }
    Ok(())
}

/// Tiles of a road or wall segment from `start` towards `end`: straight
/// along the axis on which they are further apart.
pub fn segment(start: (i64, i64), end: (i64, i64)) -> Vec<(i64, i64)> {
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    if dx.abs() >= dy.abs() {
        (0..=dx.abs())
            .map(|i| (start.0 + i * dx.signum(), start.1))
            .collect()
    } else {
        (0..=dy.abs())
            .map(|i| (start.0, start.1 + i * dy.signum()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libday_data::defs::{BuildingType, MineType, SurfaceType, TerrainType};
    use libday_data::scenario::{Building, Cell, ScenarioInfo, Surface};

    fn defs() -> Definitions {
        let mut d = Definitions::default();
        for (id, kind) in [("PLAIN", PLAIN), ("SEA", DEEP_WATER), ("BANK", RIVERBANK)] {
            d.terrains.insert(
                id.into(),
                TerrainType {
                    id: id.into(),
                    kind,
                    overlapping: false,
                    colour: [0; 3],
                    burned_id: String::new(),
                    burned_min: 0,
                    can_fire_through: true,
                },
            );
        }
        let building =
            |id, w, h, position, unique, transform: Option<i64>, prerequisite| BuildingType {
                id,
                name: format!("B{id}"),
                width: w,
                height: h,
                position,
                unique,
                transform_into: transform,
                can_build: true,
                prerequisite,
                cost: 100,
                recycle: 50,
                ..Default::default()
            };
        d.buildings
            .insert(HQ, building(HQ, 4, 4, 3, true, None, NONE));
        d.buildings
            .insert(131, building(131, 2, 2, 3, false, None, HQ));
        d.buildings
            .insert(100, building(100, 1, 1, 3, false, Some(37), HQ));
        d.buildings
            .insert(150, building(150, 2, 2, 1, false, None, HQ));
        d.buildings
            .insert(137, building(137, 2, 2, 3, true, None, 131));
        for (id, cost) in [(0, 1), (1, 3)] {
            d.roads.insert(
                id,
                SurfaceType {
                    id,
                    can_build: true,
                    cost,
                    prerequisite: NONE,
                    ..Default::default()
                },
            );
        }
        d.walls.insert(
            0,
            SurfaceType {
                id: 0,
                can_build: true,
                cost: 5,
                prerequisite: HQ,
                ..Default::default()
            },
        );
        d.walls.insert(
            3,
            SurfaceType {
                id: 3,
                prerequisite: HQ,
                ..Default::default()
            },
        );
        d.mines.insert(
            0,
            MineType {
                id: 0,
                can_build: true,
                prerequisite: HQ,
                ..Default::default()
            },
        );
        d
    }

    /// A 60×60 plain map with the HQ at (28, 28) and a road east of it, sea
    /// in columns 50+.
    fn scenario() -> Scenario {
        let size = 60;
        let cells = (0..size * size)
            .map(|i| Cell {
                id: 0,
                terrain: if i % size >= 40 { "SEA" } else { "PLAIN" }.into(),
            })
            .collect();
        let road = |x, y| Surface {
            counter: 0,
            kind: 0,
            owner: 1,
            x,
            y,
            armor: 10,
            broken: false,
        };
        Scenario {
            size,
            cells,
            buildings: vec![Building {
                counter: 0,
                id: 1,
                kind: HQ,
                owner: 1,
                armor: 100,
                x: 20,
                y: 20,
                ai_target: false,
            }],
            units: Vec::new(),
            roads: vec![road(24, 21), road(25, 21), road(26, 21)],
            walls: Vec::new(),
            ruins: Vec::new(),
            mines: Vec::new(),
            deployments: Vec::new(),
            order_targets: Vec::new(),
            objective: None,
            info: ScenarioInfo::default(),
        }
    }

    #[test]
    fn lists_follow_prerequisites_and_uniqueness() {
        let d = defs();
        let available = [100, 131, 137, HQ, 150];
        assert_eq!(
            offered(&d, &available, &[HQ], false),
            [Item::Building(131), Item::Building(150)]
        );
        assert_eq!(
            offered(&d, &available, &[HQ, 131], false),
            [
                Item::Building(131),
                Item::Building(137),
                Item::Building(150)
            ]
        );
        assert_eq!(
            offered(&d, &available, &[HQ], true),
            [
                Item::Road(0),
                Item::Road(1),
                Item::Building(100),
                Item::Wall(0),
                Item::Mine(0)
            ]
        );
        assert_eq!(
            offered(&d, &available, &[], true),
            [Item::Road(0), Item::Road(1)]
        );
    }

    #[test]
    fn roads_grow_from_the_hq_and_other_roads() {
        let (d, s) = (defs(), scenario());
        let road = |at| check(&s, &d, 1, Item::Road(0), at);
        assert_eq!(road((24, 22)), Ok(()));
        assert_eq!(road((19, 19)), Ok(()));
        assert_eq!(road((27, 21)), Ok(()));
        assert_eq!(road((28, 21)), Err(Refusal::Roads));
        assert_eq!(road((25, 21)), Err(Refusal::Occupied));
        assert_eq!(road((20, 20)), Err(Refusal::Occupied));
        assert_eq!(road((12, 20)), Err(Refusal::Border));
        assert_eq!(road((40, 21)), Err(Refusal::BridgeRoad));
    }

    #[test]
    fn buildings_need_roads_room_and_distance_from_the_hq() {
        let (d, s) = (defs(), scenario());
        let at = |kind, xy| check(&s, &d, 1, Item::Building(kind), xy);
        // Beside the road, clear of the HQ's ring.
        assert_eq!(at(131, (25, 22)), Ok(()));
        assert_eq!(at(131, (25, 19)), Ok(()));
        assert_eq!(at(131, (24, 22)), Err(Refusal::AroundHq));
        assert_eq!(at(131, (25, 23)), Err(Refusal::Roads));
        assert_eq!(at(131, (36, 30)), Err(Refusal::TooFar));
        // Water buildings need water but neither roads nor range.
        assert_eq!(at(150, (45, 30)), Ok(()));
        assert_eq!(at(150, (25, 22)), Err(Refusal::Terrain));
        let mut s2 = s.clone();
        s2.buildings.push(Building {
            counter: 0,
            id: 2,
            kind: 131,
            owner: 1,
            armor: 1,
            x: 25,
            y: 22,
            ai_target: false,
        });
        s2.roads.push(Surface {
            counter: 0,
            kind: 0,
            owner: 1,
            x: 27,
            y: 21,
            armor: 10,
            broken: false,
        });
        let at2 = |kind, xy| check(&s2, &d, 1, Item::Building(kind), xy);
        assert_eq!(at2(131, (27, 22)), Err(Refusal::Crowded));
        assert_eq!(at2(131, (25, 19)), Ok(()));
        // Towers ignore the ring and may stand by a wall.
        assert_eq!(at2(100, (27, 22)), Ok(()));
        s2.walls.push(Surface {
            counter: 0,
            kind: 0,
            owner: 1,
            x: 30,
            y: 25,
            armor: 60,
            broken: false,
        });
        assert_eq!(check(&s2, &d, 1, Item::Building(100), (30, 26)), Ok(()));
        assert_eq!(
            check(&s2, &d, 1, Item::Building(131), (30, 26)),
            Err(Refusal::Roads)
        );
    }

    #[test]
    fn walls_and_mines_need_only_range() {
        let (d, s) = (defs(), scenario());
        assert_eq!(check(&s, &d, 1, Item::Wall(0), (29, 25)), Ok(()));
        assert_eq!(
            check(&s, &d, 1, Item::Mine(0), (24, 24)),
            Err(Refusal::AroundHq)
        );
        assert_eq!(check(&s, &d, 1, Item::Mine(0), (30, 22)), Ok(()));
        assert_eq!(
            check(&s, &d, 1, Item::Wall(0), (35, 35)),
            Err(Refusal::TooFar)
        );
    }

    #[test]
    fn segments_run_along_the_longer_axis() {
        assert_eq!(segment((1, 1), (4, 2)), [(1, 1), (2, 1), (3, 1), (4, 1)]);
        assert_eq!(segment((1, 1), (0, -1)), [(1, 1), (1, 0), (1, -1)]);
        assert_eq!(segment((3, 3), (3, 3)), [(3, 3)]);
    }
}
