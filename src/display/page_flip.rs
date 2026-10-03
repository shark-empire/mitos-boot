//! Page flips and vsync (flip-complete) event handling.
//!
//! Every wait is bounded: a missing vsync event may cost us a frame, never
//! the boot.

use super::drm::{ioctl, uapi};
use crate::error::BootError;
use std::os::unix::io::RawFd;
use std::time::{Duration, Instant};

pub fn submit(fd: RawFd, crtc_id: u32, fb_id: u32) -> Result<(), BootError> {
    let mut flip = uapi::PageFlip {
        fb_id,
        crtc_id,
        flags: uapi::DRM_MODE_PAGE_FLIP_EVENT,
        sequence: 0,
        user_data: fb_id as u64,
    };
    ioctl(fd, uapi::DRM_IOCTL_MODE_PAGE_FLIP, &mut flip)
        .map_err(|e| BootError::Display(format!("page flip: {e}")))
}

/// Wait up to `timeout_ms` for a flip-complete event.
/// Ok(true) = event consumed; Ok(false) = timeout (caller continues).
pub fn wait_complete(fd: RawFd, timeout_ms: i32) -> Result<bool, BootError> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms.max(1) as u64);
    let mut buf = [0u8; 1024];
    loop {
        let now = Instant::now();
        if now >= deadline { return Ok(false); }
        let remaining = ((deadline - now).as_millis() as i32).clamp(1, timeout_ms.max(1));

        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: pfd is a valid pollfd for the duration of the call.
        let r = unsafe { libc::poll(&mut pfd, 1, remaining) };
        if r < 0 { continue; }              // EINTR
        if r == 0 { return Ok(false); }     // timeout
        if pfd.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err(BootError::Display("DRM fd error condition".into()));
        }

        // SAFETY: buf is a valid writable buffer; fd is O_NONBLOCK.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n <= 0 { continue; }             // EAGAIN / nothing useful
        if parse_events(&buf[..n as usize]) { return Ok(true); }
    }
}

/// Walk drm_event records: { u32 type; u32 length; payload[length] }.
fn parse_events(bytes: &[u8]) -> bool {
    let mut off = 0usize;
    while off + 8 <= bytes.len() {
        let ty = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        let len = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap()) as usize;
        if len < 8 { break; }
        if ty == uapi::DRM_EVENT_FLIP_COMPLETE { return true; }
        off += len;
    }
    false
}