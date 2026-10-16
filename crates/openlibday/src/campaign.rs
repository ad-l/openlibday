//! Campaign mode: the campaign's state and save, and the battles fought on
//! the player's base (the first city attack and counterattacks).

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;

use libday_data::Battle;
use libday_data::Definitions;
use libday_data::campaign::{Campaign, Continent, CounterAttackUnit, scenario_file};
use libday_data::objective::Outcome;
use libday_data::scenario::{Building, Deployment, Mine, Order, Scenario, Surface, Unit};
use libday_view::movement::{self, Access};
use serde::{Deserialize, Serialize};

use crate::Viewer;
use crate::link::Opening;
use crate::players::{CAMPAIGN_PLAYER, Players};
use crate::skirmish::{AI, Phase, Setup};

/// Technology points for repelling the first city attack. *Unverified*: no
/// such constant was found in the data or the executable, but the original
/// grants this many after the first battle.
pub const FIRST_ATTACK_TECH: i64 = 150;
/// The campaign save in the saves directory.
const SAVE: &str = "campaign.json";
/// A network campaign's autosave, on the host.
const SAVE_ONLINE: &str = "campaign_online.json";
const FORMAT: &str = "openlibday-campaign";
/// 2: per-player credits and technology ([`Member`]).
const VERSION: u32 = 2;
/// Headquarters building type.
pub const HQ: i64 = 130;
/// Deployment points per owned building (`0x47D4E7`): the HQ and Deploy
/// Mk1-4.
const DEPLOY_POINTS: [(i64, i64); 5] = [(HQ, 50), (131, 100), (173, 110), (174, 120), (175, 130)];
/// Deployment points percentage for 1 to 4 players (`0x41EF17`).
const PLAYERS_PERCENT: [i64; 4] = [100, 75, 65, 55];
/// Player zone: tiles within this many of an owned building (`0x47F8E7`).
const ZONE_RADIUS: i64 = 3;
/// AI zone: tiles this far from the player's zone and units (`0x47C737`:
/// none within 11, some within 14).
const AI_BAND: std::ops::RangeInclusive<i64> = 12..=14;
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
    /// What `now` holds over the shipped base map `base`. The players'
    /// surviving defence towers turn back into their tower buildings.
    pub fn capture(now: &Scenario, base: &Scenario, defs: &Definitions) -> Self {
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
        let mut buildings = now.buildings.clone();
        let mut id = buildings.iter().map(|b| b.id + 1).max().unwrap_or(1);
        for u in now.units.iter().filter(|u| u.owner != AI && u.arrival == 0) {
            let tower = defs
                .buildings
                .iter()
                .find(|(_, k)| k.transform_into == Some(u.kind));
            if let Some((&kind, k)) = tower {
                buildings.push(Building {
                    counter: 0,
                    id,
                    kind,
                    owner: u.owner,
                    armor: k.armor,
                    x: u.x,
                    y: u.y,
                    ai_target: false,
                });
                id += 1;
            }
        }
        Self {
            buildings,
            roads: now.roads.clone(),
            walls: now.walls.clone(),
            mines: now.mines.clone(),
            terrain,
        }
    }

    /// After a won base defence: the burnt and felled terrain (with the
    /// destroyed buildings' rubble) grows back and what survived is
    /// repaired; what was destroyed stays lost.
    pub fn repair(&mut self, defs: &Definitions) {
        self.terrain.clear();
        for b in &mut self.buildings {
            if let Some(k) = defs.buildings.get(&b.kind) {
                b.armor = k.armor;
            }
        }
        for r in &mut self.roads {
            if let Some(k) = defs.roads.get(&r.kind) {
                r.armor = k.armor;
            }
        }
        for w in &mut self.walls {
            if let Some(k) = defs.walls.get(&w.kind) {
                w.armor = k.armor;
            }
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

/// A human player of the campaign: their credits and technology (the
/// original's per-player `Server Mode Campaign` records).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// The player's slot (1 to 4) and colour.
    pub slot: i64,
    pub name: String,
    /// Base-building budget.
    pub credits: i64,
    /// Technology points to spend in ARMY.
    pub tech: i64,
    /// Technology level per unit type.
    pub unit_tech: BTreeMap<i64, i64>,
}

impl Member {
    pub(crate) fn new(slot: i64, name: &str) -> Self {
        Self {
            slot,
            name: name.into(),
            credits: 0,
            tech: 0,
            unit_tech: BTreeMap::new(),
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
    /// The human players, in slot order (one but in a network game).
    pub members: Vec<Member>,
    pub provinces: Vec<ProvinceState>,
    /// `None` until the base first changes from the shipped map.
    pub base: Option<BaseLayout>,
    /// The structures built since base building opened (single player) or
    /// since the last battle: recycled for their full cost.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fresh: Vec<crate::base_editor::Placed>,
    pub stage: Stage,
}

impl State {
    /// A new campaign on the first continent for the humans `players`
    /// (slot and name, the first naming the campaign); `roll` is the
    /// game's `rand()`.
    pub fn new(
        campaign: &Campaign,
        players: &[(i64, String)],
        difficulty: i64,
        roll: impl FnMut() -> i64,
    ) -> Self {
        let mut members: Vec<Member> = players
            .iter()
            .map(|(slot, name)| Member::new(*slot, name))
            .collect();
        if members.is_empty() {
            members.push(Member::new(CAMPAIGN_PLAYER, ""));
        }
        let mut state = Self {
            format: FORMAT.into(),
            version: VERSION,
            name: members[0].name.clone(),
            difficulty,
            continent: 0,
            members,
            provinces: Vec::new(),
            base: None,
            fresh: Vec::new(),
            stage: Stage::FirstAttack,
        };
        state.enter_continent(campaign, 0, roll);
        state
    }

    /// The human player in `slot`.
    pub fn member(&self, slot: i64) -> Option<&Member> {
        self.members.iter().find(|m| m.slot == slot)
    }

    pub fn member_mut(&mut self, slot: i64) -> Option<&mut Member> {
        self.members.iter_mut().find(|m| m.slot == slot)
    }

    /// How many humans play.
    pub fn humans(&self) -> usize {
        self.members.len().clamp(1, 4)
    }

    /// Starts continent `index` (`0x47E269`): its start cash replaces
    /// everyone's credits, each province gets a random variant, the base
    /// variant 1.
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
        for m in &mut self.members {
            m.credits = c.start_cash;
        }
        self.base = None;
        self.fresh.clear();
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

    pub fn province_mut(&mut self, number: i64) -> Option<&mut ProvinceState> {
        self.provinces.iter_mut().find(|p| p.number == number)
    }

    /// Deployment points percentage for the difficulty (`0x47AA4D`).
    pub fn deploy_percent(&self, c: &Continent) -> i64 {
        let (easy, hard) = (c.deploy_percent.easy, c.deploy_percent.hard);
        easy + (hard - easy) * self.difficulty.clamp(0, DIFFICULTY_MAX) / DIFFICULTY_MAX
    }

    /// Credits for capturing a province (`0x415653`): 125 % on the easiest
    /// difficulty down to 75 % on the hardest, truncated.
    pub fn reward(&self, credits: i64) -> i64 {
        let pct = 125 - 5 * self.difficulty.clamp(0, DIFFICULTY_MAX);
        credits * pct / 100
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        Self::from_value(serde_json::from_str(text).map_err(|e| e.to_string())?)
    }

    /// The state in `value`, of any version.
    pub fn from_value(mut value: serde_json::Value) -> Result<Self, String> {
        // Version 1 kept one player's credits and technology at the top.
        if let Some(obj) = value.as_object_mut()
            && !obj.contains_key("members")
        {
            let member = serde_json::json!({
                "slot": CAMPAIGN_PLAYER,
                "name": obj.get("name").cloned().unwrap_or_default(),
                "credits": obj.remove("credits").unwrap_or(0.into()),
                "tech": obj.remove("tech").unwrap_or(0.into()),
                "unit_tech": obj.remove("unit_tech").unwrap_or(serde_json::json!({})),
            });
            obj.insert("members".into(), serde_json::json!([member]));
        }
        let mut state: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
        if state.format != FORMAT || state.version == 0 || state.version > VERSION {
            return Err(format!(
                "unsupported campaign save {} v{}",
                state.format, state.version
            ));
        }
        if state.members.is_empty() {
            return Err("campaign save without players".into());
        }
        state.version = VERSION;
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
    pub(crate) fn campaign_save_path(&self) -> Option<PathBuf> {
        let name = if self.networked() { SAVE_ONLINE } else { SAVE };
        self.saves.as_ref().map(|d| d.join(name))
    }

    /// Whether a campaign save exists for LOAD CAMPAIGN.
    pub(crate) fn campaign_saved(&self) -> bool {
        self.campaign_save_path().is_some_and(|p| p.is_file())
    }

    /// Keeps the campaign on disk (a client leaves it to the host).
    pub(crate) fn save_campaign(&mut self) {
        if self.net.host.is_some() {
            return;
        }
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

    /// The campaign has ended: back to the main menu, or to the lobby
    /// online.
    fn campaign_over(&mut self) {
        self.campaign = None;
        self.strategic = None;
        self.base_editor = None;
        if self.networked() {
            self.open_lobby();
        } else {
            self.open_main_menu();
        }
    }

    fn delete_campaign_save(&mut self) {
        if self.net.host.is_some() {
            return;
        }
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
        let state = State::new(
            &data,
            &[(CAMPAIGN_PLAYER, name.to_owned())],
            difficulty,
            || rng.rand(),
        );
        self.campaign = Some(Session { data, state });
        self.save_campaign();
        self.begin_base_battle();
        // The first continent's introduction (`0x47ef17`).
        self.play_continent_intro();
    }

    /// START in Campaign mode (host): a new co-op campaign for everyone in
    /// the waiting area, at the chosen difficulty.
    pub(crate) fn start_online_campaign(&mut self, difficulty: i64) {
        let online = self.net.online.as_ref();
        let players: Vec<(i64, String)> = self
            .seats
            .humans()
            .map(|slot| {
                let name = online.and_then(|o| o.names.get(&slot).cloned());
                (
                    slot,
                    name.unwrap_or_else(|| libday_view::minimap::player_name(slot)),
                )
            })
            .collect();
        let data = match self.game.original_campaign() {
            Ok(data) => data,
            Err(e) => {
                self.error = Some(format!("campaign: {e}"));
                return;
            }
        };
        let rng = &mut self.rng;
        let state = State::new(&data, &players, difficulty, || rng.rand());
        self.launch_campaign(Session { data, state }, Opening::New);
    }

    /// START with a loaded save (host): everyone goes on with `state`.
    pub(crate) fn start_loaded_campaign(&mut self, state: State, opening: Opening) {
        match self.game.original_campaign() {
            Ok(data) => self.launch_campaign(Session { data, state }, opening),
            Err(e) => self.error = Some(format!("campaign: {e}")),
        }
    }

    /// The host sends everyone the campaign and opens it.
    fn launch_campaign(&mut self, session: Session, opening: Opening) {
        let seed = match &mut self.net.seeds {
            Some(seeds) => seeds.rand() as u32,
            None => self.rng.rand() as u32,
        };
        self.seats.local = self.seats.first_local();
        if let Some(online) = &mut self.net.online {
            online.started = true;
        }
        self.net.log.clear();
        self.net.desync = None;
        let humans = session.state.humans();
        let text = session.state.to_json();
        for peer in &self.net.peers {
            peer.link.send(&crate::link::ToClient::Campaign {
                state: text.clone(),
                humans,
                slot: peer.slot,
                seed,
                opening,
            });
        }
        self.join_campaign(session, seed, opening);
    }

    /// A client starts (or goes on with) the host's co-op campaign.
    pub(crate) fn follow_campaign(
        &mut self,
        state: &str,
        humans: usize,
        slot: i64,
        seed: u32,
        opening: Opening,
    ) {
        if let Some(online) = &mut self.net.online {
            online.slot = slot;
            online.started = true;
        }
        self.seats = Players::online(humans, slot);
        let session = State::from_json(state).and_then(|state| {
            self.game
                .original_campaign()
                .map(|data| Session { data, state })
                .map_err(|e| e.to_string())
        });
        match session {
            Ok(session) => self.join_campaign(session, seed, opening),
            Err(e) => self.error = Some(format!("the host's campaign: {e}")),
        }
    }

    /// Every player starts the same campaign the same way.
    fn join_campaign(&mut self, session: Session, seed: u32, opening: Opening) {
        self.rng = libday_view::scatter::Rand::new(seed);
        self.campaign = Some(session);
        self.save_campaign();
        match opening {
            Opening::New => {
                self.begin_base_battle();
                self.play_continent_intro();
            }
            Opening::Resume => self.resume_campaign(),
            Opening::BaseEditor => self.open_base_editor(),
        }
    }

    /// The current continent's `Intro` video.
    fn play_continent_intro(&mut self) {
        let video = self
            .campaign
            .as_ref()
            .and_then(Session::continent)
            .map(|c| c.intro_video.clone())
            .unwrap_or_default();
        self.play_video(&video);
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

    /// The campaign's battle stage, while one is being fought.
    pub(crate) fn campaign_battle(&self) -> Option<Stage> {
        let stage = self.campaign.as_ref()?.state.stage;
        (stage != Stage::Strategic && stage != Stage::Won).then_some(stage)
    }

    /// An owner's name: the campaign's player name for the human player,
    /// "Player N" in skirmishes and "Alien" for the computer.
    pub(crate) fn player_name(&self, owner: i64) -> String {
        let named = self
            .campaign
            .as_ref()
            .and_then(|c| c.state.member(owner))
            .filter(|m| !m.name.trim().is_empty());
        match named {
            Some(m) => m.name.clone(),
            _ => self
                .online_name(owner)
                .filter(|_| self.seats.human(owner))
                .map_or_else(|| libday_view::minimap::player_name(owner), str::to_owned),
        }
    }

    /// The AI's support budget for a battle (`brain+0x20`, set by
    /// `0x46f8b3`): the continent's `AiAutoUnitRes`, half as much again
    /// when the base is attacked; none in skirmishes.
    pub(crate) fn ai_support_budget(&self) -> i64 {
        let Some(res) = self
            .campaign_battle()
            .and(self.campaign.as_ref())
            .and_then(|c| c.continent())
            .map(|c| c.ai_automated.resources)
        else {
            return 0;
        };
        if self.defending_base() {
            (res as f64 * 1.5) as i64
        } else {
            res
        }
    }

    /// Whether the continent lets the AI call in automated unit `id`
    /// (`AI automated available`, `0x474802`).
    pub(crate) fn ai_support_available(&self, id: i64) -> bool {
        self.ai_support_types().contains(&id)
    }

    /// The continent's `AI automated available` (campaign `+0xac`); none
    /// outside campaign battles.
    pub(crate) fn ai_support_types(&self) -> Vec<i64> {
        self.campaign_battle()
            .and(self.campaign.as_ref())
            .and_then(|c| c.continent())
            .map(|c| c.ai_automated.types.clone())
            .unwrap_or_default()
    }

    /// The percent chance of a stage 2 or 7 support purchase: the
    /// continent's `AiAutoUnitChanceCall` (campaign `+0x90`), 25 on the
    /// base; none outside campaign battles.
    pub(crate) fn ai_support_chance(&self) -> Option<i64> {
        let chance = self
            .campaign_battle()
            .and(self.campaign.as_ref())
            .and_then(|c| c.continent())
            .map(|c| c.ai_automated.chance)?;
        Some(if self.defending_base() { 25 } else { chance })
    }

    /// Whether the battle is fought on the base (the AI moves first).
    pub(crate) fn defending_base(&self) -> bool {
        matches!(
            self.campaign_battle(),
            Some(Stage::FirstAttack | Stage::CounterAttack { .. })
        )
    }

    /// The base map with the campaign's base layout, players merged; for
    /// a battle (`transform`) the towers become units.
    pub(crate) fn load_base(&mut self, transform: bool) -> Option<Battle> {
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
        let humans = session.state.humans();
        let s = &mut battle.scenario;
        match &session.state.base {
            Some(layout) => layout.apply(s),
            None => {
                // The bases of players not playing.
                let mine = |o: i64| !(humans as i64 + 1..=4).contains(&o);
                s.buildings.retain(|b| mine(b.owner));
                s.roads.retain(|r| mine(r.owner));
                s.walls.retain(|w| mine(w.owner));
                s.mines.retain(|m| mine(m.owner));
            }
        }
        s.units.clear();
        s.merge_players(humans);
        // Base battles only (0x47e5be → 0x47fb4e): towers become units.
        if transform {
            s.transform_buildings(&self.defs);
        }
        battle.difficulty = session.state.difficulty;
        Some(battle)
    }

    /// The campaign's human slots.
    pub(crate) fn campaign_humans(&self) -> Vec<i64> {
        let n = self.campaign.as_ref().map_or(1, |c| c.state.humans());
        (1..=n as i64).collect()
    }

    /// Tiles of `player`'s zone: within [`ZONE_RADIUS`] of a building of
    /// theirs.
    fn base_zone(&self, s: &Scenario, player: i64) -> Vec<(i64, i64)> {
        let size = s.size as i64;
        let mut tiles = vec![false; (size * size) as usize];
        for b in s.buildings.iter().filter(|b| b.owner == player) {
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

    /// `player`'s deployment points from their buildings (`0x47D4E7`),
    /// less with more players.
    pub(crate) fn campaign_deploy_points(&self, s: &Scenario, player: i64) -> i64 {
        let Some(session) = &self.campaign else {
            return 0;
        };
        let Some(c) = session.continent() else {
            return 0;
        };
        let mut hq = false;
        let mut sum = 0;
        for b in s.buildings.iter().filter(|b| b.owner == player) {
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
        let players = PLAYERS_PERCENT[session.state.humans() - 1];
        sum * session.state.deploy_percent(c) / 100 * players / 100
    }

    /// Building types `player` owns.
    pub(crate) fn owned_buildings(&self, s: &Scenario, player: i64) -> Vec<i64> {
        let mut kinds: Vec<i64> = s
            .buildings
            .iter()
            .filter(|b| b.owner == player)
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
                    && owned
                        .iter()
                        .any(|&o| libday_view::base::satisfies(o, k.prerequisite))
            })
            .map(|((id, _), _)| *id)
            .collect()
    }

    /// Human automated units the owned buildings provide.
    pub(crate) fn campaign_automated(&self, owned: &[i64]) -> Vec<i64> {
        self.defs
            .automated
            .iter()
            .filter(|(_, k)| {
                k.race == 0
                    && owned
                        .iter()
                        .any(|&o| libday_view::base::satisfies(o, k.prerequisite))
            })
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
        let taken = s.units.iter().any(|u| {
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

    /// Places the attackers as `0x47F8E7(1)`: the AI zone is the free tiles
    /// 12-14 tiles from the player's zone and units (`0x47C737`, any
    /// distance up to 14 in the map's corners); it is shuffled
    /// (`0x4412F3`) and each attacker takes the first tile it can stand on,
    /// which with its 8 neighbours then leaves the zone (`0x47CEAE`). Later
    /// rows arrive as reinforcements.
    fn place_attackers(&mut self, s: &mut Scenario, zone: &[(i64, i64)]) {
        let Some(session) = &self.campaign else {
            return;
        };
        let roster = session.roster();
        let size = s.size as i64;
        let mut sources = zone.to_vec();
        for u in s.units.iter().filter(|u| u.owner != AI && u.arrival == 0) {
            let (w, h) = self
                .defs
                .unit(u.kind)
                .map_or((1, 1), |k| (k.width, k.height));
            for y in u.y..u.y + h {
                for x in u.x..u.x + w {
                    sources.push((x, y));
                }
            }
        }
        let dist = distances(size, &sources);
        let edge = |v: i64| v <= 1 || v >= size - 2;
        let mut cells: Vec<(i64, i64)> = (0..size * size)
            .map(|i| (i % size, i / size))
            .filter(|&(x, y)| {
                let d = dist[(y * size + x) as usize];
                d > 0 && d <= *AI_BAND.end() && (d >= *AI_BAND.start() || edge(x) && edge(y))
            })
            .collect();
        let n = cells.len();
        for i in 0..n {
            let j = self.rng.rand() as usize % n;
            cells.swap(i, j);
        }
        for row in roster {
            let Some(k) = self.defs.unit(row.unit_type) else {
                continue;
            };
            let Some(at) = cells
                .iter()
                .position(|&c| self.can_stand(s, row.unit_type, c))
            else {
                continue;
            };
            let (x, y) = cells[at];
            cells.retain(|&(cx, cy)| (cx - x).abs() > 1 || (cy - y).abs() > 1);
            let id = s.next_unit_id();
            s.units
                .push(ai_unit(id, row.unit_type, (x, y), row.turn.max(0), size, k));
        }
    }

    /// The base defends itself: the first city attack or a counterattack
    /// (`CServerPhaseCounterAttTactic`).
    pub(crate) fn begin_base_battle(&mut self) {
        let Some(mut battle) = self.load_base(true) else {
            return;
        };
        let s = &mut battle.scenario;
        let mut all = Vec::new();
        s.deployments.clear();
        let mut points = Vec::new();
        for player in self.campaign_humans() {
            let zone = self.base_zone(s, player);
            for &(x, y) in &zone {
                s.deployments.push(Deployment {
                    counter: s.deployments.len() as i64 + 1,
                    player,
                    x,
                    y,
                    turn: 0,
                });
            }
            all.extend(zone);
            points.push((player, self.campaign_deploy_points(s, player)));
        }
        self.place_attackers(s, &all);
        let owned = self.owned_buildings(s, self.seats.local);
        let units = self.campaign_units(&owned);
        let automated = self.campaign_automated(&owned);
        self.begin_campaign_battle(battle, units, automated, points);
    }

    /// Attacks province `n` (`CServerPhaseTactic`): its scenario, the
    /// player's army from the base's buildings.
    pub(crate) fn begin_tactic(&mut self, n: i64) {
        let Some(base) = self.load_base(false) else {
            return;
        };
        let points: Vec<(i64, i64)> = self
            .campaign_humans()
            .into_iter()
            .map(|p| (p, self.campaign_deploy_points(&base.scenario, p)))
            .collect();
        let owned = self.owned_buildings(&base.scenario, self.seats.local);
        let Some(session) = &self.campaign else {
            return;
        };
        let Some(file) = session.scenario_of(n) else {
            return;
        };
        let difficulty = session.state.difficulty;
        let humans = session.state.humans();
        let mut battle = match Battle::start(&self.game, &file) {
            Ok(b) => b,
            Err(e) => {
                self.error = Some(format!("{file}: {e}"));
                return;
            }
        };
        battle.scenario.restore(&self.defs);
        battle.scenario.merge_players(humans);
        battle.difficulty = difficulty;
        let units = self.campaign_units(&owned);
        let automated = self.campaign_automated(&owned);
        if let Some(session) = &mut self.campaign {
            session.state.stage = Stage::Tactic { province: n };
        }
        self.save_campaign();
        self.begin_campaign_battle(battle, units, automated, points);
    }

    /// Shows `battle` and opens its briefing with the campaign's army, the
    /// humans' deployment `points`.
    fn begin_campaign_battle(
        &mut self,
        mut battle: Battle,
        units: Vec<i64>,
        automated: Vec<i64>,
        points: Vec<(i64, i64)>,
    ) {
        self.base_editor = None;
        battle.scenario.info.points = points.first().map_or(0, |&(_, p)| p);
        self.show_campaign_map(battle);
        self.flow.campaign_points = points;
        let me = self.seats.local;
        let (tech, points) = self
            .campaign
            .as_ref()
            .and_then(|c| c.state.member(me))
            .map_or((Vec::new(), 0), |m| {
                (
                    units
                        .iter()
                        .map(|k| m.unit_tech.get(k).copied().unwrap_or(0))
                        .collect(),
                    m.tech,
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
        if !self.networked() {
            self.seats = Players::default();
        }
        self.players = self.seats.overview();
        self.player = 0;
        self.begin_briefing();
    }

    /// Shows a campaign map (a battle's or the base editor's) before play.
    pub(crate) fn show_campaign_map(&mut self, battle: Battle) {
        self.strategic = None;
        let name = battle.source.to_string();
        if let Some(i) = self.maps.iter().position(|m| m.eq_ignore_ascii_case(&name)) {
            self.index = i;
        }
        self.flow = crate::skirmish::Flow::new();
        self.flow.phase = Phase::Setup;
        self.options.deployment = false;
        self.show_units = true;
        self.show(name, battle);
    }

    /// The base battle's first turn is the AI's; the player's deployed
    /// units have no action points during it, except the defence towers.
    pub(crate) fn ai_first(&mut self) {
        let defs = &self.defs;
        for u in &mut self.map.battle.scenario.units {
            let tower = defs.unit(u.kind).is_some_and(|k| k.movement == 0);
            if u.owner != AI && !tower {
                u.action_points = Some(0);
            }
        }
        self.map.battle.clock.turn = 0;
        while self.flow.phase == Phase::Player {
            let player = self.map.battle.clock.player;
            if !self.apply_end_turn(player) {
                break;
            }
        }
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
            .any(|b| (1..=4).contains(&b.owner) && b.kind == HQ)
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
        let first = self
            .campaign
            .as_ref()
            .is_some_and(|c| c.state.stage == Stage::FirstAttack)
            && outcome == Some(Outcome::Won);
        let Some(session) = &mut self.campaign else {
            return;
        };
        let state = &mut session.state;
        match (state.stage, outcome) {
            (Stage::FirstAttack, Some(Outcome::Won)) => {
                state.members.iter_mut().for_each(|m| m.tech += reward);
                if let Some(base) = base {
                    let mut layout =
                        BaseLayout::capture(&self.map.battle.scenario, &base, &self.defs);
                    layout.repair(&self.defs);
                    state.base = Some(layout);
                }
                state.stage = Stage::Strategic;
            }
            (Stage::CounterAttack { province }, Some(Outcome::Won)) => {
                state.members.iter_mut().for_each(|m| m.tech += reward);
                if let Some(base) = base {
                    let mut layout =
                        BaseLayout::capture(&self.map.battle.scenario, &base, &self.defs);
                    layout.repair(&self.defs);
                    state.base = Some(layout);
                }
                self.province_won(province);
                return;
            }
            // Losing the base ends the campaign.
            (Stage::FirstAttack | Stage::CounterAttack { .. }, _) => {
                let video = session.data.videos.lost.clone();
                self.delete_campaign_save();
                self.campaign_over();
                self.play_video(&video);
                return;
            }
            (Stage::Tactic { province }, Some(Outcome::Won)) => {
                session
                    .state
                    .members
                    .iter_mut()
                    .for_each(|m| m.tech += reward);
                let data = session.continent().and_then(|c| c.province(province));
                let credits = data.map_or(0, |p| p.credits);
                let variant = session.state.province(province).map_or(1, |p| p.variant);
                let counter = data
                    .and_then(|p| p.scenarios.iter().find(|s| s.variant == variant))
                    .is_some_and(|s| s.counter_attack);
                let state = &mut session.state;
                let pay = state.reward(credits);
                state.members.iter_mut().for_each(|m| m.credits += pay);
                if let Some(p) = state.province_mut(province) {
                    p.captured = true;
                }
                if counter {
                    // The province's counterattack on the base.
                    state.stage = Stage::CounterAttack { province };
                    let video = session.data.videos.counter_attack.clone();
                    self.save_campaign();
                    self.begin_base_battle();
                    self.play_video(&video);
                } else {
                    self.province_won(province);
                }
                return;
            }
            // A lost province is fought on its other variant next time.
            (Stage::Tactic { province }, _) => {
                let variants: Vec<i64> = session
                    .continent()
                    .and_then(|c| c.province(province))
                    .map(|p| p.scenarios.iter().map(|s| s.variant).collect())
                    .unwrap_or_default();
                let state = &mut session.state;
                if let Some(p) = state.province_mut(province)
                    && let Some(i) = variants.iter().position(|&v| v == p.variant)
                {
                    p.variant = variants[(i + 1) % variants.len()];
                }
                state.stage = Stage::Strategic;
            }
            _ => state.stage = Stage::Strategic,
        }
        self.save_campaign();
        if first {
            // The overview's "You are victorious", CONTINUE to base building;
            // technology is spent there with ARMY.
            self.resume_campaign();
            self.strategic_victory();
        } else {
            self.resume_campaign();
        }
    }

    /// Scripts: starts continent `n` with every province but the capital
    /// captured, on the overview.
    pub(crate) fn jump_to_continent(&mut self, n: i64) -> Result<(), String> {
        let session = self.campaign.as_mut().ok_or("no campaign")?;
        let index = usize::try_from(n)
            .ok()
            .filter(|&i| i < session.data.continents.len())
            .ok_or_else(|| format!("no continent {n}"))?;
        let rng = &mut self.rng;
        session
            .state
            .enter_continent(&session.data, index, || rng.rand());
        let capital = session.continent().map_or(0, |c| c.capital_province);
        for p in &mut session.state.provinces {
            p.captured = p.number != capital;
        }
        session.state.stage = Stage::Strategic;
        self.save_campaign();
        self.resume_campaign();
        Ok(())
    }

    /// Province `n` is held: the capital completes the continent, else
    /// the overview says "You are victorious".
    fn province_won(&mut self, n: i64) {
        let Some(session) = &mut self.campaign else {
            return;
        };
        let capital = session.continent().is_some_and(|c| c.capital_province == n);
        let rng = &mut self.rng;
        let state = &mut session.state;
        let victory = if capital {
            let next = state.continent + 1;
            if next < session.data.continents.len() {
                state.enter_continent(&session.data, next, || rng.rand());
                state.stage = Stage::Strategic;
                self.save_campaign();
                self.resume_campaign();
                // The new continent's introduction (`0x47f07e`).
                self.play_continent_intro();
                return;
            } else {
                // The campaign is won.
                let video = session.data.videos.won.clone();
                self.delete_campaign_save();
                self.campaign_over();
                self.notice = Some("campaign won".into());
                self.play_video(&video);
                return;
            }
        } else {
            state.stage = Stage::Strategic;
            true
        };
        self.save_campaign();
        self.resume_campaign();
        if victory {
            self.strategic_victory();
        }
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
            members: vec![Member::new(1, "")],
            provinces: Vec::new(),
            base: None,
            stage: Stage::Strategic,
            fresh: Vec::new(),
        };
        assert_eq!(s.reward(400), 500);
        s.difficulty = 10;
        assert_eq!(s.reward(400), 300);
        s.difficulty = 4;
        assert_eq!(s.reward(450), 472);
        s.difficulty = 10;
        let json = s.to_json();
        assert_eq!(State::from_json(&json).unwrap(), s);
    }

    #[test]
    fn version_1_saves_become_one_member() {
        let json = r#"{"format":"openlibday-campaign","version":1,"name":"Max",
            "difficulty":4,"continent":1,"credits":300,"tech":20,
            "unit_tech":{"2":1},"provinces":[],"base":null,
            "stage":{"stage":"strategic"}}"#;
        let s = State::from_json(json).unwrap();
        assert_eq!(s.version, VERSION);
        assert_eq!(s.members.len(), 1);
        let m = &s.members[0];
        assert_eq!((m.slot, m.name.as_str()), (1, "Max"));
        assert_eq!((m.credits, m.tech), (300, 20));
        assert_eq!(m.unit_tech.get(&2), Some(&1));
    }

    #[test]
    fn repair_regrows_terrain_and_mends_survivors() {
        let mut defs = Definitions::default();
        defs.buildings.insert(
            7,
            libday_data::defs::BuildingType {
                armor: 50,
                ..Default::default()
            },
        );
        defs.roads.insert(
            1,
            libday_data::defs::SurfaceType {
                armor: 20,
                ..Default::default()
            },
        );
        let road = Surface {
            counter: 0,
            kind: 1,
            owner: CAMPAIGN_PLAYER,
            x: 3,
            y: 4,
            armor: 5,
            broken: false,
        };
        let mut layout = BaseLayout {
            buildings: vec![Building {
                counter: 0,
                id: 1,
                kind: 7,
                owner: CAMPAIGN_PLAYER,
                armor: 9,
                x: 1,
                y: 1,
                ai_target: false,
            }],
            roads: vec![road],
            terrain: vec![TerrainChange {
                x: 2,
                y: 2,
                terrain: "ARIX000".into(),
            }],
            ..Default::default()
        };
        layout.repair(&defs);
        assert!(layout.terrain.is_empty());
        assert_eq!(layout.buildings[0].armor, 50);
        assert_eq!(layout.roads.len(), 1);
        assert_eq!(layout.roads[0].armor, 20);
    }
}
