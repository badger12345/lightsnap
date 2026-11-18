//! Screenshots through macOS's own `screencapture` tool.
//! It runs as a short-lived child process, so the app itself stays small and idle.

use crate::platform::{CaptureKind, temp_file};
use std::path::PathBuf;
use std::process::Command;

/// Runs the capture and blocks until it is done. Returns the image file,
/// or None if the person cancelled (pressed Esc) or capture failed.
pub fn run_capture(kind: CaptureKind, sound: bool) -> Option<PathBuf> {
    let out = temp_file("shot", "png");
    let mut cmd = Command::new("/usr/sbin/screencapture");
    if !sound {
        cmd.arg("-x"); // no shutter sound
    }
    match kind {
        CaptureKind::Area => {
            cmd.arg("-i");
        }
        CaptureKind::Window => {
            cmd.args(["-i", "-w"]);
        }
        CaptureKind::FullScreen => {
            cmd.arg("-m");
        }
        CaptureKind::Timed(s) => {
            cmd.args(["-m", "-T", &s.to_string()]);
        }
    }
    cmd.arg(&out);
    let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
    if ok && out.exists() { Some(out) } else { None }
}

/// Takes one silent picture of every screen (main screen first).
pub fn capture_all_screens(count: usize) -> Vec<PathBuf> {
    let files: Vec<PathBuf> = (0..count.max(1)).map(|i| temp_file(&format!("screen{i}"), "png")).collect();
    let ok = Command::new("/usr/sbin/screencapture")
        .arg("-x")
        .args(&files)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Vec::new();
    }
    files.into_iter().filter(|f| f.exists()).collect()
}

/// Whether captures that don't ask first (full screen, timed) may run. macOS asks by
/// itself when needed, so LightSnap does not have to.
pub fn screenshot_allowed() -> bool {
    true
}

/// Asks for that permission. Nothing to do on macOS.
pub fn ask_screenshot_permission() -> bool {
    true
}
