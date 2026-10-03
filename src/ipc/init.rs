//! The IPC message set (§23) and its wire encoding.
//!
//! Frame: u32-LE payload length (1..=MAX_LEN), then payload:
//!   u8 tag, [tag-specific data]
//!   String data: u32-LE byte length (≤ MAX_STRING) + UTF-8 bytes
//! Strings are UTF-8-validated on decode (no log-injection via raw bytes),
//! and every length is bounds-checked before use.

use crate::error::BootError;

pub const MAX_LEN: usize = 512;
const MAX_STRING: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    // mitos-boot → clients (events)
    BootStarted,
    DisplayReady,
    SplashStarted,
    SplashFinished,
    SystemReady,
    BootReady,
    Handoff,
    BootError(String),
    // clients → mitos-boot (commands)
    InitReady,
    SetSystemReady,
    Ping,
    // replies
    Ack,
    Error(String),
}

impl Message {
    fn tag(&self) -> u8 {
        match self {
            Self::BootStarted => 0, Self::DisplayReady => 1, Self::SplashStarted => 2,
            Self::SplashFinished => 3, Self::SystemReady => 4, Self::BootReady => 5,
            Self::Handoff => 6, Self::BootError(_) => 7,
            Self::InitReady => 8, Self::SetSystemReady => 9, Self::Ping => 10,
            Self::Ack => 11, Self::Error(_) => 12,
        }
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        out.push(self.tag());
        if let Self::BootError(s) | Self::Error(s) = self {
            out.extend_from_slice(&(s.len() as u32).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(8);
        self.encode(&mut v);
        v
    }

    pub fn decode(buf: &[u8]) -> Result<Message, BootError> {
        let e = |m: &str| BootError::Ipc(m.to_string());
        let Some((&tag, rest)) = buf.split_first() else {
            return Err(e("empty message"));
        };
        let read_string = |b: &[u8]| -> Result<String, BootError> {
            if b.len() < 4 { return Err(e("truncated string")); }
            let n = u32::from_le_bytes(b[..4].try_into().unwrap()) as usize;
            if n > MAX_STRING || b.len() != 4 + n { return Err(e("bad string length")); }
            String::from_utf8(b[4..].to_vec()).map_err(|_| e("string is not UTF-8"))
        };
        match tag {
            0 => Ok(Self::BootStarted),
            1 => Ok(Self::DisplayReady),
            2 => Ok(Self::SplashStarted),
            3 => Ok(Self::SplashFinished),
            4 => Ok(Self::SystemReady),
            5 => Ok(Self::BootReady),
            6 => Ok(Self::Handoff),
            7 => Ok(Self::BootError(read_string(rest)?)),
            8 => Ok(Self::InitReady),
            9 => Ok(Self::SetSystemReady),
            10 => Ok(Self::Ping),
            11 => Ok(Self::Ack),
            12 => Ok(Self::Error(read_string(rest)?)),
            _ => Err(e("unknown message tag")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_all() {
        for m in [
            Message::BootStarted, Message::DisplayReady, Message::SplashStarted,
            Message::SplashFinished, Message::SystemReady, Message::BootReady,
            Message::Handoff, Message::BootError("display gone".into()),
            Message::InitReady, Message::SetSystemReady, Message::Ping,
            Message::Ack, Message::Error("nope".into()),
        ] {
            assert_eq!(Message::decode(&m.to_bytes()).unwrap(), m);
        }
    }
    #[test]
    fn rejects_garbage() {
        assert!(Message::decode(&[]).is_err());
        assert!(Message::decode(&[0xFF]).is_err());
        // Tag 7 (BootError) with a lying string length.
        let mut bad = vec![7u8];
        bad.extend_from_slice(&9999u32.to_le_bytes());
        assert!(Message::decode(&bad).is_err());
        // Invalid UTF-8 in a string payload.
        let mut bad = vec![7u8];
        bad.extend_from_slice(&2u32.to_le_bytes());
        bad.extend_from_slice(&[0xFF, 0xFE]);
        assert!(Message::decode(&bad).is_err());
    }
}