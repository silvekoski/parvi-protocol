use std::collections::VecDeque;

pub struct RouteDisplay {
    pub dest: String,
    pub via: String,
    pub cost: u32,
    pub hops: u8,
}

pub struct NeighborDisplay {
    pub name: String,
    pub rssi: i16,
    pub last_hello_ms: u64,
}

pub struct TargetDisplay {
    pub id: u16,
    pub kind: String,
    pub state: String,
    pub lat: f32,
    pub lon: f32,
    pub assigned_to: Option<u8>,
}

pub struct QueueDepths {
    pub critical: usize,
    pub normal: usize,
    pub bulk: usize,
}

pub struct AttackCounters {
    pub bad_sigs_dropped: u64,
    pub time_window_drops: u64,
    pub replayed_nonces: u64,
    pub channel_hops: u64,
    pub stream_rotations: u64,
    pub spoofed_frames_tx: u64,
    pub spoofed_frames_dropped: u64,
}

/// Pre-rendered block-char art
pub struct ImageDisplay {
    pub target_id: u16,
    pub ascii: String,
}

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

impl AppState {
    pub fn new(node_id: u8) -> Self {
        Self {
            node_id,
            epoch: 0,
            channel: 0,
            olsr_converged_in_ms: None,
            routing_table: Vec::new(),
            neighbors: Vec::new(),
            topology_edges: Vec::new(),
            targets: Vec::new(),
            queues: QueueDepths {
                critical: 0,
                normal: 0,
                bulk: 0,
            },
            counters: AttackCounters {
                bad_sigs_dropped: 0,
                time_window_drops: 0,
                replayed_nonces: 0,
                channel_hops: 0,
                stream_rotations: 0,
                spoofed_frames_tx: 0,
                spoofed_frames_dropped: 0,
            },
            image: None,
            log: VecDeque::with_capacity(200),
        }
    }

    pub fn push_log(&mut self, msg: String) {
        if self.log.len() >= 200 {
            self.log.pop_front();
        }
        self.log.push_back(msg);
    }

    pub fn mock() -> Self {
        let mut state = Self::new(1);
        state.epoch = 3;
        state.channel = 6;
        state.olsr_converged_in_ms = Some(142);

        state.routing_table = vec![
            RouteDisplay {
                dest: "NODE-2".to_string(),
                via: "NODE-2".to_string(),
                cost: 10,
                hops: 1,
            },
            RouteDisplay {
                dest: "NODE-3".to_string(),
                via: "NODE-2".to_string(),
                cost: 20,
                hops: 2,
            },
        ];

        state.neighbors = vec![
            NeighborDisplay {
                name: "NODE-2".to_string(),
                rssi: -68,
                last_hello_ms: 120,
            },
            NeighborDisplay {
                name: "NODE-4".to_string(),
                rssi: -82,
                last_hello_ms: 340,
            },
        ];

        state.topology_edges = vec![
            ("NODE-1".to_string(), "NODE-2".to_string(), 10),
            ("NODE-2".to_string(), "NODE-3".to_string(), 10),
            ("NODE-1".to_string(), "NODE-4".to_string(), 20),
        ];

        state.targets = vec![
            TargetDisplay {
                id: 101,
                kind: "Vehicle".to_string(),
                state: "ENGAGED".to_string(),
                lat: 48.1234,
                lon: 11.5678,
                assigned_to: Some(2),
            },
            TargetDisplay {
                id: 202,
                kind: "Personnel".to_string(),
                state: "DETECTED".to_string(),
                lat: 48.2345,
                lon: 11.6789,
                assigned_to: None,
            },
        ];

        state.queues = QueueDepths {
            critical: 2,
            normal: 7,
            bulk: 14,
        };

        state.counters = AttackCounters {
            bad_sigs_dropped: 13,
            time_window_drops: 4,
            replayed_nonces: 2,
            channel_hops: 1,
            stream_rotations: 3,
            spoofed_frames_tx: 50,
            spoofed_frames_dropped: 49,
        };

        state.push_log("[INFO] Node 1 started".to_string());
        state.push_log("[INFO] OLSR converged in 142ms".to_string());
        state.push_log("[WARN] Jam detected on channel 5, hopping to 6".to_string());
        state.push_log("[INFO] Target 101 assigned to node 2".to_string());
        state.push_log("[WARN] 13 frames dropped (bad signature)".to_string());

        state
    }
}
