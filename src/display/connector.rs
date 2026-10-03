//! Connector discovery and encoder/CRTC resolution.
//!
//! This component knows nothing about owls, wordmarks or login — it only
//! finds a display target (see §6 of the design document).

use super::drm::{ioctl, uapi};
use crate::error::BootError;
use std::os::unix::io::RawFd;

pub struct ConnectorInfo {
    pub id: u32,
    pub kind: u32,
    pub kind_id: u32,
    pub connection: u32,
    pub encoder_id: u32,
    pub encoders: Vec<u32>,
    pub modes: Vec<uapi::ModeInfo>,
    pub mm_width: u32,
    pub mm_height: u32,
}

pub struct EncoderInfo {
    pub id: u32,
    pub crtc_id: u32,
    pub possible_crtcs: u32,
}

// SAFETY: all-zero bits are a valid ModeInfo (plain integers/arrays).
fn zeroed_mode() -> uapi::ModeInfo { unsafe { std::mem::zeroed() } }

pub fn get_connector(fd: RawFd, id: u32) -> Result<ConnectorInfo, BootError> {
    // Two-step enumeration with sanity caps so a hostile/broken driver can
    // never make us allocate unbounded memory.
    for _ in 0..3 {
        let mut c: uapi::GetConnector = unsafe { std::mem::zeroed() };
        c.connector_id = id;
        ioctl(fd, uapi::DRM_IOCTL_MODE_GETCONNECTOR, &mut c)
            .map_err(|e| BootError::Display(format!("GETCONNECTOR: {e}")))?;

        let n_enc = c.count_encoders as usize;
        let n_modes = c.count_modes as usize;
        if n_enc > 32 || n_modes > 512 {
            return Err(BootError::Display("implausible connector counts".into()));
        }

        let mut encoders = vec![0u32; n_enc.max(1)];
        let mut modes: Vec<uapi::ModeInfo> = (0..n_modes).map(|_| zeroed_mode()).collect();
        c.encoders_ptr = encoders.as_mut_ptr() as u64;
        c.modes_ptr = modes.as_mut_ptr() as *mut uapi::ModeInfo as u64;
        c.props_ptr = 0;
        c.prop_values_ptr = 0;
        c.count_props = 0;
        c.count_encoders = n_enc as u32;
        c.count_modes = n_modes as u32;

        ioctl(fd, uapi::DRM_IOCTL_MODE_GETCONNECTOR, &mut c)
            .map_err(|e| BootError::Display(format!("GETCONNECTOR: {e}")))?;

        if (c.count_encoders as usize) <= n_enc && (c.count_modes as usize) <= n_modes {
            encoders.truncate(c.count_encoders as usize);
            modes.truncate(c.count_modes as usize);
            modes.retain(|m| m.hdisplay != 0 && m.vdisplay != 0);
            return Ok(ConnectorInfo {
                id,
                kind: c.connector_type,
                kind_id: c.connector_type_id,
                connection: c.connection,
                encoder_id: c.encoder_id,
                encoders,
                modes,
                mm_width: c.mm_width,
                mm_height: c.mm_height,
            });
        }
    }
    Err(BootError::Display("connector state changed while enumerating".into()))
}

pub fn get_encoder(fd: RawFd, id: u32) -> Result<EncoderInfo, BootError> {
    let mut e = uapi::GetEncoder { encoder_id: id, encoder_type: 0, crtc_id: 0, possible_crtcs: 0, possible_clones: 0 };
    ioctl(fd, uapi::DRM_IOCTL_MODE_GETENCODER, &mut e)
        .map_err(|e| BootError::Display(format!("GETENCODER: {e}")))?;
    Ok(EncoderInfo { id, crtc_id: e.crtc_id, possible_crtcs: e.possible_crtcs })
}

/// First connected connector that reports at least one mode.
pub fn find_connected(fd: RawFd, ids: &[u32]) -> Result<Option<ConnectorInfo>, BootError> {
    for &id in ids {
        let c = get_connector(fd, id)?;
        if c.connection == uapi::DRM_MODE_CONNECTED {
            if !c.modes.is_empty() { return Ok(Some(c)); }
            log::warn!("connector {id} connected but reports no modes");
        }
    }
    Ok(None)
}

/// Resolve a CRTC for the connector: prefer the CRTC its encoder is already
/// bound to, else the first CRTC allowed by `possible_crtcs`.
pub fn resolve_crtc(fd: RawFd, conn: &ConnectorInfo, crtc_ids: &[u32])
    -> Result<Option<u32>, BootError>
{
    let mut enc_ids: Vec<u32> = Vec::new();
    if conn.encoder_id != 0 { enc_ids.push(conn.encoder_id); }
    for &e in &conn.encoders {
        if !enc_ids.contains(&e) { enc_ids.push(e); }
    }

    let mut fallback: Option<u32> = None;
    for eid in enc_ids {
        let Ok(enc) = get_encoder(fd, eid) else { continue };
        if enc.crtc_id != 0 && crtc_ids.contains(&enc.crtc_id) {
            return Ok(Some(enc.crtc_id));
        }
        if fallback.is_none() {
            for (i, &cid) in crtc_ids.iter().enumerate() {
                if enc.possible_crtcs & (1u32 << i) != 0 { fallback = Some(cid); break; }
            }
        }
    }
    Ok(fallback)
}

pub fn type_name(kind: u32) -> &'static str {
    match kind {
        1 => "VGA", 2 => "DVI-I", 3 => "DVI-D", 4 => "DVI-A", 5 => "Composite",
        6 => "S-Video", 7 => "LVDS", 8 => "Component", 9 => "9-pin DIN",
        10 => "DisplayPort", 11 => "HDMI-A", 12 => "HDMI-B", 13 => "TV",
        14 => "eDP", 15 => "Virtual", 16 => "DSI", 17 => "DPI", 18 => "Writeback",
        _ => "unknown",
    }
}