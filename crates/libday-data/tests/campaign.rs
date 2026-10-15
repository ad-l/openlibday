//! Data-gated checks of the original campaign import.

use libday_data::GameData;
use libday_data::campaign::{Campaign, CounterAttackUnit};

fn game() -> Option<GameData> {
    match GameData::from_env() {
        Ok(game) => Some(game),
        Err(err) => {
            eprintln!("skipping: {err}");
            None
        }
    }
}

#[test]
fn original_campaign_imports() {
    let Some(game) = game() else { return };
    let campaign = game.original_campaign().unwrap();
    let ids: Vec<&str> = campaign.continents.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["C1", "C2", "C3", "C4"]);
    let sizes: Vec<usize> = campaign
        .continents
        .iter()
        .map(|c| c.provinces.len())
        .collect();
    assert_eq!(sizes, [7, 12, 15, 20]);
    let cash: Vec<i64> = campaign.continents.iter().map(|c| c.start_cash).collect();
    assert_eq!(cash, [625, 1600, 6000, 12000]);

    let c1 = &campaign.continents[0];
    assert_eq!(c1.name.en, "Ghoikhi-Bhaak");
    assert_eq!((c1.base_province, c1.capital_province), (1, 7));
    assert_eq!(c1.intro_video, "Intro1");
    assert_eq!(c1.map_animation.file, "C1");
    assert_eq!(
        (
            c1.deploy_percent.easy,
            c1.deploy_percent.average,
            c1.deploy_percent.hard
        ),
        (130, 70, 70)
    );
    assert_eq!(c1.ai_automated.types, [14, 7]);
    assert_eq!(c1.buildings.len(), 10);
    assert_eq!(c1.neighbours(1), [3, 4, 5]);
    assert_eq!(c1.neighbours(7), [6]);
    assert!(c1.province(1).unwrap().captured);
    assert!(c1.provinces[1..].iter().all(|p| !p.captured));

    let base = c1.province(1).unwrap();
    assert_eq!(base.scenarios.len(), 1);
    assert_eq!(base.scenarios[0].file, "s1-1-1.mdb");
    assert!(!base.scenarios[0].counter_attack);

    let eagle = c1.province(2).unwrap();
    assert_eq!((eagle.credits, eagle.tech), (400, 250));
    let variants: Vec<(&str, bool)> = eagle
        .scenarios
        .iter()
        .map(|s| (s.file.as_str(), s.counter_attack))
        .collect();
    assert_eq!(variants, [("s1-2-1.mdb", true), ("s1-2-2.mdb", false)]);
    let roster = &eagle.scenarios[0].counter_attack_units;
    assert_eq!(roster.len(), 9);
    assert_eq!(
        roster
            .iter()
            .filter(|u| **u
                == CounterAttackUnit {
                    unit_type: 10,
                    turn: 1
                })
            .count(),
        2
    );
    assert_eq!(eagle.scenarios[0].credits, 550);

    let every_province_playable = campaign
        .continents
        .iter()
        .flat_map(|c| &c.provinces)
        .all(|p| !p.scenarios.is_empty());
    assert!(every_province_playable);

    assert_eq!(Campaign::from_json(&campaign.to_json()).unwrap(), campaign);
}
