//! Opening files and apps, notifications, launch at login, and where logs go.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Opens a file or folder with its default app.
pub fn open_path(path: &Path) {
    let _ = Command::new("xdg-open").arg(path).spawn();
}

/// Shows a file in the file manager, selected.
pub fn reveal_path(path: &Path) {
    let uri = format!("file://{}", path.display());
    let shown = pollster::block_on(async {
        let c = zbus::Connection::session().await?;
        c.call_method(
            Some("org.freedesktop.FileManager1"),
            "/org/freedesktop/FileManager1",
            Some("org.freedesktop.FileManager1"),
            "ShowItems",
            &(vec![uri.as_str()], ""),
        )
        .await
    })
    .is_ok();
    if !shown {
        if let Some(dir) = path.parent() {
            open_path(dir);
        }
    }
}

/// Opens a file in a given app: a .desktop file, a command name such as "gimp", or
/// empty for the default app.
pub fn open_with(app: &str, path: &Path) {
    if app.trim().is_empty() {
        open_path(path);
    } else if app.ends_with(".desktop") {
        let _ = Command::new("gio").arg("launch").arg(app).arg(path).spawn();
    } else {
        let _ = Command::new(app).arg(path).spawn();
    }
}

/// Shows a desktop notification.
pub fn notify(title: &str, body: &str) {
    let (title, body) = (title.to_string(), body.to_string());
    std::thread::spawn(move || {
        let _ = pollster::block_on(async {
            let c = zbus::Connection::session().await?;
            let hints: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
            c.call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "Notify",
                &("LightSnap", 0u32, super::APP_ID, title.as_str(), body.as_str(), Vec::<&str>::new(), hints, 4000i32),
            )
            .await
        });
    });
}

// Words for the interface.
pub const THIS_COMPUTER: &str = "this computer";
pub const FILE_MANAGER: &str = "Files";
pub const VIDEO_PLAYER: &str = "Video Player";
pub const SCREEN_PERMISSION_HINT: &str = "Allow LightSnap to take screenshots when your desktop asks.";
pub const FFMPEG_HINT: &str = "install ffmpeg with your package manager";
/// Image editors offered in the settings: (value stored in the settings, label).
pub const EDITOR_APPS: &[(&str, &str)] = &[("", "Default App")];
pub const DEFAULT_EDITOR: &str = "";
/// Where "Choose…" starts, and the file type it accepts.
pub const EDITOR_DIR: &str = "/usr/share/applications";
pub const EDITOR_EXT: &str = "desktop";

/// The name of an image editor as stored in the settings: empty for the default app,
/// a .desktop file, or a command.
pub fn editor_label(app: &str) -> String {
    if app.trim().is_empty() {
        return "Default App".into();
    }
    if app.ends_with(".desktop") {
        if let Ok(text) = std::fs::read_to_string(app) {
            if let Some(n) = text.lines().find_map(|l| l.strip_prefix("Name=")) {
                return n.trim().to_string();
            }
        }
    }
    Path::new(app).file_stem().and_then(|s| s.to_str()).unwrap_or(app).to_string()
}

/// Runs once at start. Makes sure the desktop knows LightSnap: a .desktop file with
/// LightSnap's app id gives it its name and icon, and the portals need it to remember
/// what was allowed. A package or AppImage tool that installed one wins; otherwise one
/// goes into ~/.local/share and is kept pointing at this binary.
pub fn prepare() {
    let name = format!("{}.desktop", super::APP_ID);
    let user_dir = dirs::data_dir();
    let system_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    let installed_elsewhere = system_dirs
        .split(':')
        .filter(|d| !d.is_empty())
        .any(|d| Path::new(d).join("applications").join(&name).is_file());
    let Some(data) = user_dir else { return };
    if installed_elsewhere {
        return;
    }
    let exe = std::env::var_os("APPIMAGE").map(PathBuf::from).or_else(|| std::env::current_exe().ok());
    let Some(exe) = exe else { return };
    let entry = include_str!("../../../linux/io.github.badger12345.LightSnap.desktop")
        .replace("Exec=lightsnap", &format!("Exec={}", desktop_quote(&exe.display().to_string())));
    let path = data.join("applications").join(&name);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(entry.as_str()) {
        let _ = std::fs::create_dir_all(data.join("applications"));
        let _ = std::fs::write(&path, entry);
    }
    let icon = data.join("icons/hicolor/256x256/apps").join(format!("{}.png", super::APP_ID));
    if !icon.is_file() {
        let _ = std::fs::create_dir_all(icon.parent().unwrap_or(Path::new(".")));
        let _ = std::fs::write(&icon, include_bytes!("../../../linux/io.github.badger12345.LightSnap.png"));
    }
}

/// Quotes a path for the Exec line of a .desktop file.
fn desktop_quote(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)) {
        return s.to_string();
    }
    let mut q = String::from("\"");
    for c in s.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            q.push('\\');
        }
        if c == '%' {
            q.push('%');
        }
        q.push(c);
    }
    q.push('"');
    q
}

/// Nothing to set up on Linux.
pub fn event_loop_hook(_quiet: bool) -> eframe::EventLoopBuilderHook {
    Box::new(|_| {})
}

/// Bold fonts for text annotations, first one found wins.
pub const ANNOTATION_FONTS: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
    "/usr/share/fonts/liberation-sans/LiberationSans-Bold.ttf",
];

/// Font with arrow and key symbols, used as a fallback in the interface.
pub const SYMBOL_FONT: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

/// Where the crash log goes: ~/.local/state/lightsnap.
pub fn log_dir() -> Option<PathBuf> {
    dirs::state_dir().or_else(|| dirs::home_dir().map(|h| h.join(".local/state"))).map(|d| d.join("lightsnap"))
}

// ---------------------------------------------------------------- launch at login
// A .desktop file in ~/.config/autostart, the freedesktop way. Started at login, the
// app opens in the tray only.

fn autostart_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("autostart").join(format!("{}.desktop", super::APP_ID)))
}

pub fn login_item_enabled() -> bool {
    autostart_path().is_some_and(|p| p.is_file())
}

pub fn set_login_item(on: bool) {
    let Some(path) = autostart_path() else { return };
    if !on {
        let _ = std::fs::remove_file(&path);
        return;
    }
    // Inside an AppImage the binary lives in a temporary mount; start the AppImage itself.
    let exe = std::env::var_os("APPIMAGE").map(PathBuf::from).or_else(|| std::env::current_exe().ok());
    let Some(exe) = exe else { return };
    let exe = desktop_quote(&exe.display().to_string());
    let body = format!(
        "[Desktop Entry]\nType=Application\nName=LightSnap\nExec={exe} --background\nIcon={id}\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n",
        id = super::APP_ID
    );
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&path, body);
}
