use std::time::{SystemTime, UNIX_EPOCH};

use crate::crypto::{
    build_nonce, nonce24_from_stored, payload_hash12, sign_auth, verify_auth, SIG_LEN,
};
use crate::errors::FrameError;
use crate::fec::{fec_encode, fec_params, FEC_THRESHOLD_BYTES};
use crate::headers::{AuthHeader, RoutedHeader, AUTH_HEADER_LEN, ROUTED_HEADER_LEN};
use crate::identity::Identity;
use crate::message::TacticalMessage;
use crate::nonce_cache::NonceCache;
use crate::priority::Priority;
use crate::pubkey_store::PubkeyStore;

/// Minimum frame size: routed + auth + signature.
pub const MIN_FRAME_LEN: usize = ROUTED_HEADER_LEN + AUTH_HEADER_LEN + SIG_LEN;

#[derive(Debug)]
pub struct ParsedFrame {
    pub routed: RoutedHeader,
    pub auth: AuthHeader,
    pub plaintext: Vec<u8>,
    /// RSSI placeholder — populated by the radio layer, zero here.
    pub rssi_dbm: i8,
}

/// Builds a single wire frame (direct send, no next-hop override).
pub fn build_frame(
    msg: &TacticalMessage,
    prio: Priority,
    dst: u8,
    identity: &Identity,
) -> Vec<u8> {
    build_frame_inner(msg, prio, dst, identity, identity.node_id, 0)
}

/// Builds a frame with a specific next_hop written as last_hop_id in the routed header.
pub fn build_frame_for_route(
    msg: &TacticalMessage,
    prio: Priority,
    dst: u8,
    identity: &Identity,
    next_hop: u8,
) -> Vec<u8> {
    build_frame_inner(msg, prio, dst, identity, next_hop, 0)
}

/// Builds all FEC shards for messages > `FEC_THRESHOLD_BYTES`; single frame otherwise.
/// Each shard uses its own nonce so keystream is never reused across shards.
pub fn build_frames(
    msg: &TacticalMessage,
    prio: Priority,
    dst: u8,
    identity: &Identity,
) -> Vec<Vec<u8>> {
    if msg.payload.len() <= FEC_THRESHOLD_BYTES {
        return vec![build_frame(msg, prio, dst, identity)];
    }

    let (k, n) = fec_params(prio);
    let shards = fec_encode(&msg.payload, prio)
        .expect("FEC encode should not fail on a valid payload");

    let block_id: u8 = rand::random();
    let base_ts = unix_now_ms();

    shards
        .iter()
        .enumerate()
        .map(|(idx, shard)| {
            // Unique nonce per shard prevents keystream reuse.
            let (nonce24, stored_nonce) = build_nonce(identity.node_id, base_ts);
            let shard_hash = payload_hash12(shard);

            let auth = AuthHeader {
                src_node:    identity.node_id,
                dst_node:    dst,
                msg_kind:    msg.kind,
                priority:    prio,
                timestamp_ms: base_ts,
                nonce:       stored_nonce,
                payload_hash: shard_hash,
                epoch:       0,
                payload_len: shard.len() as u16,
                fec_index:   idx as u8,
                fec_k:       k as u8,
                fec_n:       n as u8,
                fec_block_id: block_id,
            };
            let auth_bytes = auth.to_bytes();
            let sig = sign_auth(&auth_bytes, identity);

            let mut ciphertext = shard.clone();
            crate::crypto::xchacha20_apply(&identity.session_key, &nonce24, &mut ciphertext);

            let routed = RoutedHeader {
                last_hop_id: identity.node_id,
                hops_taken: 0,
                flags: 0,
                reserved: 0,
            };
            assemble_frame(&routed, &auth_bytes, &sig, &ciphertext)
        })
        .collect()
}

fn build_frame_inner(
    msg: &TacticalMessage,
    prio: Priority,
    dst: u8,
    identity: &Identity,
    last_hop: u8,
    hops_taken: u8,
) -> Vec<u8> {
    let now_ms = unix_now_ms();
    let (nonce24, stored_nonce) = build_nonce(identity.node_id, now_ms);

    let phash = payload_hash12(&msg.payload);

    let (fec_k, fec_n) = if msg.payload.len() > FEC_THRESHOLD_BYTES {
        let (k, n) = fec_params(prio);
        (k as u8, n as u8)
    } else {
        (1, 1)
    };

    let auth = AuthHeader {
        src_node:    identity.node_id,
        dst_node:    dst,
        msg_kind:    msg.kind,
        priority:    prio,
        timestamp_ms: now_ms,
        nonce:       stored_nonce,
        payload_hash: phash,
        epoch:       0,
        payload_len: msg.payload.len() as u16,
        fec_index:   0,
        fec_k,
        fec_n,
        fec_block_id: 0,
    };

    let auth_bytes = auth.to_bytes();
    let sig = sign_auth(&auth_bytes, identity);

    let mut ciphertext = msg.payload.clone();
    crate::crypto::xchacha20_apply(&identity.session_key, &nonce24, &mut ciphertext);

    let routed = RoutedHeader {
        last_hop_id: last_hop,
        hops_taken,
        flags: 0,
        reserved: 0,
    };

    assemble_frame(&routed, &auth_bytes, &sig, &ciphertext)
}

fn assemble_frame(
    routed: &RoutedHeader,
    auth_bytes: &[u8; AUTH_HEADER_LEN],
    sig: &[u8; SIG_LEN],
    ciphertext: &[u8],
) -> Vec<u8> {
    let mut frame = Vec::with_capacity(MIN_FRAME_LEN + ciphertext.len());
    frame.extend_from_slice(&routed.to_bytes());
    frame.extend_from_slice(auth_bytes);
    frame.extend_from_slice(sig);
    frame.extend_from_slice(ciphertext);
    frame
}

/// Parses, signature-verifies, replay-checks, and decrypts a raw frame.
pub fn parse_and_verify_frame(
    raw: &[u8],
    known_pubkeys: &PubkeyStore,
    nonce_cache: &NonceCache,
    local_time_ms: u64,
    session_key: &[u8; 32],
) -> Result<ParsedFrame, FrameError> {
    if raw.len() < MIN_FRAME_LEN {
        return Err(FrameError::TruncatedFrame);
    }

    let routed = RoutedHeader::from_bytes(&raw[..ROUTED_HEADER_LEN])?;

    let auth_start = ROUTED_HEADER_LEN;
    let auth_end   = auth_start + AUTH_HEADER_LEN;
    let auth = AuthHeader::from_bytes(&raw[auth_start..auth_end])?;

    // 1. Sig check before any further processing.
    let sig_end = auth_end + SIG_LEN;
    let sig_bytes: &[u8; SIG_LEN] = raw[auth_end..sig_end].try_into().unwrap();
    let auth_bytes: &[u8; AUTH_HEADER_LEN] = raw[auth_start..auth_end].try_into().unwrap();

    let verifying_key = known_pubkeys
        .get(auth.src_node)
        .ok_or(FrameError::BadSignature)?;
    verify_auth(auth_bytes, sig_bytes, verifying_key)?;

    // 2. Replay / time-window check.
    nonce_cache.check_and_insert(auth.src_node, auth.timestamp_ms, &auth.nonce, local_time_ms)?;

    // 3. Decrypt.
    let nonce24 = nonce24_from_stored(&auth.nonce);
    let mut plaintext = raw[sig_end..].to_vec();
    crate::crypto::xchacha20_apply(session_key, &nonce24, &mut plaintext);

    // 4. Verify payload hash.
    let computed_hash = payload_hash12(&plaintext);
    if computed_hash != auth.payload_hash {
        return Err(FrameError::DecryptFailed);
    }

    Ok(ParsedFrame { routed, auth, plaintext, rssi_dbm: 0 })
}

pub fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{constants::BROADCAST, identity::Identity, msg_kind::MsgKind, pubkey_store::PubkeyStore};

    fn make_env() -> (Identity, PubkeyStore, NonceCache) {
        let id = Identity::generate(1);
        let mut store = PubkeyStore::new();
        store.insert(1, id.verifying_key);
        let cache = NonceCache::new();
        (id, store, cache)
    }

    #[test]
    fn build_and_parse_round_trip() {
        let (id, store, cache) = make_env();
        let msg = TacticalMessage { kind: MsgKind::Data, payload: b"hello mesh".to_vec() };
        let frame = build_frame(&msg, Priority::High, BROADCAST, &id);

        let parsed = parse_and_verify_frame(&frame, &store, &cache, unix_now_ms(), &id.session_key)
            .unwrap();
        assert_eq!(parsed.plaintext, msg.payload);
        assert_eq!(parsed.auth.src_node, 1);
        assert_eq!(parsed.auth.dst_node, BROADCAST);
    }

    #[test]
    fn tampered_frame_bad_signature() {
        let (id, store, cache) = make_env();
        let msg = TacticalMessage { kind: MsgKind::Data, payload: b"data".to_vec() };
        let mut frame = build_frame(&msg, Priority::High, BROADCAST, &id);

        // Flip a byte inside the auth header.
        frame[ROUTED_HEADER_LEN + 5] ^= 0xFF;

        assert_eq!(
            parse_and_verify_frame(&frame, &store, &cache, unix_now_ms(), &id.session_key)
                .unwrap_err(),
            FrameError::BadSignature
        );
    }

    #[test]
    fn replayed_frame_rejected() {
        let (id, store, cache) = make_env();
        let msg = TacticalMessage { kind: MsgKind::Data, payload: b"data".to_vec() };
        let frame = build_frame(&msg, Priority::High, BROADCAST, &id);

        let t = unix_now_ms();
        parse_and_verify_frame(&frame, &store, &cache, t, &id.session_key).unwrap();
        assert_eq!(
            parse_and_verify_frame(&frame, &store, &cache, t, &id.session_key)
                .unwrap_err(),
            FrameError::ReplayedNonce
        );
    }

    #[test]
    fn route_frame_sets_last_hop() {
        let (id, store, cache) = make_env();
        let msg = TacticalMessage { kind: MsgKind::OlsrHello, payload: vec![1, 2, 3] };
        let frame = build_frame_for_route(&msg, Priority::Critical, 7, &id, 3);
        let parsed = parse_and_verify_frame(&frame, &store, &cache, unix_now_ms(), &id.session_key)
            .unwrap();
        assert_eq!(parsed.routed.last_hop_id, 3);
        assert_eq!(parsed.auth.dst_node, 7);
    }

    #[test]
    fn truncated_frame_error() {
        let (_, store, cache) = make_env();
        let short = vec![0u8; MIN_FRAME_LEN - 1];
        assert_eq!(
            parse_and_verify_frame(&short, &store, &cache, unix_now_ms(), &[0u8; 32])
                .unwrap_err(),
            FrameError::TruncatedFrame
        );
    }
}
