# Plan: tacticalmesh-link

**Crate:** `tacticalmesh-link`
**Owner:** M (Link/OLSR Engineer)
**Can start:** Hour 0.5 (after hardware verified)
**Blocks:** tacticalmesh-wire (needs LinkAdapter), tacticalmesh-olsr (needs send/recv), tacticalmesh-bin (needs it all)
**PRD sections:** §3 Pillar 1, §8 Stream_id mapping, §18 LinkAdapter sketch, §11 Tech stack

---

## Goal

Wrap `kova-wfb-rs` into a `LinkAdapter` that exposes four prioritized TX/RX channels to the rest of the stack. Everything above this crate talks through `LinkAdapter` only — it never touches kova-wfb-rs directly.

---

## Interface contract (what other crates depend on)

```rust
// Public surface — do not change without coordinating with wire + olsr + bin owners
pub struct LinkAdapter { ... }

impl LinkAdapter {
    pub fn new(iface: &str, node_id: u8, session_key: [u8; 32]) -> anyhow::Result<Self>;
    pub fn send(&self, payload: &[u8], prio: Priority) -> anyhow::Result<u64>;
    pub fn recv(&self, prio: Priority) -> anyhow::Result<(Vec<u8>, RxMeta)>;
    pub fn set_channel(&self, channel: u8) -> anyhow::Result<()>;
    pub fn rotate_epoch(&mut self) -> anyhow::Result<()>;
    pub fn set_allow_list(&mut self, nodes: Option<Vec<u8>>);
    pub fn current_epoch(&self) -> u32;
}

pub struct RxMeta {
    pub rssi_dbm: i8,
    pub stream_id: u32,
    pub seq: u32,
}

pub enum Priority { Emergency = 0, Critical = 1, High = 2, Bulk = 3 }
```

---

## Tasks (in order)

### T1 — Hardware smoke test (hour 0.5–1.5)
- Disable NetworkManager, set monitor mode, channel 36 HT20, power_save off, setcap on each adapter.
- Run kova-wfb-rs `simple_txrx` between two laptops. Text round-trips. Unblock everything else.
- Run `bandwidth` example, record actual throughput number.

### T2 — Workspace scaffold (hour 1.5–2.0)
- Create `tactical-mesh/` workspace `Cargo.toml` with all five crates listed.
- Create `tacticalmesh-link/` crate skeleton with correct deps (`kova-wfb`, `tokio`, `parking_lot`, `anyhow`, `tracing`).
- Commit so teammates can `cargo check`.

### T3 — stream_id derivation (hour 2.0–2.5)
```rust
fn derive_stream_id(session_key: &[u8; 32], epoch: u32, prio: Priority) -> u32
```
- Uses blake3: hash of `session_key || epoch_le || priority_label`.
- Unit test: same inputs → same id, different prio → different id.

### T4 — Four-stream LinkAdapter (hour 2.5–4.0)
- Construct four `Tx` and four `Rx` from kova-wfb-rs, one per priority.
- `send()` dispatches to correct `Tx[prio]`, attaches a monotonic `seq`.
- `recv()` reads from correct `Rx[prio]`, returns payload + `RxMeta`.
- `rotate_epoch()` rebuilds all eight handles with new stream_ids, overlaps old+new for 1 second to avoid blackout.
- `set_channel()` shells out to `iw dev <iface> set channel <n> HT20`.
- `allow_list` filter: if set, drop any received frame whose source node_id is not in the list (used to simulate "out of range" in demo beat 4).

### T5 — Integration smoke test (hour 4.0–4.5)
- Two laptops: one sends P0 Emergency, one sends P3 Bulk simultaneously.
- Verify P0 arrives with correct RxMeta.rssi_dbm populated from radiotap.
- Verify epoch rotation doesn't lose a frame (overlap logic works).

---

## Key constants

```rust
pub const BROADCAST: u8 = 0xFF;
pub const MAX_FRAME_BYTES: usize = 1500;   // safe MTU for kova-wfb-rs
pub const EPOCH_ROTATION_SECS: u64 = 300; // 5 minutes
pub const OVERLAP_MS: u64 = 1000;         // keep old streams alive 1s on rotation
```

---

## Dependencies this crate has

- `kova-wfb` (git)
- `tokio` full
- `blake3` 1.5
- `parking_lot` 0.12
- `anyhow` 1
- `tracing` 0.1
- `serde` + `bincode` (for Priority enum shared with wire)

---

## What this crate does NOT do

- No auth, no crypto — that's tacticalmesh-wire.
- No OLSR parsing — that's tacticalmesh-olsr.
- No queuing beyond the four streams kova-wfb-rs already provides.

---

## Deliverable at hour 5 checkpoint

Two laptops exchange raw bytes at all four priority levels via `LinkAdapter`. `RxMeta.rssi_dbm` is populated. Epoch rotation works without frame loss. `scripts/setcap.sh` exists and is in the repo.
