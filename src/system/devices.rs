//! Required-device presence checks.

use std::path::Path;

pub fn present(path: &str) -> bool { Path::new(path).exists() }

pub fn count_present(paths: &[String]) -> usize {
    paths.iter().filter(|p| present(p)).count()
}