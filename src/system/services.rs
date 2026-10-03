//! Required file / service marker checks. mitos-init signals full readiness
//! by creating the init-ready marker (and/or via IPC).

use std::path::Path;

pub fn present(path: &str) -> bool { Path::new(path).exists() }

pub fn count_present(paths: &[String]) -> usize {
    paths.iter().filter(|p| present(p)).count()
}

pub fn init_marker_reached(path: &str) -> bool { present(path) }