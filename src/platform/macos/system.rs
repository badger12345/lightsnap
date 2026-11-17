//! Opening files and apps, notifications, launch at login, and where logs go.

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

const AGENT_LABEL: &str = "app.lightsnap.login";

fn agent_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join("Library/LaunchAgents").join(format!("{AGENT_LABEL}.plist")))
}

pub fn login_item_enabled() -> bool {
    agent_path().is_some_and(|p| p.is_file())
}

/// Starts LightSnap (in the menu bar, window hidden) when the person logs in.
pub fn set_login_item(on: bool) {
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
