//! Error screen and recovery flows.
//!
//! The error path uses ONLY the built-in bitmap font and a plain framebuffer
//! fill, so it works even when the TTF renderer, effects, or DRM path that
//! failed during normal boot are unavailable.

use crate::config::{Config, RecoveryChoice};
use crate::error::BootError;
use crate::renderer::text;
use std::io::Read;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};
use std::os::unix::fs::OpenOptionsExt;

pub fn handle_fatal(err: &BootError, cfg: &Config) -> ExitCode {
    crate::bootlog::fail(&err.to_string());

    let display = crate::display::open_recovery_display(cfg);
    if let Some(mut d) = display {
        if let Err(e) = draw_error_screen(d.as_mut(), err, cfg) {
            log::warn!("recovery screen unavailable: {e}");
        }
    }

    let action = choose_action(cfg);
    perform_action(action, cfg)
}

fn action_name(a: RecoveryChoice) -> &'static str {
    match a { RecoveryChoice::Recovery => "recovery", RecoveryChoice::Console => "console",
              RecoveryChoice::Reboot => "reboot", RecoveryChoice::PowerOff => "power off" }
}

fn draw_error_screen(
    display: &mut dyn crate::display::Display,
    err: &BootError,
    cfg: &Config,
) -> Result<(), BootError> {
    let (w, h) = display.size();
    {
        let mut buf = display.back_buffer();
        crate::renderer::effects::fill(&mut buf, 0xFF0B0B0E);
        let big = (h / 10).clamp(24, 160) as u32;
        let small = (h / 28).clamp(12, 48) as u32;
        let tiny = (h / 36).clamp(10, 32) as u32;

        text::draw_bitmap_centered(&mut buf, "MITOS", w as f32 / 2.0, h as f32 * 0.30, big, 0xFFF2F5FA, 1.0);
        text::draw_bitmap_centered(&mut buf, "BOOT FAILED", w as f32 / 2.0, h as f32 * 0.44, small, 0xFFE46956, 1.0);

        let mut y = h as f32 * 0.56;
        for line in wrap(&err.to_string(), 56) {
            text::draw_bitmap_centered(&mut buf, &line, w as f32 / 2.0, y, tiny, 0xFFC7CBD1, 1.0);
            y += tiny as f32 * 1.7;
        }

        text::draw_bitmap_centered(&mut buf, "[1] Recovery   [2] Console   [3] Reboot",
            w as f32 / 2.0, h as f32 * 0.78, small, 0xFFE8EEF4, 1.0);
        if cfg.recovery.timeout_s > 0 {
            let hint = format!("default: {} in {}s", action_name(cfg.recovery.default_action), cfg.recovery.timeout_s);
            text::draw_bitmap_centered(&mut buf, &hint, w as f32 / 2.0, h as f32 * 0.85, tiny, 0xFF88919C, 1.0);
        }
    }
    display.present()
}

fn wrap(s: &str, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        if line.len() + word.len() + 1 > limit {
            if !line.is_empty() { out.push(std::mem::take(&mut line)); }
        }
        if !line.is_empty() { line.push(' '); }
        line.push_str(word);
    }
    if !line.is_empty() { out.push(line); }
    if out.is_empty() { out.push("(no detail)".into()); }
    out
}

fn choose_action(cfg: &Config) -> RecoveryChoice {
    if cfg.recovery.timeout_s == 0 { return cfg.recovery.default_action; }
    let mut scanner = InputScanner::open();
    match scanner.wait_key(Duration::from_secs(cfg.recovery.timeout_s as u64)) {
        Some(1) => RecoveryChoice::Recovery,
        Some(2) => RecoveryChoice::Console,
        Some(3) => RecoveryChoice::Reboot,
        _ => cfg.recovery.default_action,
    }
}

fn perform_action(action: RecoveryChoice, cfg: &Config) -> ExitCode {
    match action {
        RecoveryChoice::Recovery => {
            match crate::handoff::validate_program(&cfg.recovery.recovery_path) {
                Ok(p) => {
                    setup_console_stdio();
                    if crate::handoff::exec_replace(&p, &[], &[("MITOS_RECOVERY", "1")]).is_ok() {
                        return ExitCode::SUCCESS;
                    }
                }
                Err(e) => log::error!("recovery program: {e}"),
            }
            console_fallback(cfg)
        }
        RecoveryChoice::Console => console_fallback(cfg),
        RecoveryChoice::Reboot => {
            unsafe {
                libc::sync();
                if libc::reboot(libc::RB_AUTOBOOT) == 0 {
                    std::thread::sleep(Duration::from_secs(10)); // system is going down
                }
            }
            log::error!("reboot(2) failed (are we PID 1?)");
            console_fallback(cfg)
        }
        RecoveryChoice::PowerOff => {
            unsafe {
                libc::sync();
                if libc::reboot(libc::RB_POWER_OFF) == 0 {
                    std::thread::sleep(Duration::from_secs(10));
                }
            }
            log::error!("poweroff failed");
            console_fallback(cfg)
        }
    }
}

fn console_fallback(cfg: &Config) -> ExitCode {
    setup_console_stdio();
    let prog = crate::handoff::validate_program(&cfg.recovery.console)
        .unwrap_or_else(|_| PathBuf::from("/bin/sh"));
    if crate::handoff::exec_replace(&prog, &["-sh"], &[("MITOS_RECOVERY", "1")]).is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Attach stdio to /dev/console and make it our controlling terminal
/// (best effort) before handing the machine to a shell or recovery tool.
fn setup_console_stdio() {
    unsafe {
        let fd = libc::open(b"/dev/console\0".as_ptr() as *const libc::c_char, libc::O_RDWR);
        if fd >= 0 {
            libc::setsid();
            libc::ioctl(fd, libc::TIOCSCTTY as _, 0 as libc::c_int);
            libc::dup2(fd, 0);
            libc::dup2(fd, 1);
            libc::dup2(fd, 2);
            if fd > 2 { libc::close(fd); }
        }
    }
}

// ---------------------------------------------------------------------------
// Minimal evdev reader: enough to read keys 1/2/3 from /dev/input/event*.
// ---------------------------------------------------------------------------

/// struct input_event on LP64: { i64 sec; i64 usec; u16 type; u16 code; i32 value }
const EVENT_SIZE: usize = 24;
const EV_KEY: u16 = 0x01;

struct InputScanner { fds: Vec<std::fs::File> }

impl InputScanner {
    fn open() -> Self {
        use std::os::unix::fs::OpenOptionsExt;
        let mut fds = Vec::new();
        for i in 0..16 {
            let p = format!("/dev/input/event{i}");
            if let Ok(f) = std::fs::OpenOptions::new().read(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC).open(&p) { fds.push(f); }
        }
        Self { fds }
    }

    /// Returns Some(1|2|3) when KEY_1..KEY_3 is pressed, None on timeout.
    fn wait_key(&mut self, timeout: Duration) -> Option<u8> {
        let deadline = Instant::now() + timeout;
        let mut buf = [0u8; 256];
        loop {
            let now = Instant::now();
            if now >= deadline { return None; }
            if self.fds.is_empty() { std::thread::sleep(deadline - now); return None; }
            let remaining = ((deadline - now).as_millis() as i32).max(0);
            let mut pfds: Vec<libc::pollfd> = self.fds.iter()
                .map(|f| libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLIN, revents: 0 })
                .collect();
            let r = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, remaining) };
            if r <= 0 { continue; } // timeout / EINTR
            for (i, pfd) in pfds.iter().enumerate() {
                if pfd.revents & libc::POLLIN == 0 { continue; }
                let n = (&self.fds[i]).read(&mut buf).unwrap_or(0);
                if let Some(k) = parse_key_events(&buf[..n]) { return Some(k); }
            }
        }
    }
}

fn parse_key_events(bytes: &[u8]) -> Option<u8> {
    let mut off = 0usize;
    while off + EVENT_SIZE <= bytes.len() {
        // Parse field-by-field to avoid unaligned casts (safe, portable).
        let ty = u16::from_ne_bytes([bytes[off + 16], bytes[off + 17]]);
        let code = u16::from_ne_bytes([bytes[off + 18], bytes[off + 19]]);
        let value = i32::from_ne_bytes(bytes[off + 20..off + 24].try_into().unwrap());
        if ty == EV_KEY && value == 1 {
            match code { 2 => return Some(1), 3 => return Some(2), 4 => return Some(3), _ => {} }
        }
        off += EVENT_SIZE;
    }
    None
}