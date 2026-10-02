//! Central boot coordinator: decides *what happens next*, never *how to draw
//! a pixel* (that is renderer/), and never synchronizes on arbitrary sleeps —
//! the timeline is gated on real system readiness.

use crate::animation::{Phase, SplashTimeline};
use crate::bootlog;
use crate::bootlog::messages as msg;
use crate::config::Config;
use crate::error::BootError;
use crate::ipc::{IpcServer, Message};
use crate::renderer::{compositor::fill_rect, PixelBuffer, Rect, Renderer};
use crate::splash::SplashScene;
use crate::state::{shutdown_requested, BootStage, SharedState};
use crate::system::ReadinessMonitor;
use crate::video;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Global stall detector: if the boot loop stops kicking for `timeout_ms`,
/// exit with a distinctive code so the surrounding initramfs/init can react.
/// The splash must never be able to wedge the boot.
struct Watchdog {
    beat: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}
impl Watchdog {
    fn start(timeout_ms: u64) -> Self {
        let beat = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = (timeout_ms > 0).then(|| {
            let (beat, stop) = (beat.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut last = 0u64;
                let mut last_change = Instant::now();
                loop {
                    std::thread::sleep(Duration::from_millis(2_000));
                    if stop.load(Ordering::Relaxed) { break; }
                    let b = beat.load(Ordering::Relaxed);
                    if b != last { last = b; last_change = Instant::now(); }
                    else if last_change.elapsed().as_millis() as u64 >= timeout_ms {
                        log::error!("watchdog: no progress for {timeout_ms} ms, aborting");
                        std::process::exit(86);
                    }
                }
            })
        });
        Self { beat, stop, handle }
    }
    fn kick(&self) { self.beat.fetch_add(1, Ordering::Relaxed); }
    fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() { let _ = h.join(); }
    }
}

pub fn run(cfg: &Config, state: &SharedState) -> Result<(), BootError> {
    state.set_stage(BootStage::Initializing);
    bootlog::ok(msg::M_BOOT_START);

    // --- IPC (failure is non-fatal: boot must proceed) -------------------
    let mut ipc: Option<IpcServer> = if cfg.ipc.enabled {
        match IpcServer::bind(cfg, state.clone()) {
            Ok(mut s) => { s.start(); Some(s) }
            Err(e) => { log::warn!("ipc unavailable: {e}"); None }
        }
    } else { None };
    ipc_send(&ipc, &Message::BootStarted);

    // --- readiness monitor + watchdog ------------------------------------
    let readiness = ReadinessMonitor::start(cfg, state.clone());
    let watchdog = Watchdog::start(cfg.reliability.watchdog_ms);

    let result = (|| -> Result<(), BootError> {
        if !cfg.boot.enabled || !cfg.splash.enabled {
            log::info!("visual boot disabled; waiting for system readiness");
            wait_headless(&readiness, &watchdog)?;
            return Ok(());
        }
        let mut display = match crate::display::open(cfg) {
            Ok(d) => {
                state.set_display_ready(true);
                state.set_stage(BootStage::DisplayReady);
                bootlog::ok(msg::M_DISPLAY_READY);
                ipc_send(&ipc, &Message::DisplayReady);
                d
            }
            Err(e) => {
                // A missing display must never block the OS from booting.
                log::warn!("no display available ({e}); continuing headless");
                wait_headless(&readiness, &watchdog)?;
                return Ok(());
            }
        };
        run_visual(cfg, state, &readiness, &ipc, display.as_mut(), &watchdog)?;
        display.release();
        Ok(())
    })();

    match result {
        Ok(()) => {}
        Err(e) => {
            state.set_error(&e);
            watchdog.stop();
            readiness.shutdown();
            return Err(e);
        }
    }

    state.set_stage(BootStage::SystemReady);
    bootlog::ok(msg::M_SYSTEM_READY);
    watchdog.stop();
    readiness.shutdown();
    crate::handoff::perform(cfg, &mut ipc, state)
}

fn ipc_send(ipc: &Option<IpcServer>, m: &Message) {
    if let Some(s) = ipc { s.send(m); }
}

fn wait_headless(readiness: &ReadinessMonitor, watchdog: &Watchdog) -> Result<(), BootError> {
    let mut warned = false;
    loop {
        watchdog.kick();
        if shutdown_requested() { return Ok(()); }
        if readiness.is_ready() { return Ok(()); }
        if readiness.is_timed_out() {
            if !warned { warned = true; bootlog::warn(msg::M_READINESS_TIMEOUT); }
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn run_visual(
    cfg: &Config,
    state: &SharedState,
    readiness: &ReadinessMonitor,
    ipc: &Option<IpcServer>,
    display: &mut dyn crate::display::Display,
    watchdog: &Watchdog,
) -> Result<(), BootError> {
    let (w, h) = display.size();
    let renderer = Renderer::new(cfg);
    state.set_renderer_ready(true);

    // Splash construction never hard-fails: a broken video falls back to a
    // static image, then to the wordmark-only experience.
    let mut scene = SplashScene::build(cfg, &renderer, w, h);
    state.set_animation_ready(scene.owl.is_some());

    let mut timeline =
        SplashTimeline::new(&cfg.animation, scene.owl.is_none(), cfg.splash.wordmark);
    state.set_stage(BootStage::Splash);
    ipc_send(ipc, &Message::SplashStarted);
    bootlog::ok(msg::M_SPLASH_START);

    let start = Instant::now();
    let mut ticker = video::timing::Ticker::new(60.0); // pacing when backend has no vsync wait
    let mut sent_system_ready = false;

    loop {
        watchdog.kick();
        if shutdown_requested() { break; }

        let now = Instant::now();
        let elapsed = (now - start).as_secs_f64();

        let video_done = scene.update(now);
        let ready = readiness.is_ready();
        let timed_out = readiness.is_timed_out();
        if ready && !sent_system_ready {
            sent_system_ready = true;
            ipc_send(ipc, &Message::SystemReady);
        }

        let prev = timeline.phase();
        let phase = timeline.update(elapsed, video_done, ready, timed_out);
        if phase != prev {
            log::debug!("timeline phase -> {phase:?}");
            if phase == Phase::WordmarkIn {
                state.set_stage(BootStage::Wordmark);
                bootlog::ok(msg::M_WORDMARK);
            }
        }
        if phase == Phase::Complete { break; }

        {
            let mut buf = display.back_buffer();
            renderer.begin_frame();
            renderer.draw_background(&mut buf, &scene.background);
            let oa = timeline.owl_alpha();
            if oa > 0.0 { scene.draw_owl(&renderer, &mut buf, oa); }
            let wa = timeline.wordmark_alpha();
            if wa > 0.0 { scene.draw_wordmark(&renderer, &mut buf, wa); }
            if cfg.debug.show_boot_messages { draw_boot_log(&mut buf, w, h); }
            if cfg.splash.show_progress { draw_progress(&mut buf, w, h, readiness.percent()); }
            renderer.end_frame();
        }
        display.present()?;
        if !display.blocks_on_vsync() { ticker.wait(); }
    }

    // Leave a clean black frame behind before releasing the display.
    {
        let mut buf = display.back_buffer();
        crate::renderer::effects::fill(&mut buf, 0xFF000000);
    }
    let _ = display.present();

    ipc_send(ipc, &Message::SplashFinished);
    bootlog::ok(msg::M_SPLASH_DONE);
    Ok(())
}

fn draw_boot_log(buf: &mut PixelBuffer, w: u32, h: u32) {
    let lines = bootlog::ring_snapshot();
    let px = 16u32;
    let line_h = px as i32 + 6;
    let max_lines = 10usize;
    let start = lines.len().saturating_sub(max_lines);
    let y0 = h as i32 - 24 - ((lines.len() - start) as i32) * line_h;
    for (i, line) in lines.iter().skip(start).enumerate() {
        let color = match line.level {
            log::Level::Error => 0xFFE26D5A,
            log::Level::Warn => 0xFFE2C08D,
            _ => 0xFFB8C0CC,
        };
        crate::renderer::text::draw_bitmap(buf, &line.text, 16, y0 + (i as i32) * line_h, px, color, 0.9);
    }
    let _ = w;
}

fn draw_progress(buf: &mut PixelBuffer, w: u32, h: u32, percent: u8) {
    let (seg_w, gap, n) = (28u32, 12u32, 5u32);
    let total = n * seg_w + (n - 1) * gap;
    let x0 = ((w as i64 - total as i64) / 2) as i32;
    let y = h as i32 - 56;
    let filled = (percent as u32 * n + 50) / 100;
    for i in 0..n {
        let x = x0 + (i * (seg_w + gap)) as i32;
        let (color, alpha) = if i < filled { (0xFFE8EEF4, 0.9) } else { (0xFFFFFFFF, 0.30) };
        fill_rect(buf, &Rect { x, y, w: seg_w, h: 6 }, color, alpha);
    }
}