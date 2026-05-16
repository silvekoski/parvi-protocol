//! T8 — TUI event loop + demo key handlers + spoofer/jammer toggle.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use tokio::sync::mpsc;
use tracing::info;

use tacticalmesh_link::Priority;
use tacticalmesh_wire::{build_frame, Identity, MsgKind, TacticalMessage as WireMsg, BROADCAST};
use tacticalmesh_app::{
    messages::{
        Bda, BdaResult, RequestImage, StateReport,
        TacticalMessage as AppMsg, TargetDetection, TargetKind,
    },
    tui::{Tui, TuiEvent},
};

use crate::{OutboundMsg, Shared};

pub async fn run(shared: Arc<Shared>, out_tx: mpsc::Sender<OutboundMsg>) -> anyhow::Result<()> {
    let mut tui = Tui::new()?;

    // Spoofer / jammer control flags
    let spoof_active: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let jam_active:   Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let blackout:     Arc<AtomicBool> = Arc::new(AtomicBool::new(false));

    {
        // Spawn spoofer task (stays dormant until spoof_active = true).
        let flag  = spoof_active.clone();
        let link  = shared.link.clone();
        let id    = shared.identity.node_id;
        let ctr   = shared.bad_sigs.clone();
        tokio::spawn(async move {
            spoofer_loop(flag, link, id, ctr).await;
        });
    }
    {
        // Spawn jammer task.
        let flag = jam_active.clone();
        let link = shared.link.clone();
        tokio::spawn(async move {
            jammer_loop(flag, link).await;
        });
    }

    let local_id = shared.identity.node_id;

    loop {
        // Render at ~60 fps.
        if let Err(e) = tui.render(&shared.app.read()) {
            tracing::warn!("TUI render: {e}");
        }

        if let Some(ev) = tui.next_event()? {
            match ev {
                TuiEvent::Quit | TuiEvent::KeyPress('q') => break,

                // t — TargetDetection (CHARLIE role)
                TuiEvent::KeyPress('t') => {
                    let msg = AppMsg::TargetDetection(TargetDetection {
                        target_id:      rand::random::<u16>() % 900 + 100,
                        kind:           TargetKind::Vehicle,
                        lat:            48.1234 + rand::random::<f32>() * 0.01,
                        lon:            11.5678 + rand::random::<f32>() * 0.01,
                        detected_at_ms: now_ms(),
                        detector:       local_id,
                    });
                    let _ = out_tx.send(OutboundMsg { app_msg: msg, dst: BROADCAST, prio: Priority::Critical }).await;
                    shared.app.write().push_log("[KEY t] TargetDetection broadcast".to_string());
                }

                // f — BDA DESTROYED (CHARLIE role)
                TuiEvent::KeyPress('f') => {
                    // BDA against the highest-id detected target, or a dummy.
                    let target_id = shared.target_board.read().targets()
                        .map(|t| t.id)
                        .max()
                        .unwrap_or(100);
                    let msg = AppMsg::Bda(Bda {
                        target_id,
                        result: BdaResult::Destroyed,
                        at_ms: now_ms(),
                    });
                    let _ = out_tx.send(OutboundMsg { app_msg: msg, dst: BROADCAST, prio: Priority::Critical }).await;
                    shared.app.write().push_log(format!("[KEY f] BDA DESTROYED for target {target_id}"));
                }

                // i — RequestImage for highlighted target
                TuiEvent::KeyPress('i') => {
                    let target_id = shared.target_board.read().targets()
                        .map(|t| t.id)
                        .next()
                        .unwrap_or(100);
                    let msg = AppMsg::RequestImage(RequestImage { target_id, requester: local_id });
                    let _ = out_tx.send(OutboundMsg { app_msg: msg, dst: BROADCAST, prio: Priority::Bulk }).await;
                    shared.app.write().push_log(format!("[KEY i] RequestImage for target {target_id}"));
                }

                // s — toggle spoofer (BRAVO role)
                TuiEvent::KeyPress('s') => {
                    let was = spoof_active.fetch_xor(true, Ordering::Relaxed);
                    let now = !was;
                    let msg = if now { "[KEY s] Spoofer ENABLED" } else { "[KEY s] Spoofer DISABLED" };
                    info!("{msg}");
                    shared.app.write().push_log(msg.to_string());
                }

                // j — toggle jammer (BRAVO role)
                TuiEvent::KeyPress('j') => {
                    let was = jam_active.fetch_xor(true, Ordering::Relaxed);
                    let now = !was;
                    let msg = if now { "[KEY j] Jammer ENABLED" } else { "[KEY j] Jammer DISABLED" };
                    info!("{msg}");
                    shared.app.write().push_log(msg.to_string());
                }

                // b — toggle simulated comms blackout (drop all received frames)
                TuiEvent::KeyPress('b') => {
                    let was = blackout.fetch_xor(true, Ordering::Relaxed);
                    let now_val = !was;
                    if now_val {
                        // Allow only ourselves — effectively blocks all RX from peers.
                        shared.link.set_allow_list(Some(vec![local_id]));
                        shared.app.write().push_log("[KEY b] Blackout ON (RX blocked)".to_string());
                    } else {
                        shared.link.set_allow_list(None);
                        shared.app.write().push_log("[KEY b] Blackout OFF".to_string());
                    }
                }

                // o — dump OLSR state to log panel
                TuiEvent::KeyPress('o') => {
                    let s = shared.olsr.read();
                    let nbr_count   = s.neighbors().count();
                    let route_count = s.routes.len();
                    let msg = format!(
                        "[OLSR] neighbors={nbr_count} routes={route_count} epoch={}",
                        shared.link.current_epoch()
                    );
                    shared.app.write().push_log(msg);
                }

                // 1-9 — send StateReport (for demo: simulates assigning target N to this node)
                TuiEvent::KeyPress(c @ '1'..='9') => {
                    let msg = AppMsg::StateReport(StateReport {
                        node_id:     local_id,
                        battery_pct: 85,
                        lat:         48.1234,
                        lon:         11.5678,
                    });
                    let _ = out_tx.send(OutboundMsg { app_msg: msg, dst: BROADCAST, prio: Priority::High }).await;
                    shared.app.write().push_log(format!("[KEY {c}] StateReport broadcast"));
                }

                _ => {}
            }
        }

        tokio::time::sleep(Duration::from_millis(16)).await;
    }

    Ok(())
}

// ── Spoofer ───────────────────────────────────────────────────────────────────
// Generates frames signed with a random (unknown) key so other nodes drop them with
// BadSignature — visible in the attack counters panel.

async fn spoofer_loop(
    active: Arc<AtomicBool>,
    link: Arc<tacticalmesh_link::LinkAdapter>,
    _node_id: u8,
    tx_counter: Arc<std::sync::atomic::AtomicU64>,
) {
    loop {
        if active.load(Ordering::Relaxed) {
            let fake = Identity::generate(0xFF); // random signing key → unknown pubkey
            let wire = WireMsg {
                kind:    MsgKind::Data,
                payload: b"SPOOF".to_vec(),
            };
            let frame = build_frame(&wire, Priority::Critical, BROADCAST, &fake);
            let _ = link.send(&frame, Priority::Critical);
            tx_counter.fetch_add(1, Ordering::Relaxed);
        }
        tokio::time::sleep(Duration::from_millis(5)).await; // ~200 pps when active
    }
}

// ── Jammer ────────────────────────────────────────────────────────────────────
// Floods the channel with random bulk frames.  Other nodes accumulate them in the
// jam window → triggers jam detection heuristic.

async fn jammer_loop(
    active: Arc<AtomicBool>,
    link: Arc<tacticalmesh_link::LinkAdapter>,
) {
    let garbage: Vec<u8> = (0..1400).map(|_| rand::random::<u8>()).collect();
    loop {
        if active.load(Ordering::Relaxed) {
            let _ = link.send(&garbage, Priority::Bulk);
        }
        tokio::time::sleep(Duration::from_millis(2)).await; // ~500 pps when active
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
