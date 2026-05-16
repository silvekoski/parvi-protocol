use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("bad Ed25519 signature")]
    BadSignature,
    #[error("replayed nonce")]
    ReplayedNonce,
    #[error("timestamp outside ±30s window")]
    TimeWindowExpired,
    #[error("XChaCha20 decryption failed")]
    DecryptFailed,
    #[error("frame too short")]
    TruncatedFrame,
    #[error("FEC decode failed: {0}")]
    FecError(String),
}
