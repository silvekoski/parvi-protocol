use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

use rand::Rng;
use tokio::sync::mpsc;

use crate::{
    messages::{AckPayload, TacticalMessage},
    Priority,
};

// ---------------------------------------------------------------------------
// Spoofer
// ---------------------------------------------------------------------------

pub struct SpooferHandle {
    running: Arc<AtomicBool>,
}

impl SpooferHandle {
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

/// Spawns a tokio task. Every 5 ms:
/// - Generate a `TacticalMessage::Ack` with a random `acked_seq`
/// - Serialize with bincode
/// - Corrupt the last 64 bytes (simulated bad Ed25519 signature)
/// - Send as `Priority::Critical`
/// - Increment `frames_tx_counter`
/// Stops when the running flag is false.
pub fn spawn_spoofer(
    tx: mpsc::Sender<(Vec<u8>, Priority)>,
    frames_tx_counter: Arc<AtomicU64>,
) -> SpooferHandle {
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);

    tokio::spawn(async move {
        while running_clone.load(Ordering::Relaxed) {
            let payload = {
                let mut rng = rand::thread_rng();
                let acked_seq: u64 = rng.gen();
                let msg = TacticalMessage::Ack(AckPayload { acked_seq });
                bincode::serialize(&msg).ok().map(|mut bytes| {
                    let len = bytes.len();
                    let corrupt_start = len.saturating_sub(64);
                    for b in &mut bytes[corrupt_start..] {
                        *b = rng.gen();
                    }
                    bytes
                })
            }; // rng dropped here, before any await

            if let Some(bytes) = payload {
                let _ = tx.send((bytes, Priority::Critical)).await;
                frames_tx_counter.fetch_add(1, Ordering::Relaxed);
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }
    });

    SpooferHandle { running }
}

// ---------------------------------------------------------------------------
// Jammer
// ---------------------------------------------------------------------------

pub struct JammerHandle {
    running: Arc<AtomicBool>,
}

impl JammerHandle {
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

/// Spawns a tokio task. Every 2 ms:
/// - Send a 1400-byte garbage `Vec<u8>` as `Priority::Bulk`
/// Simulates channel flooding.
pub fn spawn_jammer(tx: mpsc::Sender<(Vec<u8>, Priority)>) -> JammerHandle {
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);

    tokio::spawn(async move {
        while running_clone.load(Ordering::Relaxed) {
            let garbage: Vec<u8> = {
                let mut rng = rand::thread_rng();
                (0..1400).map(|_| rng.gen::<u8>()).collect()
            }; // rng dropped here, before any await
            let _ = tx.send((garbage, Priority::Bulk)).await;
            tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;
        }
    });

    JammerHandle { running }
}
