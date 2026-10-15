//! What a shot does where it lands, as the original resolves it when the
//! attack is ordered (`0x418542` → `0x41864d`, after the scatter roll).
//!
//! - One roll: `base + 1 + rand() % random` (`damageBase`, `damageRandom`
//!   of the shooter at its technology level; `base + 1` when `random` is 0).
//! - The damage spreads over a `DamageArea` square centred on the tile it
//!   lands on, falling off with the Chebyshev distance `d` from the centre:
//!   `damage - (damage / ((area + 1) / 2)) * d`.
//! - On each tile, in order (`0x418750`):
//!   1. a mine is set off by any damage (`0x418a24`);
//!   2. a destructible building loses armour to destroying (type 0)
//!      damage; at 0 it is removed, leaves `B?RUI` ruins and burns its
//!      footprint (`0x418ada`);
//!   3. a wall loses armour to destroying damage; at 0 it is removed and its
//!      neighbours are drawn broken (`0x418df1`);
//!   4. a unit loses armour to destroying damage, or action points to
//!      paralysing (type 1) damage. Destroyed, it leaves its `ruinImageID`
//!      on plain ground and the tile's damage becomes its full armour
//!      (`0x418f06`);
//!   5. a road standing under a surviving unit is shielded, except a bridge.
//!      A road whose armour the damage reaches is destroyed (dirt roads
//!      have 999), and a bridge takes the unit on it down with it
//!      (`0x419112`);
//!   6. bare ground, or ground where something was just destroyed, burns
//!      (`burned_id`) when the damage reaches `burned_min`; terrain without
//!      a `burned_id` (water, mountains, cracks) never does (`0x4188a5`).

use libday_data::defs::{BuildingType, Definitions, UnitType, terrain_kind};
use libday_data::scenario::{Ruin, Scenario};

use crate::scatter::Rand;

/// Something blown up, for its explosion animation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destroyed {
    /// A unit's `E` animation at its tile.
    Unit {
        image: String,
        owner: i64,
        at: (i64, i64),
    },
    /// A building's `BUILDING\{image}.SMK` over its footprint.
    Building { kind: i64, at: (i64, i64) },
    /// A mine's `MINE\{image}.SMK` at its tile.
    Mine { kind: i64, at: (i64, i64) },
}

/// The result of one shot, applied to the scenario by [`resolve`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub damage: i64,
    /// Indices (before removal) of the units destroyed, ascending.
    pub removed_units: Vec<usize>,
    /// Indices (before removal) of the buildings destroyed, ascending.
    pub removed_buildings: Vec<usize>,
    pub destroyed: Vec<Destroyed>,
    /// Wreckage left behind; not yet added to the scenario, so it can show
    /// once the explosions are over.
    pub ruins: Vec<Ruin>,
    /// Tiles whose terrain burned.
    pub burned: Vec<(i64, i64)>,
}

/// Rolls the damage of one shot.
pub fn roll(rng: &mut Rand, base: i64, random: i64) -> i64 {
    base + 1 + rng.rand() % random.max(1)
}

/// Damage on every on-map tile of the area, row by row.
pub fn spread(size: i64, center: (i64, i64), area: i64, damage: i64) -> Vec<((i64, i64), i64)> {
    let area = area.max(1);
    let half = (area + 1) / 2;
    let corner = (center.0 - area / 2, center.1 - area / 2);
    let mut out = Vec::new();
    for y in corner.1..corner.1 + area {
        for x in corner.0..corner.0 + area {
            if !(0..size).contains(&x) || !(0..size).contains(&y) {
                continue;
            }
            let d = (x - center.0).abs().max((y - center.1).abs());
            out.push(((x, y), damage - (damage / half) * d));
        }
    }
    out
}

/// Events recorded on a tile (the original's codes).
const MINE: u8 = 2;
const BUILDING: u8 = 4;
const WALL: u8 = 5;
const UNIT: u8 = 6;
const ROAD: u8 = 0;
const BURNED: u8 = 7;

struct Resolver<'a> {
    scenario: &'a mut Scenario,
    defs: &'a Definitions,
    rng: &'a mut Rand,
    dead_units: Vec<usize>,
    dead_buildings: Vec<usize>,
    outcome: Outcome,
}

impl Resolver<'_> {
    fn terrain_kind(&self, x: i64, y: i64) -> Option<u8> {
        let cell = self.scenario.cell(x, y)?;
        self.defs.terrain(&cell.terrain).map(|t| t.kind)
    }

    fn unit_type(&self, i: usize) -> Option<&UnitType> {
        let u = &self.scenario.units[i];
        self.defs.unit_tech(u.kind, u.tech)
    }

    fn unit_at(&self, x: i64, y: i64) -> Option<usize> {
        self.scenario.units.iter().enumerate().position(|(i, u)| {
            let (w, h) = self.unit_type(i).map_or((1, 1), |k| (k.width, k.height));
            u.arrival == 0
                && !self.dead_units.contains(&i)
                && (u.x..u.x + w).contains(&x)
                && (u.y..u.y + h).contains(&y)
        })
    }

    fn building_at(&self, x: i64, y: i64) -> Option<usize> {
        self.scenario
            .buildings
            .iter()
            .enumerate()
            .position(|(i, b)| {
                let (w, h) = self
                    .defs
                    .buildings
                    .get(&b.kind)
                    .map_or((1, 1), |k| (k.width, k.height));
                !self.dead_buildings.contains(&i)
                    && (b.x..b.x + w).contains(&x)
                    && (b.y..b.y + h).contains(&y)
            })
    }

    fn has_ruin(&self, x: i64, y: i64) -> bool {
        self.scenario
            .ruins
            .iter()
            .chain(&self.outcome.ruins)
            .any(|r| (r.x, r.y) == (x, y))
    }

    fn add_ruin(&mut self, id: String, x: i64, y: i64) {
        let counter = self
            .scenario
            .ruins
            .iter()
            .chain(&self.outcome.ruins)
            .map(|r| r.counter + 1)
            .max()
            .unwrap_or(1);
        self.outcome.ruins.push(Ruin { counter, id, x, y });
    }

    fn tile(&mut self, (x, y): (i64, i64), mut dmg: i64, kind: i64) {
        let mut events = Vec::new();
        self.mine(x, y, &mut events);
        self.building(x, y, dmg, kind, &mut events);
        self.wall(x, y, dmg, kind, &mut events);
        if let Some(full) = self.unit(x, y, dmg, kind, &mut events) {
            dmg = full;
        }
        self.road(x, y, dmg, kind, &mut events);
        self.burn(x, y, dmg, kind, &mut events);
    }

    fn mine(&mut self, x: i64, y: i64, events: &mut Vec<u8>) {
        let Some(i) = self
            .scenario
            .mines
            .iter()
            .position(|m| (m.x, m.y) == (x, y))
        else {
            return;
        };
        let m = self.scenario.mines.remove(i);
        self.scenario.mines.retain(|m| (m.x, m.y) != (x, y));
        events.push(MINE);
        self.outcome.destroyed.push(Destroyed::Mine {
            kind: m.kind,
            at: (x, y),
        });
    }

    fn building(&mut self, x: i64, y: i64, dmg: i64, kind: i64, events: &mut Vec<u8>) {
        let Some(i) = self.building_at(x, y) else {
            return;
        };
        let b = &mut self.scenario.buildings[i];
        let Some(t) = self.defs.buildings.get(&b.kind) else {
            return;
        };
        if !t.destructible || kind != 0 {
            return;
        }
        b.armor = (b.armor - dmg).max(0);
        if b.armor > 0 {
            return;
        }
        events.push(BUILDING);
        let (bx, by) = (b.x, b.y);
        self.dead_buildings.push(i);
        self.outcome.destroyed.push(Destroyed::Building {
            kind: t.id,
            at: (bx, by),
        });
        if t.has_ruin {
            self.building_ruins(t, bx, by);
        }
        for dy in 0..t.height {
            for dx in 0..t.width {
                self.burn(bx + dx, by + dy, t.armor, 0, &mut Vec::new());
            }
        }
    }

    /// Ruins over half the footprint's free plain tiles (1 to 7), picked
    /// at random, each a random `B?RUI` of the building's race
    /// (`0x440daf`).
    fn building_ruins(&mut self, t: &BuildingType, bx: i64, by: i64) {
        let mut free: Vec<(i64, i64)> = (0..t.height)
            .flat_map(|dy| (0..t.width).map(move |dx| (bx + dx, by + dy)))
            .filter(|&(x, y)| {
                self.terrain_kind(x, y) == Some(terrain_kind::PLAIN) && !self.has_ruin(x, y)
            })
            .collect();
        let n = free.len();
        for i in 0..n {
            let j = (self.rng.rand() % n as i64) as usize;
            free.swap(i, j);
        }
        let race = ["H", "T", "B", "M"]
            .get(t.race.clamp(0, 3) as usize)
            .copied()
            .unwrap_or("H");
        let prefix = format!("B{race}RUI");
        let types = self
            .defs
            .ruins
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .count() as i64;
        let count = (t.width * t.height / 2).clamp(1, 7) as usize;
        for &(x, y) in free.iter().take(count) {
            if types == 0 {
                break;
            }
            let id = format!("{prefix}{:03}", self.rng.rand() % types);
            self.add_ruin(id, x, y);
        }
    }

    fn wall(&mut self, x: i64, y: i64, dmg: i64, kind: i64, events: &mut Vec<u8>) {
        let Some(i) = self
            .scenario
            .walls
            .iter()
            .position(|w| (w.x, w.y) == (x, y))
        else {
            return;
        };
        if kind != 0 {
            return;
        }
        let w = &mut self.scenario.walls[i];
        w.armor = (w.armor - dmg).max(0);
        if w.armor > 0 {
            return;
        }
        self.scenario.walls.remove(i);
        for w in &mut self.scenario.walls {
            if (w.x - x).abs() + (w.y - y).abs() == 1 {
                w.broken = true;
            }
        }
        events.push(WALL);
    }

    /// Damages the unit on the tile; returns its full armour if destroyed.
    fn unit(&mut self, x: i64, y: i64, dmg: i64, kind: i64, events: &mut Vec<u8>) -> Option<i64> {
        let i = self.unit_at(x, y)?;
        let t = self.unit_type(i)?.clone();
        let u = &mut self.scenario.units[i];
        if kind == 1 {
            u.action_points.get_or_insert(t.action_points);
            u.drain_ap(dmg, t.action_points);
            return None;
        }
        let armor = (u.armor.unwrap_or(t.armor) - dmg).max(0);
        u.armor = Some(armor);
        if armor > 0 {
            return None;
        }
        events.push(UNIT);
        let (ux, uy, owner) = (u.x, u.y, u.owner);
        self.dead_units.push(i);
        self.outcome.destroyed.push(Destroyed::Unit {
            image: t.image.clone(),
            owner,
            at: (ux, uy),
        });
        if self.terrain_kind(ux, uy) == Some(terrain_kind::PLAIN)
            && !self.has_ruin(ux, uy)
            && self.defs.ruins.contains_key(&t.ruin)
        {
            self.add_ruin(t.ruin.clone(), ux, uy);
        }
        Some(t.armor)
    }

    fn road(&mut self, x: i64, y: i64, dmg: i64, kind: i64, events: &mut Vec<u8>) {
        let Some(i) = self
            .scenario
            .roads
            .iter()
            .position(|r| (r.x, r.y) == (x, y))
        else {
            return;
        };
        let bridge = self.terrain_kind(x, y) == Some(terrain_kind::DEEP_WATER);
        let unit = self.unit_at(x, y);
        let shielded =
            unit.is_some() && !bridge && !events.contains(&MINE) && !events.contains(&UNIT);
        if shielded || kind != 0 {
            return;
        }
        let armor = self
            .defs
            .roads
            .get(&self.scenario.roads[i].kind)
            .map_or(i64::MAX, |t| t.armor);
        if dmg < armor {
            return;
        }
        if bridge && let Some(u) = unit {
            let full = self.unit_type(u).map_or(0, |t| t.armor);
            self.unit(x, y, full, 0, events);
        }
        events.push(ROAD);
        self.scenario.roads.remove(i);
    }

    fn burn(&mut self, x: i64, y: i64, dmg: i64, kind: i64, events: &mut Vec<u8>) {
        let occupied = self.building_at(x, y).is_some()
            || self.unit_at(x, y).is_some()
            || self.scenario.walls.iter().any(|w| (w.x, w.y) == (x, y))
            || self.scenario.roads.iter().any(|r| (r.x, r.y) == (x, y))
            || self.scenario.mines.iter().any(|m| (m.x, m.y) == (x, y));
        if occupied && events.is_empty() || kind != 0 {
            return;
        }
        let Some(cell) = self.scenario.cell(x, y) else {
            return;
        };
        let Some(t) = self.defs.terrain(&cell.terrain) else {
            return;
        };
        if t.burned_id.is_empty() || dmg < t.burned_min || self.defs.terrain(&t.burned_id).is_none()
        {
            return;
        }
        let burned = t.burned_id.clone();
        let size = self.scenario.size;
        self.scenario.cells[y as usize * size + x as usize].terrain = burned;
        self.outcome.burned.push((x, y));
        events.push(BURNED);
    }
}

/// Resolves a shot of `damage` (already rolled) of `kind` (`damageType`)
/// spreading over `area` around `lands`: updates `scenario` (removing
/// what is destroyed) and returns what happened.
pub fn resolve(
    scenario: &mut Scenario,
    defs: &Definitions,
    rng: &mut Rand,
    lands: (i64, i64),
    damage: i64,
    kind: i64,
    area: i64,
) -> Outcome {
    let tiles = spread(scenario.size as i64, lands, area, damage);
    let mut r = Resolver {
        scenario,
        defs,
        rng,
        dead_units: Vec::new(),
        dead_buildings: Vec::new(),
        outcome: Outcome {
            damage,
            ..Outcome::default()
        },
    };
    for (tile, dmg) in tiles {
        r.tile(tile, dmg, kind);
    }
    let Resolver {
        scenario,
        mut dead_units,
        mut dead_buildings,
        mut outcome,
        ..
    } = r;
    dead_units.sort_unstable();
    dead_buildings.sort_unstable();
    for &i in dead_units.iter().rev() {
        scenario.units.remove(i);
    }
    for &i in dead_buildings.iter().rev() {
        scenario.buildings.remove(i);
    }
    outcome.removed_units = dead_units;
    outcome.removed_buildings = dead_buildings;
    outcome
}

/// Index of unit (or building) `i` once `removed` (ascending) are taken
/// out, or `None` if it is one of them.
pub fn remap(i: usize, removed: &[usize]) -> Option<usize> {
    match removed.binary_search(&i) {
        Ok(_) => None,
        Err(k) => Some(i - k),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roll_range() {
        let mut rng = Rand::new(1);
        // rand() = 41, 18467, ...
        assert_eq!(roll(&mut rng, 15, 10), 15 + 1 + 41 % 10);
        assert_eq!(roll(&mut rng, 100, 0), 101);
        let mut rng = Rand::new(9);
        for _ in 0..200 {
            assert!((16..=25).contains(&roll(&mut rng, 15, 10)));
        }
    }

    #[test]
    fn area_falloff() {
        assert_eq!(spread(10, (4, 4), 1, 30), vec![((4, 4), 30)]);
        let three = spread(10, (4, 4), 3, 25);
        assert_eq!(three.len(), 9);
        // half = 2: 25 - 12 * d.
        assert!(three.contains(&((4, 4), 25)));
        assert!(three.contains(&((3, 3), 13)));
        assert!(three.contains(&((5, 4), 13)));
        let five = spread(10, (4, 4), 5, 40);
        assert_eq!(five.len(), 25);
        // half = 3: 40 - 13 * d.
        assert!(five.contains(&((4, 4), 40)));
        assert!(five.contains(&((5, 5), 27)));
        assert!(five.contains(&((2, 6), 14)));
        // Off the map edge.
        assert_eq!(spread(10, (0, 0), 3, 9).len(), 4);
    }

    #[test]
    fn remapping() {
        let removed = [2, 5];
        assert_eq!(remap(0, &removed), Some(0));
        assert_eq!(remap(2, &removed), None);
        assert_eq!(remap(3, &removed), Some(2));
        assert_eq!(remap(6, &removed), Some(4));
    }

    use libday_data::defs::{RuinType, SurfaceType, TerrainType};
    use libday_data::scenario::{Building, Cell, Order, ScenarioInfo, Surface, Unit};

    fn terrain(id: &str, kind: u8, burned: &str) -> (String, TerrainType) {
        (
            id.into(),
            TerrainType {
                id: id.into(),
                kind,
                overlapping: false,
                colour: [0; 3],
                burned_id: burned.into(),
                burned_min: 1,
                can_fire_through: true,
            },
        )
    }

    fn unit_type(id: i64, armor: i64) -> UnitType {
        UnitType {
            id,
            tech: 0,
            name: String::new(),
            race: 0,
            facings: true,
            width: 1,
            height: 1,
            movement: 0,
            armor,
            action_points: 20,
            shoot_action_points: 0,
            can_overwatch: false,
            move_pix_speed: 8,
            move_animation: false,
            image: "HLAV".into(),
            cost: 0,
            cost_to_next: 0,
            damage_base: 0,
            damage_random: 0,
            damage_type: 0,
            damage_area: 1,
            ruin: "UHRUI233".into(),
            range: 10,
            min_range: 0,
            has_weapon: true,
            anti_air: false,
            damage_aa: 0,
            mine_detection: 0,
            weapon_type: 0,
            jammer: 100,
            hit: [100; 3],
            shot: Default::default(),
        }
    }

    fn defs() -> Definitions {
        let mut d = Definitions {
            terrains: [
                terrain("ARID000", terrain_kind::PLAIN, "ARIX000"),
                terrain("ARIX000", terrain_kind::PLAIN, ""),
                terrain("BORE100", terrain_kind::TREE, "BORX100"),
                terrain("BORX100", terrain_kind::PLAIN, ""),
                terrain("MONT000", terrain_kind::MOUNTAIN, ""),
                terrain("DEEP000", terrain_kind::DEEP_WATER, ""),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        d.units.insert((1, 0), unit_type(1, 30));
        for (id, armor) in [(0, 999), (1, 10)] {
            d.roads.insert(
                id,
                SurfaceType {
                    id,
                    name: String::new(),
                    armor,
                    image: String::new(),
                },
            );
        }
        d.walls.insert(
            1,
            SurfaceType {
                id: 1,
                name: String::new(),
                armor: 60,
                image: String::new(),
            },
        );
        d.buildings.insert(
            131,
            BuildingType {
                id: 131,
                name: String::new(),
                width: 2,
                height: 2,
                position: 3,
                armor: 150,
                unique: false,
                image: "HDEP".into(),
                transform_into: None,
                ai_target: true,
                damaged_images: true,
                category: 0,
                race: 0,
                destructible: true,
                has_ruin: true,
                explosion: Some((1, 25)),
            },
        );
        for id in ["UHRUI233"]
            .into_iter()
            .map(String::from)
            .chain((0..8).map(|i| format!("BHRUI{i:03}")))
        {
            d.ruins.insert(
                id.clone(),
                RuinType {
                    id,
                    can_fire_through: true,
                    can_move_through: true,
                },
            );
        }
        d
    }

    fn scenario(size: usize, terrain: &str) -> Scenario {
        Scenario {
            size,
            cells: (0..size * size)
                .map(|i| Cell {
                    id: i as i64,
                    terrain: terrain.into(),
                })
                .collect(),
            buildings: Vec::new(),
            units: Vec::new(),
            roads: Vec::new(),
            walls: Vec::new(),
            ruins: Vec::new(),
            mines: Vec::new(),
            deployments: Vec::new(),
            order_targets: Vec::new(),
            info: ScenarioInfo::default(),
        }
    }

    fn unit(x: i64, y: i64) -> Unit {
        Unit {
            counter: 0,
            id: 0,
            kind: 1,
            tech: 0,
            owner: 2,
            facing: 0,
            x,
            y,
            arrival: 0,
            reinforcement: false,
            armor: None,
            action_points: None,
            neg_ap: 0,
            order: Order::default(),
            objective_order: false,
            overwatch: false,
        }
    }

    fn surface(kind: i64, x: i64, y: i64, armor: i64) -> Surface {
        Surface {
            counter: 0,
            kind,
            owner: 0,
            x,
            y,
            armor,
            broken: false,
        }
    }

    fn terrain_at(s: &Scenario, x: i64, y: i64) -> &str {
        &s.cell(x, y).unwrap().terrain
    }

    #[test]
    fn unit_loses_armour_then_dies_leaving_a_ruin() {
        let d = defs();
        let mut s = scenario(8, "ARID000");
        s.units.push(unit(3, 3));
        let mut rng = Rand::new(1);
        let o = resolve(&mut s, &d, &mut rng, (3, 3), 12, 0, 1);
        assert_eq!(s.units[0].armor, Some(18));
        assert!(o.destroyed.is_empty());
        // A surviving unit shields the ground.
        assert_eq!(terrain_at(&s, 3, 3), "ARID000");
        let o = resolve(&mut s, &d, &mut rng, (3, 3), 20, 0, 1);
        assert!(s.units.is_empty());
        assert_eq!(o.removed_units, vec![0]);
        assert_eq!(o.ruins.len(), 1);
        assert_eq!(o.ruins[0].id, "UHRUI233");
        assert_eq!(terrain_at(&s, 3, 3), "ARIX000");
    }

    #[test]
    fn no_unit_ruin_off_plain_ground() {
        let d = defs();
        let mut s = scenario(8, "BORE100");
        s.units.push(unit(3, 3));
        let o = resolve(&mut s, &d, &mut Rand::new(1), (3, 3), 50, 0, 1);
        assert_eq!(o.removed_units, vec![0]);
        assert!(o.ruins.is_empty());
        // The forest still burns.
        assert_eq!(terrain_at(&s, 3, 3), "BORX100");
    }

    #[test]
    fn paralyser_drains_action_points() {
        let d = defs();
        let mut s = scenario(8, "ARID000");
        s.units.push(unit(3, 3));
        let o = resolve(&mut s, &d, &mut Rand::new(1), (3, 3), 101, 1, 3);
        assert!(o.destroyed.is_empty());
        let u = &s.units[0];
        assert_eq!((u.action_points, u.neg_ap, u.armor), (Some(0), 20, None));
        // Paralysing damage burns nothing.
        assert!(o.burned.is_empty());
    }

    #[test]
    fn mountains_never_burn_and_dirt_roads_hold() {
        let d = defs();
        let mut s = scenario(8, "MONT000");
        s.roads.push(surface(0, 4, 3, 999));
        s.roads.push(surface(1, 2, 3, 10));
        let o = resolve(&mut s, &d, &mut Rand::new(1), (3, 3), 500, 0, 3);
        assert!(o.burned.is_empty());
        assert_eq!(s.roads.len(), 1);
        assert_eq!((s.roads[0].x, s.roads[0].y), (4, 3));
    }

    #[test]
    fn unit_shields_its_road_but_not_a_bridge() {
        let d = defs();
        let mut s = scenario(8, "ARID000");
        s.units.push(unit(3, 3));
        s.roads.push(surface(1, 3, 3, 10));
        resolve(&mut s, &d, &mut Rand::new(1), (3, 3), 15, 0, 1);
        assert_eq!(s.roads.len(), 1);
        assert_eq!(s.units[0].armor, Some(15));

        let mut s = scenario(8, "DEEP000");
        s.units.push(unit(3, 3));
        s.roads.push(surface(1, 3, 3, 10));
        let o = resolve(&mut s, &d, &mut Rand::new(1), (3, 3), 15, 0, 1);
        assert!(s.roads.is_empty());
        assert!(s.units.is_empty(), "the bridge takes its unit down");
        assert_eq!(o.removed_units, vec![0]);
        assert!(o.ruins.is_empty());
    }

    #[test]
    fn walls_break_their_neighbours() {
        let d = defs();
        let mut s = scenario(8, "ARID000");
        for x in 2..5 {
            s.walls.push(surface(1, x, 3, 60));
        }
        resolve(&mut s, &d, &mut Rand::new(1), (3, 3), 30, 0, 1);
        assert_eq!(s.walls.len(), 3);
        assert_eq!(s.walls[1].armor, 30);
        resolve(&mut s, &d, &mut Rand::new(1), (3, 3), 30, 0, 1);
        assert_eq!(s.walls.len(), 2);
        assert!(s.walls.iter().all(|w| w.broken));
    }

    #[test]
    fn building_destroyed_leaves_ruins_and_burns() {
        let d = defs();
        let mut s = scenario(8, "ARID000");
        s.buildings.push(Building {
            counter: 0,
            id: 0,
            kind: 131,
            owner: 2,
            armor: 150,
            x: 2,
            y: 2,
            ai_target: true,
        });
        resolve(&mut s, &d, &mut Rand::new(1), (2, 2), 100, 0, 1);
        assert_eq!(s.buildings[0].armor, 50);
        assert!(d.buildings[&131].damaged(49));
        assert!(!d.buildings[&131].damaged(50));
        let o = resolve(&mut s, &d, &mut Rand::new(1), (3, 3), 60, 0, 1);
        assert!(s.buildings.is_empty());
        assert_eq!(
            o.destroyed,
            vec![Destroyed::Building {
                kind: 131,
                at: (2, 2)
            }]
        );
        // 2x2: two ruins.
        assert_eq!(o.ruins.len(), 2);
        assert!(o.ruins.iter().all(|r| r.id.starts_with("BHRUI")));
        for (x, y) in [(2, 2), (3, 2), (2, 3), (3, 3)] {
            assert_eq!(terrain_at(&s, x, y), "ARIX000");
        }
    }
}
