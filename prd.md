# Tactical Mesh: Operational Plan v5

**Hackathon:** Kova Labs Tactical Mesh Challenge
**Target prize:** Mesh Layer (€500). Stretch: Overall (€1000).
**Bonus:** Transmission Layer (€500) falls out for free.
**Priorities (in order):** Range, Packet Integrity, Encryption.
**Hardware:** 3x zsecurity RTL8812AU USB Wi-Fi adapters.
**Provided library:** `kova-wfb-rs` (raw 802.11 inject/capture, plaintext framing, no crypto).
**Routing protocol:** Simplified OLSR (RFC 3626 subset).
**Time budget:** ~26 hours.
**Team size assumed:** 2 to 3.
**Development approach:** Direct to hardware, no simulation.
**Stack:** 100% Rust.

---

## 1. Mission

Build a tactical mesh that lets a drone swarm coordinate ISR-cued strikes 50 km behind enemy lines, through forest canopy, against jamming and SIGINT. Deliver target coordinates and target imagery with priority-driven scheduling so critical data never waits behind bulk traffic. Routing via OLSR for proven protocol behavior. Authentication is stateless via Ed25519, so every frame proves itself.

## 2. Operational constraints

| Constraint | Design implication |
|---|---|
| 50 km mission range | Multi-hop OLSR routing. Single link cannot reach. |
| 1 km max inter-drone link | Minimum ~50 drones in chain. OLSR scales to this. |
| ISR-to-strike under 2 minutes | End-to-end latency budget tight but bounded. |
| Forest canopy, NLOS default | Low MCS, low frequency, OLSR's ETX-like link quality metric. |
| Heavy jamming + cyberattack | Channel hop, stateless auth, signed OLSR control traffic. |
| SIGINT-hostile | MAC randomization via stream_id rotation. |
| Payload: coordinates + imagery only | Everything else cut. |
| Kova differentiator | Drones fly THROUGH forest. Mesh enables this. |
| Targets: see + confirm + verify kill | Image data is mission-critical, not optional. |
| Overkill avoidance | Replicated target board with CRDT merge. |

## 3. Three pillars + cross-cutting priority

### Pillar 1: Range
- OLSR multi-hop routing with Dijkstra shortest path.
- Default MCS 0 for survival, adaptive escalation per link.
- Link quality feeds OLSR's cost metric so weak links are deprioritized in path selection.
- 5 GHz UNII-1 at venue (low congestion). 2.4 GHz for operational forest scenarios.

### Pillar 2: Packet integrity (stateless)
- **Per-frame Ed25519 signature.** 64-byte signature over auth header + payload hash. Verified independently by every relay and destination. Every OLSR HELLO and TC also signed.
- **Time-window verification.** Frame timestamp must be within ±30 seconds of receiver's local clock. Replaces sliding replay window. No per-source state.
- **Nonce cache.** Bounded time-windowed cache of `(src_node, timestamp, nonce)` triples. Decays automatically.
- **Reed-Solomon FEC** for messages over 1 KB. Adaptive ratio by priority class.
- **Critical-class ACK + retransmit** for P0 and P1 end-to-end.

### Pillar 3: Encryption
- **Shared PSK** loaded out-of-band. Group membership credential.
- **XChaCha20** stream cipher over payload. Ed25519 signature handles authentication.
- **Per-frame nonce** from `(node_id || timestamp || random)`.
- **Session key rotation** every 5 minutes or on jam-triggered hop.
- **Mission certificates** bind drone Ed25519 public keys to NodeIDs, signed by mission authority root (hardcoded in firmware).

### Cross-cutting: Priority scheduling
Priority enforced at four layers simultaneously: stream_id mapping (kernel demux), TX scheduler (strict precedence), RX dispatch (strict precedence), and FEC ratio (more redundancy for higher priority).

## 4. Why this stack wins for this scenario

**Stateless auth + OLSR + RTL8812AU + Rust** is a unique combination.

| Property | Why it matters |
|---|---|
| OLSR routing | Real protocol. Defence judges recognize it. Used in actual military mesh (USMC SRW). Visible routing table in TUI is impressive. |
| Stateless auth | Drone reboots, partitions, comms blackouts handled without recovery handshakes. Every frame proves itself. |
| Rust whole stack | Memory safety on a defence-grade radio. No buffer overflows in the link layer. Matches Kova's engineering culture. |
| RTL8812AU + kova-wfb-rs | Same hardware lineage as WFB-NG. Proven injection, monitor-mode capable. |
| Four priority classes | Coordinate data never waits behind image shards. P0 jam alerts get through under jamming. |
| Direct-to-hardware | No abstraction tax. Every commit runs on the real radio. |

## 5. Four priority levels

| Class | Purpose | Examples | Latency | Hops | FEC | Retransmit |
|---|---|---|---|---|---|---|
| **P0 EMERGENCY** | Life/mission critical | JamAlert, ChannelHop, Mayday | <50ms | 8 | (1,4) | 3x at 50ms |
| **P1 CRITICAL** | Operational decisions | TargetDetection, Command::Strike, BDA | <200ms | 6 | (1,3) | 3x exp 200ms |
| **P2 HIGH** | Situational awareness | StateReport, beacons, OLSR HELLO/TC, ACKs | <500ms | 4 | (1,2) | once 500ms |
| **P3 BULK** | Background | ImageShard, RequestImage, telemetry | best effort | 4 | (8,12) | none |

OLSR control traffic (HELLO, TC) runs at P2 HIGH. This guarantees the routing protocol gets through even when bulk traffic is heavy.

## 6. OLSR-lite (simplified RFC 3626)

### What we implement

1. **HELLO messages** broadcast every 1 second (faster than RFC default of 2s for demo responsiveness).
   - Lists the sender's neighbors and their link quality.
   - Receivers learn about their 2-hop neighbors.
   - Link sensing: a HELLO from N1 that lists N2 means N1↔N2 has a working link.

2. **TC (Topology Control) messages** broadcast every 2 seconds (RFC default 5s).
   - Lists the sender's neighbors (their link state).
   - Floods through the network (every node forwards TC at P2).
   - Receivers build a global topology graph.

3. **Link state database** at each node.
   - Set of (node_a, node_b, link_cost) edges.
   - Aged out: edges not refreshed in 6 seconds are removed.

4. **Routing table** computed via Dijkstra over the link-state database.
   - Recomputed on every topology change.
   - Stored as `dest_node -> (next_hop, total_cost, hop_count)`.

5. **Routed unicast forwarding.**
   - Frame addressed to destination N.
   - Look up route in routing table.
   - Forward to next_hop on appropriate priority stream.
   - No TTL flooding for unicast (route is known).

### What we skip (and why)

- **MPR (Multipoint Relays):** RFC 3626's flagship optimization. Reduces flood overhead by selecting a minimal forwarding set. **At 3 nodes, every node is its own MPR.** No bandwidth savings. Saves 200+ lines of complex code. Mention as "MPR optimization deferred to scale beyond 5 nodes" in the pitch.
- **Willingness levels:** Optional RFC feature. Skip.
- **HNA (Host Network Association):** For gateway nodes to external networks. Not applicable.
- **MID (Multiple Interface Declaration):** For multi-radio nodes. Not applicable.
- **Link types beyond SYMMETRIC:** RFC distinguishes asymmetric/lost/symmetric. We treat any heard HELLO as creating a symmetric link until proven otherwise.

### Convergence behavior

- Cold start: 3-5 seconds for routes to populate (need 2-3 HELLO rounds + 1 TC round).
- Topology change (node loss): 3-6 seconds (HELLO timeout 3s + recompute).
- Topology change (node gain): 1-2 seconds (first HELLO triggers recompute).

These are demoable: the TUI shows the routing table populating in real time.

### OLSR messages on the wire

```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum OlsrMessage {
    Hello {
        sender: u8,
        neighbors: Vec<(u8, LinkQuality)>,  // (node_id, quality)
        sent_at_ms: u64,
    },
    Tc {
        sender: u8,
        seq: u16,                            // monotonic per-source
        advertised_neighbors: Vec<(u8, LinkQuality)>,
        sent_at_ms: u64,
    },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct LinkQuality {
    pub rssi_dbm: i8,
    pub loss_rate_x100: u8,   // 0..100, percentage * 1
    pub mcs: u8,
}
```

### Link cost function

Lower is better. Used by Dijkstra.

```rust
fn link_cost(q: &LinkQuality) -> u32 {
    // Base cost from MCS (lower MCS = slower but more reliable)
    // Penalty from loss rate
    let base = match q.mcs {
        0 => 100,   // robust, slow
        1 => 90,
        2 => 80,
        3 => 70,
        4 => 60,
        _ => 50,
    };
    let loss_penalty = (q.loss_rate_x100 as u32) * 10;
    let rssi_bonus = ((-q.rssi_dbm as i32 - 30).max(0) as u32) * 2;
    base + loss_penalty + rssi_bonus
}
```

### State per node

```rust
pub struct OlsrState {
    pub local_id: u8,
    pub neighbors: HashMap<u8, NeighborEntry>,              // 1-hop, from HELLO
    pub two_hop: HashMap<u8, HashSet<u8>>,                  // 2-hop reachable via 1-hop
    pub topology: HashMap<u8, HashMap<u8, LinkQuality>>,    // global LSDB from TC
    pub routes: HashMap<u8, RouteEntry>,                    // dest -> (next_hop, cost, hops)
    pub last_tc_seq: HashMap<u8, u16>,                      // dedupe TC by source
}

pub struct NeighborEntry {
    pub node_id: u8,
    pub link_quality: LinkQuality,
    pub last_hello_ms: u64,
    pub last_seen_neighbors: Vec<u8>,        // their 1-hop neighbors (from their HELLO)
}

pub struct RouteEntry {
    pub destination: u8,
    pub next_hop: u8,
    pub cost: u32,
    pub hop_count: u8,
}
```

### Dijkstra recomputation

```rust
impl OlsrState {
    pub fn recompute_routes(&mut self) {
        let mut new_routes = HashMap::new();
        let mut dist: HashMap<u8, u32> = HashMap::new();
        let mut prev: HashMap<u8, u8> = HashMap::new();
        let mut heap = BinaryHeap::new();
        
        dist.insert(self.local_id, 0);
        heap.push(Reverse((0u32, self.local_id)));
        
        while let Some(Reverse((d, u))) = heap.pop() {
            if d > *dist.get(&u).unwrap_or(&u32::MAX) { continue; }
            
            // Edges from u: from neighbors table (1-hop) or topology table (rest)
            let edges: Vec<(u8, u32)> = if u == self.local_id {
                self.neighbors.iter()
                    .map(|(n, e)| (*n, link_cost(&e.link_quality)))
                    .collect()
            } else {
                self.topology.get(&u)
                    .map(|m| m.iter().map(|(n, q)| (*n, link_cost(q))).collect())
                    .unwrap_or_default()
            };
            
            for (v, edge_cost) in edges {
                let new_dist = d + edge_cost;
                if new_dist < *dist.get(&v).unwrap_or(&u32::MAX) {
                    dist.insert(v, new_dist);
                    prev.insert(v, u);
                    heap.push(Reverse((new_dist, v)));
                }
            }
        }
        
        for (dest, cost) in &dist {
            if *dest == self.local_id { continue; }
            let mut hop_count = 0;
            let mut next = *dest;
            while let Some(p) = prev.get(&next) {
                hop_count += 1;
                if *p == self.local_id { break; }
                next = *p;
            }
            new_routes.insert(*dest, RouteEntry {
                destination: *dest,
                next_hop: next,
                cost: *cost,
                hop_count,
            });
        }
        
        self.routes = new_routes;
    }
}
```

### TC forwarding rule (without MPR)

Without MPR optimization, every node forwards every TC it receives, exactly once per (source, seq) pair. This is more bandwidth than full OLSR but simpler. For 3-5 nodes the overhead is negligible.

```rust
fn process_tc(&mut self, tc: &Tc, from_link: u8) -> bool {
    let last_seq = self.last_tc_seq.get(&tc.sender).copied().unwrap_or(0);
    if tc.seq <= last_seq && (last_seq - tc.seq) < 100 {
        return false;  // already seen
    }
    self.last_tc_seq.insert(tc.sender, tc.seq);
    
    // Update topology DB
    let entry = self.topology.entry(tc.sender).or_default();
    entry.clear();
    for (n, q) in &tc.advertised_neighbors {
        entry.insert(*n, *q);
    }
    
    // Trigger route recomputation
    self.recompute_routes();
    
    true  // forward to peers
}
```

## 7. Wire format

```
┌──────────────────────────────────────────────────────────────┐
│ Radiotap (kova-wfb-rs)                            ~13 bytes  │
├──────────────────────────────────────────────────────────────┤
│ 802.11 MAC header (kova-wfb-rs)                    24 bytes  │
│   addr2/addr3 = 57:42:<stream_id big-endian>                 │
├──────────────────────────────────────────────────────────────┤
│ kova-wfb-rs framing header (provided)              ~8 bytes  │
├──────────────────────────────────────────────────────────────┤
│ Routed unicast header (mutable on forward)         4 bytes   │
│   last_hop_id   u8                                           │
│   hops_taken    u8                                           │
│   flags         u8                                           │
│   reserved      u8                                           │
├──────────────────────────────────────────────────────────────┤
│ Stateless auth header (signed contents start here) 32 bytes  │
│   src_node      u8                                           │
│   dst_node      u8     0xFF = broadcast (OLSR control)       │
│   msg_kind      u8     DATA | OLSR_HELLO | OLSR_TC | ACK     │
│   priority      u8                                           │
│   timestamp_ms  u64                                          │
│   nonce         [u8; 12]                                     │
│   payload_hash  [u8; 12]                                     │
│   epoch         u16                                          │
│   payload_len   u16                                          │
│   fec_index     u8                                           │
│   fec_k         u8                                           │
│   fec_n         u8                                           │
│   fec_block_id  u8                                           │
├──────────────────────────────────────────────────────────────┤
│ XChaCha20 ciphertext                            payload_len  │
│   plaintext = bincode(TacticalMessage) OR bincode(OlsrMsg)   │
├──────────────────────────────────────────────────────────────┤
│ Ed25519 signature                                  64 bytes  │
└──────────────────────────────────────────────────────────────┘
```

Total auth+routing overhead: 4 + 32 + 64 = 100 bytes per frame. Same as v4. The mesh header is now smaller (4 bytes vs 8) because we no longer need TTL for unicast (the route is computed).

For broadcast frames (HELLO, TC, JamAlert), TTL-style flooding is still needed. We use the `hops_taken` field as a hop counter and drop after 8 hops as a safety cap.

## 8. Stream_id mapping for priority

Four stream_ids derived from session key + epoch + priority label, rotated every 5 seconds:

```rust
fn derive_stream_id(session_key: &[u8; 32], epoch: u32, prio: Priority) -> u32 {
    let label = match prio {
        Priority::Emergency => "emergency",
        Priority::Critical  => "critical",
        Priority::High      => "high",       // includes OLSR HELLO/TC
        Priority::Bulk      => "bulk",
    };
    let mut h = blake3::Hasher::new();
    h.update(session_key);
    h.update(&epoch.to_le_bytes());
    h.update(label.as_bytes());
    let mut out = [0u8; 4];
    h.finalize_xof().fill(&mut out);
    u32::from_le_bytes(out)
}
```

LinkAdapter holds four Tx and four Rx instances, one per priority. Stream_id rotation provides MAC randomization (addr2/addr3 change every 5 seconds).

## 9. Target board (CRDT)

Unchanged from v4. Every drone keeps a local target table. State transitions propagate via the mesh. Merge rule:

```
DESTROYED > ABORTED > ENGAGED > ASSIGNED > DETECTED
```

Higher state wins. Same state, latest timestamp wins. Handles overkill avoidance and comms-loss reconciliation deterministically.

## 10. Image streaming

Unchanged from v4. Two modes (still-on-detection and adaptive keyframe), same wire format. FEC-encoded shards on P3. Relay cache for "mesh as CDN" behavior.

## 11. Tech stack (100% Rust)

### From kova-wfb-rs (provided)

Raw 802.11 inject + capture, radiotap parsing, stream_id demux. Used directly.

### Crates we add

```toml
[workspace.dependencies]
kova-wfb         = { git = "https://github.com/kova-labs/kova-wfb-rs.git" }
tokio            = { version = "1", features = ["full"] }
chacha20         = "0.9"
ed25519-dalek    = { version = "2", features = ["rand_core"] }
reed-solomon-simd = "3"
bincode          = "1.3"
serde            = { version = "1", features = ["derive"] }
ratatui          = "0.26"
crossterm        = "0.27"
anyhow           = "1"
thiserror        = "1"
parking_lot      = "0.12"
dashmap          = "5"
tracing          = "0.1"
tracing-subscriber = "0.3"
rand             = "0.8"
blake3           = "1.5"
clap             = { version = "4", features = ["derive"] }
```

No C dependencies. No Python. Whole stack Rust.

### Workspace layout

```
tactical-mesh/
├── Cargo.toml                  # workspace
├── tacticalmesh-link/          # kova-wfb-rs wrapper, four streams
├── tacticalmesh-wire/          # auth, FEC, mesh header, scheduler, queues
├── tacticalmesh-olsr/          # OLSR-lite: HELLO, TC, LSDB, Dijkstra
├── tacticalmesh-app/           # COP, target board, image cache, TUI
└── tacticalmesh-bin/           # main executable
```

## 12. Team roles

### Link/OLSR Engineer (M)
LinkAdapter, wire format, AEAD, Ed25519, FEC, scheduler. **Owns OLSR-lite implementation (the biggest single piece of work).**

### Application/UX Engineer (A)
TacticalMessage enum, target board CRDT, image cache, TUI, demo automation, spoofing/jamming threads.

### Demo/Integration Engineer (D)
End-to-end testing, pitch script, backup video. Drives the adversary on demo day. Fills gaps for M and A.

2-person team: M + A, with D's work compressed into hours 22 to 26.

## 13. Hour-by-hour plan (hardware-first)

### Phase 1: Hardware foundation (hours 0 to 5)

| Hour | Owner | Deliverable |
|---|---|---|
| 0.0 to 0.5 | All | Hardware picked up. ID deposited. |
| 0.5 to 1.5 | M | Monitor mode + channel 36 HT20 on each of three adapters. NetworkManager disabled. setcap applied. |
| 1.5 to 2.0 | M | Clone kova-wfb-rs, build, run `simple_txrx` between two laptops. Text round-trips. |
| 2.0 to 2.5 | M | Run `bandwidth` example. Record actual throughput numbers. |
| 2.5 to 4.0 | M | `tacticalmesh-link` crate: four-stream LinkAdapter, send/recv with seq + RSSI |
| 4.0 to 5.0 | M | `tacticalmesh-wire` crate: auth header, Ed25519 sign/verify, XChaCha20, nonce cache |
| 0.5 to 5.0 | A | TacticalMessage enum, bincode round-trip tests, ratatui scaffolding, mock state |

**Checkpoint at hour 5:** Two laptops exchange encrypted authenticated frames via kova-wfb-rs. Nonce cache rejects replays. Time-window verification works.

### Phase 2: OLSR-lite (hours 5 to 12)

| Hour | Owner | Deliverable |
|---|---|---|
| 5 to 7 | M | OlsrState struct, HELLO message format, broadcast HELLO loop at 1 Hz |
| 7 to 9 | M | Neighbor table from received HELLOs, link quality from radiotap RSSI, 2-hop discovery |
| 9 to 10 | M | TC message format, TC broadcast loop at 0.5 Hz, TC forwarding with seq dedup |
| 10 to 11 | M | LSDB updates from TC, Dijkstra route computation, routing table |
| 11 to 12 | M | Routed unicast forwarding using routing table |
| 5 to 8 | A | TX/RX scheduler with four priority queues |
| 8 to 10 | A | Target board CRDT, state merge tests |
| 10 to 12 | A | TUI panels for routing table, neighbor table, OLSR state |

**Checkpoint at hour 12:** Three nodes running OLSR. Routing table populates within 5 seconds. Kill BRAVO, routes to CHARLIE go away. Restore BRAVO, routes reappear within 6 seconds. **Mesh Layer prize in hand.**

### Phase 3: Tactical features (hours 12 to 18)

| Hour | Owner | Deliverable |
|---|---|---|
| 12 to 13 | M | Per-frame MCS selection in profile, adaptive per neighbor |
| 13 to 14 | M | Reed-Solomon FEC for messages >1KB |
| 14 to 15 | M | ACK + retransmit for P0/P1 over OLSR routes |
| 15 to 16 | M | Allow-list filter at link layer for "out of range" simulation |
| 12 to 14 | A | Spoofing attack thread (invalid Ed25519 sigs), TUI counter |
| 14 to 16 | A | Image shard assembly + cache + RequestImage handler |
| 16 to 17 | A | Adaptive streaming subscription, TUI image panel |
| 16 to 18 | M + A | Jam detector + channel hop coordinator + epoch rotation triggering stream_id rotation |

**Checkpoint at hour 18:** Full demo works end-to-end on hardware. All beats execute cleanly in 4 minutes.

### Phase 4: Polish + rehearsal (hours 18 to 24)

| Hour | Owner | Deliverable |
|---|---|---|
| 18 to 20 | All | First full demo dry-run, timed |
| 20 to 22 | All | Fix top 3 bugs |
| 22 to 23 | D | Pitch script, README, slide, backup video recorded |
| 23 to 24 | All | Final dry-run, code freeze |

### Phase 5: Buffer (hours 24 to 26)

For things that break. Driver flake, adapter death, setcap forgotten. Do not add features.

### What if OLSR takes longer than 7 hours?

The biggest risk in this plan. Mitigation:

- **Hour 8 sanity check:** If neighbor discovery is not working at hour 8, fall back to gossip-with-TTL routing (the v3/v4 design). It's known to work in 3 hours of focused effort. Lose the "we implemented OLSR" pitch line. Keep everything else.
- **Hour 10 hard deadline:** If Dijkstra-routed unicast is not working at hour 10, freeze OLSR at "neighbor discovery only" and route everything via broadcast flood with TTL. Inferior but demoable.

## 14. The demo

### Pre-demo setup (5 minutes before judges)

```bash
NIC=wlan1
sudo nmcli dev set "$NIC" managed no
sudo ip link set "$NIC" down
sudo iw dev "$NIC" set type monitor
sudo ip link set "$NIC" up
sudo iw dev "$NIC" set channel 36 HT20
sudo iw dev "$NIC" set power_save off
sudo iw dev "$NIC" set txpower fixed 3000
sudo setcap cap_net_raw,cap_net_admin=eip ./target/release/tacticalmesh-bin
./target/release/tacticalmesh-bin --node-id <1|2|3> --iface wlan1
```

### Physical layout

Three laptops, three adapters on stands (~1.5m apart) connected via USB extension cables. Antennas vertical. 5 GHz UNII-1 channel 36 HT20.

### Demo beats (4 minutes)

#### Beat 1: Mesh comes up, OLSR converges (45 seconds)

Start all three nodes within 5 seconds of each other. TUIs come alive.

For the first 3-4 seconds: neighbor tables populating, routing table empty.

Then: TUI routing tables populate. ALPHA's TUI shows:

```
ROUTING TABLE (computed via Dijkstra over LSDB):
  -> BRAVO:   next_hop BRAVO,   cost 100,  1 hop
  -> CHARLIE: next_hop BRAVO,   cost 268,  2 hops
```

**Pitch:** "Three nodes running OLSR. Each broadcasts HELLO messages every second so its neighbors know its presence and link quality. Topology Control messages every two seconds propagate link-state across the network. Every node maintains a full link-state database and computes shortest paths via Dijkstra. CHARLIE is two hops away from ALPHA via BRAVO. The routing table is visible. The routing is real."

#### Beat 2: Target detection + image stream + priority showing (60 seconds)

CHARLIE operator presses `t`. TargetDetection (P1) broadcasts. Routes via BRAVO. ALPHA target board updates within 200ms.

ALPHA operator presses `i`. RequestImage routed via OLSR. CHARLIE responds with 12 ImageShard frames (P3). BRAVO caches while relaying. ALPHA reconstructs.

Mid-stream, CHARLIE presses `t` for a second target. P1 frame jumps ahead of P3 shards. New target appears on ALPHA within 200ms.

**Pitch:** "Coordinates arrive in 200 milliseconds. Image streams in over 2 seconds, FEC-encoded, cached at the relay. Priority scheduling means coordinate updates never wait behind image shards. The mesh acts like a CDN: if STRIKE-B later requests the same image, BRAVO serves it from cache."

#### Beat 3: Spoofing rejection (30 seconds)

BRAVO operator presses `s`. Embedded spoofer floods bogus-signature frames.

ALPHA and CHARLIE TUI counter `SPOOFED FRAMES DROPPED: 0 → 1247` climbs rapidly.

**Pitch:** "Adversary inside RF environment injects forged frames. Every frame fails Ed25519 signature verification. The mesh logs and discards. Our auth is stateless: every frame proves itself. There is no session state to compromise."

Press `s` to stop.

#### Beat 4: Jamming + channel hop (45 seconds)

BRAVO presses `j`. Channel flood on ch36.

ALPHA detects via correlated link loss + noise spike. Broadcasts ChannelHop at P0 (Emergency: MCS 0 forced, 4x FEC).

All three nodes execute `iw dev wlan1 set channel 40`. Epoch increments, stream IDs rotate.

OLSR re-converges on new channel (3-4 seconds for HELLO + TC). Routes restored.

**Pitch:** "Jam detected within 1 second. Hop announce at Emergency priority with maximum FEC. Channel switch on all nodes. Stream IDs rotate so MAC fingerprints all change. OLSR converges on the new channel. Routes restored. Target board state survived intact. Three layers of attack defeated in five seconds."

#### Beat 5: Relay failure, OLSR reconverges (45 seconds)

Kill BRAVO (Ctrl+C).

ALPHA and CHARLIE see HELLO messages from BRAVO stop. After 3 seconds, neighbor entry expires. Topology recomputed. Route to CHARLIE flagged broken.

ALPHA's TUI:

```
ROUTING TABLE:
  -> BRAVO:   UNREACHABLE (no neighbor)
  -> CHARLIE: UNREACHABLE (no path via topology)
```

Restart BRAVO. First HELLO arrives within 1 second. Topology updates. Dijkstra reruns. Route reappears.

**Pitch:** "Relay drone goes down. OLSR's HELLO timeout detects the loss in 3 seconds. Topology database updates, Dijkstra recomputes, routes invalidated. When BRAVO comes back, no recovery handshake is needed. Stateless auth means the rebooted drone is indistinguishable from one that just woke up. First HELLO triggers route recomputation. Mission continues."

#### Beat 6: Overkill avoidance (30 seconds)

ALPHA assigned T-001 to both CHARLIE and a fictional STRIKE-B (pre-loaded). STRIKE-B "in comms blackout" (held back).

CHARLIE presses `f`. BDA DESTROYED propagates.

ALPHA releases STRIKE-B's update. STRIKE-B's local target board CRDT merge sees DESTROYED at higher precedence. Auto-aborts.

TUI on ALPHA: "STRIKE-B auto-aborted T-001 (CRDT merge), reassigning to T-002."

**Pitch:** "Two drones assigned same target. One destroys it. The other was in comms blackout. Reconnect, CRDT merge, see DESTROYED, auto-abort. Math handles overkill avoidance. No central controller required."

#### Beat 7: Closing (15 seconds)

**Pitch:** "Three drones is a slice. Operationally this scales to fifty drones over fifty kilometers of contested forest. OLSR routes through whatever topology emerges. Stateless authentication tolerates reboots, partitions, and comms blackouts without handshakes. Four priority classes ensure tactical decisions never wait behind bulk data. Hundred percent Rust. Built on kova-wfb-rs at the link layer. Every layer above the radio is ours. Questions?"

### When things fail on stage

- **OLSR not converging in time:** routes appear within 5 seconds. If not visible at second 6, kill and restart all three nodes. Routes will appear faster on second run.
- **TUI hangs:** switch to log output, narrate from text.
- **One node won't join:** demo with two. Lose multi-hop routing beat. Single-hop OLSR still demonstrable.
- **Setcap forgotten:** know the command, fix in 10 seconds.
- **Jammer affects judges' phones:** stop, confirm scope with mentors, restart.
- **Everything explodes:** play backup video.

## 15. What we are NOT doing

- Real-time H.264 multi-hop video
- Full RFC 3626 OLSR (MPR, willingness, HNA, MID skipped)
- Network simulation (direct to hardware)
- libp2p, BATMAN-adv, Reticulum
- C or Python anywhere in the stack
- Forward secrecy via PSK rotation (mention in pitch)
- Mission cert discovery (preload for demo)
- True per-frame TX preemption (fragment-level only)
- Persistent storage
- Custom CLI beyond clap
- Tests beyond unit tests for Ed25519, FEC, Dijkstra, CRDT merge

## 16. Risk register

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| OLSR takes >8 hours to implement | High | High | Hour 8 fallback: drop to gossip-with-TTL. Lose pitch line, keep demo. |
| kova-wfb-rs doesn't work on RTL8812AU | Medium | Catastrophic | Hour 0-1 priority. Ask mentors. |
| RTL8812AU wrong driver | High | High | `ethtool -i wlan1` shows `rtl88xxau_wfb`. Install wfb-tuned driver. |
| Adapter dies | Medium | High | We have 3. Demo needs 2. |
| Driver crashes under sustained TX | High | High | Rate-limit <100 pps. Test at hour 17. |
| Clock skew exceeds 30s window | Medium | High | NTP sync laptops. Verify `timedatectl status`. |
| Conference WiFi drowns chosen band | High | Medium | 5 GHz UNII-1 (36-48). Scan at venue. |
| Real RF range too good for "out of range" | Very high | Low | Allow-list filter at link layer. |
| setcap stripped after rebuild | Very high | Medium | scripts/setcap.sh after every build. |
| OLSR convergence visibly slow in demo | Medium | Medium | Speak to it explicitly: "watch the routes populate. Three seconds. That is the protocol working." |
| Stream_id rotation creates blackout | Medium | Medium | Overlap old and new for 1s. |
| Code freeze violated | High | Medium | After hour 24: no commits. |
| Live jammer disrupts judges | Medium | Catastrophic | Low TX power, ≤5s duration, confirm with mentors. |
| Backup video forgotten | Medium | Catastrophic | Record by hour 22. Verify it plays. |

## 17. Pre-start checklist

- [ ] Three RTL8812AU adapters powered, enumerated in `lsusb`
- [ ] `ethtool -i wlan1` shows wfb-tuned driver on each
- [ ] Monitor mode + channel 36 HT20 + power_save off
- [ ] kova-wfb-rs cloned, builds, `simple_txrx` works between two laptops
- [ ] Three USB extension cables (1.5m+)
- [ ] Three adapter stands
- [ ] `scripts/setcap.sh` exists, runs after every build
- [ ] GitHub repo, all teammates have push access
- [ ] Workspace Cargo.toml with five-crate layout committed
- [ ] Team comms channel set up
- [ ] Power: three strips, all chargers
- [ ] 5 GHz UNII-1 channel scan at venue
- [ ] Mentors introduced
- [ ] One person designated as the "no" person for scope creep
- [ ] NTP sync on all three laptops (`timedatectl status` shows synchronized)

## 18. Reference card

### Priority enum

```rust
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Priority {
    Emergency = 0,
    Critical  = 1,
    High      = 2,    // OLSR control traffic lives here
    Bulk      = 3,
}
```

### MsgKind enum

```rust
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MsgKind {
    Data = 0,
    OlsrHello = 1,
    OlsrTc = 2,
    Ack = 3,
    SessionKeyRotation = 4,
}
```

### LinkAdapter sketch

```rust
pub struct LinkAdapter {
    iface: String,
    node_id: u8,
    tx: [Tx; 4],         // one per priority
    rx: [Rx; 4],
    epoch: AtomicU32,
    session_key: [u8; 32],
    seq: AtomicU64,
    allow_list: Option<Vec<u8>>,
}

impl LinkAdapter {
    pub fn new(iface: &str, node_id: u8, session_key: [u8; 32]) -> anyhow::Result<Self> {
        let epoch = 0u32;
        let tx = [
            Tx::new(iface, derive_stream_id(&session_key, epoch, Priority::Emergency))?,
            Tx::new(iface, derive_stream_id(&session_key, epoch, Priority::Critical))?,
            Tx::new(iface, derive_stream_id(&session_key, epoch, Priority::High))?,
            Tx::new(iface, derive_stream_id(&session_key, epoch, Priority::Bulk))?,
        ];
        let rx = [
            Rx::new(iface, derive_stream_id(&session_key, epoch, Priority::Emergency))?,
            Rx::new(iface, derive_stream_id(&session_key, epoch, Priority::Critical))?,
            Rx::new(iface, derive_stream_id(&session_key, epoch, Priority::High))?,
            Rx::new(iface, derive_stream_id(&session_key, epoch, Priority::Bulk))?,
        ];
        Ok(Self { iface: iface.into(), node_id, tx, rx, epoch: AtomicU32::new(epoch),
                  session_key, seq: AtomicU64::new(0), allow_list: None })
    }

    pub fn send(&self, payload: &[u8], prio: Priority) -> anyhow::Result<u64> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let link_seq = (seq & 0xFFFF_FFFF) as u32;
        self.tx[prio as usize].send(payload, link_seq)?;
        Ok(seq)
    }

    pub fn set_channel(&self, channel: u8) -> anyhow::Result<()> {
        std::process::Command::new("iw")
            .args(["dev", &self.iface, "set", "channel", &channel.to_string(), "HT20"])
            .status()?;
        Ok(())
    }

    pub fn rotate_epoch(&mut self) -> anyhow::Result<()> {
        let new_epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        for p in [Priority::Emergency, Priority::Critical, Priority::High, Priority::Bulk] {
            let sid = derive_stream_id(&self.session_key, new_epoch, p);
            self.tx[p as usize] = Tx::new(&self.iface, sid)?;
            self.rx[p as usize] = Rx::new(&self.iface, sid)?;
        }
        Ok(())
    }
}
```

### OLSR HELLO loop

```rust
pub async fn hello_loop(state: Arc<RwLock<OlsrState>>, link: Arc<LinkAdapter>, 
                       identity: Arc<Identity>) {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tick.tick().await;
        let hello = {
            let s = state.read();
            OlsrMessage::Hello {
                sender: s.local_id,
                neighbors: s.neighbors.iter()
                    .map(|(id, e)| (*id, e.link_quality))
                    .collect(),
                sent_at_ms: now_ms(),
            }
        };
        let msg = TacticalMessage::Olsr(hello);
        let frame = build_frame(&msg, Priority::High, BROADCAST, &identity);
        link.send(&frame, Priority::High).ok();
    }
}
```

### OLSR TC loop

```rust
pub async fn tc_loop(state: Arc<RwLock<OlsrState>>, link: Arc<LinkAdapter>, 
                    identity: Arc<Identity>) {
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    let mut seq = 0u16;
    loop {
        tick.tick().await;
        seq = seq.wrapping_add(1);
        let tc = {
            let s = state.read();
            OlsrMessage::Tc {
                sender: s.local_id,
                seq,
                advertised_neighbors: s.neighbors.iter()
                    .map(|(id, e)| (*id, e.link_quality))
                    .collect(),
                sent_at_ms: now_ms(),
            }
        };
        let msg = TacticalMessage::Olsr(tc);
        let frame = build_frame(&msg, Priority::High, BROADCAST, &identity);
        link.send(&frame, Priority::High).ok();
    }
}
```

### OLSR neighbor expiration

```rust
pub async fn aging_loop(state: Arc<RwLock<OlsrState>>) {
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    loop {
        tick.tick().await;
        let now = now_ms();
        let mut s = state.write();
        let stale: Vec<u8> = s.neighbors.iter()
            .filter(|(_, e)| now.saturating_sub(e.last_hello_ms) > 3000)
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            s.neighbors.remove(&id);
            s.topology.remove(&id);
            s.two_hop.retain(|_, peers| { peers.remove(&id); !peers.is_empty() });
        }
        if !s.neighbors.is_empty() {
            s.recompute_routes();
        } else {
            s.routes.clear();
        }
    }
}
```

### Routed unicast send

```rust
pub fn route_and_send(
    state: &OlsrState,
    link: &LinkAdapter,
    identity: &Identity,
    msg: TacticalMessage,
    dst: u8,
    prio: Priority,
) -> anyhow::Result<()> {
    if dst == BROADCAST {
        let frame = build_frame(&msg, prio, dst, identity);
        link.send(&frame, prio)?;
        return Ok(());
    }
    let route = state.routes.get(&dst).ok_or(anyhow!("no route to {}", dst))?;
    let frame = build_frame_for_route(&msg, prio, dst, identity, route.next_hop);
    link.send(&frame, prio)?;
    Ok(())
}
```

### TUI layout

```
┌───────────────────────────────────────────────────────────────────────┐
│ TACTICAL MESH | ALPHA | Epoch 7 | Ch 36 HT20 | OLSR converged 4.2s    │
├──────────────────────────────────────────┬────────────────────────────┤
│  ROUTING TABLE (Dijkstra over LSDB)      │  NEIGHBORS (1-hop)         │
│  ──────────────────────                  │  ──────────────────────    │
│  -> BRAVO    via BRAVO    cost 100  1hop │  BRAVO   RSSI -42  MCS 4   │
│  -> CHARLIE  via BRAVO    cost 268  2hop │    last HELLO 0.4s ago     │
│                                          │                            │
│  TOPOLOGY (LSDB)                         │  TARGETS                   │
│  ──────────────                          │  ───────                   │
│  ALPHA  <-> BRAVO    cost 100            │  T-001 TANK  DESTROYED     │
│  BRAVO  <-> CHARLIE  cost 168            │  T-002 IFV   ENGAGED       │
│  ALPHA  <-> CHARLIE  (no edge)           │  T-003 RADAR DETECTED      │
│                                          │                            │
│  ATTACK COUNTERS                         │  QUEUES (TX/RX)            │
│  ───────────────                         │  ─────────                 │
│  Bad sigs dropped:   1247                │  P0:  0 /   1              │
│  Time-window drops:    18                │  P1:  3 /  23              │
│  Replayed nonces:       4                │  P2: 12 / 198 (OLSR)       │
│  Channel hops:          2                │  P3: 45 /  67              │
│  Stream rotations:     47                │                            │
├──────────────────────────────────────────┴────────────────────────────┤
│  IMAGE T-002 IFV  (12/12 shards via BRAVO cache)                      │
│  [88x88 thumbnail rendered]                                           │
├───────────────────────────────────────────────────────────────────────┤
│  LOG                                                                  │
│  [21:14:01] OLSR HELLO from BRAVO, link cost 100                      │
│  [21:14:02] OLSR TC from CHARLIE seq=4, topology updated              │
│  [21:14:03] Dijkstra rerun: 2 routes computed                         │
│  [21:14:04] JAM ch36 detected, hopping to ch40                        │
│  [21:14:08] OLSR reconverged on ch40, routes restored                 │
└───────────────────────────────────────────────────────────────────────┘
```

### TUI hotkeys

```
1-9     assign target N to highlighted striker
t       trigger TargetDetection (CHARLIE only)
f       trigger BDA Destroyed (CHARLIE only)
i       request image of highlighted target
s       toggle spoofer (BRAVO only)
j       toggle jammer (BRAVO only)
b       toggle simulated comms blackout for a striker
o       print OLSR state to log (debug)
q       quit
```

### Channel plan

Venue demo: 5 GHz UNII-1 channels 36, 40, 44, 48. Hop schedule: 36 → 40 on jam.
Operational: 2.4 GHz channels 1, 6, 11 for foliage. Same hop logic.

### Bandwidth budget (operational, per drone)

| Source | Rate | Size | Bytes/s |
|---|---|---|---|
| OLSR HELLO (P2) | 1 Hz | 200 B | 200 |
| OLSR TC (P2) | 0.5 Hz | 250 B | 125 |
| StateReport (P2) | 1 Hz | 100 B | 100 |
| Target updates (P1) | 0.5 Hz × 20 | 150 B | 1500 |
| Image keyframes (P3) | 3 fps × 5KB | - | 15000 |
| Emergency (P0) | ~0.05 Hz | 200 B | 10 |
| **Subtotal** | | | **~17 KB/s** |

OLSR overhead: ~325 B/s. About 2% of total. Negligible.

---

## Final reminder

Three things to protect at all costs:

1. **Hour 12 checkpoint.** OLSR converges, kill BRAVO, routes disappear, restore BRAVO, routes reappear. Three nodes routing tactical messages. If we are here at hour 12, the Mesh Layer prize is in hand.

2. **The OLSR fallback at hour 8.** If neighbor discovery is not working, drop to gossip-with-TTL. Better to lose one pitch line than lose the demo.

3. **Code freeze at hour 24.** Last-minute commits break demos.

Build OLSR. Run on hardware. Win.
