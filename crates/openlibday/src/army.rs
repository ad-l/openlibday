//! ARMY: spending the campaign's technology points on unit levels, in base
//! building (campaign battles have no technology dialog).

use libday_view::ui::Screen;

use crate::Viewer;
use crate::skirmish::Setup;

/// Research Ctr (`H_Center_Laboratory`).
const RESEARCH_CENTER: i64 = 135;

impl Viewer {
    /// The technology grid for the units the base's buildings provide the
    /// local player, at their levels and with their points.
    fn army_setup(&self) -> Option<Setup> {
        let session = self.campaign.as_ref()?;
        let me = session.state.member(self.seats.local)?;
        let owned = self.owned_buildings(&self.map.battle.scenario, self.seats.local);
        let units = self.campaign_units(&owned);
        let automated = self.campaign_automated(&owned);
        Some(Setup {
            tech: units
                .iter()
                .map(|k| me.unit_tech.get(k).copied().unwrap_or(0))
                .collect(),
            units,
            automated,
            points: me.tech,
            cell: 0,
            tech_scroll: 0,
            auto_cell: 0,
            auto_scroll: 0,
            row: 0,
            scroll: 0,
            deployed: Vec::new(),
        })
    }

    /// ARMY upgrades units only with a Research Ctr in the base.
    pub(crate) fn has_research_center(&self) -> bool {
        self.owned_buildings(&self.map.battle.scenario, self.seats.local)
            .contains(&RESEARCH_CENTER)
    }

    /// ARMY in base building.
    pub(crate) fn open_army(&mut self) {
        let Some(setup) = self.army_setup() else {
            return;
        };
        self.flow.setup = Some(setup);
        self.flow.message = None;
        self.army_view += 1;
        self.screen = Screen::Army;
    }

    /// [`Command::ArmyUpgrade`]: `player` buys the next level of unit type
    /// `kind` with their technology points.
    pub(crate) fn apply_army_upgrade(&mut self, player: i64, kind: i64) -> bool {
        let Some((base, _)) = self.base_now() else {
            return false;
        };
        let owned = self.owned_buildings(&base, player);
        if !owned.contains(&RESEARCH_CENTER) || !self.campaign_units(&owned).contains(&kind) {
            return false;
        }
        let Some(m) = self.campaign.as_ref().and_then(|c| c.state.member(player)) else {
            return false;
        };
        let level = m.unit_tech.get(&kind).copied().unwrap_or(0);
        let Some(cost) = self
            .defs
            .unit_tech(kind, level)
            .map(|k| k.cost_to_next)
            .filter(|&c| c > 0 && c <= m.tech && self.defs.units.contains_key(&(kind, level + 1)))
        else {
            return false;
        };
        if let Some(m) = self
            .campaign
            .as_mut()
            .and_then(|c| c.state.member_mut(player))
        {
            m.tech -= cost;
            m.unit_tech.insert(kind, level + 1);
        }
        if player == self.seats.local {
            self.effect(crate::sound::Effect::Cashout);
            if self.screen == Screen::Army
                && let (Some(old), Some(mut setup)) = (self.flow.setup.take(), self.army_setup())
            {
                setup.cell = old.cell.min(setup.units.len().saturating_sub(1));
                setup.tech_scroll = old.tech_scroll;
                setup.auto_cell = old.auto_cell;
                setup.auto_scroll = old.auto_scroll;
                self.flow.setup = Some(setup);
            }
        }
        true
    }
    /// BACK: from ARMY to base building, else from the read-only dialog to
    /// deployment.
    pub(crate) fn tech_back(&mut self) {
        if self.screen == Screen::Army {
            self.flow.setup = None;
            self.save_campaign();
            self.screen = Screen::BaseEditor;
        } else {
            self.screen = Screen::Deploy;
        }
    }

    /// CONTINUE: to the battle's briefing.
    pub(crate) fn tech_continue(&mut self) {
        if self.net.host.is_none() {
            self.order(crate::command::Command::Brief);
        }
    }
}
