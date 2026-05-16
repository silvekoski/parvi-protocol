use serde::{Deserialize, Serialize};

#[repr(u8)]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MsgKind {
    Data = 0,
    OlsrHello = 1,
    OlsrTc = 2,
    Ack = 3,
    SessionKeyRotation = 4,
}

impl MsgKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(MsgKind::Data),
            1 => Some(MsgKind::OlsrHello),
            2 => Some(MsgKind::OlsrTc),
            3 => Some(MsgKind::Ack),
            4 => Some(MsgKind::SessionKeyRotation),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_bincode() {
        for k in [
            MsgKind::Data,
            MsgKind::OlsrHello,
            MsgKind::OlsrTc,
            MsgKind::Ack,
            MsgKind::SessionKeyRotation,
        ] {
            let enc = bincode::serialize(&k).unwrap();
            let dec: MsgKind = bincode::deserialize(&enc).unwrap();
            assert_eq!(k, dec);
        }
    }

    #[test]
    fn from_u8_round_trip() {
        for v in 0u8..5 {
            let k = MsgKind::from_u8(v).unwrap();
            assert_eq!(k as u8, v);
        }
        assert!(MsgKind::from_u8(5).is_none());
    }
}
