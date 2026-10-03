//! Display-mode selection.

use super::drm::uapi::{self, ModeInfo};

#[derive(Debug, Clone, Copy)]
pub struct Mode { pub info: ModeInfo }

impl Mode {
    pub fn width(&self) -> u32 { self.info.hdisplay as u32 }
    pub fn height(&self) -> u32 { self.info.vdisplay as u32 }

    pub fn refresh(&self) -> u32 {
        if self.info.vrefresh != 0 { return self.info.vrefresh.clamp(1, 1000); }
        let total = (self.info.htotal.max(1) as u64) * (self.info.vtotal.max(1) as u64);
        ((self.info.clock as u64 * 1000) / total).clamp(1, 1000) as u32
    }
}

/// Preferred mode that fits the limits, else the largest that fits, else the
/// largest overall (small panels, odd firmware).
pub fn select(modes: &[ModeInfo], max_w: u32, max_h: u32) -> Option<Mode> {
    let fits = |m: &ModeInfo| (m.hdisplay as u32) <= max_w && (m.vdisplay as u32) <= max_h;
    let area = |m: &ModeInfo| (m.hdisplay as u64) * (m.vdisplay as u64);

    modes.iter()
        .find(|m| fits(m) && m.type_ & uapi::DRM_MODE_TYPE_PREFERRED != 0)
        .or_else(|| modes.iter().filter(|m| fits(m)).max_by_key(|m| area(m)))
        .or_else(|| modes.iter().max_by_key(|m| area(m)))
        .map(|m| Mode { info: *m })
}