//! Automated units (off-map supports), as the original's server executes
//! them (`0x4751f1` factory, `Execute` per category):
//!
//! 1. Anti-aircraft (`0x472a0e`): every hostile unit that is `IsAA`, can
//!    overwatch, has the AP of a shot and has the target inside its range
//!    square (`PtInRect`, so `[x - R, x + R)`) fires once, weakest weapon
//!    first, while the aircraft stands: each adds its flat `DamageAA` to
//!    the damage taken and spends a shot's AP. The animation is `…1`
//!    unharmed, `…2` damaged, `…3` shot down.
//! 2. Shot down (damage ≥ the `armor` property), nothing happens.
//!    Otherwise the effect is scaled by `s = (armor − taken) / armor`:
//!    - destroy: area damage of `s × damage`, as a weapon's, friend or foe;
//!    - repair: armour and AP restored over the area, falling off like
//!      damage;
//!    - mine: the mines within `s × range` are revealed;
//!    - drop: `nbDrop` units (half when damaged) appear around the target
//!      with full AP; an armed mine on a spot goes off instead and an
//!      unplaceable spot loses its unit;
//!    - build: one bridge section (a paved road) on the target;
//!    - transport: the unit on the target is lifted, then the second leg's
//!      anti-aircraft fire decides whether it is set down on the
//!      destination (losing its AP and the share of armour the transport
//!      lost) or destroyed.

use libday_data::defs::{AutomatedCategory, AutomatedType, auto_prop};
use libday_data::scenario::{Ruin, Scenario, Surface, Unit};
use libday_data::{Definitions, scenario::Order};

use crate::damage::{self, Destroyed};
use crate::movement::{self, Access};
use crate::scatter::Rand;

/// Road type a bridge is built as (`0x4720d8` looks up road type 1).
pub const BRIDGE_ROAD: i64 = 1;

/// One animation of a support (two for a transport) and the scenario once
/// its effect is applied.
#[derive(Clone, Debug)]
pub struct Leg {
    /// `AUTOUNIT\<image><0|1><anim>.SMK`: 1 unharmed, 2 damaged, 3 shot
    /// down.
    pub anim: u8,
    /// The transport's second leg (`…1x` animations, props 24–27).
    pub second: bool,
    pub at: (i64, i64),
    /// Anti-aircraft units that fired, as indices before this leg.
    pub shooters: Vec<usize>,
    /// The scenario after this leg.
    pub scenario: Scenario,
    /// Units and buildings gone, as indices before this leg (ascending).
    pub removed_units: Vec<usize>,
    pub removed_buildings: Vec<usize>,
    pub destroyed: Vec<Destroyed>,
    pub ruins: Vec<Ruin>,
}

impl Leg {
    /// SMK member name in `ANMSUNIT.FF`.
    pub fn animation(&self, kind: &AutomatedType) -> String {
        format!(
            "AUTOUNIT\\{}{}{}.SMK",
            kind.image,
            u8::from(self.second),
            self.anim
        )
    }

    /// Frames (0-based) of the anti-aircraft fire and of the effect.
    pub fn frames(&self, kind: &AutomatedType) -> (usize, usize) {
        let (aa, action) = if self.second {
            (
                kind.int_or(auto_prop::SECOND_FRAME_AA, 0),
                kind.int_or(auto_prop::SECOND_FRAME_ACTION, 0),
            )
        } else {
            (kind.frame_aa, kind.frame_action)
        };
        ((aa - 1).max(0) as usize, (action - 1).max(0) as usize)
    }

    /// Where anti-aircraft hits show, in 1024×768 animation pixels.
    pub fn aa_pos(&self, kind: &AutomatedType) -> (i64, i64) {
        if self.second {
            (
                kind.int_or(auto_prop::SECOND_AA_POS_X, 0),
                kind.int_or(auto_prop::SECOND_AA_POS_Y, 0),
            )
        } else {
            kind.aa_pos
        }
    }
}

/// Unit on tile `at` (footprints included), on the map.
pub fn unit_at(scenario: &Scenario, defs: &Definitions, at: (i64, i64)) -> Option<usize> {
    scenario.units.iter().position(|u| {
        let (w, h) = defs.unit(u.kind).map_or((1, 1), |k| (k.width, k.height));
        u.arrival == 0 && (u.x..u.x + w).contains(&at.0) && (u.y..u.y + h).contains(&at.1)
    })
}

/// Building on tile `at`.
pub fn building_at(scenario: &Scenario, defs: &Definitions, at: (i64, i64)) -> Option<usize> {
    scenario.buildings.iter().position(|b| {
        let (w, h) = defs
            .buildings
            .get(&b.kind)
            .map_or((1, 1), |k| (k.width, k.height));
        (b.x..b.x + w).contains(&at.0) && (b.y..b.y + h).contains(&at.1)
    })
}

/// Whether a unit of `kind` (technology `tech`) of `owner` can be put on
/// `at`: on the map, on free tiles it could stop on.
pub fn placeable(
    scenario: &Scenario,
    defs: &Definitions,
    kind: i64,
    tech: i64,
    owner: i64,
    at: (i64, i64),
) -> bool {
    let Some(k) = defs.unit_tech(kind, tech) else {
        return false;
    };
    for dy in 0..k.height.max(1) {
        for dx in 0..k.width.max(1) {
            let p = (at.0 + dx, at.1 + dy);
            if !scenario.contains(p.0, p.1)
                || unit_at(scenario, defs, p).is_some()
                || building_at(scenario, defs, p).is_some()
            {
                return false;
            }
        }
    }
    let mut probe = scenario.clone();
    probe.units.push(new_unit(-1, kind, tech, owner, at, 0, k));
    let grid = movement::move_grid(&probe, defs, probe.units.len() - 1);
    grid.is_none_or(|g| g.get(at.0 as i32, at.1 as i32).0 == Access::Stop)
}

/// Terrain kind of tile `at`, if it is on the map.
fn kind_at(scenario: &Scenario, defs: &Definitions, at: (i64, i64)) -> Option<u8> {
    scenario
        .cell(at.0, at.1)
        .and_then(|c| defs.terrain(&c.terrain))
        .map(|t| t.kind)
}

/// The tiles bridges may span, computed from the terrain as the road
/// layer does (`0x42fb2d`):
///
/// 1. a riverbank with plain behind it (west or north) and water ahead
///    (east or south) starts a span (`0x42fbb1`);
/// 2. the span crosses deep water to the next riverbank, which must have
///    plain beyond it (`0x42fe1d`);
/// 3. a span whose start and end are each next to an earlier one's is
///    dropped (`0x42ff6e`);
/// 4. a span sharing a tile with an earlier one is dropped and the rest
///    give their tiles, ends first (`0x43002d`).
pub fn bridge_sites(scenario: &Scenario, defs: &Definitions) -> Vec<(i64, i64)> {
    use libday_data::defs::terrain_kind::{DEEP_WATER, PLAIN, RIVERBANK};
    let kind = |x: i64, y: i64| kind_at(scenario, defs, (x, y));
    let water = |k: Option<u8>| matches!(k, Some(DEEP_WATER | RIVERBANK));
    let size = scenario.size as i64;
    let mut spans: Vec<((i64, i64), (i64, i64))> = Vec::new();
    for y in 0..size - 1 {
        for x in 0..size - 1 {
            if kind(x, y) != Some(RIVERBANK) {
                continue;
            }
            let dir = if water(kind(x + 1, y)) && kind(x - 1, y) == Some(PLAIN) {
                (1, 0)
            } else if water(kind(x, y + 1)) && kind(x, y - 1) == Some(PLAIN) {
                (0, 1)
            } else {
                continue;
            };
            let mut end = (x, y);
            let reached = loop {
                if end.0 >= size || end.1 >= size {
                    break false;
                }
                end = (end.0 + dir.0, end.1 + dir.1);
                match kind(end.0, end.1) {
                    Some(RIVERBANK) => break true,
                    Some(DEEP_WATER) => {}
                    _ => break false,
                }
            };
            if reached && kind(end.0 + dir.0, end.1 + dir.1) == Some(PLAIN) {
                spans.push(((x, y), end));
            }
        }
    }
    let near = |a: (i64, i64), b: (i64, i64)| (a.0 - b.0).abs() == 1 || (a.1 - b.1).abs() == 1;
    let mut i = 0;
    while i < spans.len() {
        let (s, e) = spans[i];
        let mut j = i + 1;
        while j < spans.len() {
            if near(s, spans[j].0) && near(e, spans[j].1) {
                spans.remove(j);
            } else {
                j += 1;
            }
        }
        i += 1;
    }
    let mut sites = Vec::new();
    for (s, e) in spans {
        let dir = ((e.0 - s.0).signum(), (e.1 - s.1).signum());
        let mut tiles = vec![s, e];
        let mut p = (s.0 + dir.0, s.1 + dir.1);
        while p != e {
            tiles.push(p);
            p = (p.0 + dir.0, p.1 + dir.1);
        }
        if !tiles.iter().any(|t| sites.contains(t)) {
            sites.extend(tiles);
        }
    }
    sites
}

/// Whether a bridge tile may be built on `at` now (`0x42f5dd`): a free
/// bridge site, on deep water only next to a road, so bridges grow from
/// the banks and from the tiles already built.
pub fn bridgeable(scenario: &Scenario, defs: &Definitions, at: (i64, i64)) -> bool {
    bridge_buildable(scenario, defs, &bridge_sites(scenario, defs), at)
}

/// [`bridgeable`] against precomputed [`bridge_sites`].
pub fn bridge_buildable(
    scenario: &Scenario,
    defs: &Definitions,
    sites: &[(i64, i64)],
    at: (i64, i64),
) -> bool {
    use libday_data::defs::terrain_kind::{DEEP_WATER, RIVERBANK};
    let road = |p: (i64, i64)| scenario.roads.iter().any(|r| (r.x, r.y) == p);
    if !sites.contains(&at)
        || unit_at(scenario, defs, at).is_some()
        || building_at(scenario, defs, at).is_some()
        || scenario.walls.iter().any(|w| (w.x, w.y) == at)
        || road(at)
    {
        return false;
    }
    match kind_at(scenario, defs, at) {
        Some(RIVERBANK) => true,
        Some(DEEP_WATER) => [(-1, 0), (1, 0), (0, -1), (0, 1)]
            .iter()
            .any(|d| road((at.0 + d.0, at.1 + d.1))),
        _ => false,
    }
}
/// Whether `player` may target `at` (and `dest`, for a transport's second
/// click) with support `kind`, as the selectors check it.
pub fn valid_target(
    scenario: &Scenario,
    defs: &Definitions,
    kind: &AutomatedType,
    player: i64,
    at: (i64, i64),
    dest: Option<(i64, i64)>,
) -> bool {
    if !scenario.contains(at.0, at.1) {
        return false;
    }
    match kind.category() {
        Some(AutomatedCategory::Repair) => {
            unit_at(scenario, defs, at).is_some_and(|u| scenario.units[u].owner == player)
                || building_at(scenario, defs, at)
                    .is_some_and(|b| scenario.buildings[b].owner == player)
        }
        Some(AutomatedCategory::Build) => bridgeable(scenario, defs, at),
        Some(AutomatedCategory::Transport) => {
            let Some(u) = unit_at(scenario, defs, at).filter(|&u| {
                let unit = &scenario.units[u];
                unit.owner == player && defs.unit(unit.kind).is_some_and(|k| k.movement != 0)
            }) else {
                return false;
            };
            dest.is_none_or(|d| {
                let unit = &scenario.units[u];
                let mut s = scenario.clone();
                s.units.remove(u);
                placeable(&s, defs, unit.kind, unit.tech, player, d)
            })
        }
        Some(_) => true,
        None => false,
    }
}

/// Anti-aircraft fire at a support of `player` over `at`: the units that
/// fire, each spending a shot's AP, while `taken` is below `armor`.
fn anti_air(
    scenario: &mut Scenario,
    defs: &Definitions,
    player: i64,
    at: (i64, i64),
    armor: i64,
    taken: &mut i64,
) -> Vec<usize> {
    let mut cands: Vec<(i64, i64, usize)> = scenario
        .units
        .iter()
        .enumerate()
        .filter_map(|(i, u)| {
            let k = defs.unit_tech(u.kind, u.tech)?;
            let ap = u.action_points.unwrap_or(k.action_points);
            let r = k.range;
            (u.owner != player
                && u.arrival == 0
                && k.anti_air
                && k.can_overwatch
                && ap >= k.shoot_action_points
                && (u.x - r..u.x + r).contains(&at.0)
                && (u.y - r..u.y + r).contains(&at.1))
            .then_some((
                k.damage_base + k.damage_random / 2,
                k.shoot_action_points,
                i,
            ))
        })
        .collect();
    cands.sort_by_key(|&(mid, shoot, _)| (mid, shoot));
    let mut fired = Vec::new();
    for (_, shoot, i) in cands {
        if *taken >= armor {
            break;
        }
        let u = &mut scenario.units[i];
        let Some(k) = defs.unit_tech(u.kind, u.tech) else {
            continue;
        };
        *taken += k.damage_aa;
        u.action_points = Some(u.action_points.unwrap_or(k.action_points) - shoot);
        fired.push(i);
    }
    fired
}

fn anim(taken: i64, armor: i64) -> u8 {
    if taken >= armor {
        3
    } else if taken > 0 {
        2
    } else {
        1
    }
}

/// A new unit of `owner` with full armour and AP.
fn new_unit(
    id: i64,
    kind: i64,
    tech: i64,
    owner: i64,
    (x, y): (i64, i64),
    facing: i64,
    k: &libday_data::defs::UnitType,
) -> Unit {
    Unit {
        counter: id,
        id,
        kind,
        tech,
        owner,
        facing: if k.facings { facing.rem_euclid(8) } else { 0 },
        x,
        y,
        arrival: 0,
        reinforcement: false,
        armor: Some(k.armor),
        action_points: Some(k.action_points),
        neg_ap: 0,
        order: Order::default(),
        objective_order: false,
        overwatch: true,
    }
}

/// `inc` at Chebyshev distance `d` in an `area` square, scaled by `s`
/// (`0x4731c6`).
fn amount(s: f64, inc: i64, area: i64, d: i64) -> i64 {
    let half = ((area.max(1) + 1) / 2).max(1);
    (s * (inc - (inc / half) * d) as f64) as i64
}

/// Units and buildings of `before` missing from `after`, by ID.
fn removed(before: &Scenario, after: &Scenario) -> (Vec<usize>, Vec<usize>) {
    let units = before
        .units
        .iter()
        .enumerate()
        .filter(|(_, u)| !after.units.iter().any(|n| n.id == u.id))
        .map(|(i, _)| i)
        .collect();
    let buildings = before
        .buildings
        .iter()
        .enumerate()
        .filter(|(_, b)| !after.buildings.iter().any(|n| n.id == b.id))
        .map(|(i, _)| i)
        .collect();
    (units, buildings)
}

/// Executes support `kind` of `player` on `at` (and `dest` for a
/// transport): its legs, each with the scenario it leaves.
pub fn execute(
    scenario: &Scenario,
    defs: &Definitions,
    rng: &mut Rand,
    kind: &AutomatedType,
    player: i64,
    at: (i64, i64),
    dest: Option<(i64, i64)>,
) -> Vec<Leg> {
    let Some(category) = kind.category() else {
        return Vec::new();
    };
    let armor = kind.int_or(auto_prop::ARMOR, 0).max(1);
    let mut taken = 0;
    let mut s = scenario.clone();
    let shooters = anti_air(&mut s, defs, player, at, armor, &mut taken);
    let mut legs = Vec::new();
    let mut leg = |before: &Scenario,
                   s: Scenario,
                   at,
                   anim,
                   second,
                   shooters,
                   destroyed: Vec<Destroyed>,
                   ruins| {
        let (removed_units, removed_buildings) = removed(before, &s);
        legs.push(Leg {
            anim,
            second,
            at,
            shooters,
            scenario: s,
            removed_units,
            removed_buildings,
            destroyed,
            ruins,
        });
    };
    let first = anim(taken, armor);
    if taken >= armor {
        leg(
            scenario,
            s,
            at,
            first,
            false,
            shooters,
            Vec::new(),
            Vec::new(),
        );
        return legs;
    }
    let survival = (armor - taken) as f64 / armor as f64;
    let mut destroyed = Vec::new();
    let mut ruins = Vec::new();
    let area = kind.int_or(auto_prop::DAMAGE_AREA, 0);
    match category {
        AutomatedCategory::Destroy => {
            let dmg = (survival * kind.int_or(auto_prop::DAMAGE, 0) as f64) as i64;
            let ar = kind
                .infos
                .get(&auto_prop::DAMAGE_TYPE)
                .is_some_and(|t| t.trim().eq_ignore_ascii_case("AR"));
            let rolled = damage::roll(rng, dmg, 0);
            let o = damage::resolve(&mut s, defs, rng, at, rolled, i64::from(!ar), area);
            destroyed = o.destroyed;
            ruins = o.ruins;
        }
        AutomatedCategory::Repair => {
            let armor_inc = kind.int_or(auto_prop::ARMOR_INC, 0);
            let action_inc = kind.int_or(auto_prop::ACTION_INC, 0);
            for ((x, y), _) in damage::spread(s.size as i64, at, area, 0) {
                let d = (x - at.0).abs().max((y - at.1).abs());
                let (aa, ap) = (
                    amount(survival, armor_inc, area, d),
                    amount(survival, action_inc, area, d),
                );
                if let Some(b) = building_at(&s, defs, (x, y))
                    && aa > 0
                {
                    let b = &mut s.buildings[b];
                    let max = defs.buildings.get(&b.kind).map_or(b.armor, |k| k.armor);
                    b.armor = (b.armor + aa).min(max.max(b.armor));
                }
                if let Some(u) = unit_at(&s, defs, (x, y)) {
                    let u = &mut s.units[u];
                    if let Some(k) = defs.unit_tech(u.kind, u.tech) {
                        if aa > 0 {
                            u.armor = Some((u.armor.unwrap_or(k.armor) + aa).min(k.armor));
                        }
                        if ap > 0 {
                            u.action_points = Some(
                                (u.action_points.unwrap_or(k.action_points) + ap)
                                    .min(k.action_points),
                            );
                        }
                    }
                }
                if aa > 0
                    && let Some(w) = s.walls.iter_mut().find(|w| (w.x, w.y) == (x, y))
                {
                    let max = defs.walls.get(&w.kind).map_or(w.armor, |k| k.armor);
                    w.armor = (w.armor + aa).min(max.max(w.armor));
                }
            }
        }
        AutomatedCategory::Build => {
            if bridgeable(&s, defs, at) {
                let counter = s.roads.iter().map(|r| r.counter + 1).max().unwrap_or(0);
                let armor = defs.roads.get(&BRIDGE_ROAD).map_or(10, |r| r.armor);
                s.roads.push(Surface {
                    counter,
                    kind: BRIDGE_ROAD,
                    owner: player,
                    x: at.0,
                    y: at.1,
                    armor,
                    broken: false,
                });
            }
        }
        AutomatedCategory::Mine => {
            let r = (survival * kind.int_or(auto_prop::MINE_DETECTION_RANGE, 0) as f64) as i64;
            let size = s.size as i64;
            let (x0, x1) = ((at.0 - r).max(0), (at.0 + r).min(size));
            let (y0, y1) = ((at.1 - r).max(0), (at.1 + r).min(size));
            for m in &mut s.mines {
                if (x0..x1).contains(&m.x) && (y0..y1).contains(&m.y) {
                    m.reveal_to(player);
                }
            }
        }
        AutomatedCategory::Drop => {
            let unit = kind.int_or(auto_prop::UNIT_ID_DROP, 0);
            let tech = kind.int_or(auto_prop::UNIT_TL_DROP, 0);
            let mut n = kind.int_or(auto_prop::NB_DROP, 0);
            if taken > 0 {
                n /= 2;
            }
            for ((dx, dy), facing) in kind.drops().into_iter().take(n.max(0) as usize) {
                let p = (at.0 + dx, at.1 + dy);
                if let Some(m) = s.mines.iter().position(|m| m.armed && (m.x, m.y) == p) {
                    let mine = s.mines[m].clone();
                    if let Some(mk) = defs.mines.get(&mine.kind) {
                        let dmg = damage::roll(rng, mk.damage_base, mk.damage_random);
                        let o = damage::resolve(&mut s, defs, rng, p, dmg, mk.damage_type, 1);
                        destroyed.extend(o.destroyed);
                        ruins.extend(o.ruins);
                    }
                    continue;
                }
                if !placeable(&s, defs, unit, tech, player, p) {
                    continue;
                }
                let Some(k) = defs.unit_tech(unit, tech) else {
                    continue;
                };
                let id = s.next_unit_id();
                s.units.push(new_unit(id, unit, tech, player, p, facing, k));
            }
        }
        AutomatedCategory::Transport => {
            let Some(u) = unit_at(&s, defs, at) else {
                leg(scenario, s, at, first, false, shooters, destroyed, ruins);
                return legs;
            };
            s.units[u].arrival = -1;
            let id = s.units[u].id;
            let lifted = s.clone();
            leg(
                scenario,
                s,
                at,
                first,
                false,
                shooters,
                Vec::new(),
                Vec::new(),
            );
            let Some(dest) = dest else {
                return legs;
            };
            let mut s = lifted.clone();
            let shooters = anti_air(&mut s, defs, player, dest, armor, &mut taken);
            let second = anim(taken, armor);
            let i = s.units.iter().position(|u| u.id == id);
            if let Some(i) = i {
                let unit = s.units[i].clone();
                if taken < armor && placeable(&s, defs, unit.kind, unit.tech, player, dest) {
                    let survival = (armor - taken) as f64 / armor as f64;
                    let max = defs.unit_tech(unit.kind, unit.tech).map_or(1, |k| k.armor);
                    let u = &mut s.units[i];
                    u.armor = Some(((u.armor.unwrap_or(max) as f64 * survival) as i64).max(1));
                    u.action_points = Some(0);
                    (u.x, u.y, u.arrival) = (dest.0, dest.1, 0);
                } else {
                    s.units.remove(i);
                }
            }
            leg(
                &lifted,
                s,
                dest,
                second,
                true,
                shooters,
                Vec::new(),
                Vec::new(),
            );
            return legs;
        }
    }
    leg(scenario, s, at, first, false, shooters, destroyed, ruins);
    legs
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use libday_data::defs::{MineType, TerrainType, UnitType};
    use libday_data::scenario::{Cell, Mine, ScenarioInfo};

    use super::*;

    fn kind(id: i64, armor: i64, anti_air: bool) -> UnitType {
        UnitType {
            id,
            armor,
            action_points: 100,
            shoot_action_points: 30,
            can_overwatch: true,
            anti_air,
            damage_aa: 15,
            range: 4,
            damage_base: 10,
            damage_random: 4,
            width: 1,
            height: 1,
            movement: 1,
            facings: true,
            ..UnitType::default()
        }
    }

    fn defs() -> Definitions {
        let mut d = Definitions::default();
        for k in [kind(1, 40, false), kind(2, 30, false), kind(9, 60, true)] {
            d.units.insert((k.id, 0), k);
        }
        d.terrains.insert(
            "ARID000".into(),
            TerrainType {
                id: "ARID000".into(),
                kind: libday_data::defs::terrain_kind::PLAIN,
                overlapping: false,
                colour: [0; 3],
                burned_id: String::new(),
                burned_min: 0,
                can_fire_through: true,
            },
        );
        d.mines.insert(
            0,
            MineType {
                id: 0,
                name: String::new(),
                image: String::new(),
                damage_base: 50,
                damage_random: 1,
                damage_type: 0,
            },
        );
        d
    }

    fn scenario() -> Scenario {
        Scenario {
            size: 16,
            cells: (0..256)
                .map(|i| Cell {
                    id: i,
                    terrain: "ARID000".into(),
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
            objective: None,
            info: ScenarioInfo::default(),
        }
    }

    fn unit(id: i64, kind: i64, owner: i64, x: i64, y: i64) -> Unit {
        Unit {
            counter: id,
            id,
            kind,
            tech: 0,
            owner,
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
            overwatch: true,
        }
    }

    fn support(category: i64, infos: &[(i64, &str)]) -> AutomatedType {
        AutomatedType {
            id: 0,
            name: String::new(),
            image: "HPAR".into(),
            race: 0,
            category,
            cost: 10,
            prerequisite: -1,
            aa_pos: (0, 0),
            frame_aa: 47,
            frame_action: 74,
            cursor: String::new(),
            explain: String::new(),
            deploy_once: false,
            infos: infos
                .iter()
                .map(|&(k, v)| (k, v.to_owned()))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    fn troopers() -> AutomatedType {
        support(
            4,
            &[
                (0, "20"),
                (5, "2"),
                (6, "0"),
                (7, "4"),
                (8, "-1"),
                (9, "-2"),
                (10, "2"),
                (11, "2"),
                (12, "-1"),
                (13, "1"),
                (14, "2"),
                (15, "-2"),
                (29, "6"),
                (30, "2"),
                (31, "4"),
                (32, "0"),
            ],
        )
    }

    #[test]
    fn troopers_drop_four_units_with_full_ap() {
        let d = defs();
        let s = scenario();
        let legs = execute(&s, &d, &mut Rand::new(1), &troopers(), 1, (8, 8), None);
        assert_eq!(legs.len(), 1);
        let l = &legs[0];
        assert_eq!(l.anim, 1);
        assert_eq!(l.animation(&troopers()), "AUTOUNIT\\HPAR01.SMK");
        assert_eq!(l.frames(&troopers()), (46, 73));
        let at: Vec<_> = l
            .scenario
            .units
            .iter()
            .map(|u| (u.x, u.y, u.facing))
            .collect();
        assert_eq!(at, vec![(7, 6, 6), (10, 10, 2), (7, 9, 4), (10, 6, 0)]);
        assert!(l.scenario.units.iter().all(|u| u.owner == 1));
        assert!(
            l.scenario
                .units
                .iter()
                .all(|u| u.action_points == Some(100))
        );
    }

    #[test]
    fn anti_air_halves_a_drop_and_downs_it() {
        let d = defs();
        let mut s = scenario();
        s.units.push(unit(5, 9, 0, 8, 8));
        // DamageAA 15 against 20 armour: damaged, half the troopers.
        let legs = execute(&s, &d, &mut Rand::new(1), &troopers(), 1, (8, 8), None);
        let l = &legs[0];
        assert_eq!((l.anim, l.shooters.clone()), (2, vec![0]));
        assert_eq!(l.scenario.units.len(), 3);
        assert_eq!(l.scenario.units[0].action_points, Some(70));
        // Two guns bring it down.
        s.units.push(unit(6, 9, 0, 9, 9));
        let legs = execute(&s, &d, &mut Rand::new(1), &troopers(), 1, (8, 8), None);
        assert_eq!(legs[0].anim, 3);
        assert_eq!(legs[0].scenario.units.len(), 2);
        // Out of range: [x - R, x + R).
        let mut s = scenario();
        s.units.push(unit(5, 9, 0, 4, 8));
        let legs = execute(&s, &d, &mut Rand::new(1), &troopers(), 1, (8, 8), None);
        assert_eq!(legs[0].anim, 1);
    }

    #[test]
    fn drop_onto_a_mine_sets_it_off() {
        let d = defs();
        let mut s = scenario();
        s.mines.push(Mine {
            counter: 0,
            kind: 0,
            owner: 0,
            x: 7,
            y: 6,
            armed: true,
            hidden_ai: false,
            hidden: [true; 4],
        });
        s.units.push(unit(3, 1, 1, 10, 10));
        let legs = execute(&s, &d, &mut Rand::new(1), &troopers(), 1, (8, 8), None);
        let l = &legs[0];
        assert!(l.scenario.mines.is_empty());
        // The mine's spot and the occupied spot lose their units.
        assert_eq!(l.scenario.units.len(), 3);
    }

    #[test]
    fn bomber_damages_friend_and_foe_falling_off() {
        let d = defs();
        let mut s = scenario();
        s.units.push(unit(1, 1, 0, 8, 8));
        s.units.push(unit(2, 1, 1, 9, 8));
        let bomber = support(0, &[(0, "40"), (1, "30"), (2, "AR"), (4, "3")]);
        let legs = execute(&s, &d, &mut Rand::new(1), &bomber, 1, (8, 8), None);
        let u = &legs[0].scenario.units;
        // 31 at the centre, 31 - 31 / 2 one tile away.
        assert_eq!(u[0].armor, Some(9));
        assert_eq!(u[1].armor, Some(24));
    }

    #[test]
    fn mine_sweep_reveals_within_scaled_range() {
        let d = defs();
        let mut s = scenario();
        for x in [2, 3, 12, 13] {
            s.mines.push(Mine {
                counter: x,
                kind: 0,
                owner: 0,
                x,
                y: 8,
                armed: true,
                hidden_ai: false,
                hidden: [true; 4],
            });
        }
        let sweep = support(3, &[(0, "70"), (3, "5")]);
        let legs = execute(&s, &d, &mut Rand::new(1), &sweep, 1, (8, 8), None);
        let seen: Vec<_> = legs[0]
            .scenario
            .mines
            .iter()
            .map(|m| !m.hidden_to(1))
            .collect();
        assert_eq!(seen, vec![false, true, true, false]);
    }

    #[test]
    fn repair_restores_armour_and_ap_up_to_the_maximum() {
        let d = defs();
        let mut s = scenario();
        let mut u = unit(1, 1, 1, 8, 8);
        u.armor = Some(5);
        u.action_points = Some(0);
        s.units.push(u);
        let medic = support(1, &[(0, "60"), (28, "20"), (35, "200"), (4, "1")]);
        assert!(valid_target(&s, &d, &medic, 1, (8, 8), None));
        assert!(!valid_target(&s, &d, &medic, 1, (9, 8), None));
        let legs = execute(&s, &d, &mut Rand::new(1), &medic, 1, (8, 8), None);
        let u = &legs[0].scenario.units[0];
        assert_eq!((u.armor, u.action_points), (Some(25), Some(100)));
    }

    #[test]
    fn transport_lifts_then_sets_down_without_ap() {
        let d = defs();
        let mut s = scenario();
        s.units.push(unit(1, 1, 1, 2, 2));
        s.units.push(unit(9, 9, 0, 12, 12));
        let transport = support(5, &[(0, "40"), (26, "1"), (27, "40")]);
        assert!(valid_target(&s, &d, &transport, 1, (2, 2), Some((11, 11))));
        assert!(!valid_target(&s, &d, &transport, 1, (2, 2), Some((12, 12))));
        let legs = execute(
            &s,
            &d,
            &mut Rand::new(1),
            &transport,
            1,
            (2, 2),
            Some((11, 11)),
        );
        assert_eq!(legs.len(), 2);
        assert_eq!(legs[0].scenario.units[0].arrival, -1);
        assert_eq!(legs[1].animation(&transport), "AUTOUNIT\\HPAR12.SMK");
        let u = &legs[1].scenario.units[0];
        // 15 of 40 taken: 40 * 25 / 40.
        assert_eq!((u.x, u.y, u.arrival), (11, 11, 0));
        assert_eq!((u.armor, u.action_points), (Some(25), Some(0)));
    }

    #[test]
    fn bridges_span_rivers_and_grow_from_the_banks() {
        use libday_data::defs::terrain_kind::{DEEP_WATER, RIVERBANK};
        let mut d = defs();
        for (id, kind) in [("BANK", RIVERBANK), ("DEEP", DEEP_WATER)] {
            let mut t = d.terrains["ARID000"].clone();
            (t.id, t.kind) = (id.into(), kind);
            d.terrains.insert(id.into(), t);
        }
        // A river running north-south: banks at x = 5 and 9, deep between.
        let mut s = scenario();
        for c in &mut s.cells {
            c.terrain = match c.id % 16 {
                5 | 9 => "BANK",
                6..=8 => "DEEP",
                _ => "ARID000",
            }
            .into();
        }
        let sites = bridge_sites(&s, &d);
        // Each row's crossing drops the next row's (adjacent ends); the
        // last row is never scanned.
        assert_eq!(sites.len(), 8 * 5);
        assert_eq!(&sites[..5], &[(5, 0), (9, 0), (6, 0), (7, 0), (8, 0)]);
        assert!(!sites.contains(&(5, 1)) && sites.contains(&(5, 2)));
        assert!(bridgeable(&s, &d, (5, 0)) && bridgeable(&s, &d, (9, 0)));
        assert!(!bridgeable(&s, &d, (6, 0)) && !bridgeable(&s, &d, (5, 1)));
        let bridge = support(2, &[(0, "40")]);
        let legs = execute(&s, &d, &mut Rand::new(1), &bridge, 1, (5, 0), None);
        let s = &legs[0].scenario;
        assert_eq!(s.roads.len(), 1);
        assert!(bridgeable(s, &d, (6, 0)) && !bridgeable(s, &d, (5, 0)));
    }
}
