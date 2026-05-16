use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub use crate::messages::TargetKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TargetState {
    Detected,
    Assigned,
    Engaged,
    Aborted,
    Destroyed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Target {
    pub id: u16,
    pub kind: TargetKind,
    pub state: TargetState,
    pub lat: f32,
    pub lon: f32,
    pub updated_at_ms: u64,
    pub assigned_to: Option<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TargetUpdate {
    pub target: Target,
}

pub struct TargetBoard {
    targets: HashMap<u16, Target>,
}

impl TargetBoard {
    pub fn new() -> Self {
        Self {
            targets: HashMap::new(),
        }
    }

    /// Merge rule:
    /// - higher TargetState wins
    /// - same state: latest updated_at_ms wins
    /// - idempotent, deterministic
    pub fn merge(&mut self, update: TargetUpdate) {
        let incoming = update.target;
        match self.targets.get(&incoming.id) {
            None => {
                self.targets.insert(incoming.id, incoming);
            }
            Some(existing) => {
                let should_replace = if incoming.state > existing.state {
                    true
                } else if incoming.state == existing.state {
                    incoming.updated_at_ms > existing.updated_at_ms
                } else {
                    false
                };
                if should_replace {
                    self.targets.insert(incoming.id, incoming);
                }
            }
        }
    }

    pub fn targets(&self) -> impl Iterator<Item = &Target> {
        self.targets.values()
    }

    pub fn get(&self, id: u16) -> Option<&Target> {
        self.targets.get(&id)
    }
}

impl Default for TargetBoard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_target(id: u16, state: TargetState, updated_at_ms: u64) -> Target {
        Target {
            id,
            kind: TargetKind::Vehicle,
            state,
            lat: 0.0,
            lon: 0.0,
            updated_at_ms,
            assigned_to: None,
        }
    }

    #[test]
    fn two_detected_same_id_last_timestamp_wins() {
        let mut board = TargetBoard::new();

        board.merge(TargetUpdate {
            target: make_target(1, TargetState::Detected, 100),
        });
        board.merge(TargetUpdate {
            target: make_target(1, TargetState::Detected, 200),
        });

        let t = board.get(1).expect("target missing");
        assert_eq!(t.state, TargetState::Detected);
        assert_eq!(t.updated_at_ms, 200);
    }

    #[test]
    fn destroyed_beats_late_engaged() {
        let mut board = TargetBoard::new();

        // DESTROYED arrives first
        board.merge(TargetUpdate {
            target: make_target(2, TargetState::Destroyed, 500),
        });
        // ENGAGED arrives late (lower state, higher timestamp shouldn't matter)
        board.merge(TargetUpdate {
            target: make_target(2, TargetState::Engaged, 600),
        });

        let t = board.get(2).expect("target missing");
        assert_eq!(t.state, TargetState::Destroyed);
    }

    #[test]
    fn aborted_beats_late_assigned() {
        let mut board = TargetBoard::new();

        board.merge(TargetUpdate {
            target: make_target(3, TargetState::Aborted, 300),
        });
        board.merge(TargetUpdate {
            target: make_target(3, TargetState::Assigned, 400),
        });

        let t = board.get(3).expect("target missing");
        assert_eq!(t.state, TargetState::Aborted);
    }

    #[test]
    fn comms_blackout_scenario() {
        // Node sees DETECTED, misses ENGAGED update, gets DESTROYED later
        let mut board = TargetBoard::new();

        board.merge(TargetUpdate {
            target: make_target(4, TargetState::Detected, 100),
        });
        // ENGAGED is missed (never merged)
        // DESTROYED arrives
        board.merge(TargetUpdate {
            target: make_target(4, TargetState::Destroyed, 800),
        });

        let t = board.get(4).expect("target missing");
        assert_eq!(t.state, TargetState::Destroyed);
    }

    #[test]
    fn merge_is_idempotent() {
        let mut board = TargetBoard::new();

        let update = TargetUpdate {
            target: make_target(5, TargetState::Engaged, 500),
        };
        board.merge(update.clone());
        board.merge(update.clone());
        board.merge(update);

        let t = board.get(5).expect("target missing");
        assert_eq!(t.state, TargetState::Engaged);
        assert_eq!(t.updated_at_ms, 500);
        assert_eq!(board.targets().count(), 1);
    }
}
