//! Everything that talks to the operating system: screenshots, the clipboard, the
//! list of screens, opening files, notifications, launch at login, and the parts of
//! screen recording that depend on the system.
//!
//! The rest of the app only calls the functions re-exported here. Only macOS is
//! implemented so far. A port to another system adds `platform/<os>/` with the same
//! public functions and types, and one more `cfg` line below.

use egui::{Pos2, Rect};
use std::path::PathBuf;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(not(target_os = "macos"))]
compile_error!("LightSnap only runs on macOS so far. A port adds src/platform/<os>/ (see src/platform/mod.rs).");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureKind {
    Area,
    Window,
    FullScreen,
    Timed(u32),
}

impl CaptureKind {
    #[allow(dead_code)]
    pub fn label(self) -> String {
        match self {
            CaptureKind::Area => "Capture Area".into(),
            CaptureKind::Window => "Capture Window".into(),
            CaptureKind::FullScreen => "Capture Full Screen".into(),
            CaptureKind::Timed(s) => format!("Timed Capture ({s} s)"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Display {
    /// Global position and size in points (origin at the top-left of the main screen).
    pub bounds: Rect,
    /// Pixels per point (2 on Retina screens).
    pub scale: f32,
}

/// The index of the screen that holds `p` (or the closest one).
pub fn display_index_at(displays: &[Display], p: Pos2) -> usize {
    if let Some(i) = displays.iter().position(|d| d.bounds.contains(p)) {
        return i;
    }
    displays
        .iter()
        .enumerate()
        .min_by(|a, b| {
            let da = a.1.bounds.distance_sq_to_pos(p);
            let db = b.1.bounds.distance_sq_to_pos(p);
            da.total_cmp(&db)
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// A fresh file in the app's own temp folder, like `shot-1731840000000.png`.
pub fn temp_file(prefix: &str, ext: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("lightsnap");
    let _ = std::fs::create_dir_all(&dir);
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    dir.join(format!("{prefix}-{n}.{ext}"))
}

/// Microphones and screens that the screen recorder can use.
#[derive(Clone, Debug, Default)]
pub struct Devices {
    /// Recorder input index of the main screen.
    pub screen_index: Option<usize>,
    /// Recorder input index for every screen N (in the order of `displays()`).
    pub screens: Vec<(usize, usize)>,
    pub audio_inputs: Vec<String>,
}
