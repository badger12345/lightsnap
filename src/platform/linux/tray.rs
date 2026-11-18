//! The tray icon (StatusNotifierItem over D-Bus, through ksni) and the global shortcuts
//! (GlobalShortcuts portal: the desktop asks the person once to confirm the keys).

use crate::platform::{MenuEntry, Sink, TRAY_PX, tray_pixels};
use crate::settings::Hotkey;
use ksni::blocking::{Handle, TrayMethods};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

struct Sni {
    menu: Vec<MenuEntry>,
    recording: bool,
    title: Option<String>,
    sink: Sink,
}

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
        let sni = Sni { menu, recording: false, title: None, sink };
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
    tx: Sender<Vec<(String, Hotkey)>>,
    muted: Arc<AtomicBool>,
    last: Vec<(String, Hotkey)>,
}

impl Hotkeys {
    pub fn new(sink: Sink) -> Option<Self> {
        super::portal::register();
        let (tx, rx) = channel::<Vec<(String, Hotkey)>>();
        let muted = Arc::new(AtomicBool::new(false));
        let active: Arc<Mutex<Vec<String>>> = Default::default();
        let (m, a) = (muted.clone(), active.clone());
        std::thread::spawn(move || pollster::block_on(run(rx, sink, m, a)));
        Some(Self { tx, muted, last: Vec::new() })
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
            let _ = self.tx.send(keys.to_vec());
        }
    }
}

async fn run(
    rx: std::sync::mpsc::Receiver<Vec<(String, Hotkey)>>,
    sink: Sink,
    muted: Arc<AtomicBool>,
    active: Arc<Mutex<Vec<String>>>,
) {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;
    let portal = match GlobalShortcuts::new().await {
        Ok(p) => p,
        Err(e) => {
            log::warn!(target: "lightsnap", "global shortcuts portal not available: {e}");
            return;
        }
    };
    // Key presses arrive on their own thread.
    {
        let (sink, muted, active) = (sink.clone(), muted.clone(), active.clone());
        std::thread::spawn(move || {
            pollster::block_on(async move {
                let Ok(p) = GlobalShortcuts::new().await else { return };
                let Ok(mut stream) = p.receive_activated().await else { return };
                while let Some(ev) = stream.next().await {
                    let id = ev.shortcut_id().to_string();
                    let known = active.lock().map(|a| a.contains(&id)).unwrap_or(false);
                    if known && !muted.load(Ordering::SeqCst) {
                        sink(id);
                    }
                }
            })
        });
    }
    let mut session = None;
    while let Ok(mut keys) = rx.recv() {
        // Only the newest list counts.
        while let Ok(k) = rx.try_recv() {
            keys = k;
        }
        if let Some(s) = session.take() {
            let _: ashpd::desktop::Session<GlobalShortcuts> = s;
            let _ = s.close().await;
        }
        let Ok(s) = portal.create_session(Default::default()).await else { continue };
        let shortcuts: Vec<NewShortcut> = keys
            .iter()
            .map(|(id, k)| NewShortcut::new(id.as_str(), description(id)).preferred_trigger(Some(trigger(k).as_str())))
            .collect();
        match portal.bind_shortcuts(&s, &shortcuts, None, Default::default()).await.and_then(|r| r.response()) {
            Ok(r) => {
                let ids: Vec<String> = r.shortcuts().iter().map(|s| s.id().to_string()).collect();
                log::info!(target: "lightsnap", "global shortcuts bound: {ids:?}");
                if let Ok(mut a) = active.lock() {
                    *a = ids;
                }
            }
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
