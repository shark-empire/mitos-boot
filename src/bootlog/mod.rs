//! Internal boot information (§20). Normal mode shows only MĨȚǑŠ; debug mode
//! can surface these lines on screen (see boot.rs draw_boot_log).

pub mod logger;
pub mod messages;
pub mod progress;

pub use logger::{init, ring_snapshot, LogLine};

pub fn ok(msg: &str)   { log::info!("[ OK ] {msg}"); }
pub fn warn(msg: &str) { log::warn!("[WARN] {msg}"); }
pub fn fail(msg: &str) { log::error!("[FAIL] {msg}"); }