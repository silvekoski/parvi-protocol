use serde::{Deserialize, Serialize};

pub const HELLO_INTERVAL_MS: u64 = 1_000;
pub const TC_INTERVAL_MS:    u64 = 2_000;
pub const NEIGHBOR_TIMEOUT_MS: u64 = 3_000;
pub const AGING_TICK_MS:     u64 = 500;
pub const MAX_HOPS:          u8  = 8;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct LinkQuality {
    pub rssi_dbm: i8,
    /// Packet loss percentage × 1 (0 = 0 %, 100 = 100 %).
    pub loss_rate_x100: u8,
    pub mcs: u8,
}

impl Default for LinkQuality {
    fn default() -> Self {
        LinkQuality { rssi_dbm: -80, loss_rate_x100: 0, mcs: 0 }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Hello {
    pub sender: u8,
    /// Each entry is (neighbor_id, link quality AS SEEN BY SENDER).
    pub neighbors: Vec<(u8, LinkQuality)>,
    pub sent_at_ms: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Tc {
    pub sender: u8,
    pub seq: u16,
    pub advertised_neighbors: Vec<(u8, LinkQuality)>,
    pub sent_at_ms: u64,
}

/// Discriminated union serialised as the payload inside a wire TacticalMessage.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum OlsrMessage {
    Hello(Hello),
    Tc(Tc),
}

#[derive(Clone, Debug)]
pub struct RouteEntry {
    pub destination: u8,
    pub next_hop: u8,
    pub cost: u32,
    pub hop_count: u8,
}

#[derive(Clone, Debug)]
pub struct NeighborEntry {
    pub node_id: u8,
    pub link_quality: LinkQuality,
    /// Local timestamp (ms since epoch) when we last heard a HELLO from this node.
    pub last_hello_ms: u64,
    /// Nodes that this neighbor listed in its last HELLO (its own 1-hop set).
    pub last_seen_neighbors: Vec<u8>,
}
