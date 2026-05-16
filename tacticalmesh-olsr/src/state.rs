use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use crate::types::{Hello, LinkQuality, NeighborEntry, RouteEntry, Tc, MAX_HOPS};
use crate::now_ms;

/// Lower cost = better path. Used by Dijkstra and link quality comparisons.
pub fn link_cost(q: &LinkQuality) -> u32 {
    let base: u32 = match q.mcs {
        0 => 100,
        1 => 90,
        2 => 80,
        3 => 70,
        4 => 60,
        _ => 50,
    };
    let loss_penalty = (q.loss_rate_x100 as u32) * 10;
    // Penalty increases for weak signals: rssi_dbm = -80 adds 100, -30 adds 0.
    let rssi_penalty = ((-q.rssi_dbm as i32 - 30).max(0) as u32) * 2;
    base + loss_penalty + rssi_penalty
}

pub struct OlsrState {
    pub local_id: u8,
    /// 1-hop neighbors discovered via HELLO.
    pub neighbors: HashMap<u8, NeighborEntry>,
    /// 2-hop reachability: maps 2-hop_node → set of 1-hop gateways that can reach it.
    pub two_hop: HashMap<u8, HashSet<u8>>,
    /// Global link-state database populated by TC messages: sender → (neighbor, quality).
    pub topology: HashMap<u8, HashMap<u8, LinkQuality>>,
    /// Dijkstra result: destination → route entry.
    pub routes: HashMap<u8, RouteEntry>,
    /// Most recently processed TC seq per sender (for dedup with wraparound).
    pub last_tc_seq: HashMap<u8, u16>,
}

impl OlsrState {
    pub fn new(local_id: u8) -> Self {
        Self {
            local_id,
            neighbors: HashMap::new(),
            two_hop: HashMap::new(),
            topology: HashMap::new(),
            routes: HashMap::new(),
            last_tc_seq: HashMap::new(),
        }
    }

    /// Update neighbor table from a received HELLO.
    /// `rssi_dbm` comes from the radiotap measurement of the received frame.
    pub fn process_hello(&mut self, hello: &Hello, _from_link: u8, rssi_dbm: i8) {
        let node_id = hello.sender;
        if node_id == self.local_id { return; }

        // Use our own radiotap RSSI; take MCS from what the sender reported for us.
        let mcs = hello.neighbors.iter()
            .find(|(n, _)| *n == self.local_id)
            .map(|(_, q)| q.mcs)
            .unwrap_or(0);

        let quality = LinkQuality { rssi_dbm, loss_rate_x100: 0, mcs };
        let last_seen_neighbors = hello.neighbors.iter().map(|(n, _)| *n).collect();

        self.neighbors.insert(node_id, NeighborEntry {
            node_id,
            link_quality: quality,
            last_hello_ms: now_ms(),
            last_seen_neighbors,
        });

        // Refresh two_hop: remove node_id's old gateway contribution, then re-add.
        for gateways in self.two_hop.values_mut() {
            gateways.remove(&node_id);
        }
        self.two_hop.retain(|_, gateways| !gateways.is_empty());

        for (two_hop_node, _) in &hello.neighbors {
            if *two_hop_node != self.local_id && !self.neighbors.contains_key(two_hop_node) {
                self.two_hop.entry(*two_hop_node).or_default().insert(node_id);
            }
        }

        self.recompute_routes();
    }

    /// Update LSDB from a received TC. Returns `true` if this is a new (unseen) TC that
    /// should be forwarded.
    pub fn process_tc(&mut self, tc: &Tc, _from_link: u8) -> bool {
        if tc.sender == self.local_id { return false; }

        let last_seq = self.last_tc_seq.get(&tc.sender).copied().unwrap_or(0);
        // Duplicate detection with wraparound: if seq is within 100 steps behind last, drop.
        if tc.seq <= last_seq && last_seq.wrapping_sub(tc.seq) < 100 {
            return false;
        }
        self.last_tc_seq.insert(tc.sender, tc.seq);

        let entry = self.topology.entry(tc.sender).or_default();
        entry.clear();
        for (n, q) in &tc.advertised_neighbors {
            if *n != self.local_id {
                entry.insert(*n, *q);
            }
        }

        self.recompute_routes();
        true
    }

    /// Dijkstra shortest-path over the link-state database.
    ///
    /// Edges from local_id come from the 1-hop neighbors table (directly measured RSSI).
    /// Edges from all other nodes come from the topology table (TC-reported).
    pub fn recompute_routes(&mut self) {
        let mut dist: HashMap<u8, u32> = HashMap::new();
        let mut prev: HashMap<u8, u8>  = HashMap::new();
        let mut heap: BinaryHeap<Reverse<(u32, u8)>> = BinaryHeap::new();

        dist.insert(self.local_id, 0);
        heap.push(Reverse((0u32, self.local_id)));

        while let Some(Reverse((d, u))) = heap.pop() {
            if d > *dist.get(&u).unwrap_or(&u32::MAX) { continue; }

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
                let new_dist = d.saturating_add(edge_cost);
                if new_dist < *dist.get(&v).unwrap_or(&u32::MAX) {
                    dist.insert(v, new_dist);
                    prev.insert(v, u);
                    heap.push(Reverse((new_dist, v)));
                }
            }
        }

        let mut new_routes: HashMap<u8, RouteEntry> = HashMap::new();
        for (&dest, &cost) in &dist {
            if dest == self.local_id { continue; }

            // Walk prev chain back to local_id to find next_hop and hop count.
            let mut next = dest;
            let mut hop_count: u8 = 0;
            loop {
                match prev.get(&next) {
                    Some(&p) => {
                        hop_count = hop_count.saturating_add(1);
                        if p == self.local_id { break; }
                        next = p;
                        if hop_count >= MAX_HOPS { break; }
                    }
                    None => break,
                }
            }

            new_routes.insert(dest, RouteEntry { destination: dest, next_hop: next, cost, hop_count });
        }

        self.routes = new_routes;
    }

    pub fn route_to(&self, dst: u8) -> Option<&RouteEntry> {
        self.routes.get(&dst)
    }

    pub fn neighbors(&self) -> impl Iterator<Item = (&u8, &NeighborEntry)> {
        self.neighbors.iter()
    }

    pub fn topology_edges(&self) -> impl Iterator<Item = (u8, u8, &LinkQuality)> {
        self.topology.iter()
            .flat_map(|(src, dsts)| dsts.iter().map(move |(dst, q)| (*src, *dst, q)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lq(mcs: u8, rssi_dbm: i8, loss: u8) -> LinkQuality {
        LinkQuality { mcs, rssi_dbm, loss_rate_x100: loss }
    }

    // T1: link_cost produces correct values for known inputs.
    #[test]
    fn link_cost_mcs_base() {
        assert_eq!(link_cost(&lq(0, -30, 0)), 100); // base 100, no penalties
        assert_eq!(link_cost(&lq(1, -30, 0)), 90);
        assert_eq!(link_cost(&lq(2, -30, 0)), 80);
        assert_eq!(link_cost(&lq(3, -30, 0)), 70);
        assert_eq!(link_cost(&lq(4, -30, 0)), 60);
        assert_eq!(link_cost(&lq(7, -30, 0)), 50); // fallthrough
    }

    #[test]
    fn link_cost_rssi_penalty() {
        // rssi -50: penalty = (50-30)*2 = 40
        assert_eq!(link_cost(&lq(0, -50, 0)), 100 + 40);
        // rssi -20: clamped to 0
        assert_eq!(link_cost(&lq(0, -20, 0)), 100);
    }

    #[test]
    fn link_cost_loss_penalty() {
        // loss 10% × 10 = 100
        assert_eq!(link_cost(&lq(0, -30, 10)), 100 + 100);
    }

    #[test]
    fn link_cost_combined() {
        // mcs=4 base=60, rssi=-50 penalty=40, loss=5 penalty=50 → 150
        assert_eq!(link_cost(&lq(4, -50, 5)), 60 + 40 + 50);
    }

    // T5: Dijkstra over synthetic 3-node topology.
    //
    // Topology:  local(1) --[cost 100]--> 2 --[cost 100]--> 3
    // All links: mcs=0, rssi=-30, loss=0 → cost=100 each.
    #[test]
    fn dijkstra_three_node_chain() {
        let mut state = OlsrState::new(1);

        // 1-hop neighbor: node 2
        state.neighbors.insert(2, NeighborEntry {
            node_id: 2,
            link_quality: lq(0, -30, 0),
            last_hello_ms: 0,
            last_seen_neighbors: vec![1, 3],
        });
        // Topology: node 2 knows node 3
        let mut n2_topo = HashMap::new();
        n2_topo.insert(3, lq(0, -30, 0));
        state.topology.insert(2, n2_topo);

        state.recompute_routes();

        let r2 = state.routes.get(&2).expect("route to 2");
        assert_eq!(r2.next_hop, 2);
        assert_eq!(r2.cost, 100);
        assert_eq!(r2.hop_count, 1);

        let r3 = state.routes.get(&3).expect("route to 3");
        assert_eq!(r3.next_hop, 2);
        assert_eq!(r3.cost, 200);
        assert_eq!(r3.hop_count, 2);
    }

    #[test]
    fn dijkstra_prefers_lower_cost_path() {
        let mut state = OlsrState::new(1);

        // Two neighbors: 2 (good) and 3 (weak — higher cost)
        state.neighbors.insert(2, NeighborEntry {
            node_id: 2,
            link_quality: lq(4, -30, 0), // cost 60
            last_hello_ms: 0,
            last_seen_neighbors: vec![1, 4],
        });
        state.neighbors.insert(3, NeighborEntry {
            node_id: 3,
            link_quality: lq(0, -80, 0), // cost 100 + (80-30)*2 = 200
            last_hello_ms: 0,
            last_seen_neighbors: vec![1, 4],
        });

        // Both 2 and 3 can reach node 4, but via different costs.
        let mut n2 = HashMap::new(); n2.insert(4, lq(4, -30, 0)); // cost 60 → total 120
        let mut n3 = HashMap::new(); n3.insert(4, lq(4, -30, 0)); // cost 60 → total 260
        state.topology.insert(2, n2);
        state.topology.insert(3, n3);

        state.recompute_routes();

        let r4 = state.routes.get(&4).expect("route to 4");
        // Path via 2: 60 + 60 = 120. Path via 3: 200 + 60 = 260. Should pick via 2.
        assert_eq!(r4.next_hop, 2);
        assert_eq!(r4.cost, 120);
        assert_eq!(r4.hop_count, 2);
    }

    #[test]
    fn tc_dedup_blocks_old_seq() {
        let mut state = OlsrState::new(1);
        let tc = |seq: u16| Tc {
            sender: 2,
            seq,
            advertised_neighbors: vec![],
            sent_at_ms: 0,
        };

        assert!(state.process_tc(&tc(5), 0));   // new
        assert!(!state.process_tc(&tc(5), 0));  // duplicate
        assert!(!state.process_tc(&tc(3), 0));  // old
        assert!(state.process_tc(&tc(6), 0));   // new
    }

    #[test]
    fn tc_dedup_handles_wraparound() {
        let mut state = OlsrState::new(1);
        let tc = |seq: u16| Tc {
            sender: 2,
            seq,
            advertised_neighbors: vec![],
            sent_at_ms: 0,
        };

        assert!(state.process_tc(&tc(65500), 0));
        assert!(state.process_tc(&tc(3), 0)); // wraparound: gap > 100 → forward
    }

    #[test]
    fn process_hello_updates_neighbors() {
        let mut state = OlsrState::new(1);
        let hello = Hello {
            sender: 2,
            neighbors: vec![(1, lq(0, -40, 0))],
            sent_at_ms: 1_000,
        };
        state.process_hello(&hello, 0, -45);

        let entry = state.neighbors.get(&2).expect("neighbor 2");
        assert_eq!(entry.link_quality.rssi_dbm, -45); // our radiotap measurement
        assert_eq!(entry.link_quality.mcs, 0);        // sender's reported mcs for us
    }

    #[test]
    fn aging_removes_neighbor_and_clears_routes() {
        let mut state = OlsrState::new(1);
        state.neighbors.insert(2, NeighborEntry {
            node_id: 2,
            link_quality: lq(0, -30, 0),
            last_hello_ms: 0, // already expired (now_ms() - 0 >> NEIGHBOR_TIMEOUT_MS)
            last_seen_neighbors: vec![],
        });
        let mut n2 = HashMap::new();
        n2.insert(3, lq(0, -30, 0));
        state.topology.insert(2, n2);
        state.recompute_routes();
        assert!(!state.routes.is_empty());

        // Manually perform what aging_loop does.
        let now = now_ms();
        let stale: Vec<u8> = state.neighbors.iter()
            .filter(|(_, e)| now.saturating_sub(e.last_hello_ms) > crate::types::NEIGHBOR_TIMEOUT_MS)
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            state.neighbors.remove(&id);
            state.topology.remove(&id);
            state.two_hop.retain(|_, gateways| { gateways.remove(&id); !gateways.is_empty() });
        }
        state.routes.clear();

        assert!(state.neighbors.is_empty());
        assert!(state.routes.is_empty());
    }
}
