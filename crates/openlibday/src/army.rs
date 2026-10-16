//! ARMY: spending the campaign's technology points on unit levels, in base
//! building and after the first city attack.

use libday_view::ui::Screen;

use crate::Viewer;
use crate::skirmish::Setup;

/// Where CONTINUE goes from the technology dialog after a won battle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Then {
    /// The continent overview's "You are victorious" (first city attack).
    Victory,
}

impl Viewer {
    /// The technology grid for the units the base's buildings provide, at
    /// the campaign's levels and with its points.
    fn army_setup(&self) -> Option<Setup> {
        let session = self.campaign.as_ref()?;
        let owned = self.owned_buildings(&self.map.battle.scenario);
        let units = self.campaign_units(&owned);
        let automated = self.campaign_automated(&owned);
        let levels = &session.state.unit_tech;
        Some(Setup {
            tech: units
                .iter()
                .map(|k| levels.get(k).copied().unwrap_or(0))
                .collect(),
            units,
            automated,
            points: session.state.tech,
            cell: 0,
            tech_scroll: 0,
            auto_cell: 0,
            auto_scroll: 0,
            row: 0,
            scroll: 0,
            deployed: Vec::new(),
        })
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

    /// The technology dialog after a won campaign battle.
    pub(crate) fn open_army_reward(&mut self, then: Then) {
        let Some(setup) = self.army_setup() else {
            return;
        };
        self.flow.setup = Some(setup);
        self.army_reward = Some(then);
        self.screen = Screen::Tech { read_only: false };
    }

    /// Writes the dialog's levels and points back to the campaign.
    pub(crate) fn army_sync(&mut self) {
        if self.screen != Screen::Army && self.army_reward.is_none() {
            return;
        }
        let (Some(setup), Some(session)) = (&self.flow.setup, &mut self.campaign) else {
            return;
        };
        session.state.tech = setup.points;
        for (&kind, &level) in setup.units.iter().zip(&setup.tech) {
            if level > 0 {
                session.state.unit_tech.insert(kind, level);
            }
        }
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

    /// CONTINUE: to the battle's briefing, or on with the campaign.
    pub(crate) fn tech_continue(&mut self) {
        let Some(then) = self.army_reward.take() else {
            self.begin_briefing();
            return;
        };
        self.flow.setup = None;
        self.save_campaign();
        match then {
            Then::Victory => {
                self.resume_campaign();
                self.strategic_victory();
            }
        }
    }
}
