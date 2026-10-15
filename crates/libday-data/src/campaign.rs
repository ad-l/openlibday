//! Campaign definitions: continents, provinces and their scenarios.
//!
//! A [`Campaign`] is pure data. The original campaign is imported from
//! `COLONIES\COLONIES.ZIP` (`C1.mdb`..`C4.mdb`) and the province scenario
//! databases in `SCENS`; custom campaigns use the same model serialised as
//! JSON (see `docs/campaigns.md`).

use libday_formats::{Database, Zip};
use serde::{Deserialize, Serialize};

use crate::records::Records;
use crate::{DataError, Dir, GameData};

/// Version of the JSON campaign format written by [`Campaign::to_json`].
pub const FORMAT_VERSION: u32 = 1;

/// Continent databases of the original campaign, in play order.
pub const ORIGINAL_CONTINENTS: [&str; 4] = ["C1.mdb", "C2.mdb", "C3.mdb", "C4.mdb"];

/// Archive holding the original continent databases.
pub const COLONIES_ARCHIVE: &str = "COLONIES.ZIP";

/// Scenario variants a province may have; variant `v` lives in `SCENv.ZIP`.
pub const VARIANTS: [i64; 2] = [1, 2];

/// Text in the three languages of the original release.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Localized {
    pub en: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub de: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fr: String,
}

impl Localized {
    fn read(t: &Records, row: usize, prefix: &str, suffix: &str) -> Result<Self, DataError> {
        let column = |lang: &str| format!("{prefix}{lang}{suffix}");
        Ok(Self {
            en: t.text_or(row, &column("English"))?,
            de: t.text_or(row, &column("German"))?,
            fr: t.text_or(row, &column("French"))?,
        })
    }
}

/// Videos played outside a continent (`VIDEO\<name>.SMK`). The original
/// hard-codes these names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignVideos {
    /// Played when a counterattack is announced.
    pub counter_attack: String,
    /// Played when the last continent's capital falls.
    pub won: String,
    /// Played when the campaign is lost.
    pub lost: String,
}

impl Default for CampaignVideos {
    fn default() -> Self {
        Self {
            counter_attack: "CounterA".into(),
            won: "FH2End".into(),
            lost: "FH2Lose".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Campaign {
    pub format: u32,
    pub name: Localized,
    #[serde(default)]
    pub videos: CampaignVideos,
    pub continents: Vec<Continent>,
}

/// Province-map animation on the strategic screen (`COLANMS\<file>.SMK`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapAnimation {
    pub file: String,
    pub first_frame: i64,
    pub frames: i64,
}

/// A value per difficulty level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerDifficulty {
    pub easy: i64,
    pub average: i64,
    pub hard: i64,
}

/// Automated (building-produced) units the AI may call in during battles.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiAutomated {
    /// `AiAutoUnitChanceCall`.
    pub chance: i64,
    /// `AiAutoUnitRes`.
    pub resources: i64,
    /// `AI automated available` (automated enums).
    pub types: Vec<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Continent {
    /// Short identifier; the original uses the database stem (`C1`).
    pub id: String,
    pub name: Localized,
    /// `StartCash`: base-building budget when the continent starts.
    pub start_cash: i64,
    /// `PlayerCity`: the province holding the player's base.
    pub base_province: i64,
    /// `AlienCity`: capturing it completes the continent.
    pub capital_province: i64,
    /// Video before the continent (`Intro`); empty for none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub intro_video: String,
    /// `End`. The original names files that do not ship (`End1`, `End`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub end_video: String,
    pub map_animation: MapAnimation,
    /// `DeployEasy`/`DeployAvg`/`DeployHard`: percentage applied to campaign
    /// deployment points.
    pub deploy_percent: PerDifficulty,
    pub ai_automated: AiAutomated,
    /// `Buildings Available`: building types the player may construct.
    pub buildings: Vec<i64>,
    pub provinces: Vec<Province>,
}

impl Continent {
    pub fn province(&self, number: i64) -> Option<&Province> {
        self.provinces.iter().find(|p| p.number == number)
    }

    /// Provinces linked to `number` in either direction. The original
    /// tables are directed and not always symmetric.
    pub fn neighbours(&self, number: i64) -> Vec<i64> {
        let mut out: Vec<i64> = self
            .provinces
            .iter()
            .filter(|p| p.number == number || p.adjacent.contains(&number))
            .flat_map(|p| {
                if p.number == number {
                    p.adjacent.clone()
                } else {
                    vec![p.number]
                }
            })
            .filter(|&n| n != number)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Province {
    pub number: i64,
    pub name: Localized,
    pub title: Localized,
    pub description: Localized,
    pub objective: Localized,
    pub map_size: i64,
    pub terrain: i64,
    /// `Province List.Credits`.
    pub credits: i64,
    /// `Province List.Tech`.
    pub tech: i64,
    /// `Adjacent Provinces` rows starting at this province.
    pub adjacent: Vec<i64>,
    /// `Province List Captured.Captured` at the start of the continent.
    pub captured: bool,
    pub scenarios: Vec<ProvinceScenario>,
}

/// One playable variant of a province.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvinceScenario {
    pub variant: i64,
    /// Scenario database name (`s1-2-1.mdb`).
    pub file: String,
    /// `Scenario Info.CounterAttack`: winning this map triggers a
    /// counterattack on the base.
    pub counter_attack: bool,
    /// `Scenario Info.Credits`.
    pub credits: i64,
    /// `Counter Attack` table: the attacking roster.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub counter_attack_units: Vec<CounterAttackUnit>,
}

/// A `Counter Attack` row: an AI unit and the turn it arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterAttackUnit {
    #[serde(rename = "type")]
    pub unit_type: i64,
    pub turn: i64,
}

/// `Scenario Info` fields and the `Counter Attack` roster of a scenario.
pub fn counter_attack_info(
    db: &Database,
) -> Result<(bool, i64, Vec<CounterAttackUnit>), DataError> {
    let info = Records::read(db, "Scenario Info")?;
    let (flag, credits) = if info.is_empty() {
        (false, 0)
    } else {
        (
            info.flag_or(0, "CounterAttack", false)?,
            info.int_or(0, "Credits", 0)?,
        )
    };
    let t = Records::read(db, "Counter Attack")?;
    let mut rows = Vec::with_capacity(t.len());
    for r in 0..t.len() {
        rows.push((
            t.int_or(r, "Counter", r as i64)?,
            CounterAttackUnit {
                unit_type: t.int(r, "type")?,
                turn: t.int_or(r, "turn", 0)?,
            },
        ));
    }
    rows.sort_by_key(|&(counter, _)| counter);
    Ok((flag, credits, rows.into_iter().map(|(_, u)| u).collect()))
}

/// Scenario database name of province `province` of continent `continent`
/// (0-based), variant `variant`: the original's `s%d-%d-%d.mdb`.
pub fn scenario_file(continent: usize, province: i64, variant: i64) -> String {
    format!("s{}-{province}-{variant}.mdb", continent + 1)
}

impl Campaign {
    pub fn from_json(text: &str) -> Result<Self, DataError> {
        let campaign: Self = serde_json::from_str(text).map_err(|e| DataError::Invalid {
            table: "campaign".into(),
            message: e.to_string(),
        })?;
        campaign.validate()?;
        Ok(campaign)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("campaign serialises")
    }

    /// Checks the references between continents, provinces and scenarios.
    pub fn validate(&self) -> Result<(), DataError> {
        let fail = |message: String| {
            Err(DataError::Invalid {
                table: "campaign".into(),
                message,
            })
        };
        if self.format != FORMAT_VERSION {
            return fail(format!("unsupported format {}", self.format));
        }
        if self.continents.is_empty() {
            return fail("no continents".into());
        }
        for c in &self.continents {
            for (what, number) in [("base", c.base_province), ("capital", c.capital_province)] {
                if c.province(number).is_none() {
                    return fail(format!("{}: {what} province {number} missing", c.id));
                }
            }
            for p in &c.provinces {
                if c.provinces.iter().filter(|q| q.number == p.number).count() > 1 {
                    return fail(format!("{}: duplicate province {}", c.id, p.number));
                }
                if let Some(n) = p.adjacent.iter().find(|&&n| c.province(n).is_none()) {
                    return fail(format!(
                        "{}: province {} adjacent to missing {n}",
                        c.id, p.number
                    ));
                }
                if p.scenarios.is_empty() {
                    return fail(format!("{}: province {} has no scenario", c.id, p.number));
                }
            }
        }
        Ok(())
    }

    /// Imports the original campaign.
    pub fn import_original(data: &GameData) -> Result<Self, DataError> {
        let available = data.archived_scenarios()?;
        let exists = |name: &str| {
            available.iter().any(|n| n.eq_ignore_ascii_case(name))
                || data.path(Dir::Scenarios, name).is_ok()
        };
        let archive = data.read(Dir::Colonies, COLONIES_ARCHIVE).ok();
        let mut continents = Vec::new();
        for (index, name) in ORIGINAL_CONTINENTS.iter().enumerate() {
            let bytes = match data.read(Dir::Colonies, name) {
                Ok(bytes) => bytes,
                Err(e) => {
                    let Some(zip) = archive.as_deref() else {
                        return Err(e);
                    };
                    Zip::parse(zip)
                        .and_then(|z| z.read(name))
                        .map_err(|e| DataError::Archive(COLONIES_ARCHIVE.into(), e))?
                }
            };
            let db = Database::open(bytes).map_err(|e| DataError::Database((*name).into(), e))?;
            let mut continent = load_continent(&db, name)?;
            for province in &mut continent.provinces {
                for variant in VARIANTS {
                    let file = scenario_file(index, province.number, variant);
                    if !exists(&file) {
                        continue;
                    }
                    let (counter_attack, credits, counter_attack_units) =
                        counter_attack_info(&data.scenario(&file)?)?;
                    province.scenarios.push(ProvinceScenario {
                        variant,
                        file,
                        counter_attack,
                        credits,
                        counter_attack_units,
                    });
                }
            }
            continents.push(continent);
        }
        let campaign = Self {
            format: FORMAT_VERSION,
            name: Localized {
                en: "Liberation Day".into(),
                ..Localized::default()
            },
            videos: CampaignVideos::default(),
            continents,
        };
        campaign.validate()?;
        Ok(campaign)
    }
}

fn ints(db: &Database, table: &str, column: &str) -> Result<Vec<i64>, DataError> {
    let t = Records::read(db, table)?;
    let mut rows = Vec::with_capacity(t.len());
    for r in 0..t.len() {
        rows.push((t.int_or(r, "Counter", r as i64)?, t.int(r, column)?));
    }
    rows.sort_unstable();
    Ok(rows.into_iter().map(|(_, v)| v).collect())
}

/// Reads one continent database (`Cn.mdb`); scenarios are left empty.
pub fn load_continent(db: &Database, name: &str) -> Result<Continent, DataError> {
    let info = Records::read(db, "Continent Info")?;
    if info.is_empty() {
        return Err(DataError::Invalid {
            table: format!("{name}: Continent Info"),
            message: "no rows".into(),
        });
    }
    let id = name
        .rsplit_once('.')
        .map_or(name, |(stem, _)| stem)
        .to_ascii_uppercase();

    let adjacency = Records::read(db, "Adjacent Provinces")?;
    let mut edges = Vec::with_capacity(adjacency.len());
    for r in 0..adjacency.len() {
        edges.push((
            adjacency.int_or(r, "Counter", r as i64)?,
            adjacency.int(r, "ProvinceNo")?,
            adjacency.int(r, "AdjacenTo")?,
        ));
    }
    edges.sort_unstable();

    let captured = Records::read(db, "Province List Captured")?;
    let mut captured_set = Vec::new();
    for r in 0..captured.len() {
        if captured.flag_or(r, "Captured", false)? {
            captured_set.push(captured.int(r, "NoProvince")?);
        }
    }

    let t = Records::read(db, "Province List")?;
    let mut provinces = Vec::with_capacity(t.len());
    for r in 0..t.len() {
        let number = t.int(r, "NoProvince")?;
        let mut adjacent = Vec::new();
        for &(_, from, to) in &edges {
            if from == number && !adjacent.contains(&to) {
                adjacent.push(to);
            }
        }
        provinces.push(Province {
            number,
            name: Localized::read(&t, r, "ProvinceName", "")?,
            title: Localized::read(&t, r, "Title", "")?,
            description: Localized::read(&t, r, "Description", "")?,
            objective: Localized::read(&t, r, "Objective", "")?,
            map_size: t.int_or(r, "MapSize", 0)?,
            terrain: t.int_or(r, "TerrainType", 0)?,
            credits: t.int_or(r, "Credits", 0)?,
            tech: t.int_or(r, "Tech", 0)?,
            adjacent,
            captured: captured_set.contains(&number),
            scenarios: Vec::new(),
        });
    }
    provinces.sort_by_key(|p| p.number);

    Ok(Continent {
        id,
        name: Localized::read(&info, 0, "Name", "")?,
        start_cash: info.int_or(0, "StartCash", 0)?,
        base_province: info.int_or(0, "PlayerCity", 1)?,
        capital_province: info.int_or(0, "AlienCity", 0)?,
        intro_video: info.text_or(0, "Intro")?,
        end_video: info.text_or(0, "End")?,
        map_animation: MapAnimation {
            file: info.text_or(0, "SmackProvinces")?,
            first_frame: info.int_or(0, "SmackProvFrameBegin", 0)?,
            frames: info.int_or(0, "SmackProvFrameQty", 0)?,
        },
        deploy_percent: PerDifficulty {
            easy: info.int_or(0, "DeployEasy", 100)?,
            average: info.int_or(0, "DeployAvg", 100)?,
            hard: info.int_or(0, "DeployHard", 100)?,
        },
        ai_automated: AiAutomated {
            chance: info.int_or(0, "AiAutoUnitChanceCall", 0)?,
            resources: info.int_or(0, "AiAutoUnitRes", 0)?,
            types: ints(db, "AI automated available", "Enum")?,
        },
        buildings: ints(db, "Buildings Available", "buildingType")?,
        provinces,
    })
}

impl GameData {
    /// The original campaign (see [`Campaign::import_original`]).
    pub fn original_campaign(&self) -> Result<Campaign, DataError> {
        Campaign::import_original(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn province(number: i64, adjacent: Vec<i64>) -> Province {
        Province {
            number,
            name: Localized::default(),
            title: Localized::default(),
            description: Localized::default(),
            objective: Localized::default(),
            map_size: 24,
            terrain: 0,
            credits: 0,
            tech: 0,
            adjacent,
            captured: number == 1,
            scenarios: vec![ProvinceScenario {
                variant: 1,
                file: scenario_file(0, number, 1),
                counter_attack: false,
                credits: 0,
                counter_attack_units: Vec::new(),
            }],
        }
    }

    fn sample() -> Campaign {
        Campaign {
            format: FORMAT_VERSION,
            name: Localized {
                en: "Test".into(),
                ..Localized::default()
            },
            videos: CampaignVideos::default(),
            continents: vec![Continent {
                id: "C1".into(),
                name: Localized::default(),
                start_cash: 100,
                base_province: 1,
                capital_province: 3,
                intro_video: String::new(),
                end_video: String::new(),
                map_animation: MapAnimation::default(),
                deploy_percent: PerDifficulty::default(),
                ai_automated: AiAutomated::default(),
                buildings: vec![100],
                provinces: vec![
                    province(1, vec![2]),
                    province(2, vec![]),
                    province(3, vec![2]),
                ],
            }],
        }
    }

    #[test]
    fn neighbours_are_undirected() {
        let c = &sample().continents[0];
        assert_eq!(c.neighbours(2), vec![1, 3]);
        assert_eq!(c.neighbours(1), vec![2]);
    }

    #[test]
    fn json_round_trip() {
        let campaign = sample();
        assert_eq!(Campaign::from_json(&campaign.to_json()).unwrap(), campaign);
    }

    #[test]
    fn validation_rejects_dangling_references() {
        let mut campaign = sample();
        campaign.continents[0].capital_province = 9;
        assert!(campaign.validate().is_err());
        let mut campaign = sample();
        campaign.continents[0].provinces[1].adjacent.push(7);
        assert!(campaign.validate().is_err());
        let mut campaign = sample();
        campaign.continents[0].provinces[0].scenarios.clear();
        assert!(campaign.validate().is_err());
    }

    #[test]
    fn scenario_names() {
        assert_eq!(scenario_file(0, 2, 1), "s1-2-1.mdb");
        assert_eq!(scenario_file(3, 20, 2), "s4-20-2.mdb");
    }
}
