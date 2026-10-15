//! Right-button inspection: what the info panel over the minimap shows for
//! the map tile under the cursor (`0x455845`). The tile's unit wins, then
//! its building, wall and road (`0x43ec38` collects them); bare terrain
//! shows nothing. Mines the player can see are shown too, ahead of the road
//! under them (an openlibday addition: the original shows only the road).
//!
//! Buildings (`0x455fac`), walls (`0x456132`) and roads (`0x45624b`) share
//! one card: the `LISTBOX.DBI` portrait, the name and `AR` beside it (a
//! building's or wall's current armour, a road type's `ar`), the owner
//! (`0x455ee1`: string 65 "Alien" for owner 0) below,
//! and for a building with a `TechReward` a `TECH` line (`0x4560a2`, the
//! type's reward whether or not it was captured). A mine's card has its
//! `listboxImageID` portrait (`{ImageID}999`), its name and owner; mines
//! have no armour.

use libday_data::Definitions;
use libday_data::scenario::Scenario;

use crate::minimap::player_name;

/// What the right button holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Index into `scenario.units`.
    Unit(usize),
    Building(usize),
    Wall(usize),
    Mine(usize),
    Road(usize),
}

impl Target {
    pub fn unit(self) -> Option<usize> {
        match self {
            Target::Unit(i) => Some(i),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Target::Unit(_) => "unit",
            Target::Building(_) => "building",
            Target::Wall(_) => "wall",
            Target::Mine(_) => "mine",
            Target::Road(_) => "road",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Target::Unit(i)
            | Target::Building(i)
            | Target::Wall(i)
            | Target::Mine(i)
            | Target::Road(i) => i,
        }
    }
}

/// The building, wall, mine or road on tile `(x, y)`, in the original's
/// order (the unit is looked up by the caller, which knows which are
/// shown). Mines hidden from `player` (see `Mine::hidden_to`) are passed
/// over; `None` shows them all, as outside a battle.
pub fn structure_at(
    scenario: &Scenario,
    defs: &Definitions,
    x: i64,
    y: i64,
    player: Option<i64>,
) -> Option<Target> {
    let building = scenario.buildings.iter().position(|b| {
        let (w, h) = defs
            .buildings
            .get(&b.kind)
            .map_or((1, 1), |k| (k.width, k.height));
        (b.x..b.x + w).contains(&x) && (b.y..b.y + h).contains(&y)
    });
    let at = |s: &libday_data::scenario::Surface| (s.x, s.y) == (x, y);
    building
        .map(Target::Building)
        .or_else(|| scenario.walls.iter().position(at).map(Target::Wall))
        .or_else(|| {
            scenario
                .mines
                .iter()
                .position(|m| (m.x, m.y) == (x, y) && !player.is_some_and(|p| m.hidden_to(p)))
                .map(Target::Mine)
        })
        .or_else(|| scenario.roads.iter().position(at).map(Target::Road))
}

/// A building, wall or road's info: its portrait (`{image}999` in
/// `LISTBOX.DBI`) and text rows, the first two beside the portrait and the
/// rest below it (row 2 is left empty, as in the original).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub image: String,
    pub lines: Vec<String>,
}

pub fn card(scenario: &Scenario, defs: &Definitions, target: Target) -> Option<Card> {
    let (image, name, armor, owner, tech) = match target {
        Target::Unit(_) => return None,
        Target::Mine(i) => {
            let m = scenario.mines.get(i)?;
            let k = defs.mines.get(&m.kind)?;
            return Some(Card {
                image: k.image.to_ascii_uppercase(),
                lines: vec![
                    k.name.clone(),
                    String::new(),
                    String::new(),
                    player_name(m.owner),
                ],
            });
        }
        Target::Building(i) => {
            let b = scenario.buildings.get(i)?;
            let k = defs.buildings.get(&b.kind)?;
            (&k.image, &k.name, b.armor, b.owner, k.tech_reward)
        }
        Target::Wall(i) | Target::Road(i) => {
            let (surfaces, types) = if matches!(target, Target::Wall(_)) {
                (&scenario.walls, &defs.walls)
            } else {
                (&scenario.roads, &defs.roads)
            };
            let s = surfaces.get(i)?;
            let k = types.get(&s.kind)?;
            // A wall shows what is left of its armour; roads have none of
            // their own and show their type's (`0x42efe0`).
            let armor = if matches!(target, Target::Wall(_)) {
                s.armor
            } else {
                k.armor
            };
            (&k.image, &k.name, armor, s.owner, 0)
        }
    };
    let mut lines = vec![
        name.clone(),
        format!("AR {armor}"),
        String::new(),
        player_name(owner),
    ];
    if tech > 0 {
        lines.push(format!("TECH {tech}"));
    }
    Some(Card {
        image: image.to_ascii_uppercase(),
        lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use libday_data::defs::{BuildingType, MineType, SurfaceType};
    use libday_data::scenario::{Building, Mine, ScenarioInfo, Surface};

    fn surface(kind: i64, owner: i64, x: i64, y: i64, armor: i64) -> Surface {
        Surface {
            counter: 0,
            kind,
            owner,
            x,
            y,
            armor,
            broken: false,
        }
    }

    fn fixture() -> (Scenario, Definitions) {
        let mut defs = Definitions::default();
        defs.buildings.insert(
            7,
            BuildingType {
                id: 7,
                name: "Tech Center".into(),
                width: 2,
                height: 2,
                armor: 75,
                image: "TMTC".into(),
                tech_reward: 500,
                ..Default::default()
            },
        );
        defs.buildings.insert(
            8,
            BuildingType {
                id: 8,
                name: "Barracks".into(),
                image: "Hbar".into(),
                ..Default::default()
            },
        );
        for (table, id, name, image) in [
            (&mut defs.walls, 0, "Concrete", "WALLC"),
            (&mut defs.roads, 1, "Paved Road", "ROAH"),
        ] {
            table.insert(
                id,
                SurfaceType {
                    id,
                    name: name.into(),
                    armor: 60,
                    image: image.into(),
                },
            );
        }
        defs.mines.insert(
            1,
            MineType {
                id: 1,
                name: "Grav Mine".into(),
                image: "GMINE".into(),
                damage_base: 25,
                damage_random: 10,
                damage_type: 0,
            },
        );
        let mut s = Scenario {
            size: 16,
            cells: Vec::new(),
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
        };
        let building = |kind, owner, x, y, armor| Building {
            counter: 0,
            id: 0,
            kind,
            owner,
            armor,
            x,
            y,
            ai_target: false,
        };
        s.buildings = vec![building(7, 0, 4, 4, 75), building(8, 2, 0, 0, 30)];
        // A wall and a road under the tech centre's footprint never show.
        s.walls = vec![surface(0, 0, 5, 5, 60), surface(0, 1, 8, 8, 40)];
        s.roads = vec![
            surface(1, 0, 5, 4, 10),
            surface(1, 0, 8, 8, 10),
            surface(1, 3, 9, 9, 7),
        ];
        // A mine on the road at 9,9, hidden from player 1.
        s.mines = vec![Mine {
            counter: 0,
            kind: 1,
            owner: 2,
            x: 9,
            y: 9,
            armed: true,
            hidden_ai: false,
            hidden: [true, false, false, false],
        }];
        (s, defs)
    }

    #[test]
    fn building_beats_wall_beats_road() {
        let (s, defs) = fixture();
        let at = |x, y| structure_at(&s, &defs, x, y, Some(1));
        assert_eq!(at(5, 5), Some(Target::Building(0)));
        assert_eq!(at(4, 5), Some(Target::Building(0)));
        assert_eq!(at(8, 8), Some(Target::Wall(1)));
        assert_eq!(at(9, 9), Some(Target::Road(2)));
        assert_eq!(at(6, 6), None);
    }

    #[test]
    fn a_seen_mine_beats_the_road_under_it() {
        let (s, defs) = fixture();
        let at = |p| structure_at(&s, &defs, 9, 9, p);
        assert_eq!(at(Some(2)), Some(Target::Mine(0)));
        assert_eq!(at(None), Some(Target::Mine(0)));
        assert_eq!(
            card(&s, &defs, Target::Mine(0)),
            Some(Card {
                image: "GMINE".into(),
                lines: vec![
                    "Grav Mine".into(),
                    String::new(),
                    String::new(),
                    "Player 2".into()
                ],
            })
        );
    }

    #[test]
    fn cards_match_the_original() {
        let (s, defs) = fixture();
        let lines = |t| card(&s, &defs, t).unwrap().lines;
        assert_eq!(
            card(&s, &defs, Target::Building(0)),
            Some(Card {
                image: "TMTC".into(),
                lines: vec![
                    "Tech Center".into(),
                    "AR 75".into(),
                    String::new(),
                    "Alien".into(),
                    "TECH 500".into()
                ],
            })
        );
        assert_eq!(card(&s, &defs, Target::Building(1)).unwrap().image, "HBAR");
        assert_eq!(lines(Target::Building(1))[1..], ["AR 30", "", "Player 2"]);
        assert_eq!(
            lines(Target::Wall(1)),
            ["Concrete", "AR 40", "", "Player 1"]
        );
        assert_eq!(
            lines(Target::Road(2)),
            ["Paved Road", "AR 60", "", "Player 3"]
        );
        assert_eq!(card(&s, &defs, Target::Unit(0)), None);
        assert_eq!(card(&s, &defs, Target::Wall(9)), None);
    }
}
