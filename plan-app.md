# Plan: tacticalmesh-app

**Crate:** `tacticalmesh-app`
**Owner:** A (Application/UX Engineer)
**Can start:** Hour 0.5 (no hardware dependency — works entirely on mocks until hour 8)
**Blocks:** tacticalmesh-bin (needs TUI, message types, CRDT, image cache)
**PRD sections:** §5 Priority levels, §9 Target board CRDT, §10 Image streaming, §14 Demo beats, §18 TUI layout + hotkeys

---

## Goal

All application-layer logic: `TacticalMessage` enum, target board CRDT, image shard cache, TUI panels, and the demo automation (spoofer + jammer threads). This crate can be developed entirely with mock I/O until it integrates with the real radio at hour 8.

---

## Interface contract (what other crates depend on)

```rust
// Message type (used by wire for serialization, by olsr for OLSR wrapping)
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum TacticalMessage {
    TargetDetection(TargetDetection),
    Command(Command),
    Bda(Bda),
    StateReport(StateReport),
    ImageShard(ImageShard),
    RequestImage(RequestImage),
    JamAlert(JamAlert),
    ChannelHop(ChannelHop),
    Mayday(Mayday),
    Olsr(OlsrMessage),   // wraps tacticalmesh-olsr::OlsrMessage
    Ack(AckPayload),
}

// Target board (CRDT)
pub struct TargetBoard { ... }
impl TargetBoard {
    pub fn new() -> Self;
    pub fn merge(&mut self, update: TargetUpdate);
    pub fn targets(&self) -> impl Iterator<Item = &Target>;
}

// Image cache
pub struct ImageCache { ... }
impl ImageCache {
    pub fn insert_shard(&mut self, shard: ImageShard);
    pub fn get_complete(&self, target_id: u16) -> Option<&[u8]>; // reassembled image
    pub fn has_shard(&self, target_id: u16, fec_block: u8) -> bool;
}

// TUI
pub struct Tui { ... }
impl Tui {
    pub fn new() -> anyhow::Result<Self>;
    pub fn render(&mut self, state: &AppState) -> anyhow::Result<()>;
    pub fn next_event(&mut self) -> anyhow::Result<Option<TuiEvent>>;
}

pub enum TuiEvent {
    KeyPress(char),
    Quit,
}

// Shared app state (passed into Tui::render and updated by bin)
pub struct AppState {
    pub node_id: u8,
    pub epoch: u32,
    pub channel: u8,
    pub olsr_converged_in_ms: Option<u64>,
    pub routing_table: Vec<RouteDisplay>,
    pub neighbors: Vec<NeighborDisplay>,
    pub topology_edges: Vec<(String, String, u32)>,
    pub targets: Vec<TargetDisplay>,
    pub queues: QueueDepths,
    pub counters: AttackCounters,
    pub image: Option<ImageDisplay>,
    pub log: VecDeque<String>,
}
```

---

## Tasks (in order)

### T1 — TacticalMessage enum + bincode tests (hour 0.5–2.0)
Define all variants with their payload structs:
```rust
pub struct TargetDetection { pub target_id: u16, pub kind: TargetKind, pub lat: f32, pub lon: f32, pub detected_at_ms: u64, pub detector: u8 }
pub struct Command         { pub target_id: u16, pub op: CommandOp, pub issued_by: u8 }
pub struct Bda             { pub target_id: u16, pub result: BdaResult, pub at_ms: u64 }
pub struct StateReport     { pub node_id: u8, pub battery_pct: u8, pub lat: f32, pub lon: f32 }
pub struct ImageShard      { pub target_id: u16, pub block_id: u8, pub index: u8, pub k: u8, pub n: u8, pub data: Vec<u8> }
pub struct RequestImage    { pub target_id: u16, pub requester: u8 }
pub struct JamAlert        { pub detected_by: u8, pub channel: u8, pub at_ms: u64 }
pub struct ChannelHop      { pub new_channel: u8, pub new_epoch: u32, pub initiated_by: u8 }
pub struct Mayday          { pub node_id: u8, pub at_ms: u64 }
pub struct AckPayload      { pub acked_seq: u64 }
```
Unit tests: round-trip every variant through `bincode::serialize/deserialize`.

### T2 — ratatui scaffolding (hour 1.0–3.0, parallel with T1)
- Terminal init, raw mode, alternate screen.
- Layout from §18: header bar, routing table panel, neighbors panel, targets panel, attack counters, queue depths, image panel, log panel.
- Render with static mock data. Confirm it draws without panicking.
- `next_event()` non-blocking poll, returns `TuiEvent`.

### T3 — Target board CRDT (hour 2.0–4.0)
State machine per target: `DETECTED < ASSIGNED < ENGAGED < ABORTED < DESTROYED`.
Merge rule: higher state wins. Same state: latest timestamp wins.
```rust
pub struct Target {
    pub id: u16,
    pub kind: TargetKind,
    pub state: TargetState,
    pub lat: f32,
    pub lon: f32,
    pub updated_at_ms: u64,
    pub assigned_to: Option<u8>,
}
```
- `TargetBoard::merge(update)`: idempotent, deterministic.
- Unit tests:
  - Two DETECTED at same id → last-write wins on timestamp.
  - DESTROYED + ENGAGED (late) → stays DESTROYED.
  - ABORTED + ASSIGNED (late) → stays ABORTED.
  - Comms blackout scenario from §14 Beat 6.

### T4 — Live OLSR panels in TUI (hour 5.0–7.0)
Wire `AppState.routing_table`, `neighbors`, `topology_edges` into the routing table and neighbors panels. These fields are populated by tacticalmesh-bin from the OLSR state — render them here.

Display format per §18 TUI sketch:
```
ROUTING TABLE (Dijkstra over LSDB):
  -> BRAVO    via BRAVO    cost 100  1hop
  -> CHARLIE  via BRAVO    cost 268  2hop

TOPOLOGY (LSDB):
  ALPHA  <-> BRAVO    cost 100
  BRAVO  <-> CHARLIE  cost 168
```

### T5 — Image shard assembly + cache (hour 8.0–10.0)
- `ImageCache` stores shards by `(target_id, block_id)`.
- When a block has all `k` shards (or enough for FEC reconstruction), assemble full JPEG bytes.
- `get_complete(target_id)` returns reassembled bytes if available.
- Relay behavior: `has_shard()` used by bin to decide whether to cache a forwarded shard.
- TUI image panel: render a thumbnail from JPEG bytes as block-character ASCII (88×88 cell) using the `image` crate or a simple 2-color downscale.

### T6 — Image streaming subscription (hour 10.0–11.0)
- On `TuiEvent::KeyPress('i')`: emit `RequestImage { target_id, requester: local_id }` to bin's outbound queue.
- On receipt of `RequestImage` where local node has the image: emit shards in sequence as P3 frames.
- Adaptive keyframe: for demo, emit a 5 KB still image as 12 shards (P3 FEC (8,12)).

### T7 — Demo automation threads (hour 12.0–14.0)
Spoofer thread (toggled by `'s'` key on BRAVO):
```rust
async fn spoofer_loop(link: Arc<LinkAdapter>) {
    // flood garbage frames with invalid Ed25519 signatures
    // ~200 pps, Priority::Critical stream
}
```
- Every forged frame increments `AppState.counters.spoofed_frames_tx` locally.
- Verified in parse path of other nodes → `spoofed_frames_dropped` counter increments.

Jammer thread (toggled by `'j'` key on BRAVO):
- Floods the channel with high-rate P3 bulk frames (legal sim: just many frames, not RF noise).
- Other nodes detect via correlated link loss + noise spike heuristic → trigger `JamAlert`.

### T8 — Hotkey wiring (hour 14.0–15.0)
Per §18 TUI hotkeys:
```
t  → emit TargetDetection (CHARLIE role)
f  → emit BDA DESTROYED (CHARLIE role)
i  → emit RequestImage for highlighted target
s  → toggle spoofer (BRAVO role)
j  → toggle jammer (BRAVO role)
b  → toggle simulated comms blackout
o  → dump OLSR state to log panel
q  → quit
1-9 → assign target N to highlighted striker
```

### T9 — Attack counters panel (hour 15.0–15.5)
```rust
pub struct AttackCounters {
    pub bad_sigs_dropped: u64,
    pub time_window_drops: u64,
    pub replayed_nonces: u64,
    pub channel_hops: u64,
    pub stream_rotations: u64,
}
```
TUI panel refreshes every render tick. During Beat 3 demo, `bad_sigs_dropped` should visibly climb.

---

## Mock strategy (hours 0–8, before radio is ready)

Use a `MockLink` that replaces `LinkAdapter` with an `mpsc` channel pair:

```rust
pub struct MockLink {
    tx: tokio::sync::mpsc::Sender<(Vec<u8>, Priority)>,
    rx: tokio::sync::mpsc::Receiver<(Vec<u8>, Priority)>,
}
```

All unit tests and TUI development use `MockLink`. Drop-in replace with real `LinkAdapter` at integration time.

---

## Dependencies

- `ratatui` 0.26
- `crossterm` 0.27
- `serde` 1, `bincode` 1.3
- `tokio` full
- `parking_lot` 0.12
- `anyhow`, `thiserror`
- `tracing`
- `rand` 0.8

---

## What this crate does NOT do

- No radio I/O.
- No crypto — trusts wire layer to deliver authenticated plaintext.
- No OLSR logic — reads OlsrState via AppState populated by bin.

---

## Deliverable at hour 5 checkpoint

TUI renders routing table, neighbor list, target board, and attack counters using mock data. TacticalMessage enum serializes/deserializes without error. CRDT merge tests pass.

## Deliverable at hour 12 checkpoint

Live OLSR state visible in TUI (routing table, topology edges, neighbor RSSI). Target detection and BDA propagate through CRDT. Image panel renders a static test JPEG.
