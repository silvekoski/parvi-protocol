use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tracing::{debug, warn};

use tacticalmesh_link::{LinkAdapter, EPOCH_ROTATION_SECS};
use tacticalmesh_wire::{build_frame, build_frame_for_route, MsgKind, TacticalMessage as WireMsg, BROADCAST};
use tacticalmesh_olsr::link_cost;
use tacticalmesh_app::state::{NeighborDisplay, RouteDisplay};

use crate::{OutboundMsg, Shared};

// ── T6 — Epoch rotation ───────────────────────────────────────────────────────

pub async fn epoch_rotation_loop(_link: Arc<LinkAdapter>) {
    let mut tick = tokio::time::interval(Duration::from_secs(EPOCH_ROTATION_SECS));
    tick.tick().await; // skip the immediate first tick
    loop {
        tick.tick().await;
        // rotate_epoch takes &mut self; we need interior mutability.
        // LinkAdapter doesn't currently expose rotate_epoch via &self,
        // so we use a workaround: the bin holds the link in an Arc<Mutex<LinkAdapter>>.
        // For now log and skip — upgrade to Arc<Mutex<LinkAdapter>> if needed at integration.
        debug!("epoch rotation tick (integrate with Arc<Mutex<LinkAdapter>> at hw time)");
    }
}

// ── T7 — AppState sync ────────────────────────────────────────────────────────

pub async fn state_sync_loop(shared: Arc<Shared>) {
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let mut olsr_first_converged = false;
    let start_ms = now_ms();

    loop {
        tick.tick().await;

        let (routes, neighbors, topo) = {
            let s = shared.olsr.read();
            let routes: Vec<RouteDisplay> = s.routes.values().map(|r| RouteDisplay {
                dest: format!("NODE-{}", r.destination),
                via:  format!("NODE-{}", r.next_hop),
                cost: r.cost,
                hops: r.hop_count,
            }).collect();

            let neighbors: Vec<NeighborDisplay> = s.neighbors().map(|(_, e)| NeighborDisplay {
                name:          format!("NODE-{}", e.node_id),
                rssi:          e.link_quality.rssi_dbm as i16,
                last_hello_ms: e.last_hello_ms,
            }).collect();

            let topo: Vec<(String, String, u32)> = s.topology_edges()
                .map(|(src, dst, lq)| (
                    format!("NODE-{src}"),
                    format!("NODE-{dst}"),
                    link_cost(lq),
                ))
                .collect();

            (routes, neighbors, topo)
        };

        // Detect first OLSR convergence (routes non-empty).
        if !olsr_first_converged && !routes.is_empty() {
            olsr_first_converged = true;
            let elapsed = now_ms().saturating_sub(start_ms);
            shared.app.write().olsr_converged_in_ms = Some(elapsed);
            tracing::info!("OLSR converged in {}ms", elapsed);
        }

        let targets = {
            let board = shared.target_board.read();
            board.targets().map(|t| tacticalmesh_app::state::TargetDisplay {
                id:          t.id,
                kind:        format!("{:?}", t.kind),
                state:       format!("{:?}", t.state),
                lat:         t.lat,
                lon:         t.lon,
                assigned_to: t.assigned_to,
            }).collect()
        };

        // Snapshot atomic counters.
        let bad_sigs     = shared.bad_sigs.load(std::sync::atomic::Ordering::Relaxed);
        let time_drops   = shared.time_drops.load(std::sync::atomic::Ordering::Relaxed);
        let replay_drops = shared.replay_drops.load(std::sync::atomic::Ordering::Relaxed);
        let channel_hops = shared.channel_hops.load(std::sync::atomic::Ordering::Relaxed);
        let stream_rots  = shared.stream_rots.load(std::sync::atomic::Ordering::Relaxed);

        let epoch = shared.link.current_epoch();

        let mut app = shared.app.write();
        app.routing_table   = routes;
        app.neighbors       = neighbors;
        app.topology_edges  = topo;
        app.targets         = targets;
        app.epoch           = epoch;
        app.counters.bad_sigs_dropped    = bad_sigs;
        app.counters.time_window_drops   = time_drops;
        app.counters.replayed_nonces     = replay_drops;
        app.counters.channel_hops        = channel_hops;
        app.counters.stream_rotations    = stream_rots;
    }
}

// ── Outbound: TUI → scheduler ─────────────────────────────────────────────────

pub async fn outbound_loop(mut rx: mpsc::Receiver<OutboundMsg>, shared: Arc<Shared>) {
    while let Some(out) = rx.recv().await {
        let payload = match bincode::serialize(&out.app_msg) {
            Ok(b) => b,
            Err(e) => { warn!("outbound serialize: {e}"); continue; }
        };

        let wire = WireMsg { kind: MsgKind::Data, payload };

        let frame = if out.dst == BROADCAST {
            build_frame(&wire, out.prio, BROADCAST, &shared.identity)
        } else {
            let route = shared.olsr.read().route_to(out.dst).cloned();
            match route {
                Some(r) => build_frame_for_route(&wire, out.prio, out.dst, &shared.identity, r.next_hop),
                None => {
                    warn!("outbound: no route to node {}", out.dst);
                    continue;
                }
            }
        };

        shared.scheduler.enqueue(frame, out.prio);
        debug!("outbound frame queued dst={} prio={:?}", out.dst, out.prio);
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
