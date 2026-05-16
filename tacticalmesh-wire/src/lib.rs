pub mod constants;
pub mod crypto;
pub mod errors;
pub mod fec;
pub mod frame;
pub mod headers;
pub mod identity;
pub mod message;
pub mod msg_kind;
pub mod nonce_cache;
pub mod priority;
pub mod pubkey_store;
pub mod scheduler;

// Top-level re-exports for the interface contract.
pub use constants::{BROADCAST, MAX_HOPS};
pub use errors::FrameError;
pub use fec::{FEC_THRESHOLD_BYTES, fec_decode, fec_encode};
pub use frame::{ParsedFrame, build_frame, build_frame_for_route, build_frames, parse_and_verify_frame};
pub use headers::{AuthHeader, RoutedHeader};
pub use identity::Identity;
pub use message::TacticalMessage;
pub use msg_kind::MsgKind;
pub use nonce_cache::{NonceCache, NONCE_CACHE_TTL_MS, TIME_WINDOW_MS};
pub use priority::Priority;
pub use pubkey_store::PubkeyStore;
pub use scheduler::TxScheduler;
pub use tacticalmesh_link::{LinkAdapter, RxMeta, EPOCH_ROTATION_SECS, MAX_FRAME_BYTES, OVERLAP_MS};
