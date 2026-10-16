//! The player table (multiplayer MP0, `docs/multiplayer.md`): the battle's
//! seats, the humans in slots 1 to 4 and the computer in slot 0, who
//! controls each, and which human this copy of the game plays for. The
//! humans play their turns one after the other in slot order, then the AI.
//! Credits are per slot ([`libday_data::battle::PlayerRecord`]); the
//! technology bought before a skirmish is shared by the humans for now.

pub use crate::skirmish::AI;

/// The campaign's human slot: campaigns have one human until MP6.
pub const CAMPAIGN_PLAYER: i64 = 1;

/// Who gives a seat's orders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Controller {
    /// A human at this computer.
    Local,
    /// The computer player ([`crate::ai`]).
    Ai,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seat {
    pub slot: i64,
    pub controller: Controller,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Players {
    /// The humans in turn order, then the AI.
    seats: Vec<Seat>,
    /// The human whose orders this copy's interface gives; with several
    /// humans at this computer (hotseat) it follows whose turn it is.
    pub local: i64,
}

impl Default for Players {
    fn default() -> Self {
        Self::new(1)
    }
}

impl Players {
    /// `humans` (1 to 4) humans at this computer against the AI.
    pub fn new(humans: usize) -> Self {
        let humans = humans.clamp(1, 4) as i64;
        let mut seats: Vec<Seat> = (1..=humans)
            .map(|slot| Seat {
                slot,
                controller: Controller::Local,
            })
            .collect();
        seats.push(Seat {
            slot: AI,
            controller: Controller::Ai,
        });
        Self { seats, local: 1 }
    }

    /// The human slots, in turn order.
    pub fn humans(&self) -> impl Iterator<Item = i64> + '_ {
        self.seats
            .iter()
            .filter(|s| s.controller != Controller::Ai)
            .map(|s| s.slot)
    }

    /// Whether `slot` is a human's.
    pub fn human(&self, slot: i64) -> bool {
        self.humans().any(|h| h == slot)
    }

    /// Whether `slot` is a human at this computer.
    pub fn is_local(&self, slot: i64) -> bool {
        self.seats
            .iter()
            .any(|s| s.slot == slot && s.controller == Controller::Local)
    }

    pub fn first_human(&self) -> i64 {
        self.humans().next().unwrap_or(1)
    }

    /// The human playing after `slot` in the same round, if any.
    pub fn next_human(&self, slot: i64) -> Option<i64> {
        self.humans().skip_while(|&h| h != slot).nth(1)
    }

    /// The overview's player slots: the humans, then the AI.
    pub fn overview(&self) -> Vec<i64> {
        self.seats.iter().map(|s| s.slot).collect()
    }

    /// Makes `slot` the local player if a human at this computer holds it.
    pub fn hand_over(&mut self, slot: i64) {
        if self.is_local(slot) {
            self.local = slot;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humans_play_in_slot_order_then_the_ai() {
        let p = Players::default();
        assert_eq!(p.overview(), [1, AI]);
        assert_eq!(p.next_human(1), None);
        let mut p = Players::new(3);
        assert_eq!(p.overview(), [1, 2, 3, AI]);
        assert_eq!(p.next_human(1), Some(2));
        assert_eq!(p.next_human(3), None);
        assert!(p.human(3) && !p.human(AI) && !p.human(4));
        p.hand_over(2);
        assert_eq!(p.local, 2);
        p.hand_over(AI);
        assert_eq!(p.local, 2);
    }
}
