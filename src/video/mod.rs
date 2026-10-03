//! Video subsystem (§9–§12): turn a compressed asset into decoded frames.
//!
//! The decoder is deliberately isolated behind a tiny trait so the backend can
//! change (raw MITOSV today, FFmpeg/WebM behind a cargo feature) without
//! touching boot, splash or rendering code.

pub mod decoder;
pub mod frame;
pub mod format;
pub mod player;
pub mod timing;