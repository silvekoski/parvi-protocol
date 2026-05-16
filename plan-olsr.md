# Plan: tacticalmesh-olsr

**Crate:** `tacticalmesh-olsr`
**Owner:** M (Link/OLSR Engineer)
**Can start:** Hour 5.0 (wire frame build/parse ready)
**Blocks:** tacticalmesh-bin (needs route_and_send), tacticalmesh-app (reads routing table for TUI)
**PRD sections:** §3 Pillar 1, §6 OLSR-lite (entire section), §18 HELLO/TC loops + aging loop + routed unicast

---

## Goal

Implement OLSR-lite: HELLO broadcasts, TC broadcasts, link-state database, Dijkstra route computation, and routed unicast forwarding. This is the biggest single piece of work in the project. It must reach the hour-12 checkpoint independently.

---

## Interface contract (what other crates depend on)

```rust
pub struct OlsrState { ... }  // full state, behind Arc<RwLock<OlsrState>>

impl OlsrState {
    pub fn new(local_id: u8) -> Self;
    pub fn process_hello(&mut self, hello: &Hello, from_link: u8, rssi_dbm: i8);
    pub fn process_tc(&mut self, tc: &Tc, from_link: u8) -> bool; // true = forward
    pub fn recompute_routes(&mut self);
    pub fn route_to(&self, dst: u8) -> Option<&RouteEntry>;
    pub fn neighbors(&self) -> impl Iterator<Item = (&u8, &NeighborEntry)>;
    pub fn topology_edges(&self) -> impl Iterator<Item = (u8, u8, &LinkQuality)>;
}

// Async task entry points (spawn these in tacticalmesh-bin)
pub async fn hello_loop(state: Arc<RwLock<OlsrState>>, link: Arc<LinkAdapter>, identity: Arc<Identity>);
pub async fn tc_loop(state: Arc<RwLock<OlsrState>>, link: Arc<LinkAdapter>, identity: Arc<Identity>);
pub async fn aging_loop(state: Arc<RwLock<OlsrState>>);

// Forward decision (call from RX path in bin)
pub fn should_forward_tc(state: &OlsrState, tc: &Tc) -> bool;

// Routed send (call from app layer)
pub fn route_and_send(
    state: &OlsrState,
    link: &LinkAdapter,
    identity: &Identity,
    msg: TacticalMessage,
    dst: u8,
    prio: Priority,
) -> anyhow::Result<()>;

// Wire types
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum OlsrMessage { Hello(Hello), Tc(Tc) }

pub struct Hello { pub sender: u8, pub neighbors: Vec<(u8, LinkQuality)>, pub sent_at_ms: u64 }
pub struct Tc    { pub sender: u8, pub seq: u16, pub advertised_neighbors: Vec<(u8, LinkQuality)>, pub sent_at_ms: u64 }

pub struct LinkQuality { pub rssi_dbm: i8, pub loss_rate_x100: u8, pub mcs: u8 }
pub struct RouteEntry  { pub destination: u8, pub next_hop: u8, pub cost: u32, pub hop_count: u8 }
pub struct NeighborEntry { pub node_id: u8, pub link_quality: LinkQuality, pub last_hello_ms: u64, pub last_seen_neighbors: Vec<u8> }
```

---

## Tasks (in order)

### T1 — Data structures + link cost (hour 5.0–5.5)
- `OlsrState`, `NeighborEntry`, `RouteEntry`, `LinkQuality` structs.
- `link_cost(q: &LinkQuality) -> u32` function (per §6):
  - base: MCS 0→100, 1→90, 2→80, 3→70, 4→60, _→50
  - loss penalty: `loss_rate_x100 * 10`
  - rssi bonus: `((-rssi_dbm - 30).max(0)) * 2`
- Unit test: known inputs produce expected cost.

### T2 — HELLO message broadcast (hour 5.5–6.5)
- `OlsrMessage::Hello` serialized via bincode.
- `hello_loop`: ticks at 1 Hz, reads state, builds Hello with current neighbors, calls `build_frame` at `Priority::High` with `dst = BROADCAST`, sends via link.
- `process_hello`: update `neighbors` HashMap with new `NeighborEntry`, extract their neighbor list into `two_hop`, trigger `recompute_routes`.

### T3 — Neighbor discovery test (hour 6.5–7.0)
- Two nodes (can be same machine, two terminal tabs, two adapters): each sends HELLOs, each sees the other in its neighbor table within 2 seconds.
- Log RSSI from radiotap via `RxMeta`.

### T4 — TC message broadcast (hour 7.0–7.5)
- `OlsrMessage::Tc` serialized via bincode.
- `tc_loop`: ticks at 0.5 Hz, monotonic `seq`, sends advertised_neighbors = current 1-hop neighbors.
- `process_tc`: dedupe by `(sender, seq)` using `last_tc_seq`; update `topology` DB; trigger `recompute_routes`; return `true` if forwarded (new seq).
- TC forwarding: every node forwards every TC it has not seen before (no MPR).

### T5 — Dijkstra route computation (hour 7.5–9.0)
Implement `recompute_routes` exactly as in §6:
- BinaryHeap<Reverse<(cost, node_id)>>.
- Edges from `self.local_id`: from `neighbors` table.
- Edges from other nodes: from `topology` table.
- Walk `prev` map backward to find `next_hop` and `hop_count`.
- Store result in `self.routes`.
- Unit test (no radio): build a synthetic 3-node topology, verify routes are correct.

### T6 — Neighbor aging (hour 9.0–9.5)
`aging_loop` per §18:
- Ticks at 500ms.
- Remove neighbors not heard from in 3000ms.
- On removal: also clear their topology entries, remove from two_hop.
- If neighbors non-empty: `recompute_routes`. Else: `routes.clear()`.

### T7 — Routed unicast forwarding (hour 9.5–10.5)
`route_and_send`:
- If `dst == BROADCAST`: broadcast directly.
- Else: look up `routes.get(&dst)` → `RouteEntry` → `next_hop`.
- Build frame with `next_hop` in routed header.
- Send at requested priority.
- Return `anyhow::Error` if no route.

RX path (in tacticalmesh-bin, but wire up here):
- On receive: check `routed_header.last_hop_id` and frame `dst_node`.
- If `dst_node == local_id`: deliver to app.
- If `dst_node == BROADCAST`: deliver AND re-broadcast if `hops_taken < 8`.
- If `dst_node != local_id && dst_node != BROADCAST`: look up route, forward.

### T8 — Three-node end-to-end test (hour 10.5–12.0)
- Three nodes running on three adapters (or two if one unavailable).
- Routing table populates within 5 seconds: ALPHA sees `-> CHARLIE via BRAVO, 2 hops`.
- Kill BRAVO: after 3s, ALPHA's route to CHARLIE disappears.
- Restart BRAVO: within 2s, route reappears.
- Send a P1 message from ALPHA to CHARLIE via BRAVO. Confirm delivery.

---

## What we skip (per §6)

- MPR selection — unnecessary at ≤5 nodes.
- Willingness levels.
- HNA, MID.
- Asymmetric link types — treat any heard HELLO as symmetric.

---

## Key constants

```rust
pub const HELLO_INTERVAL_MS: u64 = 1_000;
pub const TC_INTERVAL_MS:    u64 = 2_000;
pub const NEIGHBOR_TIMEOUT_MS: u64 = 3_000;
pub const AGING_TICK_MS:     u64 = 500;
pub const BROADCAST:         u8  = 0xFF;
pub const MAX_HOPS:          u8  = 8;
```

---

## Hour-8 fallback

If `process_hello` is not populating neighbor tables by hour 8, switch to gossip-with-TTL:
- Every frame gets a TTL field (use `hops_taken` already in the routed header).
- On RX: decrement TTL, re-broadcast if TTL > 0.
- No routing table, no Dijkstra. Everything floods.
- Loses the "OLSR" pitch line but keeps the demo demoable.
- Inform bin owner immediately if falling back.

---

## Dependencies

- `bincode` 1.3, `serde` 1
- `tokio` full
- `parking_lot` 0.12
- `dashmap` 5 (optional, for lock-free topology table if contention)
- `anyhow`, `thiserror`
- `tracing`

---

## Deliverable at hour 12 checkpoint

Three nodes. OLSR converges in ≤5s. Kill BRAVO → ALPHA routing table goes `UNREACHABLE`. Restart BRAVO → routes reappear within 6s. Routed unicast delivers P1 message ALPHA→CHARLIE via BRAVO. **Mesh Layer prize secured.**
