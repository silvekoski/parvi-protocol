use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use tracing::{debug, warn};

use tacticalmesh_wire::{
    build_frame, Identity, LinkAdapter, MsgKind, Priority, TacticalMessage, BROADCAST,
};

use crate::now_ms;
use crate::state::OlsrState;
use crate::types::{
    Hello, OlsrMessage, Tc, AGING_TICK_MS, HELLO_INTERVAL_MS, NEIGHBOR_TIMEOUT_MS, TC_INTERVAL_MS,
};

/// Broadcasts a HELLO every HELLO_INTERVAL_MS.
///
/// Reads 1-hop neighbor table, builds Hello, serialises to OlsrMessage, wraps in a
/// TacticalMessage frame, and sends at Priority::High.
pub async fn hello_loop(
    state: Arc<RwLock<OlsrState>>,
    link: Arc<LinkAdapter>,
    identity: Arc<Identity>,
) {
    let mut tick = tokio::time::interval(Duration::from_millis(HELLO_INTERVAL_MS));
    loop {
        tick.tick().await;

        let hello = {
            let s = state.read();
            Hello {
                sender: s.local_id,
                neighbors: s.neighbors.iter()
                    .map(|(id, e)| (*id, e.link_quality))
                    .collect(),
                sent_at_ms: now_ms(),
            }
        };

        let payload = match bincode::serialize(&OlsrMessage::Hello(hello)) {
            Ok(b) => b,
            Err(e) => { warn!("hello serialize: {e}"); continue; }
        };
        let msg = TacticalMessage { kind: MsgKind::OlsrHello, payload };
        let frame = build_frame(&msg, Priority::High, BROADCAST, &identity);
        if let Err(e) = link.send(&frame, Priority::High) {
            warn!("hello send: {e}");
        }
        debug!("HELLO sent by node {}", identity.node_id);
    }
}

/// Broadcasts a TC every TC_INTERVAL_MS.
///
/// Sends advertised_neighbors = current 1-hop neighbor set with a monotonic seq.
pub async fn tc_loop(
    state: Arc<RwLock<OlsrState>>,
    link: Arc<LinkAdapter>,
    identity: Arc<Identity>,
) {
    let mut tick = tokio::time::interval(Duration::from_millis(TC_INTERVAL_MS));
    let mut seq: u16 = 0;
    loop {
        tick.tick().await;
        seq = seq.wrapping_add(1);

        let tc = {
            let s = state.read();
            Tc {
                sender: s.local_id,
                seq,
                advertised_neighbors: s.neighbors.iter()
                    .map(|(id, e)| (*id, e.link_quality))
                    .collect(),
                sent_at_ms: now_ms(),
            }
        };

        let payload = match bincode::serialize(&OlsrMessage::Tc(tc)) {
            Ok(b) => b,
            Err(e) => { warn!("tc serialize: {e}"); continue; }
        };
        let msg = TacticalMessage { kind: MsgKind::OlsrTc, payload };
        let frame = build_frame(&msg, Priority::High, BROADCAST, &identity);
        if let Err(e) = link.send(&frame, Priority::High) {
            warn!("tc send: {e}");
        }
        debug!("TC seq={seq} sent by node {}", identity.node_id);
    }
}

/// Removes neighbors not heard from in NEIGHBOR_TIMEOUT_MS and recomputes routes.
///
/// Ticks at AGING_TICK_MS (500 ms). On stale neighbor removal also clears their topology
/// and two_hop entries. If all neighbors are gone, routes are cleared entirely.
pub async fn aging_loop(state: Arc<RwLock<OlsrState>>) {
    let mut tick = tokio::time::interval(Duration::from_millis(AGING_TICK_MS));
    loop {
        tick.tick().await;

        let now = now_ms();
        let mut s = state.write();

        let stale: Vec<u8> = s.neighbors.iter()
            .filter(|(_, e)| now.saturating_sub(e.last_hello_ms) > NEIGHBOR_TIMEOUT_MS)
            .map(|(id, _)| *id)
            .collect();

        for id in &stale {
            debug!("neighbor {id} timed out, removing");
            s.neighbors.remove(id);
            s.topology.remove(id);
            s.two_hop.retain(|_, gateways| { gateways.remove(id); !gateways.is_empty() });
        }

        if !stale.is_empty() {
            if s.neighbors.is_empty() {
                s.routes.clear();
            } else {
                s.recompute_routes();
            }
        }
    }
}
