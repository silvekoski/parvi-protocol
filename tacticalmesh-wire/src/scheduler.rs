use std::collections::VecDeque;
use std::sync::Arc;
use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::priority::Priority;
pub use tacticalmesh_link::LinkAdapter;

struct Queues {
    inner: [VecDeque<Vec<u8>>; 4],
}

impl Queues {
    fn new() -> Self {
        Self {
            inner: [
                VecDeque::new(),
                VecDeque::new(),
                VecDeque::new(),
                VecDeque::new(),
            ],
        }
    }

    fn push(&mut self, frame: Vec<u8>, prio: Priority) {
        self.inner[prio as usize].push_back(frame);
    }

    /// Pops from the highest-priority non-empty queue (P0 → P3).
    fn pop_next(&mut self) -> Option<(Vec<u8>, Priority)> {
        for (i, q) in self.inner.iter_mut().enumerate() {
            if let Some(frame) = q.pop_front() {
                let prio = Priority::from_u8(i as u8).expect("index in 0..4");
                return Some((frame, prio));
            }
        }
        None
    }
}

/// Four-queue strict-priority TX scheduler backed by the real `LinkAdapter`.
pub struct TxScheduler {
    queues: Arc<Mutex<Queues>>,
    notify: Arc<Notify>,
    link: Arc<LinkAdapter>,
}

impl TxScheduler {
    pub fn new(link: Arc<LinkAdapter>) -> Self {
        Self {
            queues: Arc::new(Mutex::new(Queues::new())),
            notify: Arc::new(Notify::new()),
            link,
        }
    }

    pub fn enqueue(&self, frame: Vec<u8>, prio: Priority) {
        self.queues.lock().push(frame, prio);
        self.notify.notify_one();
    }

    /// Strict-priority drain loop.  Runs until the task is cancelled.
    pub async fn run(&self) {
        loop {
            loop {
                let item = self.queues.lock().pop_next();
                match item {
                    Some((frame, prio)) => {
                        if let Err(e) = self.link.send(&frame, prio) {
                            tracing::warn!("link send failed ({prio:?}): {e}");
                        }
                    }
                    None => break,
                }
            }
            self.notify.notified().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_priority_p0_before_p3() {
        let link = Arc::new(
            LinkAdapter::new("wlan1", 1, [0u8; 32]).expect("stub link"),
        );
        let sched = TxScheduler::new(link);

        sched.enqueue(vec![3], Priority::Bulk);
        sched.enqueue(vec![0], Priority::Emergency);
        sched.enqueue(vec![2], Priority::High);
        sched.enqueue(vec![1], Priority::Critical);

        let order: Vec<Vec<u8>> = {
            let mut q = sched.queues.lock();
            let mut v = Vec::new();
            while let Some((f, _)) = q.pop_next() {
                v.push(f);
            }
            v
        };

        assert_eq!(order, vec![vec![0], vec![1], vec![2], vec![3]]);
    }
}
