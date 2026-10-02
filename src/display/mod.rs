//! The display subsystem answers: "Where do we draw?"
//! Primary backend: Linux DRM/KMS. Fallback: Linux framebuffer.

mod connector;
mod drm;
mod framebuffer;
mod mode;
mod page_flip;

use crate::config::{Config, RendererBackend};
use crate::error::BootError;
use crate::renderer::surface::PixelBuffer;

pub trait Display: Send {
    fn size(&self) -> (u32, u32);
    fn backend_name(&self) -> &'static str;
    /// True when `present()` blocks until vsync (DRM page-flip event).
    fn blocks_on_vsync(&self) -> bool;
    /// CPU-writable buffer for the frame that is NOT currently scanned out.
    fn back_buffer(&mut self) -> PixelBuffer<'_>;
    /// Make the back buffer visible. DRM: page flip + wait for vsync.
    fn present(&mut self) -> Result<(), BootError>;
    /// Release hardware (drop DRM master, etc). Idempotent; Drop also calls it.
    fn release(&mut self);
}

pub fn open(cfg: &Config) -> Result<Box<dyn Display>, BootError> {
    match cfg.renderer.backend {
        RendererBackend::Drm => Ok(Box::new(drm::DrmDisplay::open(cfg)?)),
        RendererBackend::Fbdev => Ok(Box::new(framebuffer::FbdevDisplay::open(cfg)?)),
        RendererBackend::Disabled => Err(BootError::Display("display disabled by configuration".into())),
        RendererBackend::Auto => match drm::DrmDisplay::open(cfg) {
            Ok(d) => Ok(Box::new(d)),
            Err(e) => {
                log::warn!("DRM/KMS unavailable ({e}); trying framebuffer fallback");
                Ok(Box::new(framebuffer::FbdevDisplay::open(cfg)?))
            }
        },
    }
}

/// Display for the error screen: prefer the simplest possible path (fbdev),
/// per "the error system should work even if the fancy renderer fails".
pub fn open_recovery_display(cfg: &Config) -> Option<Box<dyn Display>> {
    if let Ok(d) = framebuffer::FbdevDisplay::open(cfg) { return Some(Box::new(d)); }
    if let Ok(d) = drm::DrmDisplay::open(cfg) { return Some(Box::new(d)); }
    None
}