//! The ordered draw list of a scenario map.

use std::collections::{BTreeMap, BTreeSet};

use libday_data::Definitions;
use libday_data::defs::terrain_kind;
use libday_data::scenario::{Deployment, Scenario, Surface};

use crate::{Art, Sheet, footprint};

/// What a draw item depicts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    Terrain,
    Road,
    Decor,
    Wall,
    Mine,
    Building,
    Unit,
    Deployment,
}

impl Layer {
    pub fn name(self) -> &'static str {
        match self {
            Layer::Terrain => "terrain",
            Layer::Road => "roads",
            Layer::Decor => "decors",
            Layer::Wall => "walls",
            Layer::Mine => "mines",
            Layer::Building => "building",
            Layer::Unit => "unit",
            Layer::Deployment => "deployment",
        }
    }
}

/// One 54x56 sprite cell drawn on tile `(x, y)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawItem {
    pub layer: Layer,
    pub sheet: Sheet,
    pub sprite: String,
    pub x: i64,
    pub y: i64,
}

#[derive(Clone, Debug)]
pub struct Options {
    /// Show units arriving on this turn (0 = units on the map at the start).
    pub arrival: Option<i64>,
    /// Show the human players' starting zones.
    pub deployment: bool,
    /// Bare ground only: no buildings, units, overlays, trees or scenery.
    pub terrain_only: bool,
    /// Ground drawn under trees/scenery without a plain counterpart.
    pub ground: String,
    /// Indices into `scenario.units` left out of the scene (units being
    /// animated separately).
    pub hidden_units: Vec<usize>,
    /// Player whose view is shown: mines hidden from them are left out
    /// (`None`: everything).
    pub viewer: Option<i64>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            arrival: Some(0),
            deployment: false,
            terrain_only: false,
            ground: "BORE000".to_owned(),
            hidden_units: Vec::new(),
            viewer: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub size: usize,
    /// Back-to-front draw order.
    pub items: Vec<DrawItem>,
    /// Sprites (or definitions) the scene needed but the data lacks.
    pub missing: Vec<String>,
}

/// Neighbour offsets and their connection-mask bits.
const NEIGHBORS: [(i64, i64, usize); 4] = [(-1, 0, 8), (1, 0, 4), (0, -1, 2), (0, 1, 1)];
const CONNECTION_SPRITES: [u8; 16] = [0, 9, 10, 7, 8, 13, 12, 2, 11, 15, 14, 5, 6, 3, 4, 1];
const DEPLOYMENT_SPRITES: [u8; 16] = [0, 13, 15, 14, 10, 1, 7, 4, 12, 3, 9, 6, 11, 2, 8, 5];

type Key = (i64, i64);

fn kind_at(scenario: &Scenario, defs: &Definitions, x: i64, y: i64) -> Option<u8> {
    let cell = scenario.cell(x, y)?;
    defs.terrain(&cell.terrain).map(|t| t.kind)
}

/// First-placed (lowest counter) surface on each tile.
fn surface_index(items: &[Surface]) -> BTreeMap<Key, &Surface> {
    let mut index: BTreeMap<Key, &Surface> = BTreeMap::new();
    for item in items {
        let slot = index.entry((item.x, item.y)).or_insert(item);
        if item.counter < slot.counter {
            *slot = item;
        }
    }
    index
}

fn connection(index: &BTreeMap<Key, &Surface>, item: &Surface) -> u8 {
    let mask = NEIGHBORS
        .iter()
        .filter(|(dx, dy, _)| {
            index
                .get(&(item.x + dx, item.y + dy))
                .is_some_and(|n| n.kind == item.kind)
        })
        .map(|(_, _, bit)| bit)
        .sum::<usize>();
    CONNECTION_SPRITES[mask]
}

fn road_sprite(
    scenario: &Scenario,
    defs: &Definitions,
    image: &str,
    shape: u8,
    item: &Surface,
) -> String {
    let kind = kind_at(scenario, defs, item.x, item.y);
    let mode = match kind {
        Some(terrain_kind::DEEP_WATER) => 'B',
        Some(terrain_kind::RIVERBANK) => 'R',
        _ => 'G',
    };
    let mut shape = shape;
    if mode == 'R' {
        let at = |dx, dy| kind_at(scenario, defs, item.x + dx, item.y + dy);
        let water = |k: Option<u8>| matches!(k, Some(1 | 2));
        let plain = |k: Option<u8>| k == Some(terrain_kind::PLAIN);
        let orientation = if water(at(-1, 0)) && plain(at(1, 0)) {
            0
        } else if water(at(1, 0)) && plain(at(-1, 0)) {
            1
        } else if water(at(0, -1)) && plain(at(0, 1)) {
            2
        } else {
            3
        };
        let variants: [u8; 4] = match shape {
            0 => [16, 20, 24, 28],
            6 => [17, 21, 32, 32],
            8 => [18, 22, 32, 32],
            11 => [19, 23, 32, 32],
            7 => [32, 32, 25, 29],
            9 => [32, 32, 26, 30],
            10 => [32, 32, 27, 31],
            _ => [32; 4],
        };
        shape = variants[orientation];
    }
    format!("{image}{mode}{shape:03}")
}

/// `0x4385b2`: a broken wall's open ends become rubble (only concrete has
/// those frames; `0x438534` only breaks concrete). The original also turns
/// a lone pillar that was straight (6/7) at its previous update into 21/20,
/// which neighbour-by-neighbour updates never produce. `0x438773`/`0x437b84`:
/// at a third of its armour or less the wall is drawn from the `1xx`
/// damaged row.
fn wall_sprite(image: &str, shape: u8, broken: bool, damaged: bool) -> String {
    let shape = match (broken, shape) {
        (true, 8) => 19,
        (true, 9) => 18,
        (true, 10) => 16,
        (true, 11) => 17,
        _ => shape,
    };
    format!("{image}{}{shape:02}", u8::from(damaged))
}

/// `0x438552`: `armor <= max * 0.33`.
fn wall_damaged(armor: i64, max: i64) -> bool {
    armor as f64 <= max as f64 * 0.33
}

fn alpha_prefix(name: &str) -> String {
    name.chars().filter(char::is_ascii_alphabetic).collect()
}

/// Builds the scenario's draw list.
pub fn build(scenario: &Scenario, defs: &Definitions, art: &Art, options: &Options) -> Scene {
    let mut missing = Vec::new();
    let mut overlays: BTreeMap<Key, Vec<(Layer, Sheet, String)>> = BTreeMap::new();
    let mut parts: BTreeMap<Key, Vec<String>> = BTreeMap::new();
    let mut unit_parts: BTreeMap<Key, Vec<String>> = BTreeMap::new();

    if !options.terrain_only {
        if let Some(arrival) = options.arrival {
            let shown = scenario
                .units
                .iter()
                .enumerate()
                .filter(|&(i, u)| u.arrival == arrival && !options.hidden_units.contains(&i));
            for (_, unit) in shown {
                let (Some(kind), Some(frame)) =
                    (defs.unit(unit.kind), art.unit_frame(unit.kind, unit.owner))
                else {
                    missing.push(format!("unit type {}", unit.kind));
                    continue;
                };
                let in_forest = kind.width == 1
                    && kind.height == 1
                    && kind_at(scenario, defs, unit.x, unit.y) == Some(terrain_kind::TREE);
                if let Some(topper) = in_forest
                    .then(|| art.forest_topper(kind, unit.facing, frame))
                    .flatten()
                {
                    unit_parts.entry((unit.x, unit.y)).or_default().push(topper);
                    continue;
                }
                for (dx, dy) in footprint(kind.width, kind.height) {
                    unit_parts
                        .entry((unit.x + dx, unit.y + dy))
                        .or_default()
                        .push(kind.sprite(dx, dy, unit.facing, frame));
                }
            }
        }

        let roads = surface_index(&scenario.roads);
        for item in &scenario.roads {
            let Some(kind) = defs.roads.get(&item.kind) else {
                missing.push(format!("road type {}", item.kind));
                continue;
            };
            let shape = connection(&roads, item);
            let sprite = road_sprite(scenario, defs, &kind.image, shape, item);
            overlays
                .entry((item.x, item.y))
                .or_default()
                .push((Layer::Road, Sheet::Roads, sprite));
        }
        for item in &scenario.ruins {
            overlays.entry((item.x, item.y)).or_default().push((
                Layer::Decor,
                Sheet::Ruins,
                item.id.clone(),
            ));
        }
        let walls = surface_index(&scenario.walls);
        for item in &scenario.walls {
            let Some(kind) = defs.walls.get(&item.kind) else {
                missing.push(format!("wall type {}", item.kind));
                continue;
            };
            let sprite = wall_sprite(
                &kind.image,
                connection(&walls, item),
                item.broken,
                wall_damaged(item.armor, kind.armor),
            );
            overlays
                .entry((item.x, item.y))
                .or_default()
                .push((Layer::Wall, Sheet::Walls, sprite));
        }
        for item in &scenario.mines {
            if options.viewer.is_some_and(|p| item.hidden_to(p)) {
                continue;
            }
            let Some(kind) = defs.mines.get(&item.kind) else {
                missing.push(format!("mine type {}", item.kind));
                continue;
            };
            overlays.entry((item.x, item.y)).or_default().push((
                Layer::Mine,
                Sheet::Ub,
                format!("{}000", kind.image),
            ));
        }

        for building in &scenario.buildings {
            let (Some(kind), Some(&level)) = (
                defs.buildings.get(&building.kind),
                art.building_levels.get(&building.kind),
            ) else {
                missing.push(format!("building type {}", building.kind));
                continue;
            };
            for (dx, dy) in footprint(kind.width, kind.height) {
                parts
                    .entry((building.x + dx, building.y + dy))
                    .or_default()
                    .push(kind.sprite(dx, dy, kind.damaged(building.armor), level));
            }
        }
    }

    let size = scenario.size as i64;
    let mut keys: Vec<Key> = (0..size)
        .flat_map(|y| (0..size).map(move |x| (x, y)))
        .collect();
    keys.sort_by_key(|&(x, y)| (x + y, x));

    let mut zones: BTreeMap<Key, String> = if options.deployment && !options.terrain_only {
        deployment_sprites(scenario).into_iter().collect()
    } else {
        BTreeMap::new()
    };

    let mut items = Vec::new();
    let mut push = |layer, sheet, sprite: String, (x, y): Key, missing: &mut Vec<String>| {
        if !art.contains(sheet, &sprite) {
            missing.push(format!("{}:{sprite}", sheet.name()));
        }
        items.push(DrawItem {
            layer,
            sheet,
            sprite,
            x,
            y,
        });
    };
    for key in keys {
        let cell = scenario.cell(key.0, key.1).expect("key inside the map");
        let mut terrain = cell.terrain.clone();
        if options.terrain_only
            && let Some(t) = defs.terrain(&terrain)
            && (t.tree() || t.scenery())
        {
            let base = alpha_prefix(&terrain) + "000";
            terrain = if defs.terrain(&base).is_some_and(|t| t.paintable()) {
                base
            } else {
                options.ground.clone()
            };
        }
        push(Layer::Terrain, Sheet::Terrains, terrain, key, &mut missing);
        for (layer, sheet, sprite) in overlays.remove(&key).unwrap_or_default() {
            push(layer, sheet, sprite, key, &mut missing);
        }
        // Starting zones tint the ground but leave buildings and units on top.
        if let Some(sprite) = zones.remove(&key) {
            push(Layer::Deployment, Sheet::Symbols, sprite, key, &mut missing);
        }
        for sprite in parts.remove(&key).unwrap_or_default() {
            push(Layer::Building, Sheet::Ub, sprite, key, &mut missing);
        }
        for sprite in unit_parts.remove(&key).unwrap_or_default() {
            push(Layer::Unit, Sheet::Ub, sprite, key, &mut missing);
        }
    }

    missing.sort();
    missing.dedup();
    Scene {
        size: scenario.size,
        items,
        missing,
    }
}

/// Zone markers for `tiles` in deployment-zone style `DEPL{player}NN`,
/// edged where a neighbour is outside the zone.
pub fn zone_sprites(tiles: &BTreeSet<Key>, player: i64) -> Vec<(Key, String)> {
    tiles
        .iter()
        .map(|&(x, y)| {
            let mask = NEIGHBORS
                .iter()
                .filter(|(dx, dy, _)| tiles.contains(&(x + dx, y + dy)))
                .map(|(_, _, bit)| bit)
                .sum::<usize>();
            (
                (x, y),
                format!("DEPL{player}{:02}", DEPLOYMENT_SPRITES[mask]),
            )
        })
        .collect()
}

/// Starting-zone markers, sorted by tile `(x, y)`.
pub fn deployment_sprites(scenario: &Scenario) -> Vec<(Key, String)> {
    let mut zones: BTreeMap<Key, Vec<&Deployment>> = BTreeMap::new();
    for item in scenario.deployments.iter().filter(|d| d.is_start_zone()) {
        zones.entry((item.x, item.y)).or_default().push(item);
    }
    let owns = |key: Key, player: i64| {
        zones
            .get(&key)
            .is_some_and(|items| items.iter().any(|d| d.player == player))
    };
    zones
        .iter()
        .filter(|((x, y), _)| scenario.contains(*x, *y))
        .map(|(&(x, y), items)| {
            let item = items
                .iter()
                .min_by_key(|d| (d.player, d.counter))
                .expect("zones are non-empty");
            let mask = NEIGHBORS
                .iter()
                .filter(|(dx, dy, _)| owns((x + dx, y + dy), item.player))
                .map(|(_, _, bit)| bit)
                .sum::<usize>();
            (
                (x, y),
                format!("DEPL{}{:02}", item.player, DEPLOYMENT_SPRITES[mask]),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_sprites() {
        assert_eq!(wall_sprite("WALLC", 6, false, false), "WALLC006");
        assert_eq!(wall_sprite("WALLC", 8, true, false), "WALLC019");
        assert_eq!(wall_sprite("WALLC", 10, true, true), "WALLC116");
        assert_eq!(wall_sprite("WALLL", 9, false, true), "WALLL109");
        assert!(!wall_damaged(21, 60));
        assert!(wall_damaged(19, 60));
        assert!(wall_damaged(0, 60));
    }
}
