//! The tray icon (StatusNotifierItem over D-Bus, through ksni) and the global shortcuts
//! (GlobalShortcuts portal: the desktop asks the person once to confirm the keys).

use crate::platform::{MenuEntry, Sink, TRAY_PX, tray_pixels};
use crate::settings::Hotkey;
use ksni::blocking::{Handle, TrayMethods};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

struct Sni {
    menu: Vec<MenuEntry>,
    recording: bool,
    title: Option<String>,
    sink: Sink,
    /// Folder with the two icons as files (see `write_icons`), if it could be made.
    icon_dir: Option<std::path::PathBuf>,
}

const ICON_IDLE: &str = "lightsnap-tray";
const ICON_REC: &str = "lightsnap-tray-recording";

/// Writes both tray icons as files. Tray hosts reliably follow a change of icon *name*; a
/// changed picture alone is not always picked up (GNOME's AppIndicator extension keeps
/// the first one).
///
/// The camera is an SVG in KDE's text color class: Plasma paints it in the panel's text
/// color, dark on a light panel and light on a dark one. Other hosts draw it in the white
/// given in the file (GNOME's top bar is dark). The red dot is a PNG, it shows on any panel.
fn write_icons() -> Option<std::path::PathBuf> {
    let dir = dirs::cache_dir()?.join("lightsnap").join("icons");
    std::fs::create_dir_all(&dir).ok()?;
    // Up to v0.4.2 the camera was a PNG; with both files the host may pick the old one.
    let _ = std::fs::remove_file(dir.join(format!("{ICON_IDLE}.png")));
    std::fs::write(dir.join(format!("{ICON_IDLE}.svg")), CAMERA_SVG).ok()?;
    let img = image::RgbaImage::from_raw(TRAY_PX, TRAY_PX, tray_pixels(true, true))?;
    img.save(dir.join(format!("{ICON_REC}.png"))).ok()?;
    Some(dir)
}

/// The camera of `tray_pixels` as SVG (44 × 44): body, top bump, lens ring and lens.
const CAMERA_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="44" height="44" viewBox="0 0 44 44">
<style id="current-color-scheme" type="text/css">.ColorScheme-Text { color: #ffffff; }</style>
<path class="ColorScheme-Text" fill="currentColor" fill-rule="evenodd"
 d="M5 13h34v23H5z M15 8h14v5H15z M14.5 24.5a7.5 7.5 0 1 0 15 0a7.5 7.5 0 1 0 -15 0z M18 24.5a4 4 0 1 0 8 0a4 4 0 1 0 -8 0z"/>
</svg>
"#;

impl ksni::Tray for Sni {
    fn id(&self) -> String {
        super::APP_ID.into()
    }
    fn title(&self) -> String {
        match &self.title {
            Some(t) => format!("LightSnap {t}"),
            None => "LightSnap".into(),
        }
    }
    fn icon_theme_path(&self) -> String {
        self.icon_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_default()
    }
    fn icon_name(&self) -> String {
        if self.icon_dir.is_none() {
            return String::new();
        }
        (if self.recording { ICON_REC } else { ICON_IDLE }).into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        // RGBA → ARGB, as the spec wants.
        let mut data = tray_pixels(self.recording, true);
        for px in data.chunks_exact_mut(4) {
            px.rotate_right(1);
        }
        vec![ksni::Icon { width: TRAY_PX as i32, height: TRAY_PX as i32, data }]
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: self.title(), ..Default::default() }
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        build(&self.menu)
    }
}

fn build(entries: &[MenuEntry]) -> Vec<ksni::MenuItem<Sni>> {
    use ksni::menu::{StandardItem, SubMenu};
    entries
        .iter()
        .map(|e| match e {
            MenuEntry::Separator => ksni::MenuItem::Separator,
            MenuEntry::Submenu { text, items } => {
                SubMenu { label: text.clone(), submenu: build(items), ..Default::default() }.into()
            }
            MenuEntry::Item { id, text, enabled, key } => {
                let id = id.clone();
                StandardItem {
                    label: text.replace('_', "__"),
                    enabled: *enabled,
                    shortcut: key.as_ref().map(|k| vec![shortcut_parts(k)]).unwrap_or_default(),
                    activate: Box::new(move |t: &mut Sni| {
                        if !id.is_empty() {
                            (t.sink)(id.clone());
                        }
                    }),
                    ..Default::default()
                }
                .into()
            }
        })
        .collect()
}

/// Shortcut as the dbusmenu spec writes it: ["Control", "Alt", "5"].
fn shortcut_parts(k: &Hotkey) -> Vec<String> {
    let mut v = Vec::new();
    if k.cmd || k.ctrl {
        v.push("Control".into());
    }
    if k.alt {
        v.push("Alt".into());
    }
    if k.shift {
        v.push("Shift".into());
    }
    v.push(keysym(&k.code));
    v
}

pub struct Tray(Handle<Sni>);

impl Tray {
    pub fn new(menu: Vec<MenuEntry>, sink: Sink) -> Option<Self> {
        let sni = Sni { menu, recording: false, title: None, sink, icon_dir: write_icons() };
        match sni.spawn() {
            Ok(h) => Some(Self(h)),
            Err(e) => {
                log::warn!(target: "lightsnap", "tray: {e}");
                None
            }
        }
    }

    pub fn set_menu(&self, menu: Vec<MenuEntry>) {
        self.0.update(|t| t.menu = menu);
    }

    pub fn set_recording(&self, on: bool) {
        self.0.update(|t| t.recording = on);
    }

    pub fn set_title(&self, title: Option<&str>) {
        let title = title.map(str::to_string);
        self.0.update(|t| t.title = title);
    }
}

// ---------------------------------------------------------------- global shortcuts

/// Global shortcuts through the portal. Binding runs on a helper thread because the
/// desktop may show a dialog first.
pub struct Hotkeys {
    tx: Sender<Cmd>,
    muted: Arc<AtomicBool>,
    last: Vec<(String, Hotkey)>,
    live: Arc<Mutex<Option<HashMap<String, String>>>>,
    version: Arc<std::sync::atomic::AtomicU32>,
}

enum Cmd {
    Bind(Vec<(String, Hotkey)>),
    Configure,
}

/// Sent to the app's sink when the desktop's shortcuts changed (see `live_labels`).
pub const KEYS_CHANGED: &str = "keys-changed";

impl Hotkeys {
    pub fn new(sink: Sink) -> Option<Self> {
        super::portal::register();
        let (tx, rx) = channel::<Cmd>();
        let muted = Arc::new(AtomicBool::new(false));
        let live: Arc<Mutex<Option<HashMap<String, String>>>> = Default::default();
        let version = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let shared = Shared { sink, muted: muted.clone(), live: live.clone(), version: version.clone() };
        std::thread::spawn(move || pollster::block_on(run(rx, shared)));
        Some(Self { tx, muted, last: Vec::new(), live, version })
    }

    /// The shortcuts as the desktop has them (it may have kept or changed the keys
    /// LightSnap asked for), by action id. None until they are bound.
    pub fn live_labels(&self) -> Option<HashMap<String, String>> {
        self.live.lock().ok()?.clone()
    }

    /// Whether the desktop can open its own page for changing the shortcuts.
    pub fn can_configure(&self) -> bool {
        self.version.load(Ordering::SeqCst) >= 2
    }

    /// Opens the desktop's page for changing LightSnap's shortcuts.
    pub fn configure(&self) {
        let _ = self.tx.send(Cmd::Configure);
    }

    /// Replaces all shortcuts. Each pair is (action id, key). An empty list only mutes
    /// them (while a new key is being chosen in the settings): binding again would make
    /// the desktop ask again.
    pub fn set(&mut self, keys: &[(String, Hotkey)]) {
        if keys.is_empty() {
            self.muted.store(true, Ordering::SeqCst);
            return;
        }
        self.muted.store(false, Ordering::SeqCst);
        if keys != self.last.as_slice() {
            self.last = keys.to_vec();
            let _ = self.tx.send(Cmd::Bind(keys.to_vec()));
        }
    }
}

struct Shared {
    sink: Sink,
    muted: Arc<AtomicBool>,
    live: Arc<Mutex<Option<HashMap<String, String>>>>,
    version: Arc<std::sync::atomic::AtomicU32>,
}

impl Shared {
    fn set_live(&self, shortcuts: &[ashpd::desktop::global_shortcuts::Shortcut]) {
        let map: HashMap<String, String> = shortcuts
            .iter()
            .map(|s| (s.id().to_string(), tidy_trigger(s.trigger_description())))
            .collect();
        log::info!(target: "lightsnap", "global shortcuts: {map:?}");
        if let Ok(mut l) = self.live.lock() {
            *l = Some(map);
        }
        (self.sink)(KEYS_CHANGED.into());
    }
}

/// GNOME describes keys like "Press <Control><Alt>4"; show "Ctrl+Alt+4" like the rest of the app.
fn tidy_trigger(s: &str) -> String {
    let s = s.trim().strip_prefix("Press ").unwrap_or(s.trim());
    if !s.contains('<') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else { break };
        let m = &rest[start + 1..start + end];
        out.push_str(match m {
            "Control" | "Primary" | "Ctrl" => "Ctrl+",
            "Alt" => "Alt+",
            "Shift" => "Shift+",
            "Super" | "Meta" => "Super+",
            other => other,
        });
        rest = &rest[start + end + 1..];
    }
    let key = rest.trim();
    out + &if key.chars().count() == 1 { key.to_uppercase() } else { key.to_string() }
}

async fn run(rx: std::sync::mpsc::Receiver<Cmd>, shared: Shared) {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;
    let portal = match GlobalShortcuts::new().await {
        Ok(p) => p,
        Err(e) => {
            log::warn!(target: "lightsnap", "global shortcuts portal not available: {e}");
            return;
        }
    };
    shared.version.store(portal.version(), Ordering::SeqCst);
    let shared = Arc::new(shared);
    // Key presses and changes made in the desktop's settings arrive on their own threads.
    {
        let sh = shared.clone();
        std::thread::spawn(move || {
            pollster::block_on(async move {
                let Ok(p) = GlobalShortcuts::new().await else { return };
                let Ok(mut stream) = p.receive_activated().await else { return };
                while let Some(ev) = stream.next().await {
                    let id = ev.shortcut_id().to_string();
                    let known = sh.live.lock().map(|l| l.as_ref().is_some_and(|m| m.contains_key(&id))).unwrap_or(false);
                    if known && !sh.muted.load(Ordering::SeqCst) {
                        (sh.sink)(id);
                    }
                }
            })
        });
        let sh = shared.clone();
        std::thread::spawn(move || {
            pollster::block_on(async move {
                let Ok(p) = GlobalShortcuts::new().await else { return };
                let Ok(mut stream) = p.receive_shortcuts_changed().await else { return };
                while let Some(ev) = stream.next().await {
                    sh.set_live(ev.shortcuts());
                }
            })
        });
    }
    let mut session: Option<ashpd::desktop::Session<GlobalShortcuts>> = None;
    while let Ok(cmd) = rx.recv() {
        let mut keys = match cmd {
            Cmd::Configure => {
                if let Some(s) = &session {
                    if let Err(e) = portal.configure_shortcuts(s, None, Default::default()).await {
                        log::warn!(target: "lightsnap", "configure shortcuts: {e}");
                    }
                }
                continue;
            }
            Cmd::Bind(k) => k,
        };
        // Only the newest list counts.
        while let Ok(Cmd::Bind(k)) = rx.try_recv() {
            keys = k;
        }
        if let Some(s) = session.take() {
            let _ = s.close().await;
        }
        let Ok(s) = portal.create_session(Default::default()).await else { continue };
        let shortcuts: Vec<NewShortcut> = keys
            .iter()
            .map(|(id, k)| NewShortcut::new(id.as_str(), description(id)).preferred_trigger(Some(trigger(k).as_str())))
            .collect();
        match portal.bind_shortcuts(&s, &shortcuts, None, Default::default()).await.and_then(|r| r.response()) {
            Ok(r) => shared.set_live(r.shortcuts()),
            Err(e) => log::warn!(target: "lightsnap", "global shortcuts: {e}"),
        }
        session = Some(s);
    }
}

fn description(id: &str) -> String {
    crate::settings::HOTKEY_ACTIONS
        .iter()
        .chain(crate::settings::VIDEO_HOTKEY_ACTIONS.iter())
        .find(|(i, _)| *i == id)
        .map(|(_, d)| d.to_string())
        .unwrap_or_else(|| id.to_string())
}

/// The key in the XDG shortcut format: "CTRL+ALT+5".
fn trigger(k: &Hotkey) -> String {
    let mut s = String::new();
    if k.cmd || k.ctrl {
        s.push_str("CTRL+");
    }
    if k.alt {
        s.push_str("ALT+");
    }
    if k.shift {
        s.push_str("SHIFT+");
    }
    s + &keysym(&k.code)
}

/// XKB key name for a W3C key code: "KeyA" → "a", "Digit5" → "5".
fn keysym(code: &str) -> String {
    if let Some(k) = code.strip_prefix("Key") {
        return k.to_lowercase();
    }
    if let Some(d) = code.strip_prefix("Digit") {
        return d.to_string();
    }
    match code {
        "Space" => "space",
        "Minus" => "minus",
        "Equal" => "equal",
        "Comma" => "comma",
        "Period" => "period",
        "Slash" => "slash",
        "Backslash" => "backslash",
        "Semicolon" => "semicolon",
        "Quote" => "apostrophe",
        "BracketLeft" => "bracketleft",
        "BracketRight" => "bracketright",
        "Backquote" => "grave",
        "ArrowUp" => "Up",
        "ArrowDown" => "Down",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "Enter" => "Return",
        "Escape" => "Escape",
        "Backspace" => "BackSpace",
        "Tab" => "Tab",
        "PrintScreen" => "Print",
        other => other, // F1..F24 and the rest keep their name
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn tidies_gnome_triggers() {
        assert_eq!(super::tidy_trigger("Press <Control><Alt>4"), "Ctrl+Alt+4");
        assert_eq!(super::tidy_trigger("<Shift><Super>s"), "Shift+Super+S");
        assert_eq!(super::tidy_trigger("Ctrl+Alt+6"), "Ctrl+Alt+6");
        assert_eq!(super::tidy_trigger(""), "");
    }
}
