use crate::errors::FrameError;
use crate::msg_kind::MsgKind;
use crate::priority::Priority;

pub const AUTH_HEADER_LEN: usize = 44;
pub const ROUTED_HEADER_LEN: usize = 4;

/// 44-byte on-wire authentication header (§7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthHeader {
    pub src_node: u8,
    pub dst_node: u8,
    pub msg_kind: MsgKind,
    pub priority: Priority,
    pub timestamp_ms: u64,
    pub nonce: [u8; 12],
    pub payload_hash: [u8; 12],
    pub epoch: u16,
    pub payload_len: u16,
    pub fec_index: u8,
    pub fec_k: u8,
    pub fec_n: u8,
    pub fec_block_id: u8,
}

impl AuthHeader {
    pub fn to_bytes(&self) -> [u8; AUTH_HEADER_LEN] {
        let mut buf = [0u8; AUTH_HEADER_LEN];
        let mut i = 0;
        buf[i] = self.src_node;       i += 1;
        buf[i] = self.dst_node;       i += 1;
        buf[i] = self.msg_kind as u8; i += 1;
        buf[i] = self.priority as u8; i += 1;
        buf[i..i+8].copy_from_slice(&self.timestamp_ms.to_le_bytes()); i += 8;
        buf[i..i+12].copy_from_slice(&self.nonce);                     i += 12;
        buf[i..i+12].copy_from_slice(&self.payload_hash);              i += 12;
        buf[i..i+2].copy_from_slice(&self.epoch.to_le_bytes());        i += 2;
        buf[i..i+2].copy_from_slice(&self.payload_len.to_le_bytes());  i += 2;
        buf[i] = self.fec_index;   i += 1;
        buf[i] = self.fec_k;       i += 1;
        buf[i] = self.fec_n;       i += 1;
        buf[i] = self.fec_block_id;
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self, FrameError> {
        if buf.len() < AUTH_HEADER_LEN {
            return Err(FrameError::TruncatedFrame);
        }
        let mut i = 0;
        let src_node = buf[i]; i += 1;
        let dst_node = buf[i]; i += 1;
        let msg_kind = MsgKind::from_u8(buf[i]).ok_or(FrameError::TruncatedFrame)?; i += 1;
        let priority  = Priority::from_u8(buf[i]).ok_or(FrameError::TruncatedFrame)?; i += 1;
        let timestamp_ms = u64::from_le_bytes(buf[i..i+8].try_into().unwrap()); i += 8;
        let nonce: [u8; 12] = buf[i..i+12].try_into().unwrap(); i += 12;
        let payload_hash: [u8; 12] = buf[i..i+12].try_into().unwrap(); i += 12;
        let epoch = u16::from_le_bytes(buf[i..i+2].try_into().unwrap()); i += 2;
        let payload_len = u16::from_le_bytes(buf[i..i+2].try_into().unwrap()); i += 2;
        let fec_index    = buf[i]; i += 1;
        let fec_k        = buf[i]; i += 1;
        let fec_n        = buf[i]; i += 1;
        let fec_block_id = buf[i];
        Ok(AuthHeader {
            src_node, dst_node, msg_kind, priority, timestamp_ms,
            nonce, payload_hash, epoch, payload_len,
            fec_index, fec_k, fec_n, fec_block_id,
        })
    }
}

/// 4-byte hop-routing header prepended to every frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutedHeader {
    pub last_hop_id: u8,
    pub hops_taken: u8,
    pub flags: u8,
    pub reserved: u8,
}

impl RoutedHeader {
    pub fn to_bytes(&self) -> [u8; ROUTED_HEADER_LEN] {
        [self.last_hop_id, self.hops_taken, self.flags, self.reserved]
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self, FrameError> {
        if buf.len() < ROUTED_HEADER_LEN {
            return Err(FrameError::TruncatedFrame);
        }
        Ok(RoutedHeader {
            last_hop_id: buf[0],
            hops_taken:  buf[1],
            flags:       buf[2],
            reserved:    buf[3],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{msg_kind::MsgKind, priority::Priority};

    fn sample_auth() -> AuthHeader {
        AuthHeader {
            src_node: 1, dst_node: 2,
            msg_kind: MsgKind::Data, priority: Priority::Emergency,
            timestamp_ms: 1_000_000,
            nonce: [0xAB; 12], payload_hash: [0xCD; 12],
            epoch: 7, payload_len: 128,
            fec_index: 0, fec_k: 1, fec_n: 4, fec_block_id: 0,
        }
    }

    #[test]
    fn auth_header_round_trip() {
        let h = sample_auth();
        let bytes = h.to_bytes();
        assert_eq!(bytes.len(), AUTH_HEADER_LEN);
        let h2 = AuthHeader::from_bytes(&bytes).unwrap();
        assert_eq!(h, h2);
    }

    #[test]
    fn routed_header_round_trip() {
        let r = RoutedHeader { last_hop_id: 5, hops_taken: 2, flags: 0x01, reserved: 0 };
        let bytes = r.to_bytes();
        assert_eq!(bytes.len(), ROUTED_HEADER_LEN);
        let r2 = RoutedHeader::from_bytes(&bytes).unwrap();
        assert_eq!(r, r2);
    }
}
