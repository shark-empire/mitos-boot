//! Linux fbdev fallback backend.
//!
//! Single-buffered: rendering happens into a shadow buffer in the canonical
//! XRGB8888 layout and is converted to the panel's actual layout in
//! present(). Deliberately simple — this is the "works on anything" path.

use super::Display;
use super::drm::ioctl; // shared, audited ioctl helper (both backends always compile)
use crate::config::Config;
use crate::error::BootError;
use crate::renderer::surface::PixelBuffer;
use std::fs::File;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::path::PathBuf;

const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
const FBIOGET_FSCREENINFO: libc::c_ulong = 0x4601;

// --- fbdev UAPI structs (must match include/uapi/linux/fb.h) ---------------

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct FbBitfield { offset: u32, length: u32, msb_right: u32 }

#[repr(C)]
#[derive(Default)]
struct FbVarScreenInfo {
    xres: u32, yres: u32, xres_virtual: u32, yres_virtual: u32,
    xoffset: u32, yoffset: u32, bits_per_pixel: u32, grayscale: u32,
    red: FbBitfield, green: FbBitfield, blue: FbBitfield, transp: FbBitfield,
    nonstd: u32, activate: u32, height: u32, width: u32, accel_flags: u32,
    pixclock: u32, left_margin: u32, right_margin: u32, upper_margin: u32, lower_margin: u32,
    hsync_len: u32, vsync_len: u32, sync: u32, vmode: u32, rotate: u32, colorspace: u32,
    reserved: [u32; 4],
}

#[repr(C)]
struct FbFixScreenInfo {
    id: [u8; 16],
    smem_start: libc::c_ulong,
    smem_len: u32,
    type_: u32, type_aux: u32, visual: u32,
    xpanstep: u16, ypanstep: u16, ywrapstep: u16,
    line_length: u32,
    mmio_start: libc::c_ulong,
    mmio_len: u32, accel: u32,
    capabilities: u16,
    reserved: [u16; 2],
}

// --- formats ---------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum FbFormat { Xrgb8888, Xbgr8888, Rgb888, Bgr888, Rgb565 }

impl FbFormat {
    fn bpp_bytes(self) -> usize {
        match self { Self::Xrgb8888 | Self::Xbgr8888 => 4, Self::Rgb888 | Self::Bgr888 => 3, Self::Rgb565 => 2 }
    }
}

fn detect_format(var: &FbVarScreenInfo) -> Result<FbFormat, BootError> {
    let (bpp, r, g, b) = (var.bits_per_pixel, &var.red, &var.green, &var.blue);
    Ok(match (bpp, r.length, r.offset, g.length, g.offset, b.length, b.offset) {
        (32, 8, 16, 8, 8, 8, 0)  => FbFormat::Xrgb8888,
        (32, 8, 0, 8, 8, 8, 16)  => FbFormat::Xbgr8888,
        (24, 8, 16, 8, 8, 8, 0)  => FbFormat::Rgb888,
        (24, 8, 0, 8, 8, 8, 16)  => FbFormat::Bgr888,
        (16, 5, 11, 6, 5, 5, 0)  => FbFormat::Rgb565,
        _ => return Err(BootError::Display(format!("unsupported fbdev format {bpp}bpp"))),
    })
}

fn write_pixel(dst: &mut [u8], off: usize, v: u32, f: FbFormat) {
    // v is 0xAARRGGBB
    let (r, g, b) = (((v >> 16) & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8);
    let n = f.bpp_bytes();
    if off + n > dst.len() { return; } // defensive: never trust line_length math
    match f {
        FbFormat::Xbgr8888 => dst[off..off + 4].copy_from_slice(&[b, g, r, 0]),
        FbFormat::Rgb888   => dst[off..off + 3].copy_from_slice(&[b, g, r]),
        FbFormat::Bgr888   => dst[off..off + 3].copy_from_slice(&[r, g, b]),
        FbFormat::Rgb565 => {
            let p = (((r as u16) >> 3) << 11) | (((g as u16) >> 2) << 5) | ((b as u16) >> 3);
            dst[off..off + 2].copy_from_slice(&p.to_le_bytes());
        }
        FbFormat::Xrgb8888 => unreachable!("handled by the fast path"),
    }
}

// --- mmap wrapper -----------------------------------------------------------

struct Mmap { ptr: *mut libc::c_void, len: usize }
// SAFETY: process-wide mapping, moved between threads, never shared concurrently.
unsafe impl Send for Mmap {}

impl Mmap {
    fn new(file: &File, len: usize) -> Result<Self, BootError> {
        // SAFETY: standard shared mmap of the fbdev device; unmap in Drop.
        let ptr = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE,
                       libc::MAP_SHARED, file.as_raw_fd(), 0)
        };
        if ptr == libc::MAP_FAILED {
            return Err(BootError::Display(format!("mmap fb: {}", std::io::Error::last_os_error())));
        }
        Ok(Mmap { ptr, len })
    }
    fn as_mut(&mut self) -> &mut [u8] {
        // SAFETY: valid for `len` bytes until munmap.
        unsafe { std::slice::from_raw_parts_mut(self.ptr as *mut u8, self.len) }
    }
}
impl Drop for Mmap {
    fn drop(&mut self) {
        if !self.ptr.is_null() { unsafe { libc::munmap(self.ptr, self.len); } }
    }
}

// --- FbdevDisplay -----------------------------------------------------------

pub struct FbdevDisplay {
    file: Option<File>,
    map: Option<Mmap>,
    shadow: Vec<u8>,
    width: u32,
    height: u32,
    fb_stride: usize,
    format: FbFormat,
    released: bool,
}

impl FbdevDisplay {
    pub fn open(cfg: &Config) -> Result<Self, BootError> {
        let path = cfg.display.fb_device.clone().unwrap_or_else(|| PathBuf::from("/dev/fb0"));
        if !path.exists() {
            return Err(BootError::Display(format!("{}: not found", path.display())));
        }

        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_| BootError::Display("fb path contains NUL".into()))?;
        let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(BootError::Display(format!(
                "open {}: {}", path.display(), std::io::Error::last_os_error())));
        }
        let file = unsafe { File::from_raw_fd(fd) };

        let mut var = FbVarScreenInfo::default();
        ioctl(fd, FBIOGET_VSCREENINFO, &mut var)
            .map_err(|e| BootError::Display(format!("VSCREENINFO: {e}")))?;
        let mut fix: FbFixScreenInfo = unsafe { std::mem::zeroed() };
        ioctl(fd, FBIOGET_FSCREENINFO, &mut fix)
            .map_err(|e| BootError::Display(format!("FSCREENINFO: {e}")))?;

        let (w, h) = (var.xres, var.yres);
        if w == 0 || h == 0 || w > cfg.display.max_width || h > cfg.display.max_height {
            return Err(BootError::Display(format!("unsupported fbdev mode {w}x{h}")));
        }
        let format = detect_format(&var)?;
        let fb_stride = fix.line_length as usize;
        let needed = fb_stride.checked_mul(h as usize)
            .ok_or_else(|| BootError::Display("fbdev stride overflow".into()))?;
        if (fix.smem_len as usize) < needed {
            return Err(BootError::Display("fbdev memory smaller than mode".into()));
        }

        let map = Mmap::new(&file, fix.smem_len as usize)?;
        let shadow = vec![0u8; w as usize * 4 * h as usize];

        log::info!("fbdev: {w}x{h} {}bpp stride {fb_stride} on {}", var.bits_per_pixel, path.display());
        Ok(FbdevDisplay { file: Some(file), map: Some(map), shadow, width: w, height: h,
                          fb_stride, format, released: false })
    }
}

impl Display for FbdevDisplay {
    fn size(&self) -> (u32, u32) { (self.width, self.height) }
    fn backend_name(&self) -> &'static str { "fbdev" }
    /// No vsync events on fbdev: boot.rs paces us with the software ticker.
    fn blocks_on_vsync(&self) -> bool { false }

    fn back_buffer(&mut self) -> PixelBuffer<'_> {
        PixelBuffer::from_parts(&mut self.shadow, self.width, self.height, self.width as usize * 4)
    }

    fn present(&mut self) -> Result<(), BootError> {
        if self.released { return Ok(()); }
        let Some(map) = self.map.as_mut() else { return Ok(()); };
        let dst = map.as_mut();
        let w = self.width as usize;
        let h = self.height as usize;
        let src = &self.shadow;
        let src_stride = w * 4;

        if self.format == FbFormat::Xrgb8888 {
            // Fast path: identical memory layout, row-by-row copy.
            for y in 0..h {
                let s = &src[y * src_stride .. y * src_stride + src_stride];
                let d = &mut dst[y * self.fb_stride .. y * self.fb_stride + src_stride];
                d.copy_from_slice(s);
            }
        } else {
            let n = self.format.bpp_bytes();
            for y in 0..h {
                let row_off = y * self.fb_stride;
                for x in 0..w {
                    let v = u32::from_le_bytes(src[y * src_stride + x * 4 ..][..4].try_into().unwrap());
                    write_pixel(dst, row_off + x * n, v, self.format);
                }
            }
        }
        Ok(())
    }

    fn release(&mut self) {
        if self.released { return; }
        self.released = true;
        self.map.take();  // unmaps
        self.file.take(); // closes
    }
}

impl Drop for FbdevDisplay { fn drop(&mut self) { self.release(); } }