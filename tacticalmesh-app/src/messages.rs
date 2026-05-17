use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum TargetKind {
    Vehicle,
    Personnel,
    Emplacement,
    Unknown,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum CommandOp {
    Engage,
    Abort,
    Reassign,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum BdaResult {
    Destroyed,
    Damaged,
    Miss,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TargetDetection {
    pub target_id: u16,
    pub kind: TargetKind,
    pub lat: f32,
    pub lon: f32,
    pub detected_at_ms: u64,
    pub detector: u8,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Command {
    pub target_id: u16,
    pub op: CommandOp,
    pub issued_by: u8,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Bda {
    pub target_id: u16,
    pub result: BdaResult,
    pub at_ms: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StateReport {
    pub node_id: u8,
    pub battery_pct: u8,
    pub lat: f32,
    pub lon: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ImageShard {
    pub target_id: u16,
    /// How many blocks make up the full image. 0 or 1 = single-block (legacy).
    pub total_blocks: u8,
    pub block_id: u8,
    pub index: u8,
    pub k: u8,
    pub n: u8,
    pub data: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RequestImage {
    pub target_id: u16,
    pub requester: u8,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JamAlert {
    pub detected_by: u8,
    pub channel: u8,
    pub at_ms: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChannelHop {
    pub new_channel: u8,
    pub new_epoch: u32,
    pub initiated_by: u8,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Mayday {
    pub node_id: u8,
    pub at_ms: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AckPayload {
    pub acked_seq: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChatMessage {
    pub from: u8,
    pub text: String,
}

/// Placeholder until tacticalmesh-olsr is wired in
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OlsrMessage {
    pub payload: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum TacticalMessage {
    TargetDetection(TargetDetection),
    Command(Command),
    Bda(Bda),
    StateReport(StateReport),
    ImageShard(ImageShard),
    RequestImage(RequestImage),
    JamAlert(JamAlert),
    ChannelHop(ChannelHop),
    Mayday(Mayday),
    Olsr(OlsrMessage),
    Ack(AckPayload),
    Chat(ChatMessage),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(msg: &TacticalMessage) -> TacticalMessage {
        let encoded = bincode::serialize(msg).expect("serialize failed");
        bincode::deserialize(&encoded).expect("deserialize failed")
    }

    #[test]
    fn roundtrip_target_detection() {
        let msg = TacticalMessage::TargetDetection(TargetDetection {
            target_id: 42,
            kind: TargetKind::Vehicle,
            lat: 48.123,
            lon: 11.456,
            detected_at_ms: 1_000_000,
            detector: 3,
        });
        let rt = roundtrip(&msg);
        match rt {
            TacticalMessage::TargetDetection(d) => {
                assert_eq!(d.target_id, 42);
                assert_eq!(d.detector, 3);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_command() {
        let msg = TacticalMessage::Command(Command {
            target_id: 7,
            op: CommandOp::Engage,
            issued_by: 1,
        });
        let rt = roundtrip(&msg);
        match rt {
            TacticalMessage::Command(c) => {
                assert_eq!(c.target_id, 7);
                assert_eq!(c.issued_by, 1);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_bda() {
        let msg = TacticalMessage::Bda(Bda {
            target_id: 99,
            result: BdaResult::Destroyed,
            at_ms: 5_000,
        });
        let rt = roundtrip(&msg);
        match rt {
            TacticalMessage::Bda(b) => {
                assert_eq!(b.target_id, 99);
                assert_eq!(b.at_ms, 5_000);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_state_report() {
        let msg = TacticalMessage::StateReport(StateReport {
            node_id: 2,
            battery_pct: 75,
            lat: 52.0,
            lon: 13.0,
        });
        let rt = roundtrip(&msg);
        match rt {
            TacticalMessage::StateReport(s) => {
                assert_eq!(s.node_id, 2);
                assert_eq!(s.battery_pct, 75);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_image_shard() {
        let msg = TacticalMessage::ImageShard(ImageShard {
            target_id: 10,
            total_blocks: 0,
            block_id: 0,
            index: 3,
            k: 8,
            n: 12,
            data: vec![0xAA, 0xBB, 0xCC],
        });
        let rt = roundtrip(&msg);
        match rt {
            TacticalMessage::ImageShard(s) => {
                assert_eq!(s.target_id, 10);
                assert_eq!(s.index, 3);
                assert_eq!(s.data, vec![0xAA, 0xBB, 0xCC]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_request_image() {
        let msg = TacticalMessage::RequestImage(RequestImage {
            target_id: 5,
            requester: 8,
        });
        let rt = roundtrip(&msg);
        match rt {
            TacticalMessage::RequestImage(r) => {
                assert_eq!(r.target_id, 5);
                assert_eq!(r.requester, 8);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_jam_alert() {
        let msg = TacticalMessage::JamAlert(JamAlert {
            detected_by: 1,
            channel: 6,
            at_ms: 9_999,
        });
        let rt = roundtrip(&msg);
        match rt {
            TacticalMessage::JamAlert(j) => {
                assert_eq!(j.detected_by, 1);
                assert_eq!(j.channel, 6);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_channel_hop() {
        let msg = TacticalMessage::ChannelHop(ChannelHop {
            new_channel: 11,
            new_epoch: 42,
            initiated_by: 0,
        });
        let rt = roundtrip(&msg);
        match rt {
            TacticalMessage::ChannelHop(c) => {
                assert_eq!(c.new_channel, 11);
                assert_eq!(c.new_epoch, 42);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_mayday() {
        let msg = TacticalMessage::Mayday(Mayday {
            node_id: 4,
            at_ms: 123_456,
        });
        let rt = roundtrip(&msg);
        match rt {
            TacticalMessage::Mayday(m) => {
                assert_eq!(m.node_id, 4);
                assert_eq!(m.at_ms, 123_456);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_olsr() {
        let msg = TacticalMessage::Olsr(OlsrMessage {
            payload: vec![1, 2, 3, 4],
        });
        let rt = roundtrip(&msg);
        match rt {
            TacticalMessage::Olsr(o) => {
                assert_eq!(o.payload, vec![1, 2, 3, 4]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_ack() {
        let msg = TacticalMessage::Ack(AckPayload { acked_seq: 0xDEAD_BEEF });
        let rt = roundtrip(&msg);
        match rt {
            TacticalMessage::Ack(a) => {
                assert_eq!(a.acked_seq, 0xDEAD_BEEF);
            }
            _ => panic!("wrong variant"),
        }
    }
}
