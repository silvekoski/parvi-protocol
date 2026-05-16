use chacha20::cipher::{KeyIvInit, StreamCipher};
use chacha20::{XChaCha20, XNonce};
use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey};
use rand::Rng;

use crate::errors::FrameError;
use crate::identity::Identity;

pub const SIG_LEN: usize = 64;

/// Builds the 24-byte XChaCha20 nonce and returns both the encryption nonce
/// and the 12-byte slice stored in the auth header.
pub fn build_nonce(node_id: u8, timestamp_ms: u64) -> ([u8; 24], [u8; 12]) {
    let mut nonce24 = [0u8; 24];
    nonce24[0] = node_id;
    let ts_bytes = timestamp_ms.to_le_bytes();
    nonce24[1..7].copy_from_slice(&ts_bytes[..6]);
    let mut rng = rand::thread_rng();
    let random: [u8; 5] = rng.gen();
    nonce24[7..12].copy_from_slice(&random);
    // bytes 12..24 remain zero (padding)
    let stored: [u8; 12] = nonce24[..12].try_into().unwrap();
    (nonce24, stored)
}

/// Reconstructs the 24-byte nonce from the stored 12-byte header field.
pub fn nonce24_from_stored(stored: &[u8; 12]) -> [u8; 24] {
    let mut nonce24 = [0u8; 24];
    nonce24[..12].copy_from_slice(stored);
    nonce24
}

/// XChaCha20 in-place (encrypt or decrypt — same operation).
pub fn xchacha20_apply(key: &[u8; 32], nonce24: &[u8; 24], data: &mut Vec<u8>) {
    let key_ref = chacha20::Key::from_slice(key);
    let nonce_ref = XNonce::from_slice(nonce24);
    let mut cipher = XChaCha20::new(key_ref, nonce_ref);
    cipher.apply_keystream(data);
}

/// Returns the first 12 bytes of the blake3 hash of `data`.
pub fn payload_hash12(data: &[u8]) -> [u8; 12] {
    let full = blake3::hash(data);
    let mut out = [0u8; 12];
    out.copy_from_slice(&full.as_bytes()[..12]);
    out
}

/// Signs `auth_header_bytes` with the identity's signing key.
pub fn sign_auth(auth_bytes: &[u8], identity: &Identity) -> [u8; SIG_LEN] {
    let sig: Signature = identity.signing_key.sign(auth_bytes);
    sig.to_bytes()
}

/// Verifies `auth_header_bytes` against the provided signature and verifying key.
pub fn verify_auth(
    auth_bytes: &[u8],
    sig_bytes: &[u8; SIG_LEN],
    verifying_key: &VerifyingKey,
) -> Result<(), FrameError> {
    let sig = Signature::from_bytes(sig_bytes);
    verifying_key
        .verify(auth_bytes, &sig)
        .map_err(|_| FrameError::BadSignature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    #[test]
    fn xchacha20_round_trip() {
        let key = [0xABu8; 32];
        let nonce24 = [0x12u8; 24];
        let original = b"hello tactical mesh".to_vec();
        let mut data = original.clone();
        xchacha20_apply(&key, &nonce24, &mut data);
        assert_ne!(data, original);
        xchacha20_apply(&key, &nonce24, &mut data);
        assert_eq!(data, original);
    }

    #[test]
    fn sign_verify_ok() {
        let id = Identity::generate(1);
        let msg = b"auth header bytes";
        let sig = sign_auth(msg, &id);
        verify_auth(msg, &sig, &id.verifying_key).unwrap();
    }

    #[test]
    fn tampered_byte_bad_signature() {
        let id = Identity::generate(1);
        let msg = b"auth header bytes";
        let sig = sign_auth(msg, &id);
        let tampered = b"auth header XXXXX";
        assert_eq!(
            verify_auth(tampered, &sig, &id.verifying_key),
            Err(FrameError::BadSignature)
        );
    }

    #[test]
    fn nonce_build_and_restore() {
        let (nonce24, stored) = build_nonce(42, 9_876_543_210);
        let restored = nonce24_from_stored(&stored);
        assert_eq!(nonce24[..12], restored[..12]);
        assert_eq!(&restored[12..], &[0u8; 12]);
    }
}
