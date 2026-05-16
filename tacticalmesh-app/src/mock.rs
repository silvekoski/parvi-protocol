use tokio::sync::mpsc;

use crate::Priority;

pub struct MockLink {
    pub tx: mpsc::Sender<(Vec<u8>, Priority)>,
    pub rx: mpsc::Receiver<(Vec<u8>, Priority)>,
}

impl MockLink {
    /// Returns (side_a, side_b) where a.tx → b.rx and b.tx → a.rx.
    pub fn new() -> (Self, Self) {
        let (tx_a, rx_b) = mpsc::channel(256);
        let (tx_b, rx_a) = mpsc::channel(256);

        let side_a = MockLink { tx: tx_a, rx: rx_a };
        let side_b = MockLink { tx: tx_b, rx: rx_b };

        (side_a, side_b)
    }

    /// tx loops back to rx.
    pub fn new_loopback() -> Self {
        let (tx, rx) = mpsc::channel(256);
        MockLink { tx, rx }
    }
}
