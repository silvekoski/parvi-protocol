mod forward;
mod loops;
mod state;
mod types;

pub use forward::{route_and_send, should_forward_tc};
// route_and_send is sync (link.send is sync). hello_loop / tc_loop are async (use tokio timer).
pub use loops::{aging_loop, hello_loop, tc_loop};
pub use state::{link_cost, OlsrState};
pub use types::{
    Hello, LinkQuality, NeighborEntry, OlsrMessage, RouteEntry, Tc,
    AGING_TICK_MS, HELLO_INTERVAL_MS, MAX_HOPS, NEIGHBOR_TIMEOUT_MS, TC_INTERVAL_MS,
};

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
