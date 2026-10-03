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
use super::Display;
use std::ffi::CString;
use std::time::Duration;

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
    // reports more objects in between calls (bounded to avoid loops driven
    // by a misbehaving driver).
    for _ in 0..3 {
        let mut res: uapi::CardRes = unsafe { std::mem::zeroed() };
        ioctl(fd, uapi::DRM_IOCTL_MODE_GETRESOURCES, &mut res).map_err(ioerr)?;

        let n_crtcs = res.count_crtcs as usize;
        let n_conns = res.count_connectors as usize;
        if n_crtcs > 64 || n_conns > 64 {
            return Err(BootError::Display("implausible DRM resource counts".into()));
        }

        let mut crtcs = vec![0u32; n_crtcs.max(1)];
        let mut conns = vec![0u32; n_conns.max(1)];
        res.fb_id_ptr = 0;
        res.encoder_id_ptr = 0;
        res.count_fbs = 0;
        res.count_encoders = 0;
        res.crtc_id_ptr = crtcs.as_mut_ptr() as u64;
        res.connector_id_ptr = conns.as_mut_ptr() as u64;

        ioctl(fd, uapi::DRM_IOCTL_MODE_GETRESOURCES, &mut res).map_err(ioerr)?;

        if (res.count_crtcs as usize) <= crtcs.len() && (res.count_connectors as usize) <= conns.len() {
            crtcs.truncate(res.count_crtcs as usize);
            conns.truncate(res.count_connectors as usize);
            return Ok(Resources { crtc_ids: crtcs, connector_ids: conns });
        }
    }
    Err(BootError::Display("DRM resource set changed while enumerating".into()))
}

// ---------------------------------------------------------------------------
// Dumb buffers (CPU-rendered, double buffered)
// ---------------------------------------------------------------------------

/// A writable mapping of a dumb buffer.
struct Mapped { ptr: *mut u8, len: usize }

// SAFETY: the mapping is a process-wide resource; instances are only moved
// between threads and never accessed concurrently.
unsafe impl Send for Mapped {}

impl Mapped {
    fn map(fd: RawFd, handle: u32, len: usize) -> Result<Self, BootError> {
        let mut m = uapi::MapDumb { handle, pad: 0, offset: 0 };
        ioctl(fd, uapi::DRM_IOCTL_MODE_MAP_DUMB, &mut m)
            .map_err(|e| BootError::Display(format!("map dumb buffer: {e}")))?;
        // SAFETY: standard mmap of the dumb-buffer GEM object; paired with
        // munmap in `unmap`/Drop.
        let ptr = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE,
                       libc::MAP_SHARED, fd, m.offset as libc::off_t)
        };
        if ptr == libc::MAP_FAILED {
            return Err(BootError::Display(format!(
                "mmap dumb buffer: {}", std::io::Error::last_os_error())));
        }
        Ok(Mapped { ptr: ptr as *mut u8, len })
    }

    fn as_mut(&mut self) -> &mut [u8] {
        // SAFETY: the mapping is valid for `len` bytes until munmap.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    fn unmap(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: paired with the mmap in `map`.
            unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.len); }
            self.ptr = std::ptr::null_mut();
        }
    }
}

impl Drop for Mapped { fn drop(&mut self) { self.unmap(); } }

struct DumbBuffer {
    width: u32,
    height: u32,
    pitch: u32,
    handle: u32,
    fb_id: u32,
    map: Mapped,
}

impl DumbBuffer {
    fn create(fd: RawFd, width: u32, height: u32) -> Result<Self, BootError> {
        let mut c = uapi::CreateDumb { height, width, bpp: 32, flags: 0, handle: 0, pitch: 0, size: 0 };
        ioctl(fd, uapi::DRM_IOCTL_MODE_CREATE_DUMB, &mut c)
            .map_err(|e| BootError::Display(format!("create dumb buffer: {e}")))?;

        // AddFB with depth 24 / bpp 32 == DRM_FORMAT_XRGB8888.
        let mut fb = uapi::FbCmd { fb_id: 0, width, height, pitch: c.pitch, bpp: 32, depth: 24, handle: c.handle };
        if let Err(e) = ioctl(fd, uapi::DRM_IOCTL_MODE_ADDFB, &mut fb) {
            let mut d = uapi::DestroyDumb { handle: c.handle };
            let _ = ioctl(fd, uapi::DRM_IOCTL_MODE_DESTROY_DUMB, &mut d);
            return Err(BootError::Display(format!("addfb: {e}")));
        }

        let map = Mapped::map(fd, c.handle, c.size as usize)?;
        // Dumb-buffer contents are undefined: clear so the very first scanout
        // never shows garbage.
        for b in map.as_mut() { *b = 0; }

        Ok(DumbBuffer { width, height, pitch: c.pitch, handle: c.handle, fb_id: fb.fb_id, map })
    }

    fn destroy(&mut self, fd: RawFd) {
        self.map.unmap();
        let mut fb = self.fb_id;
        let _ = ioctl(fd, uapi::DRM_IOCTL_MODE_RMFB, &mut fb);
        let mut d = uapi::DestroyDumb { handle: self.handle };
        let _ = ioctl(fd, uapi::DRM_IOCTL_MODE_DESTROY_DUMB, &mut d);
        self.fb_id = 0;
        self.handle = 0;
    }
}

// ---------------------------------------------------------------------------
// DrmDisplay
// ---------------------------------------------------------------------------

fn set_crtc(fd: RawFd, crtc_id: u32, connector_id: u32, fb_id: u32, mode: &Mode)
    -> Result<(), BootError>
{
    let mut connectors = [connector_id];
    let mut sc = uapi::SetCrtc {
        set_connectors_ptr: connectors.as_mut_ptr() as u64,
        count_connectors: 1,
        crtc_id,
        fb_id,
        x: 0,
        y: 0,
        gamma_size: 0,
        mode_valid: 1,
        mode: mode.info,
    };
    ioctl(fd, uapi::DRM_IOCTL_MODE_SETCRTC, &mut sc)
        .map_err(|e| BootError::Display(format!("SET_CRTC: {e}")))
}

fn find_card(cfg: &Config) -> Option<PathBuf> {
    if let Some(p) = &cfg.display.card {
        return if p.exists() { Some(p.clone()) } else { None };
    }
    (0..16).map(|i| PathBuf::from(format!("/dev/dri/card{i}"))).find(|p| p.exists())
}

pub struct DrmDisplay {
    /// Owns the fd (opened O_CLOEXEC, closed on drop).
    file: Option<File>,
    fd: RawFd,
    crtc_id: u32,
    mode: Mode,
    bufs: [Option<DumbBuffer>; 2],
    /// Index of the buffer currently scanned out; the other one is drawn into.
    front: usize,
    released: bool,
}

impl DrmDisplay {
    pub fn open(cfg: &Config) -> Result<Self, BootError> {
        let path = find_card(cfg)
            .ok_or_else(|| BootError::Display("no DRM device available".into()))?;

        // O_CLOEXEC at open time: the fd must never leak into mitos-init.
        let c_path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| BootError::Display("card path contains NUL".into()))?;
        let fd = unsafe {
            libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC | libc::O_NONBLOCK)
        };
        if fd < 0 {
            return Err(BootError::Display(format!(
                "open {}: {}", path.display(), std::io::Error::last_os_error())));
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let fd = file.as_raw_fd();

        ioctl_noarg(fd, uapi::DRM_IOCTL_SET_MASTER)
            .map_err(|e| BootError::Display(format!("DRM_SET_MASTER failed ({e}); are we root?")))?;

        let res = get_resources(fd)?;
        let conn = connector::find_connected(fd, &res.connector_ids)?
            .ok_or_else(|| BootError::Display("no connected display found".into()))?;
        log::debug!("DRM: connector {} ({}) connected",
                    conn.id, connector::type_name(conn.connector_type));

        let mode = mode::select(&conn.modes, cfg.display.max_width, cfg.display.max_height)
            .ok_or_else(|| BootError::Display("no usable display mode".into()))?;
        let crtc_id = connector::resolve_crtc(fd, &conn, &res.crtc_ids)?
            .ok_or_else(|| BootError::Display("no CRTC available for connector".into()))?;

        let bufs = [
            Some(DumbBuffer::create(fd, mode.width(), mode.height())?),
            Some(DumbBuffer::create(fd, mode.width(), mode.height())?),
        ];
        set_crtc(fd, crtc_id, conn.id, bufs[0].as_ref().unwrap().fb_id, &mode)?;

        log::info!("DRM: {}x{} @ {} Hz on {} [{}]",
                   mode.width(), mode.height(), mode.refresh(),
                   path.display(), connector::type_name(conn.connector_type));

        Ok(DrmDisplay { file: Some(file), fd, crtc_id, mode, bufs, front: 0, released: false })
    }
}

impl Display for DrmDisplay {
    fn size(&self) -> (u32, u32) { (self.mode.width(), self.mode.height()) }
    fn backend_name(&self) -> &'static str { "drm" }
    /// present() waits for the flip-complete (vblank) event.
    fn blocks_on_vsync(&self) -> bool { true }

    fn back_buffer(&mut self) -> PixelBuffer<'_> {
        let idx = self.front ^ 1;
        let b = self.bufs[idx].as_mut().expect("back buffer");
        PixelBuffer::from_parts(b.map.as_mut(), b.width, b.height, b.pitch as usize)
    }

    fn present(&mut self) -> Result<(), BootError> {
        if self.released || self.file.is_none() { return Ok(()); }
        let back = self.front ^ 1;
        let Some(bb) = self.bufs[back].as_ref() else { return Ok(()); };
        let fb_id = bb.fb_id;

        // A failed flip is a visual problem, never a boot problem: retry a
        // few times, then drop this frame and carry on.
        let mut submitted = false;
        for _ in 0..3 {
            match page_flip::submit(self.fd, self.crtc_id, fb_id) {
                Ok(()) => { submitted = true; break; }
                Err(e) => {
                    log::debug!("page flip retry: {e}");
                    std::thread::sleep(Duration::from_millis(4));
                }
            }
        }
        if !submitted {
            log::error!("page flip failed; skipping frame");
            return Ok(());
        }

        match page_flip::wait_complete(self.fd, 100) {
            Ok(true) => {}
            Ok(false) => log::debug!("vsync event timeout; continuing"),
            Err(e) => log::warn!("vsync wait error: {e}"),
        }
        self.front = back;
        Ok(())
    }

    fn release(&mut self) {
        if self.released { return; }
        self.released = true;
        for b in self.bufs.iter_mut() {
            if let Some(mut b) = b.take() { b.destroy(self.fd); }
        }
        let _ = ioctl_noarg(self.fd, uapi::DRM_IOCTL_DROP_MASTER);
        if let Some(f) = self.file.take() { drop(f); } // closes the fd
    }
}

impl Drop for DrmDisplay { fn drop(&mut self) { self.release(); } }