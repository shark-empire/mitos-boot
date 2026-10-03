//! The boot logger: stderr (often a serial console), /dev/kmsg (best effort),
//! and a bounded in-memory ring that the debug on-screen overlay renders.
//!
//! Everything is allocation-light and failure-tolerant: a closed stderr or a
//! missing /dev/kmsg must never take the boot down.

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct LogLine {
    pub level: Level,
    pub text: String,
}

const RING_CAP: usize = 48;

struct Ring(Mutex<VecDeque<LogLine>>);
static RING: OnceLock<Ring> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();
static KMSG: OnceLock<Option<File>> = OnceLock::new();
static FILTER: AtomicUsize = AtomicUsize::new(LevelFilter::Info as usize);

struct BootLogger;

impl Log for BootLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() as usize <= FILTER.load(Ordering::Relaxed)
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) { return; }
        let text = format!("{}", record.args());
        let line = format!(
            "{:7.3}s {:<5} {text}",
            START.get_or_init(Instant::now).elapsed().as_secs_f64(),
            record.level().as_str()
        );

        // stderr — may be closed in early boot; ignore errors entirely.
        let bytes = format!("{line}\n").into_bytes();
        unsafe {
            libc::write(2, bytes.as_ptr() as *const libc::c_void, bytes.len());
        }

        // /dev/kmsg — best effort, with the kernel priority prefix.
        if let Some(f) = KMSG.get().and_then(|o| o.as_ref()) {
            let prio = match record.level() {
                Level::Error => 3, Level::Warn => 4, Level::Info => 6, _ => 7,
            };
            let _ = writeln!(f, "<{prio}>mitos-boot: {text}");
        }

        // On-screen ring (bounded).
        if let Some(r) = RING.get() {
            let mut q = r.0.lock().unwrap_or_else(|e| e.into_inner());
            if q.len() >= RING_CAP { q.pop_front(); }
            q.push_back(LogLine { level: record.level(), text: line });
        }
    }

    fn flush(&self) {
        if let Some(f) = KMSG.get().and_then(|o| o.as_ref()) {
            let _ = f.flush();
        }
    }
}

/// Install the logger (once) and set the level. Calling again only adjusts
/// the filter — main.rs calls this before and after loading the config.
pub fn init(level: LevelFilter) {
    FILTER.store(level as usize, Ordering::Relaxed);
    log::set_max_level(level);
    let _ = RING.set(Ring(Mutex::new(VecDeque::with_capacity(RING_CAP))));
    let _ = KMSG.set(
        File::options()
            .write(true).append(true)
            .custom_flags(libc::O_CLOEXEC)
            .open("/dev/kmsg")
            .ok(),
    );
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let _ = log::set_boxed_logger(Box::new(BootLogger));
    });
}

pub fn ring_snapshot() -> Vec<LogLine> {
    RING.get()
        .map(|r| r.0.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect())
        .unwrap_or_default()
}