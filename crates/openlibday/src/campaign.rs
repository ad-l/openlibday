//! Campaign mode: the campaign's state and save, and the battles fought on
//! the player's base (the first city attack and counterattacks).

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;

use libday_data::Battle;
use libday_data::campaign::{Campaign, Continent, CounterAttackUnit, scenario_file};
use libday_data::objective::Outcome;
use libday_data::scenario::{Building, Deployment, Mine, Order, Scenario, Surface, Unit};
use libday_view::movement::{self, Access};
use libday_view::ui::Screen;
use serde::{Deserialize, Serialize};

use crate::Viewer;
use crate::skirmish::{AI, PLAYER, Phase, Setup, skirmish_players};

/// Technology points for repelling the first city attack. *Unverified*: no
/// such constant was found in the data or the executable, but the original
/// grants this many after the first battle.
pub const FIRST_ATTACK_TECH: i64 = 150;
/// The campaign save in the saves directory.
const SAVE: &str = "campaign.json";
const FORMAT: &str = "openlibday-campaign";
const VERSION: u32 = 1;
/// Headquarters building type.
pub const HQ: i64 = 130;
/// Deployment points per owned building (`0x47D4E7`): the HQ and Deploy
/// Mk1-4.
const DEPLOY_POINTS: [(i64, i64); 5] = [(HQ, 50), (131, 100), (173, 110), (174, 120), (175, 130)];
/// Player zone: tiles within this many of an owned building (`0x47F8E7`).
const ZONE_RADIUS: i64 = 3;
/// AI zone: tiles this far from the player's zone (`0x47C350`).
const AI_BAND: std::ops::RangeInclusive<i64> = 11..=14;
/// Hardest difficulty.
const DIFFICULTY_MAX: i64 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum Stage {
    /// The base defends itself at the start of the campaign (state 1).
    FirstAttack,
    /// Province map, base building and army (state 3).
    Strategic,
    /// Attacking a province (state 4).
    Tactic { province: i64 },
    /// The base defends against the province's counterattack (state 6).
    CounterAttack { province: i64 },
    /// The last capital fell.
    Won,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvinceState {
    pub number: i64,
    pub captured: bool,
    /// The scenario variant played next (`RandomChosen`).
    pub variant: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainChange {
    pub x: i64,
    pub y: i64,
    pub terrain: String,
}

/// The base as built and damaged, over the base map's scenario.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseLayout {
    pub buildings: Vec<Building>,
    pub roads: Vec<Surface>,
    pub walls: Vec<Surface>,
    pub mines: Vec<Mine>,
    pub terrain: Vec<TerrainChange>,
}

impl BaseLayout {
    /// What `now` holds over the shipped base map `base`.
    pub fn capture(now: &Scenario, base: &Scenario) -> Self {
        let terrain = now
            .cells
            .iter()
            .zip(&base.cells)
            .enumerate()
            .filter(|(_, (a, b))| a.terrain != b.terrain)
            .map(|(i, (a, _))| TerrainChange {
                x: (i % now.size) as i64,
                y: (i / now.size) as i64,
                terrain: a.terrain.clone(),
            })
            .collect();
        Self {
            buildings: now.buildings.clone(),
            roads: now.roads.clone(),
            walls: now.walls.clone(),
            mines: now.mines.clone(),
            terrain,
        }
    }

    pub fn apply(&self, s: &mut Scenario) {
        s.buildings = self.buildings.clone();
        s.roads = self.roads.clone();
        s.walls = self.walls.clone();
        s.mines = self.mines.clone();
        for t in &self.terrain {
            if s.contains(t.x, t.y) {
                let i = t.y as usize * s.size + t.x as usize;
                s.cells[i].terrain = t.terrain.clone();
            }
        }
    }
}

/// A campaign in progress (the original's `Player Campaign` and
/// `Server Mode Campaign`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub format: String,
    pub version: u32,
    pub name: String,
    pub difficulty: i64,
    /// Index of the continent being played.
    pub continent: usize,
    /// Base-building budget.
    pub credits: i64,
    /// Technology points to spend in ARMY.
    pub tech: i64,
    /// Technology level per unit type.
    pub unit_tech: BTreeMap<i64, i64>,
    pub provinces: Vec<ProvinceState>,
    /// `None` until the base first changes from the shipped map.
    pub base: Option<BaseLayout>,
    pub stage: Stage,
}

impl State {
    /// A new campaign on the first continent; `roll` is the game's `rand()`.
    pub fn new(
        campaign: &Campaign,
        name: &str,
        difficulty: i64,
        roll: impl FnMut() -> i64,
    ) -> Self {
        let mut state = Self {
            format: FORMAT.into(),
            version: VERSION,
            name: name.into(),
            difficulty,
            continent: 0,
            credits: 0,
            tech: 0,
            unit_tech: BTreeMap::new(),
            provinces: Vec::new(),
            base: None,
            stage: Stage::FirstAttack,
        };
        state.enter_continent(campaign, 0, roll);
        state
    }

    /// Starts continent `index` (`0x47E269`): its start cash replaces the
    /// credits, each province gets a random variant, the base variant 1.
    pub fn enter_continent(
        &mut self,
        campaign: &Campaign,
        index: usize,
        mut roll: impl FnMut() -> i64,
    ) {
        let Some(c) = campaign.continents.get(index) else {
            return;
        };
        self.continent = index;
        self.credits = c.start_cash;
        self.base = None;
        self.provinces = c
            .provinces
            .iter()
            .map(|p| {
                let n = p.scenarios.len().max(1) as i64;
                let first = p.scenarios.first().map_or(1, |s| s.variant);
                ProvinceState {
                    number: p.number,
                    captured: p.captured || p.number == c.base_province,
                    variant: if p.number == c.base_province {
                        first
                    } else {
                        p.scenarios
                            .get((roll() % n) as usize)
                            .map_or(first, |s| s.variant)
                    },
                }
            })
            .collect();
    }

    pub fn province(&self, number: i64) -> Option<&ProvinceState> {
        self.provinces.iter().find(|p| p.number == number)
    }

    #[allow(dead_code, reason = "used by the continent overview")]
    pub fn province_mut(&mut self, number: i64) -> Option<&mut ProvinceState> {
        self.provinces.iter_mut().find(|p| p.number == number)
    }

    /// Deployment points percentage for the difficulty (`0x47AA4D`).
    pub fn deploy_percent(&self, c: &Continent) -> i64 {
        let (easy, hard) = (c.deploy_percent.easy, c.deploy_percent.hard);
        easy + (hard - easy) * self.difficulty.clamp(0, DIFFICULTY_MAX) / DIFFICULTY_MAX
    }

    /// Credits for capturing a province (`0x415653`): 125 % on the easiest
    /// difficulty down to 75 % on the hardest.
    #[allow(dead_code, reason = "used by the continent overview")]
    pub fn reward(&self, credits: i64) -> i64 {
        let pct = 125 - 5 * self.difficulty.clamp(0, DIFFICULTY_MAX);
        (credits as f64 * pct as f64 / 100.0).round() as i64
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let state: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if state.format != FORMAT || state.version == 0 || state.version > VERSION {
            return Err(format!(
                "unsupported campaign save {} v{}",
                state.format, state.version
            ));
        }
        Ok(state)
    }
}

/// A loaded campaign: its data and state.
pub struct Session {
    pub data: Campaign,
    pub state: State,
}

impl Session {
    pub fn continent(&self) -> Option<&Continent> {
        self.data.continents.get(self.state.continent)
    }

    /// The scenario of province `number` that is played next.
    #[allow(dead_code, reason = "used by the continent overview")]
    pub fn scenario_of(&self, number: i64) -> Option<String> {
        let variant = self.state.province(number)?.variant;
        let c = self.continent()?;
        let p = c.province(number)?;
        Some(
            p.scenarios
                .iter()
                .find(|s| s.variant == variant)
                .map_or_else(
                    || scenario_file(self.state.continent, number, variant),
                    |s| s.file.clone(),
                ),
        )
    }

    /// The base map's scenario.
    pub fn base_file(&self) -> Option<String> {
        let c = self.continent()?;
        let p = c.province(c.base_province)?;
        p.scenarios.first().map(|s| s.file.clone())
    }

    /// Who attacks the base: the base map's own roster on the first attack,
    /// else the won province's.
    fn roster(&self) -> Vec<CounterAttackUnit> {
        let Some(c) = self.continent() else {
            return Vec::new();
        };
        let (number, variant) = match self.state.stage {
            Stage::CounterAttack { province } => (
                province,
                self.state.province(province).map_or(1, |p| p.variant),
            ),
            _ => (c.base_province, 1),
        };
        c.province(number)
            .and_then(|p| p.scenarios.iter().find(|s| s.variant == variant))
            .map(|s| s.counter_attack_units.clone())
            .unwrap_or_default()
    }
}

/// Tiles within `radius` (Chebyshev) of `zone`, with their distance.
fn distances(size: i64, zone: &[(i64, i64)]) -> Vec<i64> {
    let mut dist = vec![i64::MAX; (size * size) as usize];
    let mut queue = VecDeque::new();
    for &(x, y) in zone {
        dist[(y * size + x) as usize] = 0;
        queue.push_back((x, y));
    }
    while let Some((x, y)) = queue.pop_front() {
        let d = dist[(y * size + x) as usize];
        for (dx, dy) in libday_view::scatter::OFFSETS {
            let (nx, ny) = (x + dx, y + dy);
            if (0..size).contains(&nx) && (0..size).contains(&ny) {
                let i = (ny * size + nx) as usize;
                if dist[i] == i64::MAX {
                    dist[i] = d + 1;
                    queue.push_back((nx, ny));
                }
            }
        }
    }
    dist
}

impl Viewer {
    fn campaign_save_path(&self) -> Option<PathBuf> {
        self.saves.as_ref().map(|d| d.join(SAVE))
    }

    /// Whether a campaign save exists for LOAD CAMPAIGN.
    pub(crate) fn campaign_saved(&self) -> bool {
        self.campaign_save_path().is_some_and(|p| p.is_file())
    }

    pub(crate) fn save_campaign(&mut self) {
        let (Some(path), Some(session)) = (self.campaign_save_path(), &self.campaign) else {
            return;
        };
        let json = session.state.to_json();
        let result = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, json));
        if let Err(e) = result {
            self.error = Some(format!("campaign save failed: {e}"));
        }
    }

    fn delete_campaign_save(&mut self) {
        if let Some(path) = self.campaign_save_path() {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Starts a new campaign for the NEW GAME dialog's name and difficulty.
    pub(crate) fn new_campaign(&mut self, name: &str, difficulty: i64) {
        let data = match self.game.original_campaign() {
            Ok(data) => data,
            Err(e) => {
                self.error = Some(format!("campaign: {e}"));
                return;
            }
        };
        let rng = &mut self.rng;
        let state = State::new(&data, name, difficulty, || rng.rand());
        self.campaign = Some(Session { data, state });
        self.save_campaign();
        self.begin_base_battle();
    }

    /// Resumes the saved campaign.
    pub(crate) fn load_campaign(&mut self) {
        let result = self
            .campaign_save_path()
            .ok_or_else(|| "no saves directory".to_string())
            .and_then(|p| std::fs::read_to_string(p).map_err(|e| e.to_string()))
            .and_then(|text| State::from_json(&text))
            .and_then(|state| {
                self.game
                    .original_campaign()
                    .map(|data| Session { data, state })
                    .map_err(|e| e.to_string())
            });
        match result {
            Ok(session) => {
                self.campaign = Some(session);
                self.resume_campaign();
            }
            Err(e) => self.error = Some(format!("load campaign failed: {e}")),
        }
    }

    /// Enters the screen of the campaign's stage.
    pub(crate) fn resume_campaign(&mut self) {
        let Some(stage) = self.campaign.as_ref().map(|c| c.state.stage) else {
            return;
        };
        match stage {
            Stage::FirstAttack | Stage::CounterAttack { .. } => self.begin_base_battle(),
            Stage::Tactic { .. } | Stage::Strategic => self.open_strategic(),
            Stage::Won => self.open_main_menu(),
        }
    }

    /// The continent overview (phase 3 placeholder).
    pub(crate) fn open_strategic(&mut self) {
        self.flow = crate::skirmish::Flow::new();
        self.notice = Some("continent overview".into());
        self.screen = Screen::MainMenu;
    }

    /// The campaign's battle stage, while one is being fought.
    pub(crate) fn campaign_battle(&self) -> Option<Stage> {
        let stage = self.campaign.as_ref()?.state.stage;
        (stage != Stage::Strategic && stage != Stage::Won).then_some(stage)
    }

    /// Whether the battle is fought on the base (the AI moves first).
    pub(crate) fn defending_base(&self) -> bool {
        matches!(
            self.campaign_battle(),
            Some(Stage::FirstAttack | Stage::CounterAttack { .. })
        )
    }

    /// The base map with the campaign's base layout, players merged.
    fn load_base(&mut self) -> Option<Battle> {
        let session = self.campaign.as_ref()?;
        let file = session.base_file()?;
        let mut battle = match Battle::start(&self.game, &file) {
            Ok(b) => b,
            Err(e) => {
                self.error = Some(format!("{file}: {e}"));
                return None;
            }
        };
        battle.scenario.restore(&self.defs);
        let s = &mut battle.scenario;
        match &session.state.base {
            Some(layout) => layout.apply(s),
            None => {
                // The other players' bases are for multiplayer.
                let mine = |o: i64| !(2..=4).contains(&o);
                s.buildings.retain(|b| mine(b.owner));
                s.roads.retain(|r| mine(r.owner));
                s.walls.retain(|w| mine(w.owner));
                s.mines.retain(|m| mine(m.owner));
            }
        }
        s.units.clear();
        s.merge_players(1);
        battle.difficulty = session.state.difficulty;
        Some(battle)
    }

    /// Tiles of the player's zone: within [`ZONE_RADIUS`] of an owned
    /// building.
    fn base_zone(&self, s: &Scenario) -> Vec<(i64, i64)> {
        let size = s.size as i64;
        let mut tiles = vec![false; (size * size) as usize];
        for b in s.buildings.iter().filter(|b| b.owner == PLAYER) {
            let (w, h) = self
                .defs
                .buildings
                .get(&b.kind)
                .map_or((1, 1), |k| (k.width, k.height));
            for y in b.y - ZONE_RADIUS..b.y + h + ZONE_RADIUS {
                for x in b.x - ZONE_RADIUS..b.x + w + ZONE_RADIUS {
                    if (0..size).contains(&x) && (0..size).contains(&y) {
                        tiles[(y * size + x) as usize] = true;
                    }
                }
            }
        }
        (0..size * size)
            .filter(|&i| tiles[i as usize])
            .map(|i| (i % size, i / size))
            .collect()
    }

    /// Deployment points from the owned buildings (`0x47D4E7`).
    pub(crate) fn campaign_deploy_points(&self, s: &Scenario) -> i64 {
        let Some(session) = &self.campaign else {
            return 0;
        };
        let Some(c) = session.continent() else {
            return 0;
        };
        let mut hq = false;
        let mut sum = 0;
        for b in s.buildings.iter().filter(|b| b.owner == PLAYER) {
            if let Some(&(_, p)) = DEPLOY_POINTS.iter().find(|(k, _)| *k == b.kind) {
                if b.kind == HQ {
                    if hq {
                        continue;
                    }
                    hq = true;
                }
                sum += p;
            }
        }
        sum * session.state.deploy_percent(c) / 100
    }

    /// Building types the player owns.
    fn owned_buildings(&self, s: &Scenario) -> Vec<i64> {
        let mut kinds: Vec<i64> = s
            .buildings
            .iter()
            .filter(|b| b.owner == PLAYER)
            .map(|b| b.kind)
            .collect();
        kinds.sort_unstable();
        kinds.dedup();
        kinds
    }

    /// Human unit types the owned buildings can deploy: mobile, armed and
    /// with their prerequisite built.
    pub(crate) fn campaign_units(&self, owned: &[i64]) -> Vec<i64> {
        self.defs
            .units
            .iter()
            .filter(|((_, tl), k)| {
                *tl == 0
                    && k.race == 0
                    && k.cost > 0
                    && k.movement != 0
                    && k.has_weapon
                    && owned.contains(&k.prerequisite)
            })
            .map(|((id, _), _)| *id)
            .collect()
    }

    /// Human automated units the owned buildings provide.
    fn campaign_automated(&self, owned: &[i64]) -> Vec<i64> {
        self.defs
            .automated
            .iter()
            .filter(|(_, k)| k.race == 0 && owned.contains(&k.prerequisite))
            .map(|(id, _)| *id)
            .collect()
    }

    /// Whether an AI unit of `kind` can stand on `(x, y)`.
    fn can_stand(&self, s: &mut Scenario, kind: i64, (x, y): (i64, i64)) -> bool {
        let Some(k) = self.defs.unit(kind) else {
            return false;
        };
        let size = s.size as i64;
        if x + k.width > size || y + k.height > size {
            return false;
        }
        let taken = s.units.iter().filter(|u| u.arrival == 0).any(|u| {
            let (w, h) = self
                .defs
                .unit(u.kind)
                .map_or((1, 1), |k| (k.width, k.height));
            u.x < x + k.width && x < u.x + w && u.y < y + k.height && y < u.y + h
        });
        if taken {
            return false;
        }
        s.units.push(ai_unit(0, kind, (x, y), 0, size, k));
        let grid = movement::move_grid(s, &self.defs, s.units.len() - 1);
        s.units.pop();
        grid.is_none_or(|g| g.get(x as i32, y as i32).0 == Access::Stop)
    }

    /// Places the attackers 11-14 tiles from the player's zone, grouped
    /// around a random tile of that band; later rows as reinforcements.
    fn place_attackers(&mut self, s: &mut Scenario, zone: &[(i64, i64)]) {
        let Some(session) = &self.campaign else {
            return;
        };
        let roster = session.roster();
        let size = s.size as i64;
        let dist = distances(size, zone);
        let band: Vec<(i64, i64)> = (0..size * size)
            .filter(|&i| AI_BAND.contains(&dist[i as usize]))
            .map(|i| (i % size, i / size))
            .collect();
        if band.is_empty() {
            return;
        }
        let anchor = band[(self.rng.rand() as usize) % band.len()];
        let mut cells = band;
        cells.sort_by_key(|&(x, y)| (x - anchor.0).pow(2) + (y - anchor.1).pow(2));
        let mut used = Vec::new();
        for row in roster {
            let Some(k) = self.defs.unit(row.unit_type) else {
                continue;
            };
            let Some(&cell) = cells
                .iter()
                .find(|&&c| !used.contains(&c) && self.can_stand(s, row.unit_type, c))
            else {
                continue;
            };
            used.push(cell);
            let id = s.next_unit_id();
            s.units
                .push(ai_unit(id, row.unit_type, cell, row.turn.max(0), size, k));
        }
    }

    /// The base defends itself: the first city attack or a counterattack
    /// (`CServerPhaseCounterAttTactic`).
    pub(crate) fn begin_base_battle(&mut self) {
        let Some(mut battle) = self.load_base() else {
            return;
        };
        let s = &mut battle.scenario;
        let zone = self.base_zone(s);
        s.deployments = zone
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| Deployment {
                counter: i as i64 + 1,
                player: PLAYER,
                x,
                y,
                turn: 0,
            })
            .collect();
        self.place_attackers(s, &zone);
        let points = self.campaign_deploy_points(s);
        s.info.points = points;
        let owned = self.owned_buildings(s);
        let units = self.campaign_units(&owned);
        let automated = self.campaign_automated(&owned);
        self.begin_campaign_battle(battle, units, automated);
    }

    /// Shows `battle` and opens its briefing with the campaign's army.
    fn begin_campaign_battle(&mut self, battle: Battle, units: Vec<i64>, automated: Vec<i64>) {
        let name = battle.source.to_string();
        if let Some(i) = self.maps.iter().position(|m| m.eq_ignore_ascii_case(&name)) {
            self.index = i;
        }
        self.flow = crate::skirmish::Flow::new();
        self.flow.phase = Phase::Setup;
        self.options.deployment = false;
        self.show_units = true;
        self.show(name, battle);
        let (tech, points) = self.campaign.as_ref().map_or((Vec::new(), 0), |c| {
            let t = &c.state.unit_tech;
            (
                units
                    .iter()
                    .map(|k| t.get(k).copied().unwrap_or(0))
                    .collect(),
                c.state.tech,
            )
        });
        self.flow.setup = Some(Setup {
            units,
            tech,
            automated,
            points,
            cell: 0,
            tech_scroll: 0,
            auto_cell: 0,
            auto_scroll: 0,
            row: 0,
            scroll: 0,
            deployed: Vec::new(),
        });
        self.players = skirmish_players();
        self.player = 0;
        self.begin_briefing();
    }

    /// The base battle's first turn is the AI's; the player's deployed
    /// units have no action points during it.
    pub(crate) fn ai_first(&mut self) {
        for u in &mut self.map.battle.scenario.units {
            if u.owner == PLAYER {
                u.action_points = Some(0);
            }
        }
        self.map.battle.clock.turn = 0;
        self.end_turn();
    }

    /// The campaign's say on a battle's outcome: a base battle goes on while
    /// attackers are still to arrive, and is lost with the headquarters.
    pub(crate) fn campaign_outcome(&self, outcome: Option<Outcome>) -> Option<Outcome> {
        if !self.defending_base() {
            return outcome;
        }
        let s = &self.map.battle.scenario;
        if !s
            .buildings
            .iter()
            .any(|b| b.owner == PLAYER && b.kind == HQ)
        {
            return Some(Outcome::Lost);
        }
        let coming = s
            .units
            .iter()
            .any(|u| u.owner == AI && u.reinforcement && u.arrival > 0);
        match outcome {
            Some(Outcome::Won) if coming && self.flow.forced.is_none() => None,
            o => o,
        }
    }

    /// Technology points a won campaign battle grants.
    pub(crate) fn campaign_tech_reward(&self) -> Option<i64> {
        match self.campaign_battle()? {
            Stage::FirstAttack => Some(FIRST_ATTACK_TECH + self.tech_found()),
            _ => Some(self.tech_found()),
        }
    }

    /// CONTINUE on a campaign battle's report.
    pub(crate) fn campaign_report_closed(&mut self) {
        let outcome = self.flow.outcome;
        let reward = self.campaign_tech_reward().unwrap_or(0);
        let base = self
            .defending_base()
            .then(|| self.map.battle.source.load(&self.game).ok())
            .flatten();
        let Some(session) = &mut self.campaign else {
            return;
        };
        let state = &mut session.state;
        match (state.stage, outcome) {
            (Stage::FirstAttack | Stage::CounterAttack { .. }, Some(Outcome::Won)) => {
                state.tech += reward;
                if let Some(base) = base {
                    let mut now = self.map.battle.scenario.clone();
                    now.units.clear();
                    state.base = Some(BaseLayout::capture(&now, &base));
                }
                state.stage = Stage::Strategic;
            }
            // Losing the base ends the campaign.
            (Stage::FirstAttack | Stage::CounterAttack { .. }, _) => {
                self.campaign = None;
                self.delete_campaign_save();
                self.open_main_menu();
                return;
            }
            _ => state.stage = Stage::Strategic,
        }
        self.save_campaign();
        self.resume_campaign();
    }
}

/// An attacker of `kind` on `(x, y)`, arriving on `turn` (0: at the start).
fn ai_unit(
    id: i64,
    kind: i64,
    (x, y): (i64, i64),
    turn: i64,
    size: i64,
    k: &libday_data::defs::UnitType,
) -> Unit {
    let facing = if k.facings {
        libday_data::scenario::deploy_facing(size, x, y)
    } else {
        0
    };
    Unit {
        counter: id,
        id,
        kind,
        tech: 0,
        owner: AI,
        facing,
        x,
        y,
        arrival: turn,
        reinforcement: turn > 0,
        armor: Some(k.armor),
        action_points: Some(k.action_points),
        neg_ap: 0,
        order: Order::default(),
        objective_order: false,
        overwatch: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_are_chebyshev() {
        let d = distances(20, &[(5, 5)]);
        assert_eq!(d[5 * 20 + 5], 0);
        assert_eq!(d[8 * 20 + 9], 4);
        assert_eq!(d[5 * 20 + 19], 14);
    }

    #[test]
    fn reward_scales_with_difficulty() {
        let mut s = State {
            format: FORMAT.into(),
            version: VERSION,
            name: String::new(),
            difficulty: 0,
            continent: 0,
            credits: 0,
            tech: 0,
            unit_tech: BTreeMap::new(),
            provinces: Vec::new(),
            base: None,
            stage: Stage::Strategic,
        };
        assert_eq!(s.reward(400), 500);
        s.difficulty = 10;
        assert_eq!(s.reward(400), 300);
        let json = s.to_json();
        assert_eq!(State::from_json(&json).unwrap(), s);
    }
}
