use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::rngs::OsRng;

pub struct Identity {
    pub node_id: u8,
    pub signing_key: SigningKey,
    pub verifying_key: VerifyingKey,
    /// PSK for XChaCha20 encryption (32 bytes).
    pub session_key: [u8; 32],
}

impl Identity {
    pub fn generate(node_id: u8) -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let session_key = rand::random();
        Identity { node_id, signing_key, verifying_key, session_key }
    }

    pub fn from_parts(
        node_id: u8,
        signing_key: SigningKey,
        session_key: [u8; 32],
    ) -> Self {
        let verifying_key = signing_key.verifying_key();
        Identity { node_id, signing_key, verifying_key, session_key }
    }

    /// Deterministic identity derived from `node_id` + shared PSK.
    ///
    /// Each node gets a unique signing key (so signatures are attributable) while all
    /// nodes share the same `session_key` (so any node can decrypt any frame).
    pub fn from_seed(node_id: u8, psk: &[u8; 32]) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(b"tacticalmesh-signing-key-v1");
        h.update(&[node_id]);
        h.update(psk);
        let seed: [u8; 32] = *h.finalize().as_bytes();
        let signing_key = SigningKey::from_bytes(&seed);
        Self::from_parts(node_id, signing_key, *psk)
    }
}
