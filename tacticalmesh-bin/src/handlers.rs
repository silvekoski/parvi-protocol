use std::sync::Arc;

use tracing::{debug, info, warn};

use tacticalmesh_link::Priority;
use tacticalmesh_wire::{
    build_frame, build_frame_for_route, MsgKind, TacticalMessage as WireMsg, BROADCAST,
};
use tacticalmesh_app::{
    crdt::{Target, TargetState, TargetUpdate},
    messages::{
        BdaResult, Bda, ChannelHop, ChatMessage, ImageShard, JamAlert, Mayday, RequestImage,
        StateReport, TacticalMessage as AppMsg, TargetDetection, TargetKind,
    },
    state::ImageRxProgress,
};

use crate::Shared;

pub async fn handle_tactical(msg: AppMsg, src_node: u8, shared: &Arc<Shared>) {
    match msg {
        AppMsg::TargetDetection(td) => handle_target_detection(td, shared),
        AppMsg::Bda(bda)            => handle_bda(bda, shared),
        AppMsg::Command(cmd)        => handle_command(cmd, shared),
        AppMsg::StateReport(sr)     => handle_state_report(sr, shared),
        AppMsg::ImageShard(shard)   => handle_image_shard(shard, shared),
        AppMsg::RequestImage(req)   => {
            let shared2 = shared.clone();
            tokio::spawn(async move {
                handle_request_image(req, src_node, &shared2).await;
            });
        }
        AppMsg::JamAlert(ja)        => handle_jam_alert(ja, shared).await,
        AppMsg::ChannelHop(ch)      => handle_channel_hop(ch, shared).await,
        AppMsg::Mayday(m)           => handle_mayday(m, shared),
        AppMsg::Chat(chat)              => handle_chat(chat, shared),
        AppMsg::Ack(_) | AppMsg::Olsr(_) => {} // handled upstream
    }
}

fn handle_target_detection(td: TargetDetection, shared: &Arc<Shared>) {
    let update = TargetUpdate {
        target: Target {
            id:          td.target_id,
            kind:        td.kind,
            state:       TargetState::Detected,
            lat:         td.lat,
            lon:         td.lon,
            updated_at_ms: td.detected_at_ms,
            assigned_to: Some(td.detector),
        },
    };
    shared.target_board.write().merge(update);
    debug!("TargetDetection id={} merged", td.target_id);
}

fn handle_bda(bda: Bda, shared: &Arc<Shared>) {
    // Preserve existing lat/lon since BDA doesn't include them.
    let (lat, lon, kind) = {
        let board = shared.target_board.read();
        board.get(bda.target_id)
            .map(|t| (t.lat, t.lon, t.kind.clone()))
            .unwrap_or((0.0, 0.0, TargetKind::Unknown))
    };
    let state = match bda.result {
        BdaResult::Destroyed => TargetState::Destroyed,
        BdaResult::Damaged   => TargetState::Engaged,
        BdaResult::Miss      => TargetState::Engaged,
    };
    let update = TargetUpdate {
        target: Target { id: bda.target_id, kind, state, lat, lon, updated_at_ms: bda.at_ms, assigned_to: None },
    };
    shared.target_board.write().merge(update);
    debug!("BDA id={} state={:?}", bda.target_id, state);
}

fn handle_command(
    cmd: tacticalmesh_app::messages::Command,
    shared: &Arc<Shared>,
) {
    use tacticalmesh_app::messages::CommandOp;
    let (lat, lon, kind) = {
        let board = shared.target_board.read();
        board.get(cmd.target_id)
            .map(|t| (t.lat, t.lon, t.kind.clone()))
            .unwrap_or((0.0, 0.0, TargetKind::Unknown))
    };
    let (state, assigned_to) = match cmd.op {
        CommandOp::Engage    => (TargetState::Engaged,  None),
        CommandOp::Abort     => (TargetState::Aborted,  None),
        CommandOp::Reassign  => (TargetState::Assigned, Some(cmd.issued_by)),
    };
    let update = TargetUpdate {
        target: Target {
            id: cmd.target_id, kind, state, lat, lon,
            updated_at_ms: now_ms(), assigned_to,
        },
    };
    shared.target_board.write().merge(update);
    debug!("Command id={} op={:?}", cmd.target_id, cmd.op);
}

fn handle_state_report(sr: StateReport, shared: &Arc<Shared>) {
    let msg = format!(
        "[INFO] StateReport from NODE-{}: battery={}%",
        sr.node_id, sr.battery_pct
    );
    shared.app.write().push_log(msg);
    debug!("StateReport from node {}", sr.node_id);
}

fn handle_image_shard(shard: ImageShard, shared: &Arc<Shared>) {
    let target_id = shard.target_id;
    let total_blocks = shard.total_blocks.max(1);
    let block_id = shard.block_id;

    let (completed, blocks_done) = {
        let mut cache = shared.image_cache.lock();
        let completed = cache.insert_shard(shard);
        let done = cache.blocks_assembled(target_id);
        (completed, done)
    };

    if blocks_done == 1 {
        info!("ImageRx target={target_id}: first shard received (0/{total_blocks})");
    }
    debug!("ImageShard block={block_id} target={target_id} {blocks_done}/{total_blocks} completed={completed}");

    if completed {
        let assembled = shared.image_cache.lock().get_complete(target_id).map(|b| b.to_vec());
        if let Some(assembled) = assembled {
            info!("ImageRx target={target_id}: all {total_blocks} blocks received, spawning decode ({} bytes)", assembled.len());
            // Decode off the rx_loop hot path so we don't stall the frame consumer.
            let shared2 = shared.clone();
            tokio::spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    tacticalmesh_app::image_codec::decode_jpeg(&assembled)
                        .unwrap_or_else(|| {
                            warn!("ImageRx target={target_id}: JPEG decode failed, using raw fallback");
                            (assembled, 640, 480)
                        })
                }).await;
                if let Ok((pixels, width, height)) = result {
                    info!("ImageRx target={target_id}: decoded OK ({width}×{height})");
                    let mut app = shared2.app.write();
                    app.image_rx = None;
                    app.image = Some(tacticalmesh_app::state::ImageDisplay {
                        target_id, pixels, width, height,
                    });
                    app.push_log(format!("[IMG] Image for target {target_id} ready ({width}×{height})"));
                }
            });
        }
    } else {
        shared.app.write().image_rx = Some(ImageRxProgress {
            target_id,
            blocks_done,
            blocks_total: total_blocks,
        });
    }
}

async fn handle_request_image(req: RequestImage, _src_node: u8, shared: &Arc<Shared>) {
    if req.requester == shared.identity.node_id {
        debug!("RequestImage: ignoring self-request for target {}", req.target_id);
        return;
    }

    let local = shared.local_image.lock().clone();
    let Some((raw, width, height)) = local else {
        warn!("RequestImage: no local image to serve for target {}", req.target_id);
        return;
    };

    info!("RequestImage: encoding {}×{} image for target {} → requester {}",
        width, height, req.target_id, req.requester);

    // Encode to JPEG off the async thread — CPU-intensive (~50-200ms).
    let target_id = req.target_id;
    let image_bytes = tokio::task::spawn_blocking(move || {
        tacticalmesh_app::image_codec::encode_jpeg(&raw, width, height, 75)
    }).await.ok().flatten();
    let Some(image_bytes) = image_bytes else {
        warn!("RequestImage: JPEG encode failed for target {}", target_id);
        return;
    };
    // Re-bind req fields used below after move.
    let req = RequestImage { target_id, requester: req.requester };

    info!("RequestImage: JPEG {} bytes, chunking into shards", image_bytes.len());

    // Send shards as Bulk so image traffic has its own dedicated port/channel,
    // separate from OLSR High traffic. Prefer a routed unicast reply to the
    // requester; broadcast Data frames are intentionally not reflooded by rx.rs.
    // Pace sends: 4 shards then a 10 ms yield — prevents bursting all shards
    // at once which overflows the receiver's 512-slot Rx channel.
    const SHARD_SIZE: usize = 1207;
    const BURST: usize = 4;
    let chunks: Vec<Vec<u8>> = image_bytes.chunks(SHARD_SIZE).map(|c| c.to_vec()).collect();
    let total_blocks = chunks.len() as u8;

    for (block_id, data) in chunks.iter().enumerate() {
        let shard = tacticalmesh_app::messages::ImageShard {
            target_id: req.target_id,
            total_blocks,
            block_id: block_id as u8,
            index: 0,
            k: 1,
            n: 1,
            data: data.clone(),
        };
        let app_msg = AppMsg::ImageShard(shard);
        let payload = match bincode::serialize(&app_msg) {
            Ok(b) => b,
            Err(e) => { warn!("ImageShard serialize: {e}"); continue; }
        };
        let wire = WireMsg { kind: MsgKind::Data, payload };
        let frame = match shared.olsr.read().route_to(req.requester).cloned() {
            Some(route) => build_frame_for_route(
                &wire,
                Priority::Bulk,
                req.requester,
                &shared.identity,
                route.next_hop,
            ),
            None => {
                if block_id == 0 {
                    warn!(
                        "RequestImage: no route to requester {}, falling back to direct broadcast",
                        req.requester
                    );
                }
                build_frame(&wire, Priority::Bulk, BROADCAST, &shared.identity)
            }
        };
        shared.scheduler.enqueue(frame, Priority::Bulk);

        if (block_id + 1) % BURST == 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }
    }
    info!("RequestImage: enqueued {total_blocks} shards for target {} to requester {} (Bulk, paced)",
        req.target_id, req.requester);
    shared.app.write().push_log(format!(
        "[IMG] Sending {} shards for target {} to NODE-{} ({}×{} JPEG {} bytes)",
        total_blocks, req.target_id, req.requester, width, height, image_bytes.len()
    ));
}

pub async fn handle_jam_alert(ja: JamAlert, shared: &Arc<Shared>) {
    let msg = format!(
        "[WARN] JamAlert from NODE-{}: ch={} at={}ms",
        ja.detected_by, ja.channel, ja.at_ms
    );
    shared.app.write().push_log(msg);
    shared.channel_hops.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    // Initiate channel hop to a pre-agreed fallback channel.
    let hop_ch: u8 = if ja.channel == 36 { 40 } else { 36 };
    let hop = ChannelHop {
        new_channel: hop_ch,
        new_epoch: shared.link.current_epoch() + 1,
        initiated_by: shared.identity.node_id,
    };
    handle_channel_hop(hop, shared).await;
}

pub async fn handle_channel_hop(ch: ChannelHop, shared: &Arc<Shared>) {
    info!(
        "ChannelHop: ch{} → ch{} epoch={}",
        shared.link.current_epoch(),
        ch.new_channel,
        ch.new_epoch,
    );

    if let Err(e) = shared.link.set_channel(ch.new_channel) {
        warn!("set_channel failed: {e}");
    }
    // epoch rotation deferred to link layer mutex upgrade; log and count
    shared.stream_rots.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    shared.channel_hops.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let msg = format!(
        "[INFO] Hopped to ch{}, epoch {}",
        ch.new_channel, ch.new_epoch
    );
    let mut app = shared.app.write();
    app.channel = ch.new_channel;
    app.push_log(msg);
}

fn handle_chat(chat: ChatMessage, shared: &Arc<Shared>) {
    // Skip if this node sent it — tui_loop already logged it on send.
    if chat.from == shared.identity.node_id {
        return;
    }
    let msg = format!("[{}] [CHAT] NODE-{}: {}", hms(), chat.from, chat.text);
    shared.app.write().push_log(msg);
    debug!("Chat from node {}: {}", chat.from, chat.text);
}

fn handle_mayday(m: Mayday, shared: &Arc<Shared>) {
    let msg = format!("[MAYDAY] NODE-{} at {}ms — all units respond", m.node_id, m.at_ms);
    shared.app.write().push_log(msg);
    warn!("MAYDAY from node {}", m.node_id);
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn hms() -> String {
    let secs = now_ms() / 1000;
    format!("{:02}:{:02}:{:02}", (secs / 3600) % 24, (secs / 60) % 60, secs % 60)
}
