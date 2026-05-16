# Plan: tacticalmesh-wire

**Crate:** `tacticalmesh-wire`
**Owner:** M (Link/OLSR Engineer) — starts after T4 of plan-link
**Can start:** Hour 4.0 (LinkAdapter send/recv ready)
**Blocks:** tacticalmesh-olsr (needs `build_frame`), tacticalmesh-app (needs `build_frame`, scheduler), tacticalmesh-bin
**PRD sections:** §2 Operational constraints, §3 Pillar 2 + 3, §7 Wire format, §5 Priority levels, §18 Priority + MsgKind enums

---

## Goal

Everything that touches a byte between `LinkAdapter` and the application layer lives here: the wire frame format, Ed25519 sign/verify, XChaCha20 encrypt/decrypt, Reed-Solomon FEC, nonce/replay cache, and the four-queue TX scheduler.

---

## Interface contract (what other crates depend on)

```rust
// Shared types (re-exported from this crate)
pub use priority::Priority;
pub use msg_kind::MsgKind;
pub struct AuthHeader { ... }   // 32 bytes on wire
pub struct RoutedHeader { ... } // 4 bytes on wire

// Frame construction
pub fn build_frame(
    msg: &TacticalMessage,  // from tacticalmesh-app
    prio: Priority,
    dst: u8,
    identity: &Identity,
) -> Vec<u8>;

pub fn build_frame_for_route(
    msg: &TacticalMessage,
    prio: Priority,
    dst: u8,
    identity: &Identity,
    next_hop: u8,
) -> Vec<u8>;

// Frame parsing + verification
pub fn parse_and_verify_frame(
    raw: &[u8],
    known_pubkeys: &PubkeyStore,    // mission cert registry
    nonce_cache: &NonceCache,
    local_time_ms: u64,
) -> Result<ParsedFrame, FrameError>;

pub struct ParsedFrame {
    pub routed: RoutedHeader,
    pub auth: AuthHeader,
    pub plaintext: Vec<u8>,       // decrypted payload
    pub rssi_dbm: i8,
}

pub enum FrameError {
    BadSignature,
    ReplayedNonce,
    TimeWindowExpired,
    DecryptFailed,
    TruncatedFrame,
}

// Scheduler
pub struct TxScheduler { ... }
impl TxScheduler {
    pub fn new(link: Arc<LinkAdapter>) -> Self;
    pub fn enqueue(&self, frame: Vec<u8>, prio: Priority);
    pub async fn run(&self);   // strict priority drain loop
}

// Identity
pub struct Identity {
    pub node_id: u8,
    pub signing_key: ed25519_dalek::SigningKey,
    pub verifying_key: ed25519_dalek::VerifyingKey,
    pub session_key: [u8; 32],   // PSK for XChaCha20
}
```

---

## Tasks (in order)

### T1 — Shared enums (hour 4.0–4.5, can do earlier on mock)
```rust
#[repr(u8)] pub enum Priority    { Emergency=0, Critical=1, High=2, Bulk=3 }
#[repr(u8)] pub enum MsgKind     { Data=0, OlsrHello=1, OlsrTc=2, Ack=3, SessionKeyRotation=4 }
```
- Derive `Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash`.
- Unit test round-trips.

### T2 — Auth header + wire layout (hour 4.0–4.5)
Wire layout (32-byte auth header, per §7):
```
src_node      u8
dst_node      u8
msg_kind      u8
priority      u8
timestamp_ms  u64
nonce         [u8; 12]
payload_hash  [u8; 12]   // first 12 bytes of blake3(plaintext)
epoch         u16
payload_len   u16
fec_index     u8
fec_k         u8
fec_n         u8
fec_block_id  u8
```
Total: 1+1+1+1+8+12+12+2+2+1+1+1+1 = 44 bytes. (PRD says 32 — adjust if needed, keep layout stable.)

Routed header (4 bytes):
```
last_hop_id  u8
hops_taken   u8
flags        u8
reserved     u8
```

### T3 — Ed25519 sign/verify (hour 4.5–5.5)
- `build_frame`: sign over `auth_header_bytes || payload_hash`. Append 64-byte sig.
- `parse_and_verify_frame`: verify sig before any other processing.
- Unit test: tampered byte → `BadSignature`.

### T4 — XChaCha20 encrypt/decrypt (hour 5.5–6.0)
- Nonce = `node_id || timestamp_ms[6 bytes] || random[5 bytes]` padded to 24 bytes.
- Encrypt plaintext → ciphertext with PSK.
- Decrypt in parse path after sig verification.
- Unit test round-trip.

### T5 — Nonce + time-window cache (hour 6.0–6.5)
```rust
pub struct NonceCache { ... }  // bounded HashMap<(u8, u64, [u8;12]), ()>
impl NonceCache {
    pub fn check_and_insert(src: u8, ts_ms: u64, nonce: &[u8;12]) -> Result<(), FrameError>;
}
```
- Reject if `|local_time_ms - timestamp_ms| > 30_000`.
- Reject if `(src, ts_ms, nonce)` already in cache.
- Evict entries older than 60 seconds on insert (keeps size bounded).

### T6 — Reed-Solomon FEC (hour 6.5–7.5)
- Only for messages > 1 KB (use `reed-solomon-simd`).
- FEC ratios per priority:
  - P0: (1,4) — 1 data shard, 4 total (3 parity)
  - P1: (1,3)
  - P2: (1,2)
  - P3: (8,12)
- `fec_index`, `fec_k`, `fec_n`, `fec_block_id` in auth header carry shard metadata.
- Unit test: corrupt 2 of 12 P3 shards → reconstruct.

### T7 — TX scheduler (hour 7.5–8.0)
```rust
pub struct TxScheduler {
    queues: [VecDeque<Vec<u8>>; 4],  // indexed by Priority
    link: Arc<LinkAdapter>,
}
```
- Strict priority: drain P0 fully, then P1, then P2, then P3.
- Single tokio task (`scheduler.run()`), woken by a `Notify` on enqueue.
- Unit test: enqueue P3 then P0, verify P0 pops first.

### T8 — ACK + retransmit for P0/P1 (hour 14–15, see Phase 3)
- Outbound P0/P1 frames get a `seq` stored in a pending-ack map.
- On receipt of `MsgKind::Ack`, remove from map.
- Retransmit 3× with backoff (50ms for P0, 200ms for P1 exp).
- Drop after 3 retries.

---

## Key constants

```rust
pub const TIME_WINDOW_MS: u64 = 30_000;
pub const NONCE_CACHE_TTL_MS: u64 = 60_000;
pub const FEC_THRESHOLD_BYTES: usize = 1024;
pub const BROADCAST: u8 = 0xFF;
pub const MAX_HOPS: u8 = 8;
```

---

## Dependencies

- `ed25519-dalek` 2 (features: rand_core)
- `chacha20` 0.9
- `reed-solomon-simd` 3
- `blake3` 1.5
- `bincode` 1.3
- `serde` 1
- `rand` 0.8
- `parking_lot` 0.12
- `tokio` full
- `anyhow`, `thiserror`

---

## What this crate does NOT do

- No radio I/O — uses LinkAdapter from tacticalmesh-link.
- No OLSR logic — that's tacticalmesh-olsr.
- No TUI — that's tacticalmesh-app.

---

## Deliverable at hour 5 checkpoint

`build_frame` + `parse_and_verify_frame` round-trip with real Ed25519 keys and XChaCha20. Nonce cache rejects replays. Time-window check rejects stale frames. Verified on loopback (no radio needed for unit tests).

## Deliverable at hour 12 checkpoint

FEC encoding/decoding for P3 bulk. TxScheduler draining with strict priority. ACK/retransmit deferred to Phase 3.
