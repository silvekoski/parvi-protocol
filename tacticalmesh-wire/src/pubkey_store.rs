use std::collections::HashMap;
use ed25519_dalek::VerifyingKey;

/// Mission certificate registry: node_id → verifying key.
pub struct PubkeyStore {
    keys: HashMap<u8, VerifyingKey>,
}

impl PubkeyStore {
    pub fn new() -> Self {
        Self { keys: HashMap::new() }
    }

    pub fn insert(&mut self, node_id: u8, key: VerifyingKey) {
        self.keys.insert(node_id, key);
    }

    pub fn get(&self, node_id: u8) -> Option<&VerifyingKey> {
        self.keys.get(&node_id)
    }
}

impl Default for PubkeyStore {
    fn default() -> Self { Self::new() }
}
