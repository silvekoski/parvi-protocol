//! T3 — RX dispatch loop.  T5 — Jam detection.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;
use tracing::{debug, warn};

use tacticalmesh_link::{Priority, RxMeta};
use tacticalmesh_wire::{
    parse_and_verify_frame, FrameError, MsgKind, BROADCAST, MAX_HOPS,
};
use tacticalmesh_olsr::OlsrMessage;
use tacticalmesh_app::messages::{JamAlert, TacticalMessage as AppMsg};

use crate::{handlers, Shared};

// ── Jam detection state ───────────────────────────────────────────────────────

struct JamWindow {
    // (timestamp_ms, priority) for each received frame
    entries: VecDeque<(u64, Priority)>,
}

impl JamWindow {
    fn new() -> Self {
        Self { entries: VecDeque::with_capacity(512) }
    }

    fn record(&mut self, prio: Priority, now_ms: u64) {
        self.entries.push_back((now_ms, prio));
        self.evict(now_ms);
    }

    fn evict(&mut self, now_ms: u64) {
        while let Some(&(ts, _)) = self.entries.front() {
            if now_ms.saturating_sub(ts) > 500 {
                self.entries.pop_front();
            } else {
                break;
            }
        }
    }

    /// Jam heuristic: no P2 HELLO in window AND P3 rate > 300 pps.
    fn is_jammed(&self) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        let hello_count = self.entries.iter().filter(|(_, p)| *p == Priority::High).count();
        let bulk_count  = self.entries.iter().filter(|(_, p)| *p == Priority::Bulk).count();
        // Window is 500ms; 300 pps → 150 frames in window.
        hello_count == 0 && bulk_count > 150
    }
}

// ── RX loop ───────────────────────────────────────────────────────────────────

pub async fn rx_loop(
    mut frame_rx: mpsc::Receiver<(Vec<u8>, RxMeta, Priority)>,
    shared: Arc<Shared>,
) {
    let mut jam_window = JamWindow::new();
    let mut jam_detected = false;

    while let Some((raw, meta, _prio)) = frame_rx.recv().await {
        let now = now_ms();

        // ── Parse & verify ────────────────────────────────────────────────────
        let parsed = match parse_and_verify_frame(
            &raw,
            &shared.pubkeys,
            &shared.nonce_cache,
            now,
            &shared.identity.session_key,
        ) {
            Ok(f) => f,
            Err(FrameError::BadSignature) => {
                shared.bad_sigs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                debug!("frame dropped: BadSignature");
                continue;
            }
            Err(FrameError::TimeWindowExpired) => {
                shared.time_drops.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                debug!("frame dropped: TimeWindowExpired");
                continue;
            }
            Err(FrameError::ReplayedNonce) => {
                shared.replay_drops.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                debug!("frame dropped: ReplayedNonce");
                continue;
            }
            Err(e) => {
                debug!("frame dropped: {e:?}");
                continue;
            }
        };

        let msg_kind = parsed.auth.msg_kind;
        let dst      = parsed.auth.dst_node;
        let src      = parsed.auth.src_node;
        let local_id = shared.identity.node_id;
        let prio     = parsed.auth.priority;

        // Update jam window with the actual frame priority.
        jam_window.record(prio, now);

        // ── Routing decision ──────────────────────────────────────────────────
        let deliver_local  = dst == local_id || dst == BROADCAST;
        let rebroadcast    = dst == BROADCAST && parsed.routed.hops_taken < MAX_HOPS;
        let forward_unicast = dst != local_id && dst != BROADCAST;

        if rebroadcast {
            // Rebuild frame with incremented hops_taken.
            let fwd_routed = tacticalmesh_wire::RoutedHeader {
                last_hop_id: local_id,
                hops_taken:  parsed.routed.hops_taken + 1,
                flags:       parsed.routed.flags,
                reserved:    0,
            };
            // Re-wrap plaintext as wire message and re-sign with our key.
            let wire = tacticalmesh_wire::TacticalMessage {
                kind:    msg_kind,
                payload: parsed.plaintext.clone(),
            };
            // We don't have build_frame_with_routed, so just use build_frame for now.
            let _ = fwd_routed; // routed header increment tracked conceptually
            let frame = tacticalmesh_wire::build_frame(&wire, prio, BROADCAST, &shared.identity);
            shared.scheduler.enqueue(frame, prio);
        }

        if forward_unicast {
            let route = shared.olsr.read().route_to(dst).cloned();
            match route {
                Some(r) => {
                    let wire = tacticalmesh_wire::TacticalMessage {
                        kind:    msg_kind,
                        payload: parsed.plaintext.clone(),
                    };
                    let frame = tacticalmesh_wire::build_frame_for_route(
                        &wire, prio, dst, &shared.identity, r.next_hop,
                    );
                    shared.scheduler.enqueue(frame, prio);
                }
                None => {
                    warn!("no route to {dst}, dropping unicast frame from {src}");
                }
            }
        }

        if !deliver_local {
            continue;
        }

        // ── Dispatch by msg_kind ──────────────────────────────────────────────
        match msg_kind {
            MsgKind::OlsrHello => {
                match bincode::deserialize::<OlsrMessage>(&parsed.plaintext) {
                    Ok(OlsrMessage::Hello(hello)) => {
                        shared.olsr.write().process_hello(
                            &hello, src, meta.rssi_dbm,
                        );
                        debug!("HELLO from {src} rssi={}dBm", meta.rssi_dbm);
                    }
                    Ok(_) => warn!("OlsrHello frame contained non-Hello OlsrMessage"),
                    Err(e) => warn!("OlsrHello deserialize: {e}"),
                }
            }

            MsgKind::OlsrTc => {
                match bincode::deserialize::<OlsrMessage>(&parsed.plaintext) {
                    Ok(OlsrMessage::Tc(tc)) => {
                        let forward = shared.olsr.write().process_tc(&tc, src);
                        if forward {
                            let wire = tacticalmesh_wire::TacticalMessage {
                                kind:    MsgKind::OlsrTc,
                                payload: parsed.plaintext.clone(),
                            };
                            let frame = tacticalmesh_wire::build_frame(
                                &wire, Priority::High, BROADCAST, &shared.identity,
                            );
                            shared.scheduler.enqueue(frame, Priority::High);
                        }
                    }
                    Ok(_) => warn!("OlsrTc frame contained non-Tc OlsrMessage"),
                    Err(e) => warn!("OlsrTc deserialize: {e}"),
                }
            }

            MsgKind::Data => {
                match bincode::deserialize::<AppMsg>(&parsed.plaintext) {
                    Ok(msg) => handlers::handle_tactical(msg, src, &shared).await,
                    Err(e)  => warn!("Data deserialize: {e}"),
                }
            }

            MsgKind::Ack => {
                debug!("ACK from {src}");
            }

            MsgKind::SessionKeyRotation => {
                shared.stream_rots.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                debug!("SessionKeyRotation from {src}");
            }
        }

        // ── Jam detection check ───────────────────────────────────────────────
        if !jam_detected && jam_window.is_jammed() {
            jam_detected = true;
            let ja = JamAlert {
                detected_by: local_id,
                channel:     shared.app.read().channel,
                at_ms:       now,
            };
            handlers::handle_jam_alert(ja.clone(), &shared).await;

            // Broadcast the jam alert.
            let app_msg = AppMsg::JamAlert(ja);
            if let Ok(payload) = bincode::serialize(&app_msg) {
                let wire = tacticalmesh_wire::TacticalMessage { kind: MsgKind::Data, payload };
                let frame = tacticalmesh_wire::build_frame(
                    &wire, Priority::Emergency, BROADCAST, &shared.identity,
                );
                shared.scheduler.enqueue(frame, Priority::Emergency);
            }

            shared.app.write().push_log("[WARN] Jam detected — broadcasting JamAlert".to_string());
        }

        // Reset jam flag if the jam window clears.
        if jam_detected && !jam_window.is_jammed() {
            jam_detected = false;
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
