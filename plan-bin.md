# Plan: tacticalmesh-bin

**Crate:** `tacticalmesh-bin`
**Owner:** M + A (joint, last to integrate — can start scaffolding at hour 5, full integration at hour 12)
**Can start scaffolding:** Hour 5.0 (types defined)
**Full integration:** Hour 12.0 (OLSR converging, wire verified, TUI rendering)
**PRD sections:** §14 Demo setup + beats, §8 Stream_id mapping, §16 Risk register, §17 Pre-start checklist, §18 Channel plan

---

## Goal

The main binary. Wires all four crates together: spawns OLSR loops, starts the TUI, routes received frames to the right handler, drives jam detection and channel hop, and runs the demo. Also owns `scripts/setcap.sh`.

---

## Tasks (in order)

### T1 — CLI + startup (hour 5.0–6.0, scaffold only)
```bash
tacticalmesh-bin --node-id <1|2|3> --iface wlan1 [--psk-file ./psk.bin]
```
Using `clap` derive:
```rust
#[derive(Parser)]
struct Cli {
    #[arg(long)] node_id: u8,
    #[arg(long, default_value = "wlan1")] iface: String,
    #[arg(long)] psk_file: Option<PathBuf>,
}
```
- Load PSK from file or generate random (dev only).
- Generate or load Ed25519 keypair (keyed from `node_id` for determinism in demo).
- Print startup banner and wait for hardware.

### T2 — Top-level tokio runtime (hour 5.0–6.0)
```rust
#[tokio::main]
async fn main() {
    let link = Arc::new(LinkAdapter::new(...)?);
    let identity = Arc::new(Identity { ... });
    let olsr_state = Arc::new(RwLock::new(OlsrState::new(node_id)));
    let app_state = Arc::new(RwLock::new(AppState::new(node_id)));
    let scheduler = Arc::new(TxScheduler::new(link.clone()));
    let nonce_cache = Arc::new(Mutex::new(NonceCache::new()));
    let image_cache = Arc::new(Mutex::new(ImageCache::new()));
    let target_board = Arc::new(RwLock::new(TargetBoard::new()));

    tokio::spawn(hello_loop(olsr_state.clone(), link.clone(), identity.clone()));
    tokio::spawn(tc_loop(olsr_state.clone(), link.clone(), identity.clone()));
    tokio::spawn(aging_loop(olsr_state.clone()));
    tokio::spawn(rx_loop(link.clone(), ...));
    tokio::spawn(scheduler.clone().run());
    tokio::spawn(epoch_rotation_loop(link.clone()));
    tui_loop(app_state.clone(), target_board.clone(), ...).await?;
}
```

### T3 — RX dispatch loop (hour 12.0–14.0)
Polls all four priority Rx channels round-robin (P0 first) at ~1ms tick:
```rust
async fn rx_loop(link, olsr_state, app_state, target_board, image_cache, nonce_cache, scheduler, identity) {
    loop {
        for prio in [P0, P1, P2, P3] {
            if let Ok((raw, meta)) = link.recv(prio) {
                handle_frame(raw, meta, prio, &mut state).await;
            }
        }
        tokio::task::yield_now().await;
    }
}
```

`handle_frame`:
1. `parse_and_verify_frame` → drop on any `FrameError`, increment counter.
2. Check `dst_node`: if mine → dispatch to handler. If broadcast → dispatch AND re-broadcast if `hops_taken < MAX_HOPS`. If foreign unicast → forward via routing table.
3. Dispatch by `msg_kind`:
   - `OlsrHello` → `olsr_state.process_hello()`
   - `OlsrTc` → `olsr_state.process_tc()` → if true, re-enqueue at P2
   - `Data` → decode `TacticalMessage`, route to app handler
   - `Ack` → wire layer ACK handler
   - `SessionKeyRotation` → trigger `link.rotate_epoch()`

### T4 — App message handlers (hour 14.0–15.0)
```rust
fn handle_tactical(msg: TacticalMessage, ...) {
    match msg {
        TargetDetection(td) => target_board.merge(TargetUpdate::from(td)),
        Bda(bda)            => target_board.merge(TargetUpdate::from(bda)),
        Command(cmd)        => target_board.merge(TargetUpdate::from(cmd)),
        StateReport(sr)     => app_state.update_peer(sr),
        ImageShard(shard)   => image_cache.insert_shard(shard),  // also cache if relaying
        RequestImage(req)   => handle_image_request(req, image_cache, scheduler, identity),
        JamAlert(ja)        => handle_jam_alert(ja, link, olsr_state, app_state),
        ChannelHop(ch)      => handle_channel_hop(ch, link, olsr_state, app_state),
        Mayday(_)           => { /* log + counter */ }
        Ack(_) | Olsr(_)    => { /* handled upstream */ }
    }
}
```

### T5 — Jam detection + channel hop (hour 16.0–18.0)
Jam detector (runs in rx_loop side):
- Track rolling 500ms window of frames received per priority.
- Jam heuristic: P2 OLSR HELLO count drops to 0 AND P3 frame rate spikes above 300 pps.
- On detection: broadcast `JamAlert { detected_by, channel, at_ms }` at P0.
- Log to `app_state.counters.channel_hops`.

Channel hop handler (on receipt of `ChannelHop` or self-triggered):
```rust
async fn handle_channel_hop(ch: ChannelHop, link, olsr_state, app_state) {
    link.set_channel(ch.new_channel).await?;
    link.rotate_epoch()?;    // also rotates stream_ids → MAC randomization
    app_state.write().channel = ch.new_channel;
    app_state.write().epoch = ch.new_epoch;
    log!("Hopped to ch{}, epoch {}", ch.new_channel, ch.new_epoch);
}
```

### T6 — Epoch rotation loop (hour 8.0, simple)
```rust
async fn epoch_rotation_loop(link: Arc<LinkAdapter>) {
    let mut tick = tokio::time::interval(Duration::from_secs(300));
    loop {
        tick.tick().await;
        link.write().rotate_epoch().ok();
    }
}
```

### T7 — AppState sync loop (hour 12.0–13.0)
Every 100ms, snapshot OLSR state into `AppState` for the TUI:
```rust
async fn state_sync_loop(olsr_state, app_state) {
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    loop {
        tick.tick().await;
        let s = olsr_state.read();
        let mut a = app_state.write();
        a.routing_table = s.routes.values().map(RouteDisplay::from).collect();
        a.neighbors = s.neighbors.values().map(NeighborDisplay::from).collect();
        a.topology_edges = /* flatten s.topology */ ...;
    }
}
```

### T8 — TUI event loop (hour 13.0–14.0)
```rust
async fn tui_loop(tui, app_state, outbound: mpsc::Sender<(TacticalMessage, u8, Priority)>) {
    loop {
        tui.render(&app_state.read())?;
        if let Some(ev) = tui.next_event()? {
            match ev {
                TuiEvent::KeyPress('q') => break,
                TuiEvent::KeyPress('t') => outbound.send((TargetDetection { ... }, BROADCAST, Priority::Critical)).await?,
                TuiEvent::KeyPress('f') => outbound.send((Bda { ... }, BROADCAST, Priority::Critical)).await?,
                TuiEvent::KeyPress('i') => outbound.send((RequestImage { ... }, dst, Priority::Bulk)).await?,
                TuiEvent::KeyPress('s') => toggle_spoofer(&app_state),
                TuiEvent::KeyPress('j') => toggle_jammer(&app_state),
                _ => {}
            }
        }
        tokio::time::sleep(Duration::from_millis(16)).await; // ~60fps TUI
    }
}
```

### T9 — scripts/setcap.sh (hour 2.0, do immediately)
```bash
#!/usr/bin/env bash
sudo setcap cap_net_raw,cap_net_admin=eip ./target/release/tacticalmesh-bin
echo "setcap applied"
```
Commit this file. Add to README. Run after every `cargo build --release`.

### T10 — Demo dry-run integration (hour 18.0–20.0)
Run through all 7 demo beats per §14:
- Beat 1: start all three nodes, wait for OLSR convergence, verify TUI routing table.
- Beat 2: `t` on CHARLIE, `i` on ALPHA, second `t` mid-stream.
- Beat 3: `s` on BRAVO, watch counter.
- Beat 4: `j` on BRAVO, watch hop, verify convergence on ch40.
- Beat 5: kill BRAVO, watch routes go UNREACHABLE, restart, watch routes return.
- Beat 6: CRDT overkill avoidance scenario.
- Beat 7: clean shutdown.

---

## Error handling philosophy

- Radio errors: log, continue. Never crash on a single bad frame.
- Route not found: log "no route to X", drop frame, increment counter. Do not retry here (wire ACK/retransmit handles P0/P1).
- TUI error: log and continue rendering. If terminal is broken, exit gracefully.
- `anyhow::Result` at all async task boundaries, `?` everywhere inside.

---

## Demo setup script (encode in bin or as shell script)

Per §14:
```bash
NIC=wlan1
sudo nmcli dev set "$NIC" managed no
sudo ip link set "$NIC" down
sudo iw dev "$NIC" set type monitor
sudo ip link set "$NIC" up
sudo iw dev "$NIC" set channel 36 HT20
sudo iw dev "$NIC" set power_save off
sudo iw dev "$NIC" set txpower fixed 3000
./scripts/setcap.sh
./target/release/tacticalmesh-bin --node-id 1 --iface wlan1
```

---

## Dependencies

- All four workspace crates
- `tokio` full
- `clap` 4 (derive)
- `anyhow`
- `tracing`, `tracing-subscriber`

---

## Deliverable at hour 18 checkpoint

Full demo works end-to-end. All 7 beats execute in ≤4 minutes. Backup video recorded. Code freeze begins at hour 24.
