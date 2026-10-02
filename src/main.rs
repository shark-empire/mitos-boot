//! mitos-boot — MITOS early boot & visual startup system.
//!
//! Responsibilities (and nothing more):
//!   initialize logging, load configuration, initialize display/renderer/
//!   animation, run the boot sequence, wait for system readiness, hand off
//!   to mitos-init. Login, session and desktop logic live elsewhere.

mod animation;
mod boot;
mod bootlog;
mod config;
mod display;
mod error;
mod handoff;
mod ipc;
mod renderer;
mod splash;
mod state;
mod system;
mod video;

use std::path::PathBuf;
use std::process::ExitCode;

struct Options {
    config: Option<PathBuf>,
    debug: bool,
    no_splash: bool,
    recovery: bool,
    version: bool,
    help: bool,
}

fn parse_args() -> Options {
    let mut o = Options { config: None, debug: false, no_splash: false, recovery: false, version: false, help: false };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let (key, inline) = match a.split_once('=') { Some((k, v)) => (k.to_string(), Some(v.to_string())), None => (a.clone(), None) };
        match key.as_str() {
            "--config" | "-c" => o.config = inline.map(PathBuf::from).or_else(|| it.next().map(PathBuf::from)),
            "--debug" | "-d" => o.debug = true,
            "--no-splash" => o.no_splash = true,
            "--recovery" => o.recovery = true,
            "--version" | "-V" => o.version = true,
            "--help" | "-h" => o.help = true,
            _ => {}
        }
    }
    o
}

fn install_panic_hook() {
    // Panics must be visible on early-boot systems where stderr may be gone.
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        default(info);
    }));
}

fn main() -> ExitCode {
    let opts = parse_args();
    if opts.help {
        eprintln!("mitos-boot {}\n\n\
            --config PATH   configuration file (default /etc/mitos/boot.toml)\n\
            --no-splash     skip the visual boot, wait for readiness only\n\
            --debug         show boot messages on screen, verbose logging\n\
            --recovery      enter recovery immediately\n\
            --version       print version", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if opts.version {
        println!("mitos-boot {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    // Earliest possible logging (kernel log + inherited stdio).
    bootlog::init(log::LevelFilter::Info);
    install_panic_hook();

    let cfg = match config::load(opts.config.as_deref()) {
        Ok(mut c) => {
            if opts.no_splash { c.splash.enabled = false; }
            if opts.debug { c.debug.show_boot_messages = true; c.debug.log_level = "debug".into(); }
            config::apply_cmdline_overrides(&mut c);
            c
        }
        Err(e) => {
            log::error!("fatal configuration error: {e}");
            return ExitCode::from(78); // EX_CONFIG
        }
    };
    bootlog::init(config::parse_level(&cfg.debug.log_level).unwrap_or(log::LevelFilter::Info));

    if unsafe { libc::geteuid() } != 0 {
        log::warn!("not running as root: DRM master, IPC and handoff may fail");
    }
    state::install_signal_handlers();

    if opts.recovery || cfg.forced_recovery {
        return error::recovery::handle_fatal(
            &error::BootError::System("recovery mode requested".into()), &cfg);
    }

    let state = state::SharedState::new();
    match boot::run(&cfg, &state) {
        Ok(()) => { log::info!("mitos-boot: boot complete"); ExitCode::SUCCESS }
        Err(e) => error::recovery::handle_fatal(&e, &cfg),
    }
}