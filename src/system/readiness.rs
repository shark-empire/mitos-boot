//! The readiness monitor (§21–§22): a background thread that polls the real
//! system conditions until they hold, the readiness timeout fires, or
//! mitos-init declares readiness over IPC (which is authoritative).

use super::{devices, mounts, services};
use crate::bootlog::messages as msg;
use crate::bootlog::progress::Progress;
use crate::config::Config;
use crate::state::SharedState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const POLL_MS: u64 = 200;

pub struct ReadinessMonitor {
    inner: Arc<Inner>,
    handle: Option<JoinHandle<()>>,
}

struct Inner {
    stop: AtomicBool,
    ready: AtomicBool,
    timed_out: AtomicBool,
    progress: Mutex<Progress>,
}

impl ReadinessMonitor {
    pub fn start(cfg: &Config, state: SharedState) -> Self {
        let mounts = cfg.system.require_mounts.clone();
        let devices = cfg.system.require_devices.clone();
        let files = cfg.system.require_files.clone();
        let marker = cfg.system.init_ready_marker.clone();
        let timeout_ms = cfg.system.readiness_timeout_ms;
        let total = mounts.len() + devices.len() + files.len() + 1; // +1 = init marker

        let inner = Arc::new(Inner {
            stop: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            timed_out: AtomicBool::new(false),
            progress: Mutex::new(Progress::new(total)),
        });

        let inner_t = Arc::clone(&inner);
        let handle = std::thread::spawn(move || {
            let deadline = (timeout_ms > 0)
                .then(|| Instant::now() + Duration::from_millis(timeout_ms as u64));
            // Log each condition exactly once, when it first becomes true.
            let (mut lm, mut ld, mut lf, mut li) = (false, false, false, false);

            loop {
                if inner_t.stop.load(Ordering::Relaxed) { return; }

                // IPC from mitos-init is authoritative (§23).
                if state.system_ready() {
                    inner_t.ready.store(true, Ordering::Relaxed);
                    inner_t.progress.lock().unwrap_or_else(|e| e.into_inner()).complete();
                    return;
                }

                let mpts = mounts::mount_points();
                let m_count = mounts.iter().filter(|m| mpts.contains(m)).count();
                if m_count == mounts.len() && !mounts.is_empty() && !lm {
                    lm = true;
                    crate::bootlog::ok(msg::M_MOUNTS_OK);
                }
                let d_count = devices::count_present(&devices);
                if d_count == devices.len() && !devices.is_empty() && !ld {
                    ld = true;
                    crate::bootlog::ok(msg::M_DEVICES_OK);
                }
                let f_count = services::count_present(&files);
                if f_count == files.len() && !files.is_empty() && !lf {
                    lf = true;
                    crate::bootlog::ok(msg::M_FILES_OK);
                }
                let i_ok = services::init_marker_reached(&marker);
                if i_ok && !li {
                    li = true;
                    crate::bootlog::ok(msg::M_INIT_READY);
                    state.set_init_ready(true);
                }

                let done = m_count + d_count + f_count + usize::from(i_ok);
                inner_t.progress.lock().unwrap_or_else(|e| e.into_inner()).set_done(done);

                if done == total {
                    inner_t.ready.store(true, Ordering::Relaxed);
                    state.set_system_ready(true);
                    return;
                }
                if let Some(dl) = deadline {
                    if Instant::now() >= dl {
                        inner_t.timed_out.store(true, Ordering::Relaxed);
                        log::warn!("readiness timeout after {timeout_ms} ms");
                        return;
                    }
                }
                std::thread::sleep(Duration::from_millis(POLL_MS));
            }
        });

        Self { inner, handle: Some(handle) }
    }

    pub fn is_ready(&self) -> bool { self.inner.ready.load(Ordering::Relaxed) }
    pub fn is_timed_out(&self) -> bool { self.inner.timed_out.load(Ordering::Relaxed) }

    pub fn percent(&self) -> u8 {
        self.inner.progress.lock().unwrap_or_else(|e| e.into_inner()).percent()
    }

    pub fn shutdown(mut self) {
        self.inner.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() { let _ = h.join(); }
    }
}