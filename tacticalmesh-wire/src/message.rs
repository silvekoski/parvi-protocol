use crate::msg_kind::MsgKind;
use serde::{Deserialize, Serialize};

/// Application-layer message. Defined here as a stub until `tacticalmesh-app` exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TacticalMessage {
    pub kind: MsgKind,
    pub payload: Vec<u8>,
}
