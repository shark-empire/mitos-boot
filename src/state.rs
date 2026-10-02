//! Runtime boot state. Thread-safe; queried by the IPC reader thread, the
//! readiness monitor and the render loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootStage {
    Starting, Initializing, DisplayReady, Splash, Wordmark, SystemReady, Handoff, Error, Recovery,
}

impl BootStage {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Starting => "starting", Self::Initializing => "initializing",
            Self::DisplayReady => "display-ready", Self::Splash => "splash",
            Self::Wordmark => "wordmark", Self::SystemReady => "system-ready",
            Self::Handoff => "handoff", Self::Error => "error", Self::Recovery => "recovery",
        }
    }
}

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

struct Inner {
    stage: Mutex<BootStage>,
    start: Instant,
    display_ready: AtomicBool,
    renderer_ready: AtomicBool,
    animation_ready: AtomicBool,
    system_ready: AtomicBool,
    init_ready: AtomicBool,
    error: Mutex<Option<String>>,
}

#[derive(Clone)]
pub struct SharedState(Arc<Inner>);

impl SharedState {
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            stage: Mutex::new(BootStage::Starting),
            start: Instant::now(),
            display_ready: AtomicBool::new(false),
            renderer_ready: AtomicBool::new(false),
            animation_ready: AtomicBool::new(false),
            system_ready: AtomicBool::new(false),
            init_ready: AtomicBool::new(false),
            error: Mutex::new(None),
        }))
    }

    pub fn stage(&self) -> BootStage { *self.0.stage.lock().unwrap() }
    pub fn set_stage(&self, s: BootStage) {
        let mut g = self.0.stage.lock().unwrap();
        if *g != s { log::debug!("stage: {} -> {}", g.name(), s.name()); *g = s; }
    }
    pub fn elapsed(&self) -> Duration { self.0.start.elapsed() }

    fn flag(b: &AtomicBool, v: bool) { b.store(v, Ordering::SeqCst); }
    pub fn set_display_ready(&self, v: bool) { Self::flag(&self.0.display_ready, v); }
    pub fn set_renderer_ready(&self, v: bool) { Self::flag(&self.0.renderer_ready, v); }
    pub fn set_animation_ready(&self, v: bool) { Self::flag(&self.0.animation_ready, v); }
    pub fn set_init_ready(&self, v: bool) { Self::flag(&self.0.init_ready, v); }

    /// Set once-only; log the transition the first time.
    pub fn set_system_ready(&self, v: bool) {
        if self.0.system_ready.swap(v, Ordering::SeqCst) != v && v {
            log::info!("system readiness signalled");
        }
    }
    pub fn system_ready(&self) -> bool { self.0.system_ready.load(Ordering::SeqCst) }

    pub fn set_error(&self, e: impl std::fmt::Display) {
        *self.0.error.lock().unwrap() = Some(e.to_string());
        self.set_stage(BootStage::Error);
    }
    pub fn error(&self) -> Option<String> { self.0.error.lock().unwrap().clone() }
}

pub fn request_shutdown() { SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst); }
pub fn shutdown_requested() -> bool { SHUTDOWN_REQUESTED.load(Ordering::SeqCst) }

extern "C" fn on_signal(_sig: libc::c_int) { request_shutdown(); }

pub fn install_signal_handlers() {
    // Handler only flips an atomic flag; async-signal-safe.
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as libc::sighandler_t);
    }
}