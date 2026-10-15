//! Skirmish mode: the selector, technology upgrades, deployment and the turn
//! rotation between the player and the (still passive) AI.

use std::time::Duration;

use libday_data::Battle;
use libday_data::battle::PlayerRecord;
use libday_data::defs::UnitType;
use libday_data::scenario::{Order, SkirmishEntry, Unit};
use libday_view::movement::{self, Access};
use libday_view::ui::{
    AUTO_COLUMNS, AUTO_ROWS, DEPLOY_ROWS, SKIRMISH_ROWS, Screen, TECH_COLUMNS, TECH_ROWS,
};

use crate::Viewer;
use crate::sound::Effect;

/// The human player's slot.
pub const PLAYER: i64 = 1;
/// The AI's player slot.
pub const AI: i64 = 0;
/// How long the AI's (empty) turn stays on screen.
const AI_TURN: Duration = Duration::from_millis(1500);

/// The overview's players in a skirmish: the human, then the AI.
pub(crate) fn skirmish_players() -> Vec<i64> {
    vec![PLAYER, AI]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Map viewer: every unit can be moved, no turns.
    Free,
    /// Skirmish selector and technology upgrades.
    Setup,
    /// Starting zone overview and unit placement.
    Deploy,
    /// The player's turn.
    Player,
    /// The AI's turn.
    Ai,
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
    /// Units placed during deployment (ids), which may be recalled.
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
        }
    }
}

impl Viewer {
    /// The screen a closed overview or goal dialog returns to.
    pub(crate) fn home(&self) -> Screen {
        match self.flow.phase {
            Phase::Deploy => Screen::Deploy,
            Phase::Ai => Screen::AiTurn,
            _ => Screen::Battle,
        }
    }

    /// Whether the player may give orders to unit `i`.
    pub(crate) fn controllable(&self, i: usize) -> bool {
        match self.flow.phase {
            Phase::Free => true,
            Phase::Player => self
                .map
                .battle
                .scenario
                .units
                .get(i)
                .is_some_and(|u| u.owner == PLAYER),
            _ => false,
        }
    }

    /// Definition of unit `i` at its technology level.
    pub(crate) fn kind_of(&self, i: usize) -> Option<&UnitType> {
        let unit = self.map.battle.scenario.units.get(i)?;
        self.defs.unit_tech(unit.kind, unit.tech)
    }

    /// The player's credits (deployment points).
    pub(crate) fn credits(&self) -> i64 {
        self.map
            .battle
            .players
            .iter()
            .find(|p| p.player == PLAYER)
            .map_or(self.map.battle.scenario.info.points, |p| p.deploy_points)
    }

    fn credits_mut(&mut self) -> &mut i64 {
        let players = &mut self.map.battle.players;
        let i = match players.iter().position(|p| p.player == PLAYER) {
            Some(i) => i,
            None => {
                players.push(PlayerRecord {
                    player: PLAYER,
                    ..PlayerRecord::default()
                });
                players.len() - 1
            }
        };
        &mut players[i].deploy_points
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
        let humans = skirmish_players().iter().filter(|&&p| p != AI).count();
        battle.scenario.merge_players(humans);
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
        let points = self.defs.tech_points(info.points);
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
        self.players = skirmish_players();
        self.player = 0;
        self.screen = Screen::Tech { read_only: false };
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

    pub(crate) fn can_upgrade(&self) -> bool {
        let Some(setup) = &self.flow.setup else {
            return false;
        };
        self.screen == (Screen::Tech { read_only: false })
            && self
                .next_tech_cost(setup.cell)
                .is_some_and(|c| c <= setup.points)
    }

    pub(crate) fn upgrade(&mut self) {
        if !self.can_upgrade() {
            return;
        }
        let cell = self.flow.setup.as_ref().map_or(0, |s| s.cell);
        let Some(cost) = self.next_tech_cost(cell) else {
            return;
        };
        if let Some(setup) = &mut self.flow.setup {
            setup.points -= cost;
            setup.tech[cell] += 1;
        }
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

    /// Starting zones of the player, or of anyone if the map has none for
    /// player 1.
    pub(crate) fn zone(&self, x: i64, y: i64) -> bool {
        let deps = &self.map.battle.scenario.deployments;
        let own = deps.iter().any(|d| d.is_start_zone() && d.player == PLAYER);
        deps.iter()
            .any(|d| d.is_start_zone() && (!own || d.player == PLAYER) && (d.x, d.y) == (x, y))
    }

    pub(crate) fn begin_briefing(&mut self) {
        self.flow.phase = Phase::Deploy;
        self.screen = Screen::Briefing;
        let info_points = self.map.battle.scenario.info.points;
        *self.credits_mut() = info_points;
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

    /// The unit type selected in the deployment list.
    fn deploy_choice(&self) -> Option<(i64, i64, i64)> {
        let setup = self.flow.setup.as_ref()?;
        let i = *self.deploy_list().get(setup.row)?;
        let (kind, tl) = self.tech_unit(i)?;
        Some((kind.id, tl, kind.cost))
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

    /// Whether a unit of `kind` may be placed with its top corner on `(x, y)`.
    fn can_deploy(&mut self, kind: i64, tech: i64, x: i64, y: i64) -> bool {
        let Some(k) = self.defs.unit_tech(kind, tech) else {
            return false;
        };
        let (w, h) = (k.width, k.height);
        for dy in 0..h {
            for dx in 0..w {
                if !self.zone(x + dx, y + dy) || self.unit_at(x + dx, y + dy).is_some() {
                    return false;
                }
            }
        }
        let scenario = &mut self.map.battle.scenario;
        let size = scenario.size as i64;
        scenario
            .units
            .push(new_unit(0, kind, tech, (x, y), size, k));
        let grid = movement::move_grid(scenario, &self.defs, scenario.units.len() - 1);
        scenario.units.pop();
        // Larger units cannot move, hence have no grid: the zone check is all.
        grid.is_none_or(|g| g.get(x as i32, y as i32).0 == Access::Stop)
    }

    /// The unit on `(x, y)` that was deployed this phase, hence recallable.
    pub(crate) fn recallable(&self, x: i64, y: i64) -> Option<usize> {
        let i = self.unit_at(x, y)?;
        let id = self.map.battle.scenario.units[i].id;
        let setup = self.flow.setup.as_ref()?;
        setup.deployed.contains(&id).then_some(i)
    }

    /// A click on map tile `(x, y)` while deploying (the original's
    /// `0x44C27F`: zone, then recall, then deployment, each failure with its
    /// message and the INVALID sound).
    pub(crate) fn deploy_click(&mut self, x: i64, y: i64) {
        if !self.zone(x, y) {
            self.refuse("You can only deploy units in the indicated zone");
            return;
        }
        if let Some(i) = self.recallable(x, y) {
            let id = self.map.battle.scenario.units[i].id;
            let (refund, name) = self
                .kind_of(i)
                .map_or((0, String::new()), |k| (k.cost, k.name.clone()));
            self.map.battle.scenario.units.remove(i);
            if let Some(setup) = &mut self.flow.setup {
                setup.deployed.retain(|&d| d != id);
            }
            *self.credits_mut() += refund;
            self.flow.message = Some(format!("Unit {name} has been recalled"));
            self.effect(Effect::Recall);
            self.deployed_changed();
            return;
        }
        let Some((kind, tech, cost)) = self.deploy_choice() else {
            return;
        };
        let Some(name) = self.defs.unit_tech(kind, tech).map(|k| k.name.clone()) else {
            return;
        };
        if cost > self.credits() {
            self.refuse("Need more tactical resource");
            return;
        }
        if !self.can_deploy(kind, tech, x, y) {
            self.refuse("Can't deploy unit there");
            return;
        }
        let id = self.map.battle.scenario.next_unit_id();
        let Some(k) = self.defs.unit_tech(kind, tech) else {
            return;
        };
        let size = self.map.battle.scenario.size as i64;
        let unit = new_unit(id, kind, tech, (x, y), size, k);
        self.map.battle.scenario.units.push(unit);
        if let Some(setup) = &mut self.flow.setup {
            setup.deployed.push(id);
        }
        *self.credits_mut() -= cost;
        self.flow.message = Some(format!("Unit {name} has been deployed"));
        self.effect(Effect::Deploy);
        self.deployed_changed();
    }

    /// Shows why an action was refused, with the INVALID sound.
    fn refuse(&mut self, message: &str) {
        self.flow.message = Some(message.into());
        self.effect(Effect::Invalid);
    }

    fn deployed_changed(&mut self) {
        self.rebuild();
        self.markers = libday_view::minimap::markers(&self.map.battle.scenario, &self.defs);
        self.players = skirmish_players();
        self.selected = None;
    }

    /// CONTINUE on the deployment screen.
    pub(crate) fn finish_deploy(&mut self) {
        if self.credits() > 0 {
            self.screen = Screen::Confirm;
        } else {
            self.start_battle();
        }
    }

    // Turns.

    pub(crate) fn start_battle(&mut self) {
        self.flow.phase = Phase::Player;
        self.options.deployment = false;
        let reinf = self.map.battle.scenario.info.reinf_res;
        *self.credits_mut() = reinf;
        self.map.battle.clock.turn = 1;
        self.map.battle.clock.player = PLAYER;
        self.rebuild();
        self.screen = Screen::Battle;
        self.begin_player_turn();
        self.flow.message = Some("The battle has begun".into());
    }

    fn begin_player_turn(&mut self) {
        self.flow.phase = Phase::Player;
        self.map.battle.clock.player = PLAYER;
        self.selected = self
            .map
            .battle
            .scenario
            .units
            .iter()
            .position(|u| u.owner == PLAYER && u.arrival == 0);
        self.preview = None;
        self.lock = None;
        self.fire_mode = None;
        self.started = self.clock.now();
        self.centre_on_selected();
    }

    pub(crate) fn can_end_turn(&self) -> bool {
        self.flow.phase == Phase::Player && self.motion.is_none()
    }

    pub(crate) fn end_turn(&mut self) {
        if !self.can_end_turn() {
            return;
        }
        self.flow.phase = Phase::Ai;
        self.map.battle.clock.player = AI;
        self.screen = Screen::AiTurn;
        self.selected = None;
        self.preview = None;
        self.lock = None;
        self.fire_mode = None;
        self.started = self.clock.now();
        self.flow.message = Some(format!(
            "{}'s turn has begun",
            libday_view::minimap::player_name(AI)
        ));
        self.flow.ai_until = Some(self.started + AI_TURN);
    }

    /// Ends the AI's turn once its time is up; returns whether it did.
    pub(crate) fn ai_tick(&mut self) -> bool {
        match self.flow.ai_until {
            Some(until) if self.clock.now() >= until => {}
            _ => return false,
        }
        self.flow.ai_until = None;
        self.map.battle.clock.turn += 1;
        let reinf = self.map.battle.scenario.info.reinf_res;
        *self.credits_mut() += reinf;
        for unit in &mut self.map.battle.scenario.units {
            if let Some(k) = self.defs.unit_tech(unit.kind, unit.tech) {
                unit.restore_ap(k.action_points);
            }
        }
        if matches!(self.screen, Screen::AiTurn) {
            self.screen = Screen::Battle;
        }
        self.begin_player_turn();
        self.flow.message = Some(format!(
            "{}'s turn has begun",
            libday_view::minimap::player_name(PLAYER)
        ));
        true
    }
}

/// A freshly deployed unit on `(x, y)` of a `size`-tile map, facing the
/// centre when its type has facings.
fn new_unit(id: i64, kind: i64, tech: i64, (x, y): (i64, i64), size: i64, k: &UnitType) -> Unit {
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
        owner: PLAYER,
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
