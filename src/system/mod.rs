//! System readiness (§21–§22): determines whether MITOS can continue.
//! BOOT ANIMATION ≠ SYSTEM READINESS.

pub mod devices;
pub mod mounts;
pub mod readiness;
pub mod services;

pub use readiness::ReadinessMonitor;