mod handlers;
mod loops;
mod rx;
mod tui_loop;

use std::path::PathBuf;
use std::sync::{
    atomic::AtomicU64,
    Arc,
};

use clap::Parser;
use parking_lot::{Mutex, RwLock};
use tracing::info;

use tacticalmesh_link::{LinkAdapter, Priority};
use tacticalmesh_wire::{Identity, NonceCache, PubkeyStore, TxScheduler};
use tacticalmesh_olsr::OlsrState;
use tacticalmesh_app::{
    crdt::TargetBoard,
    image_cache::ImageCache,
    messages::TacticalMessage as AppMsg,
    state::AppState,
};

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(author, version, about = "TacticalMesh node")]
struct Cli {
    #[arg(long)]
    node_id: u8,
    #[arg(long, default_value = "wlan1")]
    iface: String,
    #[arg(long)]
    psk_file: Option<PathBuf>,
    /// Image to serve in response to RequestImage.
    /// Accepts raw 640×480 greyscale (307200 bytes) or a JPEG file.
    /// Omit to use the built-in test pattern.
    #[arg(long)]
    image_file: Option<PathBuf>,
}

// ── Outbound message from TUI → scheduler ────────────────────────────────────

pub struct OutboundMsg {
    pub app_msg: AppMsg,
    pub dst: u8,
    pub prio: Priority,
}

// ── Shared state (all tasks hold an Arc<Shared>) ─────────────────────────────

pub struct Shared {
    pub olsr:         Arc<RwLock<OlsrState>>,
    pub app:          Arc<RwLock<AppState>>,
    pub target_board: Arc<RwLock<TargetBoard>>,
    pub image_cache:  Arc<Mutex<ImageCache>>,
    /// Greyscale pixels + dimensions this node will serve on RequestImage.
    pub local_image:  Arc<Mutex<Option<(Vec<u8>, u32, u32)>>>,
    pub nonce_cache:  Arc<NonceCache>,
    pub identity:     Arc<Identity>,
    pub pubkeys:      Arc<PubkeyStore>,
    pub link:         Arc<LinkAdapter>,
    pub scheduler:    Arc<TxScheduler>,
    // Attack counters — incremented by rx_loop, snapshot by state_sync_loop.
    pub bad_sigs:     Arc<AtomicU64>,
    pub time_drops:   Arc<AtomicU64>,
    pub replay_drops: Arc<AtomicU64>,
    pub channel_hops: Arc<AtomicU64>,
    pub stream_rots:  Arc<AtomicU64>,
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Restore terminal on panic so the shell isn't left in raw mode.
    std::panic::set_hook(Box::new(|info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen
        );
        eprintln!("PANIC: {info}");
    }));

    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    // Load or generate PSK.
    let psk: [u8; 32] = if let Some(ref path) = cli.psk_file {
        let bytes = std::fs::read(path)?;
        bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("PSK file must be exactly 32 bytes"))?
    } else {
        tracing::warn!("no PSK file — using demo PSK (deterministic from node_id)");
        // Fixed demo PSK so all three demo nodes agree.
        *b"TacticalMeshDemoPSK-hackathon!!!"
    };

    // Deterministic per-node signing key, shared PSK for encryption.
    let identity = Arc::new(Identity::from_seed(cli.node_id, &psk));

    // Pre-register all three demo nodes so we can verify their frames.
    let mut pubkeys = PubkeyStore::new();
    for nid in 1u8..=3 {
        let demo_id = Identity::from_seed(nid, &psk);
        pubkeys.insert(nid, demo_id.verifying_key);
    }
    let pubkeys = Arc::new(pubkeys);

    let link = Arc::new(LinkAdapter::new(&cli.iface, cli.node_id, psk)?);
    let scheduler = Arc::new(TxScheduler::new(link.clone()));

    const IMG_MAX_W: u32 = 320;
    const IMG_MAX_H: u32 = 240;

    let local_image: Option<(Vec<u8>, u32, u32)> = if let Some(ref path) = cli.image_file {
        let bytes = std::fs::read(path)?;
        if bytes.len() == 640 * 480 {
            info!("loaded raw 640×480 greyscale image from {:?}", path);
            Some((bytes, 640, 480))
        } else {
            let (raw, w, h) = tacticalmesh_app::image_codec::decode_and_scale(&bytes, IMG_MAX_W, IMG_MAX_H)
                .ok_or_else(|| anyhow::anyhow!(
                    "--image-file: not a raw 640×480 greyscale file and not a valid image"
                ))?;
            info!("loaded image {}×{} (scaled) from {:?}", w, h, path);
            Some((raw, w, h))
        }
    } else {
        let tank = std::path::Path::new("assets/tank.jpg");
        if tank.exists() {
            match tacticalmesh_app::image_codec::decode_and_scale(&std::fs::read(tank)?, IMG_MAX_W, IMG_MAX_H) {
                Some((raw, w, h)) => {
                    info!("loaded default tank image {}×{}", w, h);
                    Some((raw, w, h))
                }
                None => {
                    tracing::warn!("failed to decode assets/tank.jpg, using test pattern");
                    Some(generate_test_pattern())
                }
            }
        } else {
            info!("no --image-file provided; using built-in test pattern");
            Some(generate_test_pattern())
        }
    };

    let shared = Arc::new(Shared {
        olsr:         Arc::new(RwLock::new(OlsrState::new(cli.node_id))),
        app:          Arc::new(RwLock::new(AppState::new(cli.node_id))),
        target_board: Arc::new(RwLock::new(TargetBoard::new())),
        image_cache:  Arc::new(Mutex::new(ImageCache::new())),
        local_image:  Arc::new(Mutex::new(local_image)),
        nonce_cache:  Arc::new(NonceCache::new()),
        identity:     identity.clone(),
        pubkeys,
        link:         link.clone(),
        scheduler:    scheduler.clone(),
        bad_sigs:     Arc::new(AtomicU64::new(0)),
        time_drops:   Arc::new(AtomicU64::new(0)),
        replay_drops: Arc::new(AtomicU64::new(0)),
        channel_hops: Arc::new(AtomicU64::new(0)),
        stream_rots:  Arc::new(AtomicU64::new(0)),
    });

    info!(
        "TacticalMesh node {} starting on {} (epoch {})",
        cli.node_id,
        cli.iface,
        link.current_epoch()
    );

    // TUI → outbound channel.
    let (out_tx, out_rx) = tokio::sync::mpsc::channel::<OutboundMsg>(64);

    // ── Spawn background tasks ────────────────────────────────────────────────

    // OLSR protocol loops.
    tokio::spawn(tacticalmesh_olsr::hello_loop(
        shared.olsr.clone(),
        shared.link.clone(),
        shared.identity.clone(),
    ));
    tokio::spawn(tacticalmesh_olsr::tc_loop(
        shared.olsr.clone(),
        shared.link.clone(),
        shared.identity.clone(),
    ));
    tokio::spawn(tacticalmesh_olsr::aging_loop(shared.olsr.clone()));

    // Wire / housekeeping loops.
    {
        let sched = shared.scheduler.clone();
        tokio::spawn(async move { sched.run().await });
    }
    tokio::spawn(loops::epoch_rotation_loop(shared.link.clone()));
    tokio::spawn(loops::state_sync_loop(shared.clone()));
    tokio::spawn(loops::outbound_loop(out_rx, shared.clone()));

    // RX: one blocking thread per priority → one async handler task per priority.
    {
        let (frame_tx, frame_rx) =
            tokio::sync::mpsc::channel::<(Vec<u8>, tacticalmesh_link::RxMeta, Priority)>(2048);

        for prio in [
            Priority::Emergency,
            Priority::Critical,
            Priority::High,
            Priority::Bulk,
        ] {
            let link2 = shared.link.clone();
            let tx2 = frame_tx.clone();
            std::thread::spawn(move || {
                loop {
                    match link2.recv(prio) {
                        Ok((frame, meta)) => {
                            if tx2.blocking_send((frame, meta, prio)).is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::error!("link recv error ({prio:?}): {e}");
                            break;
                        }
                    }
                }
            });
        }
        drop(frame_tx); // channel closes when all reader threads exit

        let s = shared.clone();
        tokio::spawn(async move { rx::rx_loop(frame_rx, s).await });
    }

    // ── TUI blocks the main task ──────────────────────────────────────────────
    tui_loop::run(shared.clone(), out_tx).await?;

    info!("node {} shutting down", cli.node_id);
    Ok(())
}

/// 640×480 greyscale test pattern: concentric rings + diagonal gradient.
fn generate_test_pattern() -> (Vec<u8>, u32, u32) {
    const W: usize = 640;
    const H: usize = 480;
    let cx = W as f32 / 2.0;
    let cy = H as f32 / 2.0;
    let max_r = (cx * cx + cy * cy).sqrt();

    let pixels: Vec<u8> = (0..H).flat_map(|y| {
        (0..W).map(move |x| {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let r = (dx * dx + dy * dy).sqrt();
            let ring = ((r / max_r * 8.0) as u8 % 2) * 127;
            let grad = ((x + y) % 256) as u8;
            ring.saturating_add(grad / 2)
        })
    }).collect();
    (pixels, W as u32, H as u32)
}
