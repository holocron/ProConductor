//! Per-OS glue. The whole `windows`/`macos` modules are cfg-gated; everything
//! cross-platform (kill functions) lives in `process.rs`.

#[cfg(windows)]
pub(crate) mod windows;
#[cfg(target_os = "macos")]
pub(crate) mod macos;

use std::process::Command;
use std::path::PathBuf;

pub(crate) fn open_path(path: &str) {
    #[cfg(target_os = "windows")] { let _ = Command::new("explorer").arg(path).spawn(); }
    #[cfg(target_os = "macos")]   { let _ = Command::new("open").arg(path).spawn(); }
    #[cfg(target_os = "linux")]   { let _ = Command::new("xdg-open").arg(path).spawn(); }
}

pub(crate) fn reveal_path(path: &str) {
    let p = PathBuf::from(path);
    let dir = if p.is_file() { p.parent().unwrap_or(&p).to_string_lossy().to_string() } else { path.to_string() };
    open_path(&dir);
}
