//! Opening files and apps, notifications, launch at login, and where logs go.

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Opens a file or folder with its default app.
pub fn open_path(path: &Path) {
    let _ = Command::new("/usr/bin/open").arg(path).spawn();
}

/// Shows a file in the Finder.
pub fn reveal_path(path: &Path) {
    let _ = Command::new("/usr/bin/open").arg("-R").arg(path).spawn();
}

/// Opens a file in a given app (an app name like "Preview" or the path of an .app).
pub fn open_with(app: &str, path: &Path) {
    let _ = Command::new("/usr/bin/open").arg("-a").arg(app).arg(path).spawn();
}

/// Shows a system notification.
pub fn notify(title: &str, body: &str) {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("display notification \"{}\" with title \"LightSnap\" subtitle \"{}\"", esc(body), esc(title));
    let _ = Command::new("/usr/bin/osascript").args(["-e", &script]).spawn();
}

/// Sets up the app's event loop: a menu-bar app without a Dock icon. With `quiet`
/// (check pictures) the app also starts without taking focus.
pub fn event_loop_hook(quiet: bool) -> eframe::EventLoopBuilderHook {
    Box::new(move |b| {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        // winit would otherwise switch to a regular app with a Dock icon, even with
        // LSUIElement in Info.plist.
        b.with_activation_policy(ActivationPolicy::Accessory);
        if quiet {
            b.with_activate_ignoring_other_apps(false);
        }
    })
}

/// Bold fonts for text annotations, first one found wins.
pub const ANNOTATION_FONTS: &[&str] = &[
    "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
    "/Library/Fonts/Arial Bold.ttf",
    "/System/Library/Fonts/Supplemental/Verdana Bold.ttf",
    "/System/Library/Fonts/Supplemental/Tahoma Bold.ttf",
];

/// Small font with the ⌥ ⌘ ⇧ key symbols, used as a fallback in the interface.
pub const SYMBOL_FONT: &str = "/System/Library/Fonts/Apple Symbols.ttf";

/// Where the crash log goes: ~/Library/Logs/LightSnap.
pub fn log_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join("Library/Logs/LightSnap"))
}

// ---------------------------------------------------------------- launch at login
// Inside LightSnap.app this uses SMAppService, so the app shows up in System Settings ›
// General › Login Items. A bare binary (`cargo run`) can't use SMAppService and falls
// back to a LaunchAgent file. Started at login, the app opens in the menu bar only
// (see `main.rs`: after the first start it always starts quietly).

#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

fn in_app_bundle() -> bool {
    std::env::current_exe()
        .ok()
        .is_some_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
}

fn main_app_service() -> Option<Retained<AnyObject>> {
    let cls = AnyClass::get(c"SMAppService")?;
    unsafe { msg_send![cls, mainAppService] }
}

pub fn login_item_enabled() -> bool {
    if in_app_bundle() {
        let Some(svc) = main_app_service() else { return false };
        let status: isize = unsafe { msg_send![&*svc, status] };
        // 1 = enabled, 2 = waiting for approval in System Settings.
        return matches!(status, 1 | 2);
    }
    agent_path().is_some_and(|p| p.is_file())
}

/// Starts LightSnap (in the menu bar, window hidden) when the person logs in.
pub fn set_login_item(on: bool) {
    if in_app_bundle() {
        // Builds before the app bundle wrote a LaunchAgent file; SMAppService replaces it.
        if let Some(p) = agent_path() {
            let _ = std::fs::remove_file(p);
        }
        let Some(svc) = main_app_service() else { return };
        let no_error: *mut *mut AnyObject = std::ptr::null_mut();
        let _ok: bool = unsafe {
            if on {
                msg_send![&*svc, registerAndReturnError: no_error]
            } else {
                msg_send![&*svc, unregisterAndReturnError: no_error]
            }
        };
        return;
    }
    set_launch_agent(on);
}

const AGENT_LABEL: &str = "app.lightsnap.login";

fn agent_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join("Library/LaunchAgents").join(format!("{AGENT_LABEL}.plist")))
}

fn set_launch_agent(on: bool) {
    let Some(path) = agent_path() else { return };
    if !on {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let exe = exe.display().to_string().replace('&', "&amp;").replace('<', "&lt;");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{AGENT_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>--background</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#
    );
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&path, plist);
}
