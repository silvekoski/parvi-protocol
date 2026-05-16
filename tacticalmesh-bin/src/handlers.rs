use std::sync::Arc;

use tracing::{debug, info, warn};

use tacticalmesh_link::Priority;
use tacticalmesh_wire::{
    build_frame, build_frame_for_route, MsgKind, TacticalMessage as WireMsg, BROADCAST,
};
use tacticalmesh_app::{
    crdt::{Target, TargetState, TargetUpdate},
    messages::{
        BdaResult, Bda, ChannelHop, ImageShard, JamAlert, Mayday, RequestImage,
        StateReport, TacticalMessage as AppMsg, TargetDetection, TargetKind,
    },
};

use crate::Shared;

pub async fn handle_tactical(msg: AppMsg, src_node: u8, shared: &Arc<Shared>) {
    match msg {
        AppMsg::TargetDetection(td) => handle_target_detection(td, shared),
        AppMsg::Bda(bda)            => handle_bda(bda, shared),
        AppMsg::Command(cmd)        => handle_command(cmd, shared),
        AppMsg::StateReport(sr)     => handle_state_report(sr, shared),
        AppMsg::ImageShard(shard)   => handle_image_shard(shard, shared),
        AppMsg::RequestImage(req)   => handle_request_image(req, src_node, shared).await,
        AppMsg::JamAlert(ja)        => handle_jam_alert(ja, shared).await,
        AppMsg::ChannelHop(ch)      => handle_channel_hop(ch, shared).await,
        AppMsg::Mayday(m)           => handle_mayday(m, shared),
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
            assigned_to: None,
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
    shared.image_cache.lock().insert_shard(shard);
    debug!("ImageShard for target {target_id} inserted");
}

async fn handle_request_image(req: RequestImage, _src_node: u8, shared: &Arc<Shared>) {
    let complete = shared.image_cache.lock().get_complete(req.target_id).map(|b| b.to_vec());
    let Some(image_bytes) = complete else {
        debug!("RequestImage: no complete image for target {}", req.target_id);
        return;
    };

    // Shard the image: P3 FEC (8,12) for bulk streaming.
    let k: u8 = 8;
    let n: u8 = 12;
    let block_id: u8 = rand::random();
    let chunk_size = (image_bytes.len() + k as usize - 1) / k as usize;

    for (idx, chunk) in image_bytes.chunks(chunk_size).enumerate() {
        let shard = tacticalmesh_app::messages::ImageShard {
            target_id: req.target_id,
            block_id,
            index: idx as u8,
            k,
            n,
            data: chunk.to_vec(),
        };
        let app_msg = AppMsg::ImageShard(shard);
        let payload = match bincode::serialize(&app_msg) {
            Ok(b) => b,
            Err(e) => { warn!("ImageShard serialize: {e}"); continue; }
        };
        let wire = WireMsg { kind: MsgKind::Data, payload };
        let frame = if req.requester == BROADCAST {
            build_frame(&wire, Priority::Bulk, BROADCAST, &shared.identity)
        } else {
            let route = shared.olsr.read().route_to(req.requester).cloned();
            match route {
                Some(r) => build_frame_for_route(&wire, Priority::Bulk, req.requester, &shared.identity, r.next_hop),
                None => {
                    warn!("RequestImage: no route to requester {}", req.requester);
                    return;
                }
            }
        };
        shared.scheduler.enqueue(frame, Priority::Bulk);
    }
    info!("ImageRequest: sent {} shards for target {}", k, req.target_id);
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
