//! Canonical boot messages (§20). Keep them short and stable; the debug
//! overlay and mitos-init may key off them.

pub const M_BOOT_START: &str = "MITOS boot started";
pub const M_DISPLAY_READY: &str = "Display initialized";
pub const M_SPLASH_START: &str = "Splash started";
pub const M_WORDMARK: &str = "MITOS wordmark";
pub const M_SPLASH_DONE: &str = "Splash finished";
pub const M_SYSTEM_READY: &str = "System ready";
pub const M_HANDOFF: &str = "Handing off to mitos-init";
pub const M_READINESS_TIMEOUT: &str = "System readiness timeout; continuing boot";
pub const M_MOUNTS_OK: &str = "Required mounts available";
pub const M_DEVICES_OK: &str = "Required devices available";
pub const M_FILES_OK: &str = "Required files available";
pub const M_INIT_READY: &str = "mitos-init ready";