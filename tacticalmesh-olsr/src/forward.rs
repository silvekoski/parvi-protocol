use anyhow::anyhow;
use tracing::debug;

use tacticalmesh_wire::{
    build_frame, build_frame_for_route, Identity, LinkAdapter, MsgKind, Priority,
    TacticalMessage, BROADCAST,
};

use crate::state::OlsrState;
use crate::types::Tc;

/// Returns `true` if this TC has not been seen before and should be forwarded.
///
/// Read-only: mirrors the dedup check in `OlsrState::process_tc` without mutating state.
/// The bin crate calls this before deciding whether to rebroadcast a received TC frame.
pub fn should_forward_tc(state: &OlsrState, tc: &Tc) -> bool {
    if tc.sender == state.local_id { return false; }
    let last_seq = state.last_tc_seq.get(&tc.sender).copied().unwrap_or(0);
    !(tc.seq <= last_seq && last_seq.wrapping_sub(tc.seq) < 100)
}

/// Routes a pre-serialised payload to `dst` via the OLSR routing table.
///
/// - `dst == BROADCAST`: sends directly without consulting routing table.
/// - Otherwise: looks up `dst` in `state.routes`, uses `next_hop` to address the frame,
///   and sends via `link.send`.
///
/// Returns `Err` if `dst` is not BROADCAST and no route exists.
///
/// The caller is responsible for serialising the message to `payload` and providing the
/// appropriate `msg_kind` (e.g. `MsgKind::Data` for tactical messages).
pub fn route_and_send(
    state: &OlsrState,
    link: &LinkAdapter,
    identity: &Identity,
    payload: Vec<u8>,
    msg_kind: MsgKind,
    dst: u8,
    prio: Priority,
) -> anyhow::Result<()> {
    let msg = TacticalMessage { kind: msg_kind, payload };

    if dst == BROADCAST {
        let frame = build_frame(&msg, prio, dst, identity);
        link.send(&frame, prio)?;
        debug!("broadcast send dst=0xFF prio={prio:?}");
        return Ok(());
    }

    let route = state.routes.get(&dst)
        .ok_or_else(|| anyhow!("no route to node {dst}"))?;

    let frame = build_frame_for_route(&msg, prio, dst, identity, route.next_hop);
    link.send(&frame, prio)?;
    debug!(
        "routed send dst={dst} next_hop={} cost={} hops={}",
        route.next_hop, route.cost, route.hop_count
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::OlsrState;

    #[test]
    fn should_forward_tc_new_seq() {
        let state = OlsrState::new(1);
        let tc = Tc { sender: 2, seq: 5, advertised_neighbors: vec![], sent_at_ms: 0 };
        assert!(should_forward_tc(&state, &tc));
    }

    #[test]
    fn should_forward_tc_same_seq_is_dup() {
        let mut state = OlsrState::new(1);
        state.last_tc_seq.insert(2, 5);
        let tc = Tc { sender: 2, seq: 5, advertised_neighbors: vec![], sent_at_ms: 0 };
        assert!(!should_forward_tc(&state, &tc));
    }

    #[test]
    fn should_forward_tc_local_sender_ignored() {
        let state = OlsrState::new(1);
        let tc = Tc { sender: 1, seq: 1, advertised_neighbors: vec![], sent_at_ms: 0 };
        assert!(!should_forward_tc(&state, &tc));
    }

    #[test]
    fn should_forward_tc_wraparound() {
        let mut state = OlsrState::new(1);
        state.last_tc_seq.insert(2, 65500);
        // Sequence 10 after wraparound: gap > 100, should forward.
        let tc = Tc { sender: 2, seq: 10, advertised_neighbors: vec![], sent_at_ms: 0 };
        assert!(should_forward_tc(&state, &tc));
    }
}
