//! Skirmish mode: the selector, technology upgrades, deployment and the turn
//! rotation between the player and the (still passive) AI.

use std::collections::BTreeSet;
use std::time::Duration;

use libday_data::Battle;
use libday_data::battle::PlayerRecord;
use libday_data::defs::UnitType;
use libday_data::objective::{self, Outcome};
use libday_data::scenario::{Order, OrderTarget, SkirmishEntry, Target, Unit};
use libday_view::movement::{self, Access};
use libday_view::ui::{
    AUTO_COLUMNS, AUTO_ROWS, DEPLOY_ROWS, SKIRMISH_ROWS, Screen, TECH_COLUMNS, TECH_ROWS,
};

use crate::Viewer;
use crate::command::Command;
use crate::sound::Effect;

/// The AI's player slot.
pub const AI: i64 = 0;
/// How long the AI's (empty) turn stays on screen.
const AI_TURN: Duration = Duration::from_millis(1500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Map viewer: every unit can be moved, no turns.
    Free,
    /// Skirmish selector and technology upgrades.
    Setup,
    /// Starting zone overview and unit placement.
    Deploy,
    /// A human's turn (`clock.player`'s).
    Player,
    /// The AI's turn.
    Ai,
    /// The battle is won or lost: the battle report.
    Over,
}

/// The army being prepared for a skirmish.
pub struct Setup {
    /// Buyable unit types, in the technology grid's order.
    pub units: Vec<i64>,
    /// Technology level chosen for each of `units`.
    pub tech: Vec<i64>,
    /// Automated unit types (shown, not yet usable).
    pub automated: Vec<i64>,
    /// Technology points left.
    pub points: i64,
    /// Selected technology grid cell and the grid's first visible row.
    pub cell: usize,
    pub tech_scroll: usize,
    /// Selected automated unit and the automated grid's first visible row.
    pub auto_cell: usize,
    pub auto_scroll: usize,
    /// Selected deployment list row (index into the list) and its scroll.
    pub row: usize,
    pub scroll: usize,
    /// Units placed during deployment or the current reinforcement
    /// (ids), which may be recalled.
    pub deployed: Vec<i64>,
}

pub struct Flow {
    pub phase: Phase,
    /// Skirmish catalogue, its selected row and scroll.
    pub catalog: Vec<SkirmishEntry>,
    pub pick: usize,
    pub scroll: usize,
    pub setup: Option<Setup>,
    /// Last game message, shown in the bottom bar.
    pub message: Option<String>,
    /// Clock time the AI's turn ends.
    pub ai_until: Option<Duration>,
    /// Outcome imposed by the win / lose cheats.
    pub forced: Option<Outcome>,
    /// How the battle ended, once it has.
    pub outcome: Option<Outcome>,
    /// The battle report's text, once the battle is over.
    pub report: Option<Report>,
    /// Whether the player is placing reinforcements during their turn,
    /// and the unit selected before.
    pub reinforcing: bool,
    pub resume: Option<usize>,
    /// AUTOMATED UNITS sidebar: the armed support's row, the list's
    /// scroll, a transport's pickup tile, and the once-per-battle supports
    /// already used.
    pub support_row: Option<usize>,
    pub support_scroll: usize,
    pub pickup: Option<(i64, i64)>,
    pub used_once: Vec<i64>,
    /// What the AI may still spend on supports this battle.
    pub ai_budget: i64,
    /// Technology buildings captured by the player's units (ids).
    pub tech: BTreeSet<i64>,
    /// The humans done deploying.
    pub ready: BTreeSet<i64>,
}

impl Flow {
    pub fn new() -> Self {
        Self {
            phase: Phase::Free,
            catalog: Vec::new(),
            pick: 0,
            scroll: 0,
            setup: None,
            message: None,
            ai_until: None,
            forced: None,
            outcome: None,
            report: None,
            reinforcing: false,
            resume: None,
            support_row: None,
            support_scroll: 0,
            pickup: None,
            used_once: Vec::new(),
            ai_budget: 0,
            tech: BTreeSet::new(),
            ready: BTreeSet::new(),
        }
    }
}

impl Viewer {
    /// The screen a closed overview or goal dialog returns to.
    pub(crate) fn home(&self) -> Screen {
        if self.base_editor.is_some() {
            return Screen::BaseEditor;
        }
        if let Some(screen) = self.strategic_screen() {
            return screen;
        }
        match self.flow.phase {
            Phase::Deploy => Screen::Deploy,
            Phase::Player if self.flow.reinforcing => Screen::Deploy,
            Phase::Player if !self.seats.is_local(self.map.battle.clock.player) => Screen::AiTurn,
            Phase::Ai => Screen::AiTurn,
            Phase::Over => Screen::Report,
            _ => Screen::Battle,
        }
    }

    /// Whether the local player may give orders to unit `i`.
    pub(crate) fn controllable(&self, i: usize) -> bool {
        match self.flow.phase {
            Phase::Free => true,
            Phase::Player if self.flow.reinforcing => false,
            Phase::Player => {
                self.my_turn()
                    && self
                        .map
                        .battle
                        .scenario
                        .units
                        .get(i)
                        .is_some_and(|u| u.owner == self.seats.local)
            }
            _ => false,
        }
    }

    /// Whether it is human `player`'s turn.
    pub(crate) fn turn_of(&self, player: i64) -> bool {
        self.flow.phase == Phase::Player && self.map.battle.clock.player == player
    }

    /// Whether it is the local player's turn.
    pub(crate) fn my_turn(&self) -> bool {
        self.turn_of(self.seats.local)
    }

    /// Definition of unit `i` at its technology level.
    pub(crate) fn kind_of(&self, i: usize) -> Option<&UnitType> {
        let unit = self.map.battle.scenario.units.get(i)?;
        self.defs.unit_tech(unit.kind, unit.tech)
    }

    /// The local player's credits (deployment points).
    pub(crate) fn credits(&self) -> i64 {
        self.credits_of(self.seats.local)
    }

    pub(crate) fn credits_mut(&mut self) -> &mut i64 {
        self.credits_mut_of(self.seats.local)
    }

    /// Human `player`'s credits: the campaign's in the base editor.
    pub(crate) fn credits_of(&self, player: i64) -> i64 {
        if self.base_editor.is_some()
            && let Some(c) = &self.campaign
        {
            return c.state.credits;
        }
        self.map
            .battle
            .players
            .iter()
            .find(|p| p.player == player)
            .map_or(self.map.battle.scenario.info.points, |p| p.deploy_points)
    }

    pub(crate) fn credits_mut_of(&mut self, player: i64) -> &mut i64 {
        if self.base_editor.is_some()
            && let Some(c) = &mut self.campaign
        {
            return &mut c.state.credits;
        }
        let players = &mut self.map.battle.players;
        let i = match players.iter().position(|p| p.player == player) {
            Some(i) => i,
            None => {
                players.push(PlayerRecord {
                    player,
                    ..PlayerRecord::default()
                });
                players.len() - 1
            }
        };
        &mut players[i].deploy_points
    }

    /// Sets every human's credits to `credits`.
    fn credit_humans(&mut self, credits: i64) {
        for h in self.seats.humans().collect::<Vec<_>>() {
            *self.credits_mut_of(h) = credits;
        }
    }

    // Skirmish selector.

    pub(crate) fn open_selector(&mut self) -> Result<(), String> {
        if self.flow.catalog.is_empty() {
            self.flow.catalog = self.game.skirmishes().map_err(|e| e.to_string())?;
        }
        if self.flow.catalog.is_empty() {
            return Err("no skirmishes found (SCENS\\SKIRMISH.MDB is empty)".into());
        }
        self.flow.phase = Phase::Setup;
        self.flow.setup = None;
        self.flow.message = None;
        self.screen = Screen::Skirmishes;
        Ok(())
    }

    pub(crate) fn pick_step(&mut self, step: isize) {
        let last = self.flow.catalog.len().saturating_sub(1);
        self.flow.pick = self.flow.pick.saturating_add_signed(step).min(last);
        self.show_pick();
    }

    /// Scrolls the selector so the selected row is visible.
    pub(crate) fn show_pick(&mut self) {
        let f = &mut self.flow;
        if f.pick < f.scroll {
            f.scroll = f.pick;
        } else if f.pick >= f.scroll + SKIRMISH_ROWS {
            f.scroll = f.pick + 1 - SKIRMISH_ROWS;
        }
    }

    /// Loads the selected skirmish and opens the technology screen.
    pub(crate) fn choose_skirmish(&mut self) {
        let Some(entry) = self.flow.catalog.get(self.flow.pick).cloned() else {
            return;
        };
        let mut battle = match Battle::start(&self.game, &entry.file_name) {
            Ok(battle) => battle,
            Err(err) => {
                self.error = Some(format!("{}: {err}", entry.file_name));
                return;
            }
        };
        battle.scenario.restore(&self.defs);
        self.seats.local = self.seats.first_local();
        let humans = self.seats.humans().count();
        battle.scenario.merge_players(humans);
        if let Some(online) = &mut self.net.online {
            online.started = true;
            self.net.log.clear();
            self.net.desync = None;
        }
        for peer in &self.net.peers {
            peer.link.send(&crate::link::ToClient::Start {
                map: entry.file_name.clone(),
                humans,
                slot: peer.slot,
            });
        }
        if let Some(i) = self
            .maps
            .iter()
            .position(|m| m.eq_ignore_ascii_case(&entry.file_name))
        {
            self.index = i;
        }
        self.options.deployment = false;
        self.show_units = true;
        self.show(entry.file_name.clone(), battle);
        let info = &self.map.battle.scenario.info;
        let units: Vec<i64> = info
            .available_units
            .iter()
            .copied()
            .filter(|&k| self.defs.unit(k).is_some())
            .collect();
        let automated = info.available_automated.clone();
        let points = info
            .tech_points
            .unwrap_or_else(|| self.defs.tech_points(info.points));
        self.flow.setup = Some(Setup {
            tech: vec![0; units.len()],
            units,
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
        self.players = self.seats.overview();
        self.player = 0;
        if points == 0 {
            // Nothing to buy (openlibday's `TechPoints` = 0): straight to the briefing.
            self.begin_briefing();
        } else {
            self.screen = Screen::Tech { read_only: false };
        }
    }

    /// A client told by the host to start skirmish `map` with `humans`
    /// humans, playing `slot`.
    pub(crate) fn join_skirmish(&mut self, map: &str, humans: usize, slot: i64) {
        self.seats = crate::players::Players::online(humans, slot);
        if let Err(e) = self.open_selector() {
            self.error = Some(e);
            return;
        }
        let Some(pick) = self
            .flow
            .catalog
            .iter()
            .position(|e| e.file_name.eq_ignore_ascii_case(map))
        else {
            self.error = Some(format!("the host's skirmish {map} is missing"));
            return;
        };
        self.flow.pick = pick;
        self.show_pick();
        self.choose_skirmish();
    }

    // Technology.

    /// Unit type and level of technology cell `i`.
    pub(crate) fn tech_unit(&self, i: usize) -> Option<(&UnitType, i64)> {
        let setup = self.flow.setup.as_ref()?;
        let (kind, tl) = (*setup.units.get(i)?, setup.tech[i]);
        Some((self.defs.unit_tech(kind, tl)?, tl))
    }

    /// Price of the next technology level of cell `i`, if there is one.
    pub(crate) fn next_tech_cost(&self, i: usize) -> Option<i64> {
        let (kind, tl) = self.tech_unit(i)?;
        let next = self.defs.units.contains_key(&(kind.id, tl + 1));
        (next && kind.cost_to_next > 0).then_some(kind.cost_to_next)
    }

    /// Whether the local player may buy technology: the host buys for
    /// everyone.
    pub(crate) fn can_upgrade(&self) -> bool {
        let Some(setup) = &self.flow.setup else {
            return false;
        };
        if self.net.host.is_some() {
            return false;
        }
        let open = match self.screen {
            Screen::Tech { read_only: false } => true,
            Screen::Army => self.has_research_center(),
            _ => false,
        };
        open && self
            .next_tech_cost(setup.cell)
            .is_some_and(|c| c <= setup.points)
    }

    pub(crate) fn upgrade(&mut self) {
        if !self.can_upgrade() {
            return;
        }
        let cell = self.flow.setup.as_ref().map_or(0, |s| s.cell);
        self.order(Command::Upgrade { cell });
    }

    /// [`Command::Upgrade`].
    pub(crate) fn apply_upgrade(&mut self, cell: usize) -> bool {
        let Some(cost) = self.next_tech_cost(cell) else {
            return false;
        };
        let Some(setup) = self.flow.setup.as_mut().filter(|s| s.points >= cost) else {
            return false;
        };
        setup.points -= cost;
        setup.tech[cell] += 1;
        self.effect(Effect::Cashout);
        self.army_sync();
        true
    }

    /// Moves the technology selection by `step` cells, scrolling the grid
    /// to keep it visible.
    pub(crate) fn tech_step(&mut self, step: isize) {
        if let Some(s) = &mut self.flow.setup
            && let Some((cell, scroll)) = grid_step(
                s.cell,
                s.tech_scroll,
                s.units.len(),
                step,
                TECH_COLUMNS,
                TECH_ROWS,
            )
        {
            (s.cell, s.tech_scroll) = (cell, scroll);
        }
    }

    pub(crate) fn tech_step_ok(&self, step: isize) -> bool {
        self.flow.setup.as_ref().is_some_and(|s| {
            grid_step(s.cell, 0, s.units.len(), step, TECH_COLUMNS, TECH_ROWS).is_some()
        })
    }

    pub(crate) const TECH_ROW: isize = TECH_COLUMNS as isize;

    /// Moves the automated units selection by `step` cells.
    pub(crate) fn auto_step(&mut self, step: isize) {
        if let Some(s) = &mut self.flow.setup
            && let Some((cell, scroll)) = grid_step(
                s.auto_cell,
                s.auto_scroll,
                s.automated.len(),
                step,
                AUTO_COLUMNS,
                AUTO_ROWS,
            )
        {
            (s.auto_cell, s.auto_scroll) = (cell, scroll);
        }
    }

    pub(crate) fn auto_step_ok(&self, step: isize) -> bool {
        self.flow.setup.as_ref().is_some_and(|s| {
            grid_step(
                s.auto_cell,
                0,
                s.automated.len(),
                step,
                AUTO_COLUMNS,
                AUTO_ROWS,
            )
            .is_some()
        })
    }

    pub(crate) const AUTO_ROW: isize = AUTO_COLUMNS as isize;

    // Deployment.

    /// Whether `(x, y)` is in the local player's starting zone.
    pub(crate) fn zone(&self, x: i64, y: i64) -> bool {
        self.zone_of(self.seats.local, x, y)
    }

    /// Starting zones of `player`, or of anyone if the map has none for
    /// them.
    pub(crate) fn zone_of(&self, player: i64, x: i64, y: i64) -> bool {
        let deps = &self.map.battle.scenario.deployments;
        let own = deps.iter().any(|d| d.is_start_zone() && d.player == player);
        deps.iter()
            .any(|d| d.is_start_zone() && (!own || d.player == player) && (d.x, d.y) == (x, y))
    }

    /// [`Command::Brief`].
    pub(crate) fn apply_brief(&mut self) -> bool {
        if self.flow.setup.is_none()
            || matches!(
                self.flow.phase,
                Phase::Deploy | Phase::Player | Phase::Ai | Phase::Over
            )
        {
            return false;
        }
        self.begin_briefing();
        true
    }

    pub(crate) fn begin_briefing(&mut self) {
        self.flow.phase = Phase::Deploy;
        self.flow.ready.clear();
        self.screen = Screen::Briefing;
        self.seats.local = self.seats.first_local();
        let info_points = self.map.battle.scenario.info.points;
        self.credit_humans(info_points);
        self.centre_on_zone();
    }

    pub(crate) fn begin_deploy(&mut self) {
        self.screen = Screen::Deploy;
        self.options.deployment = true;
        self.rebuild();
        self.flow.message = Some("Place your units in the starting zone".into());
    }

    fn centre_on_zone(&mut self) {
        let zone: Vec<(i64, i64)> = self
            .map
            .battle
            .scenario
            .deployments
            .iter()
            .filter(|d| self.zone(d.x, d.y))
            .map(|d| (d.x, d.y))
            .collect();
        if zone.is_empty() {
            return;
        }
        let n = zone.len() as f64;
        let (sx, sy) = zone.iter().fold((0.0, 0.0), |(a, b), &(x, y)| {
            (a + x as f64 + 0.5, b + y as f64 + 0.5)
        });
        self.centre_on_tile(sx / n, sy / n);
    }

    /// Indices into `setup.units` in deployment list order (by price).
    pub(crate) fn deploy_list(&self) -> Vec<usize> {
        let Some(setup) = &self.flow.setup else {
            return Vec::new();
        };
        let mut list: Vec<usize> = (0..setup.units.len()).collect();
        list.sort_by_key(|&i| self.tech_unit(i).map_or(0, |(k, _)| k.cost));
        list
    }

    /// The unit type selected in the deployment list, with its price.
    fn deploy_choice(&self) -> Option<(i64, i64, i64)> {
        let setup = self.flow.setup.as_ref()?;
        let i = *self.deploy_list().get(setup.row)?;
        let (kind, tl) = self.tech_unit(i)?;
        Some((kind.id, tl, self.price(kind.cost)))
    }

    /// What a unit of `cost` costs now: its cost while deploying, then 20%
    /// more per turn as a reinforcement (`0x4363f0`:
    /// `(int)(Cost * (turn * 0.2f + 1))`).
    pub(crate) fn price(&self, cost: i64) -> i64 {
        if !self.flow.reinforcing {
            return cost;
        }
        let turn = self.map.battle.clock.turn as f64;
        (cost as f64 * (turn * f64::from(0.2f32) + 1.0)) as i64
    }

    /// The deployment list row of the unit type named `name`.
    pub(crate) fn deploy_row_named(&self, name: &str) -> Option<usize> {
        self.deploy_list().iter().position(|&i| {
            self.tech_unit(i)
                .is_some_and(|(k, _)| k.name.eq_ignore_ascii_case(name))
        })
    }

    pub(crate) fn deploy_select(&mut self, row: usize) {
        let len = self.deploy_list().len();
        if let Some(setup) = &mut self.flow.setup
            && row < len
        {
            setup.row = row;
        }
    }

    pub(crate) fn deploy_scroll(&mut self, step: isize) {
        let len = self.deploy_list().len();
        if let Some(setup) = &mut self.flow.setup {
            let last = len.saturating_sub(DEPLOY_ROWS);
            setup.scroll = setup.scroll.saturating_add_signed(step).min(last);
        }
    }

    pub(crate) fn deploy_scroll_ok(&self, step: isize) -> bool {
        let len = self.deploy_list().len();
        self.flow.setup.as_ref().is_some_and(|s| {
            if step < 0 {
                s.scroll > 0
            } else {
                s.scroll + DEPLOY_ROWS < len
            }
        })
    }

    /// Whether `player` may place a unit of `kind` with its top corner on
    /// `(x, y)`.
    fn can_deploy(&mut self, player: i64, kind: i64, tech: i64, x: i64, y: i64) -> bool {
        let Some(k) = self.defs.unit_tech(kind, tech) else {
            return false;
        };
        let (w, h) = (k.width, k.height);
        for dy in 0..h {
            for dx in 0..w {
                if !self.zone_of(player, x + dx, y + dy) || self.unit_at(x + dx, y + dy).is_some() {
                    return false;
                }
            }
        }
        let scenario = &mut self.map.battle.scenario;
        let size = scenario.size as i64;
        scenario
            .units
            .push(new_unit(0, kind, tech, player, (x, y), size, k));
        let grid = movement::move_grid(scenario, &self.defs, scenario.units.len() - 1);
        scenario.units.pop();
        // Larger units cannot move, hence have no grid: the zone check is all.
        grid.is_none_or(|g| g.get(x as i32, y as i32).0 == Access::Stop)
    }

    /// `player`'s unit on `(x, y)` that was deployed this phase, hence
    /// recallable.
    pub(crate) fn recallable(&self, player: i64, x: i64, y: i64) -> Option<usize> {
        let i = self.unit_at(x, y)?;
        let u = &self.map.battle.scenario.units[i];
        let setup = self.flow.setup.as_ref()?;
        (u.owner == player && setup.deployed.contains(&u.id)).then_some(i)
    }

    /// Whether `player` may deploy now: everyone's in the deployment phase,
    /// theirs while they reinforce.
    pub(crate) fn deploying(&self, player: i64) -> bool {
        self.seats.human(player)
            && match self.flow.phase {
                Phase::Deploy => !self.flow.ready.contains(&player),
                Phase::Player => self.flow.reinforcing && self.turn_of(player),
                _ => false,
            }
    }

    /// A click on map tile `(x, y)` while deploying (the original's
    /// `0x44C27F`: zone, then recall, then deployment, each failure with its
    /// message and the INVALID sound).
    pub(crate) fn deploy_click(&mut self, x: i64, y: i64) {
        if !self.deploying(self.seats.local) {
            self.refuse("Waiting for the other players");
            return;
        }
        if !self.zone(x, y) {
            self.refuse("You can only deploy units in the indicated zone");
            return;
        }
        if self.recallable(self.seats.local, x, y).is_some() {
            self.order(Command::Recall { x, y });
            return;
        }
        let Some((kind, tech, cost)) = self.deploy_choice() else {
            return;
        };
        if self.defs.unit_tech(kind, tech).is_none() {
            return;
        }
        if cost > self.credits() {
            self.refuse("Need more tactical resource");
            return;
        }
        if !self.can_deploy(self.seats.local, kind, tech, x, y) {
            self.refuse("Can't deploy unit there");
            return;
        }
        self.order(Command::Deploy { x, y, kind, tech });
    }

    /// [`Command::Recall`].
    pub(crate) fn apply_recall(&mut self, player: i64, x: i64, y: i64) -> bool {
        if !self.deploying(player) || !self.zone_of(player, x, y) {
            return false;
        }
        if let Some(i) = self.recallable(player, x, y) {
            let id = self.map.battle.scenario.units[i].id;
            let (refund, name) = self
                .kind_of(i)
                .map_or((0, String::new()), |k| (self.price(k.cost), k.name.clone()));
            self.map.battle.scenario.units.remove(i);
            if let Some(setup) = &mut self.flow.setup {
                setup.deployed.retain(|&d| d != id);
            }
            *self.credits_mut_of(player) += refund;
            if player == self.seats.local {
                self.flow.message = Some(format!("Unit {name} has been recalled"));
                self.effect(Effect::Recall);
            }
            self.deployed_changed();
            return true;
        }
        false
    }

    /// [`Command::Deploy`]: a unit of type `kind` at level `tech`, bought
    /// at its current price.
    pub(crate) fn apply_deploy(
        &mut self,
        player: i64,
        x: i64,
        y: i64,
        kind: i64,
        tech: i64,
    ) -> bool {
        let Some(k) = self.defs.unit_tech(kind, tech) else {
            return false;
        };
        let (name, cost) = (k.name.clone(), self.price(k.cost));
        if !self.deploying(player)
            || cost > self.credits_of(player)
            || !self.can_deploy(player, kind, tech, x, y)
        {
            return false;
        }
        let id = self.map.battle.scenario.next_unit_id();
        let Some(k) = self.defs.unit_tech(kind, tech) else {
            return false;
        };
        let size = self.map.battle.scenario.size as i64;
        let unit = new_unit(id, kind, tech, player, (x, y), size, k);
        self.map.battle.scenario.units.push(unit);
        if let Some(setup) = &mut self.flow.setup {
            setup.deployed.push(id);
        }
        *self.credits_mut_of(player) -= cost;
        if player == self.seats.local {
            self.flow.message = Some(format!("Unit {name} has been deployed"));
            self.effect(Effect::Deploy);
        }
        self.deployed_changed();
        true
    }

    /// Shows why an action was refused, with the INVALID sound.
    fn refuse(&mut self, message: &str) {
        self.flow.message = Some(message.into());
        self.effect(Effect::Invalid);
    }

    fn deployed_changed(&mut self) {
        self.rebuild();
        self.markers = libday_view::minimap::markers(&self.map.battle.scenario, &self.defs);
        self.players = self.seats.overview();
        self.selected = None;
    }

    /// CONTINUE on the deployment screen.
    pub(crate) fn finish_deploy(&mut self) {
        if self.flow.reinforcing {
            self.order(Command::FinishReinforce);
        } else if self.credits() > 0 {
            self.screen = Screen::Confirm;
        } else {
            self.deploy_done();
        }
    }

    /// The local player has deployed: the next human at this computer
    /// deploys (hotseat); the battle starts once every human has.
    pub(crate) fn deploy_done(&mut self) {
        let me = self.seats.local;
        if !self.deploying(me) {
            return;
        }
        self.order(Command::Ready);
        if self.flow.phase != Phase::Deploy {
            return;
        }
        let next = self
            .seats
            .humans()
            .find(|&h| h != me && self.seats.is_local(h) && !self.flow.ready.contains(&h));
        let Some(next) = next else {
            self.screen = Screen::Deploy;
            self.flow.message = Some("Waiting for the other players".into());
            return;
        };
        self.seats.local = next;
        if let Some(setup) = &mut self.flow.setup {
            setup.row = 0;
            setup.scroll = 0;
        }
        self.screen = Screen::Deploy;
        self.centre_on_zone();
        self.flow.message = Some(format!(
            "{}: place your units in the starting zone",
            self.player_name(next)
        ));
    }

    /// [`Command::Ready`]: `player` is done deploying; the battle starts
    /// once every human is.
    pub(crate) fn apply_ready(&mut self, player: i64) -> bool {
        if self.flow.phase != Phase::Deploy || !self.deploying(player) {
            return false;
        }
        self.flow.ready.insert(player);
        let units = &self.map.battle.scenario.units;
        if let Some(setup) = &mut self.flow.setup {
            setup
                .deployed
                .retain(|id| !units.iter().any(|u| u.id == *id && u.owner == player));
        }
        if self.seats.humans().all(|h| self.flow.ready.contains(&h)) {
            self.apply_start_battle();
        }
        true
    }

    // Reinforcements.

    /// Whether REINFORCEMENT opens the deployment screen.
    pub(crate) fn can_reinforce(&self) -> bool {
        self.can_reinforce_for(self.seats.local)
    }

    fn can_reinforce_for(&self, player: i64) -> bool {
        self.turn_of(player) && !self.flow.reinforcing && self.flow.setup.is_some() && !self.busy()
    }

    /// REINFORCEMENT: the deployment screen over the battle, where units
    /// bought with the turn's credits are placed and recalled until
    /// CONTINUE.
    pub(crate) fn begin_reinforce(&mut self) {
        if self.can_reinforce() {
            self.order(Command::BeginReinforce);
        }
    }

    /// [`Command::BeginReinforce`].
    pub(crate) fn apply_begin_reinforce(&mut self, player: i64) -> bool {
        if !self.can_reinforce_for(player) {
            return false;
        }
        self.flow.reinforcing = true;
        if let Some(setup) = &mut self.flow.setup {
            setup.deployed.clear();
        }
        if player != self.seats.local {
            return true;
        }
        self.flow.resume = self.selected;
        self.preview = None;
        self.lock = None;
        self.fire_mode = None;
        self.selected = None;
        self.screen = Screen::Deploy;
        self.options.deployment = true;
        self.rebuild();
        self.flow.message = None;
        true
    }

    /// CONTINUE while reinforcing ([`Command::FinishReinforce`]): the placed
    /// units join the battle with no action points left this turn.
    pub(crate) fn apply_finish_reinforce(&mut self, player: i64) -> bool {
        if !self.flow.reinforcing || !self.turn_of(player) {
            return false;
        }
        let placed = self
            .flow
            .setup
            .as_mut()
            .map(|s| std::mem::take(&mut s.deployed))
            .unwrap_or_default();
        for unit in &mut self.map.battle.scenario.units {
            if placed.contains(&unit.id) {
                unit.action_points = Some(0);
            }
        }
        self.flow.reinforcing = false;
        if player == self.seats.local {
            self.options.deployment = false;
            self.selected = self.flow.resume.take();
            self.screen = Screen::Battle;
        }
        self.rebuild();
        true
    }

    // Turns.

    /// [`Command::StartBattle`].
    pub(crate) fn apply_start_battle(&mut self) {
        if !matches!(self.flow.phase, Phase::Setup | Phase::Deploy) {
            return;
        }
        self.flow.phase = Phase::Player;
        self.flow.reinforcing = false;
        self.flow.used_once.clear();
        self.flow.ai_budget = self.ai_support_budget();
        self.flow.tech.clear();
        if let Some(setup) = &mut self.flow.setup {
            setup.deployed.clear();
        }
        self.options.deployment = false;
        let reinf = self.map.battle.scenario.info.reinf_res;
        self.credit_humans(reinf);
        self.map.battle.clock.turn = 1;
        self.rebuild();
        self.screen = Screen::Battle;
        self.begin_player_turn(self.seats.first_human());
        self.flow.message = Some("The battle has begun".into());
        if self.defending_base() {
            self.ai_first();
        }
    }

    /// Human `player`'s turn begins (handed to them if at this computer).
    pub(crate) fn begin_player_turn(&mut self, player: i64) {
        self.flow.phase = Phase::Player;
        self.map.battle.clock.player = player;
        self.seats.hand_over(player);
        let local = self.seats.is_local(player);
        if !local && self.screen == Screen::Battle {
            self.screen = Screen::AiTurn;
        } else if local && self.screen == Screen::AiTurn {
            self.screen = Screen::Battle;
        }
        self.selected = self
            .map
            .battle
            .scenario
            .units
            .iter()
            .position(|u| u.owner == player && u.arrival == 0)
            .filter(|_| self.my_turn());
        self.preview = None;
        self.lock = None;
        self.fire_mode = None;
        self.started = self.clock.now();
        self.centre_on_selected();
    }

    pub(crate) fn can_end_turn(&self) -> bool {
        self.can_end_turn_for(self.seats.local)
    }

    fn can_end_turn_for(&self, player: i64) -> bool {
        self.turn_of(player) && !self.flow.reinforcing && self.motions.is_empty()
    }

    pub(crate) fn end_turn(&mut self) {
        if self.can_end_turn() {
            self.order(Command::EndTurn);
        }
    }

    /// [`Command::EndTurn`]: the next human's turn begins, or the AI's
    /// after the last.
    pub(crate) fn apply_end_turn(&mut self, player: i64) -> bool {
        if !self.can_end_turn_for(player) {
            return false;
        }
        if let Some(next) = self.seats.next_human(player) {
            if matches!(self.screen, Screen::Automated) {
                self.screen = Screen::Battle;
            }
            self.flow.support_row = None;
            self.flow.pickup = None;
            self.begin_player_turn(next);
            self.flow.message = Some(format!("{}'s turn has begun", self.player_name(next)));
            self.rebuild();
            return true;
        }
        self.flow.phase = Phase::Ai;
        self.map.battle.clock.player = AI;
        self.screen = Screen::AiTurn;
        self.selected = None;
        self.preview = None;
        self.lock = None;
        self.fire_mode = None;
        self.started = self.clock.now();
        self.flow.message = Some(format!("{}'s turn has begun", self.player_name(AI)));
        self.flow.ai_until = Some(self.started + AI_TURN);
        for unit in &mut self.map.battle.scenario.units {
            if !self.seats.human(unit.owner)
                && let Some(k) = self.defs.unit_tech(unit.kind, unit.tech)
            {
                unit.restore_ap(k.action_points);
            }
        }
        // Only the copy that runs the AI thinks; the others follow.
        if !self.net.follows() {
            self.brain = self.ai_mode.map(crate::ai::Brain::new);
        }
        // The restored AP undims the AI's units.
        self.rebuild();
        true
    }

    /// Plays the AI's turn, then ends it once its time is up and nothing
    /// is playing; returns whether anything changed.
    pub(crate) fn ai_tick(&mut self) -> bool {
        let Some(until) = self.flow.ai_until else {
            return false;
        };
        if self.net.follows() {
            return false;
        }
        if self.launch_authored_support() {
            return true;
        }
        if let Some(acted) = self.run_brain() {
            return acted;
        }
        if self.busy() || self.clock.now() < until {
            return false;
        }
        self.order_ai(Command::EndAiTurn);
        true
    }

    /// [`Command::EndAiTurn`]: the human's turn begins.
    pub(crate) fn apply_end_ai_turn(&mut self) {
        if self.flow.ai_until.is_none() {
            return;
        }
        self.flow.ai_until = None;
        self.map.battle.clock.turn += 1;
        let reinf = self.map.battle.scenario.info.reinf_res;
        for h in self.seats.humans().collect::<Vec<_>>() {
            *self.credits_mut_of(h) += reinf;
        }
        for unit in &mut self.map.battle.scenario.units {
            if self.seats.human(unit.owner)
                && let Some(k) = self.defs.unit_tech(unit.kind, unit.tech)
            {
                unit.restore_ap(k.action_points);
            }
        }
        if matches!(self.screen, Screen::AiTurn) {
            self.screen = Screen::Battle;
        }
        let first = self.seats.first_human();
        self.begin_player_turn(first);
        self.flow.message = Some(format!("{}'s turn has begun", self.player_name(first)));
        self.arrive_reinforcements();
        self.rebuild();
    }

    /// The turn flow, technology capture, then the objective: returns
    /// whether anything changed.
    pub(crate) fn battle_tick(&mut self) -> bool {
        let received = self.pump();
        let turned = self.ai_tick() || received;
        let captured = self.capture_tech();
        self.judge() || turned || captured
    }

    /// Re-judges which technology buildings the player's units stand next
    /// to (`0x41ae03`) while nothing plays; a new capture lights its bulb
    /// with the `tech` sound (`CIsoAnimTech`). Returns whether any changed.
    pub(crate) fn capture_tech(&mut self) -> bool {
        if !matches!(self.flow.phase, Phase::Free | Phase::Player | Phase::Ai) || self.busy() {
            return false;
        }
        let now = libday_view::tech::captured(&self.map.battle.scenario, &self.defs);
        if now == self.flow.tech {
            return false;
        }
        if now.difference(&self.flow.tech).next().is_some() {
            self.effect(Effect::Tech);
        }
        self.flow.tech = now;
        true
    }

    /// Technology found: the rewards of the buildings captured now.
    pub(crate) fn tech_found(&self) -> i64 {
        libday_view::tech::total(&self.map.battle.scenario, &self.defs, &self.flow.tech)
    }

    /// Reinforcements due by now enter the map (`0x416576`, after the AI's
    /// phase): each on its own tile, or later once that is free.
    fn arrive_reinforcements(&mut self) {
        let turn = self.map.battle.clock.turn;
        let due: Vec<usize> = self
            .map
            .battle
            .scenario
            .units
            .iter()
            .enumerate()
            .filter(|(_, u)| u.reinforcement && u.arrival > 0 && u.arrival <= turn)
            .map(|(i, _)| i)
            .collect();
        let mut arrived = Vec::new();
        for i in due {
            let s = &self.map.battle.scenario;
            let u = &s.units[i];
            let Some(k) = self.defs.unit(u.kind) else {
                continue;
            };
            if !self.reinforcement_fits(u.id, (u.x, u.y), (k.width, k.height)) {
                continue;
            }
            let size = s.size as i64;
            let (armor, facings, name) = (k.armor, k.facings, k.name.clone());
            let u = &mut self.map.battle.scenario.units[i];
            u.arrival = 0;
            u.owner = AI;
            u.tech = 0;
            u.armor = Some(armor);
            // Placed with its AP spent (`0x434179`): it acts next turn.
            u.action_points = Some(0);
            u.neg_ap = 0;
            if facings {
                u.facing = libday_data::scenario::deploy_facing(size, u.x, u.y);
            }
            let (id, ordered) = (u.id, u.objective_order);
            if ordered {
                self.objective_order(id);
            }
            arrived.push(name);
        }
        // The arrival notice (`0x4542c5`): string 896 in the status bar
        // and the `reinf` sound.
        if !arrived.is_empty() {
            let text = self.string(896, "%1 reinforcements have arrived");
            self.flow.message = Some(text.replace("%1", &self.player_name(AI)));
            self.effect(Effect::Reinf);
            self.rebuild();
        }
    }

    /// Whether a reinforcement `id` of footprint `(w, h)` can enter on
    /// `(x, y)` (`0x4405d4`): on the map, its id unused, no unit or
    /// building in the way.
    fn reinforcement_fits(&self, id: i64, (x, y): (i64, i64), (w, h): (i64, i64)) -> bool {
        let s = &self.map.battle.scenario;
        let size = s.size as i64;
        if x < 0 || y < 0 || x + w > size || y + h > size {
            return false;
        }
        let overlaps = |(bx, by, bw, bh): (i64, i64, i64, i64)| {
            bx < x + w && x < bx + bw && by < y + h && y < by + bh
        };
        let units = s.units.iter().filter(|u| u.arrival == 0).any(|u| {
            let (uw, uh) = self
                .defs
                .unit(u.kind)
                .map_or((1, 1), |k| (k.width, k.height));
            u.id == id || overlaps((u.x, u.y, uw, uh))
        });
        let buildings = s.buildings.iter().any(|b| {
            let (bw, bh) = self
                .defs
                .buildings
                .get(&b.kind)
                .map_or((1, 1), |k| (k.width, k.height));
            b.id == id || overlaps((b.x, b.y, bw, bh))
        });
        !units && !buildings
    }

    /// Gives arrived reinforcement `id` the order serving the scenario's
    /// objective, with its targets (`0x416576`).
    fn objective_order(&mut self, id: i64) {
        let s = &mut self.map.battle.scenario;
        let Some(o) = &s.objective else {
            return;
        };
        let ids = || o.ids.iter().map(|&i| Target::Id(i)).collect::<Vec<_>>();
        let tiles = || {
            o.tiles
                .iter()
                .map(|&(x, y)| Target::Position { x, y })
                .collect::<Vec<_>>()
        };
        use objective::kind;
        let (order, targets) = match o.kind {
            kind::DESTROY if o.ids.is_empty() => (5, tiles()),
            kind::DESTROY | kind::INTERCEPT => (3, ids()),
            kind::HOLD | kind::PROTECT => (5, tiles()),
            kind::ESCORT | kind::KEEP => (6, ids()),
            _ => return,
        };
        let base = s.order_targets.iter().map(|t| t.counter).max().unwrap_or(0);
        s.order_targets.retain(|t| t.unit != id);
        for (counter, target) in (base + 1..).zip(targets) {
            s.order_targets.push(OrderTarget {
                counter,
                unit: id,
                target,
            });
        }
        if let Some(u) = s.units.iter_mut().find(|u| u.id == id) {
            u.order.id = order;
            u.order.expiry = 99;
            u.order.probability = 100;
        }
    }

    /// The original's turn counter, which advances when the AI's phase
    /// begins.
    fn objective_turn(&self) -> i64 {
        let turn = self.map.battle.clock.turn;
        if self.flow.phase == Phase::Ai {
            turn + 1
        } else {
            turn
        }
    }

    /// Judges the objective while the battle is on and nothing plays,
    /// ending the battle on an outcome; returns whether it ended.
    pub(crate) fn judge(&mut self) -> bool {
        if !matches!(self.flow.phase, Phase::Player | Phase::Ai)
            || self.flow.reinforcing
            || self.busy()
        {
            return false;
        }
        let turn = self.objective_turn();
        let none = libday_data::scenario::Objective {
            kind: -1,
            ..Default::default()
        };
        let battle = &mut self.map.battle;
        let o = battle.scenario.objective.as_ref().unwrap_or(&none);
        let outcome = objective::check(
            o,
            &battle.scenario,
            &self.defs,
            &mut battle.clock,
            turn,
            self.flow.forced,
        );
        match self.campaign_outcome(outcome) {
            Some(o) => {
                self.end_battle(o);
                true
            }
            None => false,
        }
    }

    /// The battle is over: the battle report shows.
    pub(crate) fn end_battle(&mut self, outcome: Outcome) {
        // The final judgement of the technology found (`0x41b07a`).
        self.flow.tech = libday_view::tech::captured(&self.map.battle.scenario, &self.defs);
        self.flow.phase = Phase::Over;
        self.flow.reinforcing = false;
        self.options.deployment = false;
        self.flow.outcome = Some(outcome);
        self.flow.ai_until = None;
        self.brain = None;
        self.selected = None;
        self.preview = None;
        self.lock = None;
        self.fire_mode = None;
        self.flow.report = Some(self.report(outcome));
        self.screen = Screen::Report;
    }

    /// The battle report's text (`0x45ded0`).
    fn report(&self, outcome: Outcome) -> Report {
        let strings = self.game.strings().unwrap_or_default();
        let s = |id: u32, fallback: &str| {
            strings
                .get(&id)
                .cloned()
                .unwrap_or_else(|| fallback.to_owned())
        };
        let count =
            |id: u32, fallback: &str, n: i64| s(id, fallback).replace("%1!d!", &n.to_string());
        let record = self
            .map
            .battle
            .players
            .iter()
            .find(|p| p.player == self.seats.local)
            .cloned()
            .unwrap_or_default();
        let [ud, ul, bd, bl] = record.totals();
        let units = |n| count(0x204, "%1!d! Units", n);
        let buildings = |n| count(0x205, "%1!d! Buildings", n);
        Report {
            title: s(0x201, "BATTLE REPORT"),
            message: match outcome {
                Outcome::Won => s(0x207, "Congratulations ! You have defeated the enemy."),
                Outcome::Lost => s(
                    0x208,
                    "You have failed this scenario. You will need to try again.",
                ),
            },
            headers: [
                s(0xb4, "PLAYER"),
                s(0x200, "DESTROYED"),
                s(0x203, "LOST"),
                s(0x202, "RANK"),
            ],
            rows: [
                [
                    self.player_name(self.seats.local),
                    units(ud),
                    units(ul),
                    record.rank(&self.defs).to_string(),
                ],
                [String::new(), buildings(bd), buildings(bl), String::new()],
            ],
            tech: count(
                0x206,
                "New technologies found : %1!d!",
                self.campaign_tech_reward()
                    .filter(|_| outcome == Outcome::Won)
                    .unwrap_or_else(|| self.tech_found()),
            ),
        }
    }

    /// CONTINUE on the battle report: back to the skirmish selector, or on
    /// with the campaign.
    pub(crate) fn close_report(&mut self) {
        if self.campaign_battle().is_some() {
            self.campaign_report_closed();
            self.flow.outcome = None;
            self.flow.forced = None;
            self.flow.report = None;
            return;
        }
        self.flow.outcome = None;
        self.flow.forced = None;
        self.flow.report = None;
        if self.online() {
            self.open_lobby();
            return;
        }
        if let Err(e) = self.open_selector() {
            eprintln!("{e}");
            self.quit = true;
        }
    }
}

/// The battle report's text.
pub struct Report {
    pub title: String,
    pub message: String,
    /// PLAYER, DESTROYED, LOST, RANK.
    pub headers: [String; 4],
    /// The player's units row (with the rank) and buildings row.
    pub rows: [[String; 4]; 2],
    /// Technology found, under the rows (`0x45e4cd`; the original shows it
    /// in campaigns only).
    pub tech: String,
}

/// A freshly deployed unit on `(x, y)` of a `size`-tile map, facing the
/// centre when its type has facings.
fn new_unit(
    id: i64,
    kind: i64,
    tech: i64,
    owner: i64,
    (x, y): (i64, i64),
    size: i64,
    k: &UnitType,
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
        tech,
        owner,
        facing,
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

/// Moves a selection `cell` of a `len`-item grid of `columns` by `step`,
/// returning the new cell and first visible row (of `rows`) or `None` when
/// the move leaves the grid.
fn grid_step(
    cell: usize,
    scroll: usize,
    len: usize,
    step: isize,
    columns: usize,
    rows: usize,
) -> Option<(usize, usize)> {
    let cell = cell.checked_add_signed(step).filter(|&c| c < len)?;
    let row = cell / columns;
    let scroll = scroll.min(row).max((row + 1).saturating_sub(rows));
    Some((cell, scroll))
}

#[cfg(test)]
mod grid_tests {
    use super::grid_step;

    #[test]
    fn grid_scrolls_to_keep_selection_visible() {
        assert_eq!(grid_step(20, 0, 36, 4, 4, 6), Some((24, 1)));
        assert_eq!(grid_step(24, 1, 36, 4, 4, 6), Some((28, 2)));
        assert_eq!(grid_step(28, 2, 36, -4, 4, 6), Some((24, 2)));
        assert_eq!(grid_step(8, 2, 36, -4, 4, 6), Some((4, 1)));
        assert_eq!(grid_step(33, 3, 36, 4, 4, 6), None);
        assert_eq!(grid_step(2, 0, 5, 2, 2, 2), Some((4, 1)));
    }
}
