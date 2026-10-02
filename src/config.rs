//! Configuration loading, validation and hardening.
//!
//! All values are clamped or rejected before use; asset paths are validated
//! against an allowlist of roots at open time (see `validate_asset_path`).

use crate::error::BootError;
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const DEFAULT_CONFIG: &str = "/etc/mitos/boot.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RendererBackend { Auto, Drm, Fbdev, Disabled }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HandoffMode { Notify, Exec }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecoveryChoice { Recovery, Console, Reboot, PowerOff }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundStyle { Solid, Gradient }

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    pub boot: BootSection,
    pub splash: SplashSection,
    pub background: BackgroundSection,
    pub animation: AnimationSection,
    pub renderer: RendererSection,
    pub display: DisplaySection,
    pub video: VideoSection,
    pub system: SystemSection,
    pub ipc: IpcSection,
    pub handoff: HandoffSection,
    pub debug: DebugSection,
    pub reliability: ReliabilitySection,
    pub recovery: RecoverySection,
    pub security: SecuritySection,
    #[serde(skip)]
    pub forced_recovery: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BootSection { pub enabled: bool }
impl Default for BootSection { fn default() -> Self { Self { enabled: true } } }

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SplashSection {
    pub enabled: bool,
    pub owl_video: bool,
    pub owl_video_path: PathBuf,
    pub owl_static_fallback: Option<PathBuf>,
    pub owl_scale: f32,          // fraction of min(screen w,h)
    pub owl_center_y: f32,       // 0.0 = top, 1.0 = bottom
    pub wordmark: bool,
    pub wordmark_text: String,
    pub wordmark_font: PathBuf,
    pub font_fallbacks: Vec<PathBuf>,
    pub wordmark_size_px: u32,   // reference size at 1080p; scaled to panel
    pub wordmark_center_y: f32,
    pub wordmark_glow: bool,
    pub glow_color: [u8; 3],
    pub show_progress: bool,
}
impl Default for SplashSection {
    fn default() -> Self {
        Self {
            enabled: true,
            owl_video: true,
            owl_video_path: PathBuf::from("/usr/share/mitos/boot/owl.webm"),
            owl_static_fallback: Some(PathBuf::from("/usr/share/mitos/boot/owl.png")),
            owl_scale: 0.6,
            owl_center_y: 0.44,
            wordmark: true,
            wordmark_text: "MĨȚǑŠ".to_string(),
            wordmark_font: PathBuf::from("/usr/share/mitos/fonts/MitosDisplay-Regular.ttf"),
            font_fallbacks: vec![
                PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
                PathBuf::from("/usr/share/fonts/TTF/DejaVuSans-Bold.ttf"),
                PathBuf::from("/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf"),
            ],
            wordmark_size_px: 96,
            wordmark_center_y: 0.46,
            wordmark_glow: true,
            glow_color: [122, 168, 255],
            show_progress: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BackgroundSection {
    pub style: BackgroundStyle,
    pub color: [u8; 3],
    pub gradient_top: [u8; 3],
    pub gradient_bottom: [u8; 3],
}
impl Default for BackgroundSection {
    fn default() -> Self {
        Self { style: BackgroundStyle::Solid, color: [0, 0, 0],
               gradient_top: [10, 12, 20], gradient_bottom: [0, 0, 0] }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct AnimationSection {
    pub fade_in_ms: u32,
    pub fade_out_ms: u32,
    pub wordmark_in_ms: u32,
    pub wordmark_out_ms: u32,
    pub wordmark_hold_ms: u32,
    pub minimum_duration_ms: u32,
    pub fast_when_ready: bool,
}
impl Default for AnimationSection {
    fn default() -> Self {
        Self { fade_in_ms: 400, fade_out_ms: 500, wordmark_in_ms: 500,
               wordmark_out_ms: 500, wordmark_hold_ms: 600,
               minimum_duration_ms: 1800, fast_when_ready: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RendererSection { pub backend: RendererBackend }
impl Default for RendererSection { fn default() -> Self { Self { backend: RendererBackend::Auto } } }

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DisplaySection {
    pub card: Option<PathBuf>,   // None = scan /dev/dri/card0..N
    pub fb_device: Option<PathBuf>,
    pub max_width: u32,
    pub max_height: u32,
}
impl Default for DisplaySection {
    fn default() -> Self { Self { card: None, fb_device: None, max_width: 3840, max_height: 2160 } }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct VideoSection {
    pub max_asset_bytes: u64,
    pub max_decode_per_update_ms: u32,
    pub default_fps: u32,
}
impl Default for VideoSection {
    fn default() -> Self { Self { max_asset_bytes: 512 << 20, max_decode_per_update_ms: 40, default_fps: 30 } }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SystemSection {
    pub init_path: PathBuf,
    pub require_mounts: Vec<String>,
    pub require_devices: Vec<String>,
    pub require_files: Vec<String>,
    pub init_ready_marker: String,
    pub readiness_timeout_ms: u32, // 0 = wait indefinitely (watchdog still applies)
}
impl Default for SystemSection {
    fn default() -> Self {
        Self {
            init_path: PathBuf::from("/usr/lib/mitos/mitos-init"),
            require_mounts: vec!["/proc".into(), "/sys".into(), "/dev".into(), "/run".into()],
            require_devices: vec![],
            require_files: vec![],
            init_ready_marker: "/run/mitos/system-ready".into(),
            readiness_timeout_ms: 30_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct IpcSection { pub enabled: bool, pub socket: String }
impl Default for IpcSection {
    fn default() -> Self { Self { enabled: true, socket: "/run/mitos/boot.sock".into() } }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct HandoffSection { pub mode: HandoffMode }
impl Default for HandoffSection { fn default() -> Self { Self { mode: HandoffMode::Notify } } }

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DebugSection { pub show_boot_messages: bool, pub log_level: String }
impl Default for DebugSection {
    fn default() -> Self { Self { show_boot_messages: false, log_level: "info".into() } }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ReliabilitySection { pub watchdog_ms: u64 } // 0 = disabled
impl Default for ReliabilitySection { fn default() -> Self { Self { watchdog_ms: 120_000 } } }

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RecoverySection {
    pub timeout_s: u32,
    pub default_action: RecoveryChoice,
    pub recovery_path: PathBuf,
    pub console: PathBuf,
}
impl Default for RecoverySection {
    fn default() -> Self {
        Self { timeout_s: 30, default_action: RecoveryChoice::Console,
               recovery_path: PathBuf::from("/usr/lib/mitos/mitos-recovery"),
               console: PathBuf::from("/bin/sh") }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SecuritySection {
    pub allowed_asset_roots: Vec<String>,
    pub allow_any_path: bool, // development escape hatch; logs a warning
}
impl Default for SecuritySection {
    fn default() -> Self {
        Self {
            allowed_asset_roots: vec![
                "/usr/share/mitos".into(), "/etc/mitos".into(),
                "/usr/lib/mitos".into(), "/run/mitos".into(), "/usr/share/fonts".into(),
            ],
            allow_any_path: false,
        }
    }
}

impl Config {
    /// Clamp / reject everything. Boot software must fail safely, so numeric
    /// nonsense is clamped rather than trusted; structural errors are fatal.
    pub fn validate(&mut self) -> Result<(), BootError> {
        let e = |m: &str| BootError::Config(m.to_string());
        let clampu = |v: &mut u32, lo: u32, hi: u32| *v = (*v).clamp(lo, hi);
        let clampf = |v: &mut f32, lo: f32, hi: f32| *v = (*v).clamp(lo, hi);

        clampu(&mut self.animation.fade_in_ms, 0, 10_000);
        clampu(&mut self.animation.fade_out_ms, 0, 10_000);
        clampu(&mut self.animation.wordmark_in_ms, 0, 10_000);
        clampu(&mut self.animation.wordmark_out_ms, 0, 10_000);
        clampu(&mut self.animation.wordmark_hold_ms, 0, 60_000);
        clampu(&mut self.animation.minimum_duration_ms, 0, 120_000);

        clampf(&mut self.splash.owl_scale, 0.05, 1.0);
        clampf(&mut self.splash.owl_center_y, 0.0, 1.0);
        clampf(&mut self.splash.wordmark_center_y, 0.0, 1.0);
        clampu(&mut self.splash.wordmark_size_px, 24, 512);
        if self.splash.wordmark_text.chars().count() > 32 { return Err(e("wordmark_text too long")); }

        clampu(&mut self.video.max_decode_per_update_ms, 5, 500);
        clampu(&mut self.video.default_fps, 1, 240);
        if self.video.max_asset_bytes == 0 || self.video.max_asset_bytes > (4u64 << 30) {
            return Err(e("video.max_asset_bytes out of range"));
        }

        clampu(&mut self.display.max_width, 640, 16_384);
        clampu(&mut self.display.max_height, 640, 16_384);

        clampu(&mut self.system.readiness_timeout_ms, 0, 600_000);
        clampu(&mut self.reliability.watchdog_ms, 0, 600_000);
        clampu(&mut self.recovery.timeout_s, 0, 600);

        for p in [&self.system.init_path, &self.recovery.recovery_path, &self.recovery.console] {
            if !p.is_absolute() { return Err(e(&format!("{}: must be absolute", p.display()))); }
        }
        if !Path::new(&self.ipc.socket).is_absolute() { return Err(e("ipc.socket must be absolute")); }
        if !self.ipc.socket.starts_with("/run/") {
            return Err(e("ipc.socket must live under /run"));
        }
        for r in &self.security.allowed_asset_roots {
            if !Path::new(r).is_absolute() { return Err(e("allowed_asset_roots must be absolute")); }
        }
        if self.security.allow_any_path {
            log::warn!("security.allow_any_path is enabled — asset allowlisting disabled");
        }
        if parse_level(&self.debug.log_level).is_none() {
            return Err(e("debug.log_level must be error|warn|info|debug|trace"));
        }
        Ok(())
    }
}

pub fn parse_level(s: &str) -> Option<log::LevelFilter> {
    match s {
        "error" => Some(log::LevelFilter::Error), "warn" | "warning" => Some(log::LevelFilter::Warn),
        "info" => Some(log::LevelFilter::Info), "debug" => Some(log::LevelFilter::Debug),
        "trace" => Some(log::LevelFilter::Trace), _ => None,
    }
}

fn from_file(p: &Path) -> Result<Config, BootError> {
    let meta = std::fs::metadata(p).map_err(|e| BootError::Config(format!("{}: {e}", p.display())))?;
    if meta.len() > 1 << 20 { return Err(BootError::Config("config file too large".into())); }
    let s = std::fs::read_to_string(p).map_err(|e| BootError::Config(format!("{}: {e}", p.display())))?;
    toml::from_str(&s).map_err(|e| BootError::Config(format!("parse {}: {e}", p.display())))
}

pub fn load(path: Option<&Path>) -> Result<Config, BootError> {
    let mut cfg = match path {
        Some(p) => from_file(p)?,
        None => {
            let def = Path::new(DEFAULT_CONFIG);
            if def.exists() { from_file(def)? } else { Config::default() }
        }
    };
    cfg.validate()?;
    Ok(cfg)
}

/// Kernel command line overrides: mitos-boot.splash=0, mitos-boot.debug=1,
/// mitos-boot.log=debug, mitos-boot.recovery=1, mitos-boot.headless=1.
pub fn apply_cmdline_overrides(cfg: &mut Config) {
    let Ok(line) = std::fs::read_to_string("/proc/cmdline") else { return };
    if line.len() > 8192 { return; }
    for tok in line.split_whitespace() {
        let Some(kv) = tok.strip_prefix("mitos-boot.") else { continue };
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        match k {
            "splash" => cfg.splash.enabled = v != "0",
            "debug" => { cfg.debug.show_boot_messages = v != "0"; cfg.debug.log_level = "debug".into(); }
            "log" => { if parse_level(v).is_some() { cfg.debug.log_level = v.to_string(); } }
            "recovery" => if v != "0" { cfg.forced_recovery = true; },
            "headless" => if v != "0" { cfg.splash.enabled = false; },
            _ => {}
        }
    }
}

/// Validate an asset path: must exist, be absolute, and (unless explicitly
/// disabled) resolve — through symlinks — inside an allowed root.
pub fn validate_asset_path(sec: &SecuritySection, path: &Path) -> Result<PathBuf, BootError> {
    if !path.is_absolute() {
        return Err(BootError::Asset(format!("{}: asset paths must be absolute", path.display())));
    }
    let canon = std::fs::canonicalize(path)
        .map_err(|_| BootError::Asset(format!("{}: asset not found", path.display())))?;
    if !canon.is_file() {
        return Err(BootError::Asset(format!("{}: not a regular file", path.display())));
    }
    if sec.allow_any_path { return Ok(canon); }
    if sec.allowed_asset_roots.iter().any(|r| canon.starts_with(Path::new(r))) {
        return Ok(canon);
    }
    Err(BootError::Asset(format!("{}: asset outside allowed roots", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_and_clamp() {
        let src = r#"
[animation]
fade_in_ms = 999999
minimum_duration_ms = 1800
[debug]
log_level = "debug"
"#;
        let mut c: Config = toml::from_str(src).unwrap();
        c.validate().unwrap();
        assert_eq!(c.animation.fade_in_ms, 10_000);
        assert_eq!(c.debug.log_level, "debug");
    }
    #[test]
    fn rejects_bad_level() {
        let mut c = Config::default();
        c.debug.log_level = "loud".into();
        assert!(c.validate().is_err());
    }
}