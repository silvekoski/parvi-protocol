//! tacticalmesh-link — wraps kova-wfb-rs into a four-stream `LinkAdapter`.
//!
//! Build with `default = ["stub"]` (the default) for `cargo check` on any machine.
//! Remove the `stub` feature and add kova-wfb to Cargo.toml when running on hardware.

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{sync_channel, Receiver, SyncSender},
        Arc,
    },
    time::Duration,
};

use anyhow::anyhow;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::warn;

// ── radio shim ────────────────────────────────────────────────────────────────
// Three backends selected by feature flags (mutually exclusive, checked in order):
//   stub  — no-op, for CI / cargo check without hardware
//   udp   — UDP broadcast over any interface; works with any driver (DEFAULT)
//   wfb   — raw 802.11 injection via wfb_rs; requires monitor-mode + injection-capable driver

#[cfg(feature = "stub")]
mod radio {
    use std::time::Duration;

    pub struct RxFrame {
        pub data: Vec<u8>,
        pub rssi_dbm: i8,
        pub stream_id: u32,
        pub seq: u32,
    }
    pub struct Tx { pub stream_id: u32 }
    impl Tx {
        pub fn new(_iface: &str, stream_id: u32, _port: u16) -> anyhow::Result<Self> { Ok(Self { stream_id }) }
        pub fn send(&self, _data: &[u8], _seq: u32) -> anyhow::Result<()> { Ok(()) }
    }
    pub struct Rx { pub stream_id: u32 }
    impl Rx {
        pub fn new(_iface: &str, stream_id: u32, _port: u16) -> anyhow::Result<Self> { Ok(Self { stream_id }) }
        pub fn recv_timeout(&mut self, timeout: Duration) -> anyhow::Result<Option<RxFrame>> {
            std::thread::sleep(timeout);
            Ok(None)
        }
    }
}

// UDP broadcast backend — works with any driver, no special capabilities needed.
// Each priority stream maps to a separate UDP port: BASE_PORT + priority_index.
// Frames are prefixed with an 8-byte header: [stream_id: u32 LE][seq: u32 LE].
#[cfg(all(feature = "udp", not(feature = "stub")))]
mod radio {
    use std::net::{SocketAddr, UdpSocket};
    use std::time::{Duration, Instant};
    use socket2::{Domain, Protocol, Socket, Type};

    const MAX_UDP: usize = 65507;

    pub struct RxFrame {
        pub data: Vec<u8>,
        pub rssi_dbm: i8,
        pub stream_id: u32,
        pub seq: u32,
    }

    pub struct Tx {
        pub stream_id: u32,
        sock: UdpSocket,
        bcast: SocketAddr,
        seq: u32,
    }
    impl Tx {
        pub fn new(_iface: &str, stream_id: u32, port: u16) -> anyhow::Result<Self> {
            let sock = UdpSocket::bind("0.0.0.0:0")?;
            sock.set_broadcast(true)?;
            let bcast: SocketAddr = format!("255.255.255.255:{port}").parse().unwrap();
            Ok(Self { stream_id, sock, bcast, seq: 0 })
        }
        pub fn send(&mut self, data: &[u8], _seq: u32) -> anyhow::Result<()> {
            let mut pkt = Vec::with_capacity(8 + data.len());
            pkt.extend_from_slice(&self.stream_id.to_le_bytes());
            pkt.extend_from_slice(&self.seq.to_le_bytes());
            self.seq = self.seq.wrapping_add(1);
            pkt.extend_from_slice(data);
            self.sock.send_to(&pkt, self.bcast)?;
            Ok(())
        }
    }

    pub struct Rx {
        pub stream_id: u32,
        sock: UdpSocket,
    }
    impl Rx {
        pub fn new(_iface: &str, stream_id: u32, port: u16) -> anyhow::Result<Self> {
            // SO_REUSEPORT ensures every socket on this port gets a copy of each
            // broadcast packet — required when multiple nodes share the same host.
            // Each priority MUST have a dedicated port so SO_REUSEPORT load-balancing
            // doesn't scatter datagrams across the wrong priority's Rx sockets.
            let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
            s.set_reuse_address(true)?;
            s.set_reuse_port(true)?;
            s.set_broadcast(true)?;
            s.set_nonblocking(true)?;
            s.bind(&format!("0.0.0.0:{port}").parse::<std::net::SocketAddr>()?.into())?;
            let sock: UdpSocket = s.into();
            Ok(Self { stream_id, sock })
        }
        pub fn recv_timeout(&mut self, timeout: Duration) -> anyhow::Result<Option<RxFrame>> {
            let mut buf = vec![0u8; MAX_UDP];
            let deadline = Instant::now() + timeout;
            loop {
                match self.sock.recv_from(&mut buf) {
                    Ok((n, _addr)) if n >= 8 => {
                        let sid = u32::from_le_bytes(buf[..4].try_into().unwrap());
                        if sid != self.stream_id { continue; }
                        let seq = u32::from_le_bytes(buf[4..8].try_into().unwrap());
                        return Ok(Some(RxFrame {
                            data: buf[8..n].to_vec(),
                            rssi_dbm: -60,
                            stream_id: sid,
                            seq,
                        }));
                    }
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline { return Ok(None); }
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
}

#[cfg(all(not(feature = "stub"), not(feature = "udp")))]
mod radio {
    use std::time::Duration;
    use std::sync::Mutex;
    use wfb_rs::{WfbTx, WfbTxConfig, WfbRx, WfbRxConfig, WFB_FRAME_TYPE_DATA};

    pub struct RxFrame {
        pub data: Vec<u8>,
        pub rssi_dbm: i8,
        pub stream_id: u32,
        pub seq: u32,
    }
    pub struct Tx {
        pub stream_id: u32,
        inner: Mutex<WfbTx>,
    }
    impl Tx {
        pub fn new(iface: &str, stream_id: u32, _port: u16) -> anyhow::Result<Self> {
            let cfg = WfbTxConfig {
                iface: iface.to_owned(),
                stream_id,
                frame_type: WFB_FRAME_TYPE_DATA,
                mcs_index: 1,
                bandwidth: 20,
            };
            let tx = WfbTx::open(&cfg).map_err(|e| anyhow::anyhow!("{e:?}"))?;
            Ok(Self { stream_id, inner: Mutex::new(tx) })
        }
        pub fn send(&self, data: &[u8], seq: u32) -> anyhow::Result<()> {
            self.inner.lock().unwrap().send(data, seq).map_err(|e| anyhow::anyhow!("{e:?}"))
        }
    }
    pub struct Rx {
        pub stream_id: u32,
        inner: WfbRx,
        buf: Vec<u8>,
    }
    impl Rx {
        pub fn new(iface: &str, stream_id: u32, _port: u16) -> anyhow::Result<Self> {
            let cfg = WfbRxConfig {
                iface: iface.to_owned(),
                stream_id,
                rcv_buf_size: None,
                ignore_self_injected: false,
                ring_size: 256,
            };
            let inner = WfbRx::open(&cfg).map_err(|e| anyhow::anyhow!("{e:?}"))?;
            Ok(Self { stream_id, inner, buf: vec![0u8; 2048] })
        }
        pub fn recv_timeout(&mut self, timeout: Duration) -> anyhow::Result<Option<RxFrame>> {
            match self.inner.recv(&mut self.buf, timeout).map_err(|e| anyhow::anyhow!("{e:?}"))? {
                None => Ok(None),
                Some((len, meta)) => Ok(Some(RxFrame {
                    data: self.buf[..len].to_vec(),
                    rssi_dbm: meta.rssi[0],
                    stream_id: self.stream_id,
                    seq: meta.seq,
                })),
            }
        }
    }
}

// ── Public constants ──────────────────────────────────────────────────────────
pub const BROADCAST: u8 = 0xFF;
pub const MAX_FRAME_BYTES: usize = 1500;
pub const EPOCH_ROTATION_SECS: u64 = 300;
pub const OVERLAP_MS: u64 = 1000;

// Base UDP port for the four priority streams (Emergency=42800, Critical=42801,
// High=42802, Bulk=42803).  Each priority gets a dedicated port so that
// SO_REUSEPORT load-balancing never scatters datagrams across the wrong Rx thread.
const UDP_BASE_PORT: u16 = 42800;

const RX_CHAN_DEPTH: usize = 2048;
const RX_POLL_TIMEOUT: Duration = Duration::from_millis(50);

// Byte offset of `src_node` in the application payload delivered by kova-wfb.
// Wire layout: [RoutedHeader: 4 bytes][AuthHeader: src_node(1) dst_node(1) ...]
const SRC_NODE_OFFSET: usize = 4;

const PRIORITIES: [Priority; 4] = [
    Priority::Emergency,
    Priority::Critical,
    Priority::High,
    Priority::Bulk,
];

// ── Priority ──────────────────────────────────────────────────────────────────
#[repr(u8)]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Priority {
    Emergency = 0,
    Critical = 1,
    High = 2,
    Bulk = 3,
}

impl Priority {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Priority::Emergency),
            1 => Some(Priority::Critical),
            2 => Some(Priority::High),
            3 => Some(Priority::Bulk),
            _ => None,
        }
    }
}

// ── RxMeta ────────────────────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub struct RxMeta {
    pub rssi_dbm: i8,
    pub stream_id: u32,
    pub seq: u32,
}

// ── stream_id derivation ──────────────────────────────────────────────────────
fn derive_stream_id(session_key: &[u8; 32], epoch: u32, prio: Priority) -> u32 {
    let label: &[u8] = match prio {
        Priority::Emergency => b"emergency",
        Priority::Critical => b"critical",
        Priority::High => b"high",
        Priority::Bulk => b"bulk",
    };
    let mut h = blake3::Hasher::new();
    h.update(session_key);
    h.update(&epoch.to_le_bytes());
    h.update(label);
    let hash = h.finalize();
    u32::from_le_bytes(hash.as_bytes()[..4].try_into().unwrap())
}

// ── Background reader thread ──────────────────────────────────────────────────
fn spawn_reader(
    mut rx: radio::Rx,
    sender: SyncSender<(Vec<u8>, RxMeta)>,
    stop: Arc<AtomicBool>,
    allow_list: Arc<RwLock<Option<Vec<u8>>>>,
    node_id: u8,
) {
    std::thread::spawn(move || loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match rx.recv_timeout(RX_POLL_TIMEOUT) {
            Ok(Some(frame)) => {
                tracing::debug!("rx: got frame {} bytes on stream {:08x}", frame.data.len(), frame.stream_id);
                // Drop frames we originated ourselves (self-reception via UDP broadcast).
                if frame.data.len() > SRC_NODE_OFFSET
                    && frame.data[SRC_NODE_OFFSET] == node_id
                {
                    continue;
                }
                if let Some(ref list) = *allow_list.read() {
                    if frame.data.len() > SRC_NODE_OFFSET
                        && !list.contains(&frame.data[SRC_NODE_OFFSET])
                    {
                        continue;
                    }
                }
                let stream_id = frame.stream_id;
                let meta = RxMeta {
                    rssi_dbm: frame.rssi_dbm,
                    stream_id,
                    seq: frame.seq,
                };
                if sender.try_send((frame.data, meta)).is_err() {
                    warn!("rx channel full, frame dropped on stream {stream_id:08x}");
                }
            }
            Ok(None) => {}
            Err(e) => {
                warn!("radio rx error: {e}");
                break;
            }
        }
    });
}

// ── LinkAdapter ───────────────────────────────────────────────────────────────
pub struct LinkAdapter {
    iface: String,
    node_id: u8,
    tx: [parking_lot::Mutex<radio::Tx>; 4],
    // Receiver side: Mutex makes LinkAdapter Sync (Receiver<T> is !Sync).
    rx_chans: [parking_lot::Mutex<Receiver<(Vec<u8>, RxMeta)>>; 4],
    // Persistent senders shared with all reader threads (old and new after rotation).
    rx_senders: [SyncSender<(Vec<u8>, RxMeta)>; 4],
    // Stop signals for the current reader threads; rotated with each epoch.
    rx_stops: [Arc<AtomicBool>; 4],
    epoch: AtomicU32,
    session_key: [u8; 32],
    seq: AtomicU64,
    allow_list: Arc<RwLock<Option<Vec<u8>>>>,
}

impl LinkAdapter {
    pub fn new(iface: &str, node_id: u8, session_key: [u8; 32]) -> anyhow::Result<Self> {
        let epoch = 0u32;
        let allow_list = Arc::new(RwLock::new(None::<Vec<u8>>));

        // Build TX handles for each priority.
        // Port = BASE_PORT + priority_index.  Each priority gets a dedicated port so that
        // SO_REUSEPORT load-balancing never scatters datagrams across the wrong Rx thread.
        let tx_vec: anyhow::Result<Vec<parking_lot::Mutex<radio::Tx>>> = PRIORITIES
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let sid  = derive_stream_id(&session_key, epoch, p);
                let port = UDP_BASE_PORT + i as u16;
                radio::Tx::new(iface, sid, port).map(parking_lot::Mutex::new)
            })
            .collect();
        let tx: [parking_lot::Mutex<radio::Tx>; 4] = tx_vec?
            .try_into()
            .map_err(|_| anyhow!("priority count mismatch"))?;

        // Build RX channels.
        let (senders, receivers): (Vec<_>, Vec<_>) = (0..4)
            .map(|_| sync_channel::<(Vec<u8>, RxMeta)>(RX_CHAN_DEPTH))
            .unzip();

        let rx_senders: [SyncSender<(Vec<u8>, RxMeta)>; 4] = senders
            .try_into()
            .map_err(|_| anyhow!("sender count mismatch"))?;

        let rx_chans: [parking_lot::Mutex<Receiver<(Vec<u8>, RxMeta)>>; 4] = receivers
            .into_iter()
            .map(parking_lot::Mutex::new)
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|_| anyhow!("receiver count mismatch"))?;

        // Spawn one reader thread per priority stream.
        let mut stop_vec: Vec<Arc<AtomicBool>> = Vec::with_capacity(4);
        for (i, &prio) in PRIORITIES.iter().enumerate() {
            let port = UDP_BASE_PORT + i as u16;
            let rx = radio::Rx::new(iface, derive_stream_id(&session_key, epoch, prio), port)?;
            let stop = Arc::new(AtomicBool::new(false));
            spawn_reader(
                rx,
                rx_senders[i].clone(),
                Arc::clone(&stop),
                Arc::clone(&allow_list),
                node_id,
            );
            stop_vec.push(stop);
        }
        let rx_stops: [Arc<AtomicBool>; 4] = stop_vec
            .try_into()
            .map_err(|_| anyhow!("stop count mismatch"))?;

        Ok(Self {
            iface: iface.to_owned(),
            node_id,
            tx,
            rx_chans,
            rx_senders,
            rx_stops,
            epoch: AtomicU32::new(epoch),
            session_key,
            seq: AtomicU64::new(0),
            allow_list,
        })
    }

    /// Send `payload` on the chosen priority stream.  Returns the global sequence number.
    pub fn send(&self, payload: &[u8], prio: Priority) -> anyhow::Result<u64> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let link_seq = (seq & 0xFFFF_FFFF) as u32;
        self.tx[prio as usize].lock().send(payload, link_seq)?;
        Ok(seq)
    }

    /// Block until a frame arrives on `prio`.
    pub fn recv(&self, prio: Priority) -> anyhow::Result<(Vec<u8>, RxMeta)> {
        self.rx_chans[prio as usize]
            .lock()
            .recv()
            .map_err(|_| anyhow!("rx channel closed for {:?}", prio))
    }

    /// Channel-hop: shells out to `iw dev <iface> set channel <n> HT20`.
    pub fn set_channel(&self, channel: u8) -> anyhow::Result<()> {
        let status = std::process::Command::new("iw")
            .args([
                "dev",
                &self.iface,
                "set",
                "channel",
                &channel.to_string(),
                "HT20",
            ])
            .status()?;
        if !status.success() {
            anyhow::bail!("iw set channel {channel} exited {status}");
        }
        Ok(())
    }

    /// Rotate to the next epoch: rebuilds all eight kova-wfb handles with new
    /// stream_ids.  Old reader threads overlap for `OVERLAP_MS` so no frames
    /// are lost during the MAC-address rotation.
    pub fn rotate_epoch(&mut self) -> anyhow::Result<()> {
        let new_epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;

        for (i, &prio) in PRIORITIES.iter().enumerate() {
            let new_sid = derive_stream_id(&self.session_key, new_epoch, prio);
            let port = UDP_BASE_PORT + i as u16;

            // Replace TX handle immediately.
            *self.tx[i].lock() = radio::Tx::new(&self.iface, new_sid, port)?;

            // Schedule old reader thread to stop after the overlap window.
            let old_stop = Arc::clone(&self.rx_stops[i]);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(OVERLAP_MS));
                old_stop.store(true, Ordering::Relaxed);
            });

            // Start new reader thread on new stream; reuse the same sender so
            // recv() sees frames from both old and new streams transparently.
            let new_stop = Arc::new(AtomicBool::new(false));
            let new_rx = radio::Rx::new(&self.iface, new_sid, port)?;
            spawn_reader(
                new_rx,
                self.rx_senders[i].clone(),
                Arc::clone(&new_stop),
                Arc::clone(&self.allow_list),
                self.node_id,
            );
            self.rx_stops[i] = new_stop;
        }

        Ok(())
    }

    /// If `nodes` is Some, drop received frames from any node not in the list.
    /// Pass `None` to disable filtering.  Used to simulate "out of range" in demos.
    pub fn set_allow_list(&self, nodes: Option<Vec<u8>>) {
        *self.allow_list.write() = nodes;
    }

    pub fn current_epoch(&self) -> u32 {
        self.epoch.load(Ordering::Acquire)
    }

    pub fn node_id(&self) -> u8 {
        self.node_id
    }
}

impl Drop for LinkAdapter {
    fn drop(&mut self) {
        for stop in &self.rx_stops {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_id_deterministic() {
        let key = [42u8; 32];
        let a = derive_stream_id(&key, 0, Priority::Emergency);
        let b = derive_stream_id(&key, 0, Priority::Emergency);
        assert_eq!(a, b, "same inputs must produce same stream_id");
    }

    #[test]
    fn stream_id_differs_by_priority() {
        let key = [42u8; 32];
        let ids: Vec<u32> = PRIORITIES
            .iter()
            .map(|&p| derive_stream_id(&key, 0, p))
            .collect();
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                assert_ne!(ids[i], ids[j], "priorities {:?} and {:?} must differ", PRIORITIES[i], PRIORITIES[j]);
            }
        }
    }

    #[test]
    fn stream_id_differs_by_epoch() {
        let key = [7u8; 32];
        let a = derive_stream_id(&key, 0, Priority::High);
        let b = derive_stream_id(&key, 1, Priority::High);
        assert_ne!(a, b, "epoch rotation must change stream_id");
    }

    #[test]
    fn stream_id_differs_by_session_key() {
        let a = derive_stream_id(&[0u8; 32], 0, Priority::Emergency);
        let b = derive_stream_id(&[1u8; 32], 0, Priority::Emergency);
        assert_ne!(a, b, "different session keys must produce different stream_ids");
    }

    #[test]
    fn priority_from_u8_round_trip() {
        for v in 0u8..4 {
            let p = Priority::from_u8(v).unwrap();
            assert_eq!(p as u8, v);
        }
        assert!(Priority::from_u8(4).is_none());
    }

    #[test]
    fn priority_bincode_round_trip() {
        for p in PRIORITIES {
            let enc = bincode::serialize(&p).unwrap();
            let dec: Priority = bincode::deserialize(&enc).unwrap();
            assert_eq!(p, dec);
        }
    }

    #[cfg(feature = "stub")]
    #[test]
    fn link_adapter_new_and_send() {
        let key = [0u8; 32];
        let link = LinkAdapter::new("wlan1", 1, key).unwrap();
        // send should succeed without hardware in stub mode
        let seq = link.send(b"hello", Priority::High).unwrap();
        assert_eq!(seq, 0);
        let seq2 = link.send(b"world", Priority::Bulk).unwrap();
        assert_eq!(seq2, 1);
    }

    #[cfg(feature = "stub")]
    #[test]
    fn epoch_rotation_changes_stream_ids() {
        let key = [5u8; 32];
        let sid_e0 = derive_stream_id(&key, 0, Priority::Emergency);
        let sid_e1 = derive_stream_id(&key, 1, Priority::Emergency);
        assert_ne!(sid_e0, sid_e1);

        let mut link = LinkAdapter::new("wlan1", 1, key).unwrap();
        assert_eq!(link.current_epoch(), 0);
        link.rotate_epoch().unwrap();
        assert_eq!(link.current_epoch(), 1);
    }
}
