//! Simulation state hash: what every multiplayer peer must agree on after
//! each command (see `docs/multiplayer.md`). It covers the rules' state
//! only, never the view (camera, selection, animations, sounds).

use std::fmt::{self, Write};

use crate::Viewer;

/// 64-bit FNV-1a over formatted text: stable across runs and machines
/// (unlike `std`'s randomly keyed hasher).
pub(crate) struct Fnv(u64);

impl Fnv {
    pub(crate) fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub(crate) fn finish(&self) -> u64 {
        self.0
    }
}

impl Write for Fnv {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
        Ok(())
    }
}

impl Viewer {
    /// Hash of the simulation state: the battle (every scenario layer,
    /// clock, records and settings), the battle flow's rule state and the
    /// campaign. What only the AI's copy keeps is left out: the units' AI
    /// orders, the AI's support budget and `rand()`, which every command
    /// restarts from its seed.
    pub(crate) fn state_hash(&self) -> u64 {
        let mut h = Fnv::new();
        let b = &self.map.battle;
        let f = &self.flow;
        let mut scenario = b.scenario.clone();
        scenario.order_targets.clear();
        for u in &mut scenario.units {
            u.order = Default::default();
        }
        // Writing to `Fnv` cannot fail.
        let _ = write!(
            h,
            "{:?}|{:?}|{:?}|{}|{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}",
            scenario,
            b.clock,
            b.players,
            b.difficulty,
            b.turn_time,
            b.auto_fire,
            f.phase,
            f.used_once,
            f.tech,
            f.outcome,
            self.campaign.as_ref().map(|c| &c.state),
        );
        if let Some(s) = &f.setup {
            let _ = write!(
                h,
                "|{:?}|{:?}|{}|{:?}",
                s.units, s.tech, s.points, s.deployed
            );
        }
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_matches_the_reference() {
        let mut h = Fnv::new();
        h.write_str("a").unwrap();
        assert_eq!(h.finish(), 0xaf63_dc4c_8601_ec8c);
    }
}
