//! LightSnap: a light screenshot and screen-recording app for macOS.
//! No cloud, no network: everything is saved locally or copied to the clipboard.

#![allow(deprecated)]

mod app;
mod doc;
mod editor;
mod icons;
mod platform;
mod prefs;
mod record;
mod settings;
mod video;

use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    install_crash_log();
    let args: Vec<String> = std::env::args().collect();
    let screenshot = args
        .iter()
        .position(|a| a == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    // `--no-focus` opens like a check run but stays open (for measuring idle use).
    let no_focus = args.iter().any(|a| a == "--no-focus");
    let quiet = screenshot.is_some() || no_focus;
    // `--background` (used by "Launch at login") starts in the menu bar with no window.
    let mut background = args.iter().any(|a| a == "--background") && screenshot.is_none();
    // A picture or video given on the command line opens right away.
    let open = args
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(i, a)| !a.starts_with("--") && args.get(i - 1).is_none_or(|p| p != "--screenshot"))
        .map(|(_, a)| PathBuf::from(a))
        .find(|p| p.is_file())
        // Absolute, so ffmpeg never reads a name like "x:y.mov" as a protocol.
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p));

    // Start quietly in the menu bar; only the very first start shows the window.
    if screenshot.is_none() && !no_focus && open.is_none() {
        let mut st = settings::Settings::load();
        if st.welcomed {
            background = true;
        } else {
            st.welcomed = true;
            st.save();
        }
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("LightSnap")
        .with_inner_size([980.0, 620.0])
        .with_min_inner_size([900.0, 440.0]);
    if background {
        viewport = viewport.with_visible(false);
    }
    if quiet {
        // Open without taking focus so a check never gets in the way.
        viewport = viewport.with_active(false);
        #[cfg(target_os = "windows")]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
    }

    let mut options = eframe::NativeOptions {
        viewport,
        // Only repaint when something happens: near-zero CPU while idle.
        run_and_return: false,
        ..Default::default()
    };
    if quiet {
        #[cfg(target_os = "macos")]
        {
            options.event_loop_builder = Some(Box::new(|b| {
                use winit::platform::macos::EventLoopBuilderExtMacOS;
                b.with_activate_ignoring_other_apps(false);
            }));
        }
    }

    eframe::run_native(
        "LightSnap",
        options,
        Box::new(move |cc| Ok(Box::new(app::LightSnap::new(cc, screenshot, open, quiet, background)))),
    )
}

/// If LightSnap ever stops because of an error, write what happened to
/// crash.log in the system's log folder (see `platform::log_dir`) so it can be found and fixed.
fn install_crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if EXPECTED_PANIC.with(|c| c.get()) {
            return; // handled by show_immediate
        }
        if let Some(dir) = platform::log_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let text = format!(
                "--- {} ---\n{info}\n{}\n",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                std::backtrace::Backtrace::force_capture()
            );
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
                let _ = f.write_all(text.as_bytes());
            }
        }
        default(info);
    }));
}

thread_local! {
    static EXPECTED_PANIC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Shows an extra window (immediate viewport). While the main window is hidden,
/// eframe sometimes can't create a brand-new window in that frame and egui then stops
/// the app. Here that frame is simply skipped: the window gets created right after it,
/// and the next frame draws it.
pub fn show_immediate<T: Default>(
    ctx: &egui::Context,
    id: egui::ViewportId,
    builder: egui::ViewportBuilder,
    cb: impl FnMut(&mut egui::Ui, egui::ViewportClass) -> T,
) -> T {
    EXPECTED_PANIC.with(|c| c.set(true));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctx.show_viewport_immediate(id, builder, cb)));
    EXPECTED_PANIC.with(|c| c.set(false));
    match r {
        Ok(v) => v,
        Err(_) => {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            T::default()
        }
    }
}
