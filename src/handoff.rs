//! Ending mitos-boot: stop visuals, release DRM, notify mitos-init, and
//! (optionally) exec the next stage. No shells, no arbitrary code, ever.

use crate::bootlog;
use crate::bootlog::messages as msg;
use crate::config::{Config, HandoffMode};
use crate::error::BootError;
use crate::ipc::{IpcServer, Message};
use crate::state::{BootStage, SharedState};
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;

pub fn perform(cfg: &Config, ipc: &mut Option<IpcServer>, state: &SharedState) -> Result<(), BootError> {
    state.set_stage(BootStage::Handoff);
    bootlog::ok(msg::M_HANDOFF);

    if let Some(s) = ipc.as_ref() {
        s.send(&Message::BootReady);
        s.send(&Message::Handoff);
    }
    if let Some(s) = ipc.take() { s.shutdown(); } // unlinks the socket

    match cfg.handoff.mode {
        HandoffMode::Notify => {
            log::info!("handoff complete (notify mode); mitos-boot exiting");
            Ok(())
        }
        HandoffMode::Exec => {
            let path = validate_program(&cfg.system.init_path)?;
            if unsafe { libc::getpid() } == 1 {
                // PID 1 may never exit: replace ourselves with mitos-init.
                exec_replace(&path, &[], &[("MITOS_BOOT", "1"), ("MITOS_BOOT_STAGE", "handoff")])?;
                Err(BootError::Handoff("execve returned unexpectedly".into()))
            } else {
                let status = std::process::Command::new(&path)
                    .env("MITOS_BOOT", "1")
                    .env("MITOS_BOOT_STAGE", "handoff")
                    .status()
                    .map_err(|e| BootError::Handoff(format!("spawn mitos-init: {e}")))?;
                std::process::exit(status.code().unwrap_or(1));
            }
        }
    }
}

/// Validate a program we are allowed to exec: absolute, exists, regular file,
/// executable, and not world-writable (tamper resistance for early boot).
pub(crate) fn validate_program(path: &Path) -> Result<PathBuf, BootError> {
    let p = PathBuf::from(path);
    if !p.is_absolute() {
        return Err(BootError::Handoff(format!("{}: must be an absolute path", p.display())));
    }
    let canon = std::fs::canonicalize(&p)
        .map_err(|_| BootError::Handoff(format!("{}: not found", p.display())))?;
    let md = std::fs::metadata(&canon)
        .map_err(|e| BootError::Handoff(format!("{}: {e}", canon.display())))?;
    if !md.is_file() {
        return Err(BootError::Handoff(format!("{}: not a regular file", canon.display())));
    }
    use std::os::unix::fs::PermissionsExt;
    let mode = md.permissions().mode();
    if mode & 0o111 == 0 {
        return Err(BootError::Handoff(format!("{}: not executable", canon.display())));
    }
    if mode & 0o002 != 0 {
        return Err(BootError::Handoff(format!("{}: world-writable, refusing", canon.display())));
    }
    Ok(canon)
}

/// execve(2) with a clean environment and signal state. Never returns on
/// success. Used only with paths that passed `validate_program`.
pub(crate) fn exec_replace(
    program: &Path,
    args: &[&str],
    env_extra: &[(&str, &str)],
) -> Result<(), BootError> {
    let cstr = |b: &[u8]| CString::new(b).map_err(|_| BootError::Handoff("argument contains NUL".into()))?;
    let prog = cstr(program.as_os_str().as_bytes())?;

    let mut argv: Vec<CString> = vec![prog.clone()];
    for a in args { argv.push(cstr(a.as_bytes())?); }

    let mut envp: Vec<CString> = Vec::new();
    for (k, v) in std::env::vars_os() {
        let mut b = k.into_vec();
        b.push(b'=');
        b.extend_from_slice(v.as_bytes());
        if let Ok(c) = cstr(&b) { envp.push(c); }
    }
    for (k, v) in env_extra { envp.push(cstr(format!("{k}={v}").as_bytes())?); }

    let mut argv_p: Vec<*const libc::c_char> = argv.iter().map(|c| c.as_ptr()).collect();
    argv_p.push(ptr::null());
    let mut envp_p: Vec<*const libc::c_char> = envp.iter().map(|c| c.as_ptr()).collect();
    envp_p.push(ptr::null());

    unsafe {
        // Clean signal state for the new program.
        let empty: libc::sigset_t = std::mem::zeroed();
        libc::sigprocmask(libc::SIG_SETMASK, &empty, ptr::null_mut());
        libc::execve(prog.as_ptr(), argv_p.as_ptr(), envp_p.as_ptr());
    }
    Err(BootError::Handoff(format!(
        "execve {}: {}", program.display(), std::io::Error::last_os_error())))
}