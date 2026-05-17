# Tactical Mesh: Technical Overview

## What Is This?

Tactical Mesh is a drone swarm coordination network built entirely in Rust. It enables a fleet of drones to share target coordinates, battle damage assessments, commands, and imagery over a self-healing mesh radio link, with no infrastructure dependency and no single point of failure.

The system was built for the Kova Labs Tactical Mesh Challenge hackathon. The mission: deliver target coordinates and imagery reliably across 50 km of forest canopy, under active jamming and spoofing, using commodity USB radio adapters.

---

## System Architecture

The software is a Cargo workspace of five crates. Each crate owns one layer of the stack.

```
tacticalmesh-bin        <- executable, wires everything together
    tacticalmesh-app    <- messages, CRDT state, TUI
    tacticalmesh-olsr   <- routing (OLSR-lite + Dijkstra)
    tacticalmesh-wire   <- auth, encryption, FEC, scheduling
    tacticalmesh-link   <- radio I/O (wraps kova-wfb-rs)
```

The radio hardware is an RTL8812AU USB adapter running in monitor mode with raw 802.11 frame injection. The external library `kova-wfb-rs` handles raw frame injection and capture; `tacticalmesh-link` wraps it with priority queues and epoch rotation.

---

## Crate Details

### tacticalmesh-link

Wraps the radio hardware into a four-priority adapter. Each priority level gets its own logical stream, derived by hashing `(session_key || epoch || priority_label)` with BLAKE3. Stream IDs rotate every 5 minutes (configurable) to anonymize MAC addresses and break traffic analysis.

**Priority levels:**

| Level | Label | Use |
|-------|-------|-----|
| 0 | Emergency | JamAlert, ChannelHop, Mayday |
| 1 | Critical | Target detections, commands, BDA |
| 2 | High | State reports, OLSR HELLOs and TCs |
| 3 | Bulk | Image shards |

The radio backend is selected at compile time via Cargo features:

- `stub` compiles to no-ops for CI with no hardware
- `udp` sends UDP broadcasts over any NIC (development default)
- `wfb` injects raw 802.11 frames via kova-wfb-rs (production)

### tacticalmesh-wire

Handles everything between the application and the radio byte stream: frame construction, signing, encryption, FEC encoding, and scheduled transmission.

**Wire frame layout (per frame):**

```
RoutedHeader   4 bytes    last_hop_id, hops_taken, flags
AuthHeader    44 bytes    src, dst, kind, priority, timestamp, nonce,
                          payload_hash, epoch, payload_len, FEC fields
Ciphertext    variable    XChaCha20 over bincode(TacticalMessage)
Signature     64 bytes    Ed25519 over (AuthHeader || payload_hash)
```

**Submodules:**

- `crypto.rs`: Ed25519 sign/verify, XChaCha20 encrypt/decrypt, nonce generation
- `frame.rs`: `build_frame()`, `parse_and_verify_frame()`, shard assembly
- `fec.rs`: Reed-Solomon encoding with priority-dependent redundancy ratios
- `nonce_cache.rs`: stateless replay detection via `(src, timestamp, nonce)` triples; 30 second time window
- `scheduler.rs`: `TxScheduler` with strict priority drain (P0 serviced before P1, P1 before P2, P2 before P3)
- `identity.rs`: Ed25519 keypair and session key
- `pubkey_store.rs`: mission certificate registry for peer verification

**FEC redundancy by priority:**

| Priority | Data shards | Total shards | Overhead |
|----------|-------------|--------------|---------|
| P0 Emergency | 1 | 4 | 300% |
| P1 Critical | 1 | 3 | 200% |
| P2 High | 1 | 2 | 100% |
| P3 Bulk | 8 | 12 | 50% |

Emergency traffic gets maximum redundancy. Bulk imagery accepts less because it can be retransmitted.

### tacticalmesh-olsr

Implements OLSR-lite: a subset of RFC 3626 without the MPR optimization (which is unnecessary at swarm scale).

**State tracked:**

```
neighbors       1-hop neighbors discovered from HELLOs
two_hop         2-hop reachability (for TC generation)
topology        link state database from TC messages
routes          Dijkstra result, recomputed on every update
last_tc_seq     TC deduplication per originator
```

**Protocol timers:**

- HELLO broadcast: 1 Hz at P2
- TC broadcast: 0.5 Hz (every 2 seconds) at P2
- Neighbor expiry: neighbors not heard within 3 seconds are removed

**Link cost function:**

The cost of a link combines MCS rate, packet loss, and RSSI:

```
base        = cost table indexed by MCS (lower MCS = higher cost)
loss_pen    = loss_rate_x100 * 10
rssi_pen    = max(0, (-rssi_dbm - 30)) * 2
cost        = base + loss_pen + rssi_pen
```

Dijkstra is recomputed on every HELLO or TC arrival. Cold-start convergence is 3 to 5 seconds; re-convergence after a node reappears is 1 to 2 seconds; node loss detected within 3 to 6 seconds.

### tacticalmesh-app

Application layer: message types, shared tactical state as a CRDT, image cache, and TUI.

**Message types and priorities:**

| Message | Priority |
|---------|----------|
| JamAlert, ChannelHop, Mayday | P0 Emergency |
| TargetDetection, Command, Bda | P1 Critical |
| StateReport, OlsrMessage | P2 High |
| ImageShard, RequestImage | P3 Bulk |

**Target board CRDT:**

The shared target list is a grow-only state machine where each target progresses through:

```
Detected -> Assigned -> Engaged -> Aborted -> Destroyed
```

Merge rule: higher state always wins; ties broken by latest timestamp. This makes the target board conflict-free across concurrent updates from multiple drones. If two drones both assign the same target during a comms blackout, reconnection merges deterministically without human intervention. Overkill is avoided because the first drone to reach `Engaged` wins.

**Image pipeline:**

Images are JPEG-encoded, split into FEC shards (P3), transmitted as `ImageShard` messages, and reassembled when enough shards arrive. The `ImageCache` stores shards by `(target_id, block_id)` and triggers reassembly once `k` shards are present.

**TUI:**

Built with ratatui. Panels:

- Routing table and neighbor list
- Topology edges with link costs
- Target board with states
- TX queue depths per priority
- Attack counters (bad sigs, replayed nonces, channel hops, stream rotations)
- Image display
- Log

**Hotkeys:**

| Key | Action |
|-----|--------|
| `t` | Emit TargetDetection |
| `f` | Emit BDA Destroyed |
| `i` | Request image for selected target |
| `s` | Toggle spoofer (floods invalid signatures) |
| `j` | Toggle jammer (floods P3 traffic) |
| `b` | Toggle comms blackout simulation |
| `1-9` | Assign target to striker by number |
| `q` | Quit |

### tacticalmesh-bin

The executable. Parses CLI arguments, initializes all shared state behind `Arc` handles, and spawns the full async task tree with Tokio.

**CLI:**

```bash
tacticalmesh-bin --node-id <1|2|3> --iface wlan1 [--psk-file ./psk.bin] [--image-file test.jpg]
```

**Task tree:**

1. `hello_loop` - broadcasts OLSR HELLOs at P2
2. `tc_loop` - broadcasts OLSR TCs at P2
3. `aging_loop` - expires stale neighbors
4. `rx_loop` - receives frames across all four priority channels, parses and verifies, forwards broadcasts, routes unicast
5. `scheduler.run` - TX drain loop with strict priority ordering
6. `state_sync_loop` - snapshots OLSR state into AppState every 100 ms
7. `jam_detector_loop` - watches HELLO loss rate and P3 spike patterns, emits ChannelHop at P0 when jamming is detected
8. `epoch_rotation_loop` - rotates stream IDs every 5 minutes
9. `tui_loop` - renders TUI at 60 fps, handles keyboard input
10. `spoofer_loop` (demo only) - floods invalid Ed25519 signatures
11. `jammer_loop` (demo only) - floods high-rate P3 frames

**RX dispatch:**

```
dst == local_id           -> deliver to app handlers
dst == BROADCAST          -> deliver AND re-broadcast (if hops_taken < 8)
dst != local_id           -> route lookup, forward via LinkAdapter
```

---

## Security Model

**Threat model:** an active adversary on the RF channel who can inject, replay, and jam frames. No infrastructure-based PKI. Drones may reboot at any time.

**Defenses:**

| Attack | Defense |
|--------|---------|
| Frame injection / spoofing | Ed25519 signature on every frame; invalid sigs dropped and counted |
| Replay | Nonce cache: `(src, timestamp, nonce)` triples with 30 s window |
| Traffic analysis | BLAKE3-derived stream IDs rotate every 5 min; MAC randomized per epoch |
| Eavesdropping | XChaCha20 encryption with per-frame nonces; PSK membership credential |
| Jamming | Jam detector triggers P0 ChannelHop; epoch rotates on hop |
| Priority inversion | Strict priority TX scheduler; P0 always ahead of P3 |

Ed25519 is stateless: signing and verification require no prior handshake, so reboots and comms blackouts do not break authentication.

---

## Build

```bash
# Development (UDP backend, no hardware required)
cargo build --features udp

# Production (raw 802.11 injection via RTL8812AU)
cargo build --release --features wfb

# CI (no-op stub, no hardware)
cargo build --features stub

# Grant raw socket capabilities after production build
sudo ./scripts/setcap.sh
```

**Key dependencies:**

| Crate | Purpose |
|-------|---------|
| tokio | Async runtime |
| ed25519-dalek | Ed25519 signatures |
| chacha20 | XChaCha20 stream cipher |
| reed-solomon-simd | SIMD-accelerated FEC |
| bincode | Frame serialization |
| ratatui + crossterm | Terminal UI |
| blake3 | Stream ID derivation |
| parking_lot | Low-latency shared state |
| clap | CLI argument parsing |
| tracing | Structured logging |

---

## Demo Scenario (three nodes, four minutes)

Three laptops, each with one RTL8812AU adapter, running nodes ALPHA, BRAVO, and CHARLIE. BRAVO is the relay between ALPHA and CHARLIE.

**Seven beats:**

1. **OLSR convergence** (45 s): routing table populates, CHARLIE reachable via BRAVO
2. **Target detection**: CHARLIE emits a TargetDetection at P1; coordinates arrive in under 200 ms; images stream as P3 shards in the background
3. **Spoofing rejection**: BRAVO floods invalid signatures; counter increments; no frame accepted
4. **Jamming and channel hop**: jammer floods P3; HELLO loss detected; ChannelHop emitted at P0; epoch rotates; OLSR re-converges on new channel
5. **Relay failure and recovery**: BRAVO killed; routes drop; BRAVO restored; routes reappear within 3 seconds
6. **Overkill avoidance**: ALPHA and CHARLIE both assign the same target during a blackout; on reconnection CRDT merge resolves deterministically; only one strike proceeds
7. **Closing**: message on scale, stateless auth, and Rust memory safety

---

## File Layout

```
tactical-mesh-plan/
  Cargo.toml                      workspace root
  Cargo.lock
  scripts/setcap.sh               grant raw socket capabilities
  kova-wfb-rs/                    provided 802.11 injection library
  tacticalmesh-link/
    Cargo.toml
    src/lib.rs                    LinkAdapter, Priority, epoch rotation
  tacticalmesh-wire/
    Cargo.toml
    src/
      lib.rs
      frame.rs                    build_frame, parse_and_verify_frame
      crypto.rs                   Ed25519, XChaCha20, nonce generation
      fec.rs                      Reed-Solomon encode/decode
      nonce_cache.rs              replay detection
      scheduler.rs                TxScheduler
      identity.rs                 Ed25519 keypair + session key
      pubkey_store.rs             peer certificate registry
  tacticalmesh-olsr/
    Cargo.toml
    src/
      lib.rs
      state.rs                    OlsrState, neighbor/topology tables
      loops.rs                    hello_loop, tc_loop, aging_loop
      types.rs                    Hello, Tc, LinkQuality
      forward.rs                  TC forwarding logic
  tacticalmesh-app/
    Cargo.toml
    src/
      lib.rs
      messages.rs                 TacticalMessage enum
      crdt.rs                     Target board CRDT
      image_cache.rs              shard storage and reassembly
      image_codec.rs              JPEG encode/decode
      state.rs                    AppState, AttackCounters, QueueDepths
      demo.rs                     automated demo sequencing
      mock.rs                     simulated node data for testing
      tui/
        mod.rs
        render.rs                 ratatui layout and draw calls
  tacticalmesh-bin/
    Cargo.toml
    src/
      main.rs                     entry point, task spawning
      loops.rs                    jam_detector, epoch_rotation, state_sync
      rx.rs                       RX dispatch
      handlers.rs                 per-message-type app handlers
      tui_loop.rs                 render loop and input
      link_test.rs                hardware connectivity verification
      pcap_sniff.rs               raw frame inspection
  prd.md                          product requirements document
  plan-link.md                    link layer implementation plan
  plan-wire.md                    wire layer implementation plan
  plan-olsr.md                    OLSR implementation plan
  plan-app.md                     app layer implementation plan
  plan-bin.md                     binary integration plan
```
