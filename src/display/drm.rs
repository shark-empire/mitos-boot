//! Raw Linux DRM/KMS support, implemented directly against the stable UAPI
//! with no external DRM crate. Keeps early-userspace dependencies minimal and
//! the unsafe surface auditable.
//!
//! Pipeline: /dev/dri/cardN -> connector -> CRTC -> dumb framebuffer ->
//! page flips (double buffered, vsync-paced via flip-complete events).

use super::connector::{self, ConnectorInfo};
use super::mode::{self, Mode};
use super::page_flip;
use crate::config::Config;
use crate::error::BootError;
use crate::renderer::surface::PixelBuffer;
use std::fs::File;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// DRM UAPI (struct layouts must match include/uapi/drm/drm_mode.h exactly)
// ---------------------------------------------------------------------------

pub(crate) mod uapi {
    use std::mem::size_of;
    use std::os::raw::c_ulong;

    const IOC_WRITE: u32 = 1;
    const IOC_READ: u32 = 2;

    const fn ioc(dir: u32, nr: u8, size: usize) -> c_ulong {
        ((dir << 30) | ((size as u32) << 16) | (b'd' as u32) << 8 | (nr as u32)) as c_ulong
    }
    const fn io(nr: u8) -> c_ulong { ioc(0, nr, 0) }
    const fn iowr<T>(nr: u8) -> c_ulong { ioc(IOC_READ | IOC_WRITE, nr, size_of::<T>()) }

    #[repr(C)] #[derive(Clone, Copy)]
    pub struct ModeInfo {
        pub clock: u32,
        pub hdisplay: u16, pub hsync_start: u16, pub hsync_end: u16, pub htotal: u16, pub hskew: u16,
        pub vdisplay: u16, pub vsync_start: u16, pub vsync_end: u16, pub vtotal: u16, pub vscan: u16,
        pub vrefresh: u32, pub flags: u32, pub type_: u32,
        pub name: [u8; 32],
        pub status: u32,
    }

    #[repr(C)] pub struct CardRes {
        pub fb_id_ptr: u64, pub crtc_id_ptr: u64,
        pub count_fbs: u32, pub count_crtcs: u32,
        pub connector_id_ptr: u64, pub encoder_id_ptr: u64,
        pub count_connectors: u32, pub count_encoders: u32,
        pub min_width: u32, pub max_width: u32, pub min_height: u32, pub max_height: u32,
    }

    #[repr(C)] pub struct GetEncoder {
        pub encoder_id: u32, pub encoder_type: u32, pub crtc_id: u32,
        pub possible_crtcs: u32, pub possible_clones: u32,
    }

    #[repr(C)] pub struct GetConnector {
        pub encoders_ptr: u64, pub modes_ptr: u64, pub props_ptr: u64, pub prop_values_ptr: u64,
        pub count_modes: u32, pub count_props: u32, pub count_encoders: u32,
        pub encoder_id: u32, pub connector_id: u32, pub connector_type: u32, pub connector_type_id: u32,
        pub connection: u32, pub mm_width: u32, pub mm_height: u32, pub subpixel: u32, pub pad: u32,
    }

    #[repr(C)] pub struct SetCrtc {
        pub set_connectors_ptr: u64, pub count_connectors: u32,
        pub crtc_id: u32, pub fb_id: u32, pub x: u32, pub y: u32,
        pub gamma_size: u32, pub mode_valid: u32, pub mode: ModeInfo,
    }

    #[repr(C)] pub struct FbCmd {
        pub fb_id: u32, pub width: u32, pub height: u32,
        pub pitch: u32, pub bpp: u32, pub depth: u32, pub handle: u32,
    }

    #[repr(C)] pub struct PageFlip {
        pub fb_id: u32, pub crtc_id: u32, pub flags: u32, pub sequence: u32, pub user_data: u64,
    }

    #[repr(C)] pub struct CreateDumb {
        pub height: u32, pub width: u32, pub bpp: u32, pub flags: u32,
        pub handle: u32, pub pitch: u32, pub size: u64,
    }

    #[repr(C)] pub struct MapDumb { pub handle: u32, pub pad: u32, pub offset: u64 }

    #[repr(C)] pub struct DestroyDumb { pub handle: u32 }

    pub const DRM_IOCTL_SET_MASTER: c_ulong = io(0x1e);
    pub const DRM_IOCTL_DROP_MASTER: c_ulong = io(0x1f);
    pub const DRM_IOCTL_MODE_GETRESOURCES: c_ulong = iowr::<CardRes>(0xa0);
    pub const DRM_IOCTL_MODE_SETCRTC: c_ulong = iowr::<SetCrtc>(0x82);
    pub const DRM_IOCTL_MODE_GETENCODER: c_ulong = iowr::<GetEncoder>(0xa6);
    pub const DRM_IOCTL_MODE_GETCONNECTOR: c_ulong = iowr::<GetConnector>(0xa7);
    pub const DRM_IOCTL_MODE_ADDFB: c_ulong = iowr::<FbCmd>(0xae);
    pub const DRM_IOCTL_MODE_RMFB: c_ulong = iowr::<u32>(0xaf);
    pub const DRM_IOCTL_MODE_PAGE_FLIP: c_ulong = iowr::<PageFlip>(0xb0);
    pub const DRM_IOCTL_MODE_CREATE_DUMB: c_ulong = iowr::<CreateDumb>(0xb2);
    pub const DRM_IOCTL_MODE_MAP_DUMB: c_ulong = iowr::<MapDumb>(0xb3);
    pub const DRM_IOCTL_MODE_DESTROY_DUMB: c_ulong = iowr::<DestroyDumb>(0xb4);

    pub const DRM_MODE_PAGE_FLIP_EVENT: u32 = 1;
    pub const DRM_MODE_CONNECTED: u32 = 1;
    pub const DRM_MODE_TYPE_PREFERRED: u32 = 2;
    pub const DRM_EVENT_FLIP_COMPLETE: u32 = 0x03;
}

// ---------------------------------------------------------------------------

pub(crate) fn ioctl<T>(fd: RawFd, req: std::os::raw::c_ulong, arg: &mut T) -> std::io::Result<()> {
    // SAFETY: `arg` is a valid, properly laid out ioctl payload for `req`.
    let r = unsafe { libc::ioctl(fd, req, arg as *mut T as *mut libc::c_void) };
    if r < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
}

fn ioctl_noarg(fd: RawFd, req: std::os::raw::c_ulong) -> std::io::Result<()> {
    // SAFETY: no-argument ioctl (SET_MASTER / DROP_MASTER).
    let r = unsafe { libc::ioctl(fd, req, std::ptr::null_mut::<libc::c_void>()) };
    if r < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
}

fn ioerr(e: std::io::Error) -> BootError { BootError::Display(e.to_string()) }

pub(crate) struct Resources {
    pub crtc_ids: Vec<u32>,
    pub connector_ids: Vec<u32>,
}

pub(crate) fn get_resources(fd: RawFd) -> Result<Resources, BootError> {
    // Two-step ioctl: query counts, then fill arrays. Retry if the kernel
    // reports more objects in between calls.
    for _ in 0..3