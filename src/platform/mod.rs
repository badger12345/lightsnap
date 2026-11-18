//! Everything that talks to the operating system: screenshots, the clipboard, the
//! list of screens, opening files, notifications, launch at login, and the parts of
//! screen recording that depend on the system.
//!
//! The rest of the app only calls the functions re-exported here. Each system has its
//! own `platform/<os>/` with the same public functions and types: macOS, and Linux
//! (Wayland only).

use egui::{Pos2, Rect};
use std::path::PathBuf;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("LightSnap runs on macOS and Linux. A port adds src/platform/<os>/ (see src/platform/mod.rs).");

/// The app id. On Linux it names the .desktop file and is how the portals remember
/// what was allowed.
pub const APP_ID: &str = "io.github.badger12345.LightSnap";

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

/// One entry of the tray menu. The app builds the menu from these, each system shows it
/// its own way.
#[derive(Clone, Debug)]
pub enum MenuEntry {
    Item {
        /// Sent back to the app when the item is chosen. Empty for an item that does nothing.
        id: String,
        text: String,
        enabled: bool,
        /// Shortcut shown next to the item.
        key: Option<crate::settings::Hotkey>,
    },
    Separator,
    Submenu { text: String, items: Vec<MenuEntry> },
}

/// Called with the id of a chosen menu item or a pressed global shortcut. It runs on a
/// system thread, so it only hands the id on.
pub type Sink = std::sync::Arc<dyn Fn(String) + Send + Sync>;

/// The tray icon as RGBA pixels, `TRAY_PX` × `TRAY_PX`: a camera, or a red dot while recording.
/// `light` draws the camera white for dark panels.
pub fn tray_pixels(recording: bool, light: bool) -> Vec<u8> {
    use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};
    let n = TRAY_PX;
    let Some(mut pm) = Pixmap::new(n, n) else { return vec![0; (n * n * 4) as usize] };
    let mut paint = Paint::default();
    paint.anti_alias = true;
    if recording {
        paint.set_color_rgba8(230, 40, 40, 255);
        if let Some(c) = PathBuilder::from_circle(22.0, 22.0, 13.0) {
            pm.fill_path(&c, &paint, FillRule::Winding, Transform::identity(), None);
        }
    } else {
        let v = if light { 255 } else { 0 };
        paint.set_color_rgba8(v, v, v, 255);
        if let Some(r) = tiny_skia::Rect::from_xywh(5.0, 13.0, 34.0, 23.0) {
            pm.fill_rect(r, &paint, Transform::identity(), None);
        }
        if let Some(r) = tiny_skia::Rect::from_xywh(15.0, 8.0, 14.0, 6.0) {
            pm.fill_rect(r, &paint, Transform::identity(), None);
        }
        paint.blend_mode = tiny_skia::BlendMode::Clear;
        if let Some(c) = PathBuilder::from_circle(22.0, 24.5, 7.5) {
            pm.fill_path(&c, &paint, FillRule::Winding, Transform::identity(), None);
        }
        paint.blend_mode = tiny_skia::BlendMode::SourceOver;
        if let Some(c) = PathBuilder::from_circle(22.0, 24.5, 4.0) {
            pm.fill_path(&c, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }
    pm.take()
}

pub const TRAY_PX: u32 = 44;
