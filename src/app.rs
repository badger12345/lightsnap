//! The main window, the menu-bar icon, global shortcuts and the capture/record flow.

use crate::platform::{self, CaptureKind};
use crate::doc::Style;
use crate::editor::{Editor, ResizeDlg, Tool};
use crate::icons::{self, Icon};
use crate::platform::Devices;
use crate::record::{self, RecordOptions, Recorder};
use crate::settings::{AfterCapture, ImageFormat, Quality, Settings};
use crate::video::VideoView;
use ab_glyph::FontVec;
use egui::{
    Align, Align2, Color32, ColorImage, CornerRadius, FontId, Frame, Id, Key, Layout, Margin, Modifiers,
    Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, Vec2, ViewportBuilder,
    ViewportCommand, ViewportId, WindowLevel, pos2, vec2,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- colours

const BAR: Color32 = Color32::from_rgb(44, 44, 46);
const BTN: Color32 = Color32::from_rgb(58, 58, 60);
const BTN_HOVER: Color32 = Color32::from_rgb(72, 72, 76);
const EDGE: Color32 = Color32::from_rgb(24, 24, 26);
const BLUE: Color32 = Color32::from_rgb(40, 76, 186);
const BLUE_HOVER: Color32 = Color32::from_rgb(54, 92, 206);
const RED: Color32 = Color32::from_rgb(226, 38, 38);
const TEXT: Color32 = Color32::from_rgb(235, 235, 240);
const DIM_TEXT: Color32 = Color32::from_rgb(150, 150, 156);
/// Height of every control in the top bar (fields, buttons, button groups).
const BAR_ITEM_H: f32 = TOOL_H;
/// Height of every control in the bottom tool bar.
const TOOL_H: f32 = 26.0;
/// Width of every icon button, top and bottom.
const TOOL_W: f32 = 36.0;
/// Text size inside the top bar, kept small so the buttons look as tall as the tool buttons.
const BAR_FONT: f32 = 13.0;

const PALETTE: [Color32; 10] = [
    Color32::from_rgb(235, 51, 36),
    Color32::from_rgb(255, 149, 0),
    Color32::from_rgb(255, 214, 10),
    Color32::from_rgb(52, 199, 89),
    Color32::from_rgb(0, 122, 255),
    Color32::from_rgb(175, 82, 222),
    Color32::from_rgb(255, 55, 140),
    Color32::from_rgb(255, 255, 255),
    Color32::from_rgb(142, 142, 147),
    Color32::from_rgb(0, 0, 0),
];

// ---------------------------------------------------------------- actions

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Capture(CaptureKind),
    Record,
    Show,
    OpenImage,
    OpenClipboard,
    OpenLatest,
    OpenFolder,
    Settings,
    Quit,
    FinishRec,
    CancelRec,
    PauseRec,
}

fn action_from_id(id: &str, delay: u32) -> Option<Action> {
    Some(match id {
        "area" => Action::Capture(CaptureKind::Area),
        "window" => Action::Capture(CaptureKind::Window),
        "full" => Action::Capture(CaptureKind::FullScreen),
        "timed" => Action::Capture(CaptureKind::Timed(delay)),
        "record" => Action::Record,
        "show" => Action::Show,
        "open" => Action::OpenImage,
        "clipboard" => Action::OpenClipboard,
        "latest" => Action::OpenLatest,
        "folder" => Action::OpenFolder,
        "settings" => Action::Settings,
        "quit" => Action::Quit,
        "finish" => Action::FinishRec,
        "cancel" => Action::CancelRec,
        "pause" | "rec_pause" => Action::PauseRec,
        "rec_stop" => Action::FinishRec,
        _ => return None,
    })
}

enum AppEvent {
    Menu(String),
    Hotkey(u32),
    Dialog(DialogKind, Option<PathBuf>),
}

/// File dialogs. They open as a sheet without blocking; a blocking dialog would make
/// macOS deliver events while one is still being handled, which stops the app.
#[derive(Clone, Copy, Debug)]
pub enum DialogKind {
    OpenImage,
    SaveAs,
    SaveDir,
    EditorApp,
}

struct Job {
    /// The captured picture(s): one per screen when preparing a recording; empty if cancelled.
    rx: Receiver<Vec<PathBuf>>,
    for_record: bool,
    show_after: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum SelDrag {
    New(Pos2),
    Move(Pos2, Rect),
    Resize(u8, Rect),
}

/// One screen while choosing the area to record: its frozen picture and where it is.
struct SelScreen {
    bounds: Rect,
    scale: f32,
    img: Option<ColorImage>,
    tex: Option<TextureHandle>,
}

/// Choosing the area to record. One overlay window per screen; the area is in global
/// points, so it can be dragged from one screen to another.
struct Select {
    screens: Vec<SelScreen>,
    rect: Option<Rect>,
    /// The area before "full screen" was clicked, to switch back.
    before_full: Option<Rect>,
    drag: Option<SelDrag>,
    settings_open: bool,
    focused: bool,
}

#[derive(Default)]
enum SelectOut {
    #[default]
    None,
    Cancel,
    Record(Rect),
}

struct RecState {
    rec: Recorder,
    rect: Rect,
    /// The screen being recorded (global points).
    screen: Rect,
    last_secs: u64,
}

struct Toast {
    text: String,
    until: Instant,
    path: Option<PathBuf>,
}

struct Screenshot {
    path: PathBuf,
    frames: u32,
    sent: bool,
}

pub struct LightSnap {
    settings: Settings,
    font: Arc<FontVec>,
    ed: Option<Editor>,
    video: Option<VideoView>,
    tool: Tool,
    checker: Option<TextureHandle>,
    job: Option<Job>,
    select: Option<Select>,
    recording: Option<RecState>,
    finishing: Option<Receiver<Result<PathBuf, String>>>,
    rec_started_name: String,
    video_saving: Option<Receiver<Result<PathBuf, String>>>,
    copy_after_save: bool,
    ffmpeg: Option<PathBuf>,
    devices: Arc<Mutex<Option<Devices>>>,
    tray: Option<tray_icon::TrayIcon>,
    tray_recording: bool,
    events: Receiver<AppEvent>,
    _tx: Sender<AppEvent>,
    _hotkeys: Option<global_hotkey::GlobalHotKeyManager>,
    hotkey_map: Vec<(u32, Action)>,
    toast: Option<Toast>,
    prefs: crate::prefs::Prefs,
    registered: Vec<global_hotkey::hotkey::HotKey>,
    background: bool,
    test_prefs_done: bool,
    /// Last editor version put on the clipboard, and when the next copy is due.
    clip_version: u64,
    clip_due: Option<Instant>,
    clip_gen: Arc<std::sync::atomic::AtomicU64>,
    test_dialog_done: bool,
    test_select_since: Option<Instant>,
    egui_ctx: egui::Context,
    shot: Option<Screenshot>,
    visible: bool,
    monitor: Vec2,
    native_ppp: f32,
    recent: Vec<PathBuf>,
    recent_dirty: bool,
    tray_menu_dirty: bool,
    close_after_save: bool,
    pending_size: Option<Vec2>,
    pending_pos: Option<Pos2>,
}

impl LightSnap {
    pub fn new(cc: &eframe::CreationContext<'_>, screenshot: Option<PathBuf>, open: Option<PathBuf>, quiet: bool, background: bool) -> Self {
        let ctx = cc.egui_ctx.clone();
        let (font, font_bytes) = crate::doc::load_font();
        setup_style(&ctx, font_bytes);
        let settings = Settings::load();
        let (tx, rx) = channel();

        let mut app = Self {
            tool: Tool::Arrow,
            settings,
            font: Arc::new(font),
            ed: None,
            video: None,
            checker: None,
            job: None,
            select: None,
            recording: None,
            finishing: None,
            rec_started_name: String::new(),
            video_saving: None,
            copy_after_save: false,
            ffmpeg: record::find_ffmpeg(),
            devices: Arc::new(Mutex::new(None)),
            tray: None,
            tray_recording: false,
            events: rx,
            _tx: tx.clone(),
            _hotkeys: None,
            hotkey_map: Vec::new(),
            toast: None,
            prefs: Default::default(),
            registered: Vec::new(),
            background,
            test_prefs_done: false,
            clip_version: 0,
            clip_due: None,
            clip_gen: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            test_dialog_done: false,
            test_select_since: None,
            egui_ctx: ctx.clone(),
            shot: screenshot.map(|path| Screenshot { path, frames: 0, sent: false }),
            visible: !background,
            monitor: vec2(1440.0, 900.0),
            native_ppp: 2.0,
            recent: Vec::new(),
            recent_dirty: true,
            tray_menu_dirty: false,
            close_after_save: false,
            pending_size: None,
            pending_pos: None,
        };

        // The menu-bar icon and global shortcuts are skipped for a quick check picture.
        if !quiet || test_env("LS_TEST_TRAY") {
            app.tray = build_tray(&app.settings);
            {
                let tx = tx.clone();
                let ctx = ctx.clone();
                tray_icon::menu::MenuEvent::set_event_handler(Some(move |e: tray_icon::menu::MenuEvent| {
                    let _ = tx.send(AppEvent::Menu(e.id.0.clone()));
                    ctx.request_repaint();
                }));
            }
            if app.settings.hotkeys {
                app.setup_hotkeys(&ctx, tx);
            }
        }
        if let Some(p) = open {
            app.open_file(&ctx, &p);
        }
        // For check pictures: LS_PREFS=general|advanced|hotkeys draws the settings inside the main window.
        if let (true, Ok(tab)) = (app.shot.is_some() && cfg!(debug_assertions), std::env::var("LS_PREFS")) {
            ctx.set_embed_viewports(true);
            app.pending_size = Some(vec2(900.0, 840.0));
            app.prefs.open();
            app.prefs.tab = match tab.as_str() {
                "advanced" => crate::prefs::Tab::Advanced,
                "hotkeys" => crate::prefs::Tab::Hotkeys,
                _ => crate::prefs::Tab::General,
            };
        }
        let _ = app.background;
        app
    }

    fn setup_hotkeys(&mut self, ctx: &egui::Context, tx: Sender<AppEvent>) {
        let Ok(mgr) = global_hotkey::GlobalHotKeyManager::new() else { return };
        let ctx = ctx.clone();
        global_hotkey::GlobalHotKeyEvent::set_event_handler(Some(move |e: global_hotkey::GlobalHotKeyEvent| {
            if e.state == global_hotkey::HotKeyState::Pressed {
                let _ = tx.send(AppEvent::Hotkey(e.id));
                ctx.request_repaint();
            }
        }));
        self._hotkeys = Some(mgr);
        self.register_hotkeys();
    }

    /// Waits for a sheet dialog on a helper thread and reports the chosen path.
    fn spawn_dialog(
        &mut self,
        ctx: &egui::Context,
        kind: DialogKind,
        fut: impl std::future::Future<Output = Option<rfd::FileHandle>> + Send + 'static,
    ) {
        let tx = self._tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let path = pollster::block_on(fut).map(|h| h.path().to_path_buf());
            let _ = tx.send(AppEvent::Dialog(kind, path));
            ctx.request_repaint();
        });
    }

    fn unregister_hotkeys(&mut self) {
        if let Some(mgr) = &self._hotkeys {
            let _ = mgr.unregister_all(&self.registered);
        }
        self.registered.clear();
        self.hotkey_map.clear();
    }

    /// (Re)registers the global shortcuts from the settings.
    fn register_hotkeys(&mut self) {
        self.unregister_hotkeys();
        if !self.settings.hotkeys {
            return;
        }
        let Some(mgr) = &self._hotkeys else { return };
        let ids = crate::settings::HOTKEY_ACTIONS
            .iter()
            .chain(crate::settings::VIDEO_HOTKEY_ACTIONS.iter())
            .map(|(id, _)| *id);
        for id in ids {
            let Some(hk) = self.settings.hotkey(id).and_then(|k| to_global_hotkey(&k)) else { continue };
            let Some(action) = action_from_id(id, self.settings.delay_secs) else { continue };
            if mgr.register(hk).is_ok() {
                self.registered.push(hk);
                self.hotkey_map.push((hk.id(), action));
            }
        }
    }

    fn style(&self) -> Style {
        let ppp = self.ed.as_ref().map(|e| e.ppp).unwrap_or(1.0);
        let [r, g, b, a] = self.settings.color;
        Style { color: Color32::from_rgba_unmultiplied(r, g, b, a), width: self.settings.width * ppp }
    }

    fn toast(&mut self, text: impl Into<String>, path: Option<PathBuf>) {
        self.toast = Some(Toast { text: text.into(), until: Instant::now() + Duration::from_secs(4), path });
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.visible = true;
    }

    fn hide_window(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        self.visible = false;
    }

    fn busy(&self) -> bool {
        self.job.is_some() || self.select.is_some() || self.recording.is_some() || self.finishing.is_some()
    }

    // ------------------------------------------------------------ actions

    fn run_action(&mut self, ctx: &egui::Context, a: Action) {
        match a {
            Action::Capture(kind) => self.start_capture(ctx, kind, false),
            Action::Record => {
                if self.recording.is_none() {
                    self.start_capture(ctx, CaptureKind::FullScreen, true)
                }
            }
            Action::Show => self.show_window(ctx),
            Action::OpenClipboard => {
                let tmp = platform::temp_file("clip", "png");
                if platform::clipboard_png_to_file(&tmp) {
                    match image::open(&tmp) {
                        Ok(img) => {
                            let ed = Editor::new(img.to_rgba8(), self.settings.make_name("Screenshot"), self.native_ppp);
                            self.set_editor(ctx, ed);
                        }
                        Err(_) => self.toast("There is no picture on the clipboard", None),
                    }
                } else {
                    self.toast("There is no picture on the clipboard", None);
                }
                let _ = std::fs::remove_file(&tmp);
                self.show_window(ctx);
            }
            Action::OpenLatest => {
                // The last screenshot taken, even if it was never saved.
                let latest = latest_capture_path().filter(|p| p.is_file()).and_then(|p| image::open(&p).ok());
                if let Some(img) = latest {
                    let name = latest_capture_path()
                        .map(|p| p.with_extension("txt"))
                        .and_then(|p| std::fs::read_to_string(p).ok())
                        .map(|n| n.trim().to_string())
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| self.settings.make_name("Screenshot"));
                    let ed = Editor::new(img.to_rgba8(), name, self.native_ppp);
                    self.set_editor(ctx, ed);
                } else if let Some(p) = recent_files(&self.settings.save_dir, 20).into_iter().find(|p| is_picture(p)) {
                    self.open_file(ctx, &p);
                } else {
                    self.toast("No screenshot taken yet", None);
                }
                self.show_window(ctx);
            }
            Action::OpenImage => {
                self.show_window(ctx);
                let d = rfd::AsyncFileDialog::new().add_filter("Pictures", &["png", "jpg", "jpeg", "mp4", "mov"]);
                self.spawn_dialog(ctx, DialogKind::OpenImage, d.pick_file());
            }
            Action::OpenFolder => {
                let _ = std::fs::create_dir_all(&self.settings.save_dir);
                platform::open_path(&self.settings.save_dir);
            }
            Action::Settings => {
                self.prefs.open();
                if !self.visible {
                    ctx.send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);
                }
            }
            Action::Quit => {
                if let Some(r) = self.recording.take() {
                    r.rec.cancel();
                }
                self.settings.save();
                std::process::exit(0);
            }
            Action::FinishRec => self.finish_recording(ctx),
            Action::CancelRec => {
                if let Some(r) = self.recording.take() {
                    r.rec.cancel();
                }
                self.set_tray_recording(false);
                self.toast("Recording cancelled", None);
            }
            Action::PauseRec => {
                if let Some(r) = &mut self.recording {
                    r.rec.toggle_pause();
                    r.last_secs = u64::MAX;
                }
            }
        }
    }

    fn start_capture(&mut self, ctx: &egui::Context, kind: CaptureKind, for_record: bool) {
        if self.busy() {
            return;
        }
        if let Some(v) = &mut self.video {
            v.stop();
        }
        let was_visible = self.visible;
        if was_visible {
            self.hide_window(ctx);
        }
        if for_record {
            if let Some(ff) = self.ffmpeg.clone() {
                let slot = self.devices.clone();
                std::thread::spawn(move || {
                    let d = platform::list_devices(&ff);
                    if let Ok(mut s) = slot.lock() {
                        *s = Some(d);
                    }
                });
            }
        }
        let (tx, rx) = channel();
        let ctx2 = ctx.clone();
        let sound = self.settings.sound && !for_record;
        std::thread::spawn(move || {
            if was_visible {
                std::thread::sleep(Duration::from_millis(350));
            }
            let r = if for_record {
                // A frozen picture of every screen to choose the area on.
                platform::capture_all_screens(platform::displays().len())
            } else {
                platform::run_capture(kind, sound).into_iter().collect()
            };
            let _ = tx.send(r);
            ctx2.request_repaint();
        });
        self.job = Some(Job { rx, for_record, show_after: was_visible });
    }

    fn open_file(&mut self, ctx: &egui::Context, p: &Path) {
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if matches!(ext.as_str(), "mp4" | "mov" | "m4v") {
            if self.ffmpeg.is_some() {
                let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Video").to_string();
                let mut v = VideoView::new(p.to_path_buf(), self.ffmpeg.clone(), name, 0.0);
                v.saved = Some(p.to_path_buf());
                v.seek(ctx, 0.0);
                self.ed = None;
                self.video = Some(v);
            } else {
                platform::open_path(p);
            }
            return;
        }
        match image::open(p) {
            Ok(img) => {
                let name = if self.settings.apply_template_reopened {
                    self.settings.make_name("Screenshot")
                } else {
                    p.file_stem().and_then(|s| s.to_str()).unwrap_or("Picture").to_string()
                };
                let mut ed = Editor::new(img.to_rgba8(), name.clone(), self.native_ppp);
                if p.starts_with(&self.settings.save_dir) {
                    ed.saved = Some(p.to_path_buf());
                    ed.saved_name = name;
                }
                self.set_editor(ctx, ed);
            }
            Err(e) => self.toast(format!("Could not open the picture: {e}"), None),
        }
    }

    fn set_editor(&mut self, ctx: &egui::Context, ed: Editor) {
        // A freshly opened picture is not an edit; don't copy it again.
        self.clip_version = ed.version;
        self.clip_due = None;
        let size = ed.doc.size() / ed.ppp;
        let want = vec2((size.x + 60.0).max(980.0), (size.y + 52.0 + 52.0 + 60.0).max(480.0));
        // At most 75% of the usable screen (below the menu bar), centred on it.
        let menu_bar = 25.0;
        let title_bar = 28.0;
        let usable = self.monitor - vec2(0.0, menu_bar);
        let max = (usable * 0.75 - vec2(0.0, title_bar)).max(vec2(900.0, 440.0));
        let inner = want.min(max);
        let outer = inner + vec2(0.0, title_bar);
        self.pending_size = Some(inner);
        self.pending_pos = Some(pos2(
            ((self.monitor.x - outer.x) / 2.0).max(0.0),
            menu_bar + ((usable.y - outer.y) / 2.0).max(0.0),
        ));
        self.video = None;
        self.ed = Some(ed);
        let _ = ctx;
    }

    fn handle_capture(&mut self, ctx: &egui::Context, path: PathBuf, show_after: bool) {
        let name = self.settings.make_name("Screenshot");
        let mut ppp = self.native_ppp;
        // "Shrink Retina snaps": store at normal (non-Retina) size.
        if self.settings.shrink_retina && ppp > 1.01 {
            if let Ok(img) = image::open(&path) {
                let w = (img.width() as f32 / ppp).round().max(1.0) as u32;
                let h = (img.height() as f32 / ppp).round().max(1.0) as u32;
                let small = image::imageops::resize(&img.to_rgba8(), w, h, image::imageops::FilterType::Triangle);
                let _ = small.save(&path);
                ppp = 1.0;
            }
        }
        // Keep a copy of the newest screenshot for "Open Latest Image".
        if let Some(latest) = latest_capture_path() {
            if let Some(d) = latest.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if std::fs::copy(&path, &latest).is_ok() {
                let _ = std::fs::write(latest.with_extension("txt"), &name);
            }
        }
        // "Autosave screenshots" (and the save-and-copy mode) put the capture in the folder right away.
        let mut saved = None;
        if self.settings.autosave || self.settings.after_capture == AfterCapture::SaveAndCopy {
            let dest = self.settings.free_path(&name, self.settings.format.ext());
            let ok = match self.settings.format {
                ImageFormat::Png => std::fs::copy(&path, &dest).is_ok(),
                ImageFormat::Jpg => image::open(&path)
                    .map(|i| save_image(&i.to_rgba8(), &dest, self.settings.jpeg_quality).is_ok())
                    .unwrap_or(false),
            };
            if ok {
                saved = Some(dest);
                self.recent_dirty = true;
                self.tray_menu_dirty = true;
            }
        }
        // Every capture goes straight to the clipboard, ready to paste.
        self.copy_file_to_clipboard(&path, saved.as_deref());
        match self.settings.after_capture {
            AfterCapture::OpenEditor => {
                match image::open(&path) {
                    Ok(img) => {
                        let mut ed = Editor::new(img.to_rgba8(), name.clone(), ppp);
                        if let Some(p) = &saved {
                            ed.saved = Some(p.clone());
                            ed.saved_name = name;
                        }
                        self.set_editor(ctx, ed);
                    }
                    Err(e) => self.toast(format!("Could not read the capture: {e}"), None),
                }
                self.show_window(ctx);
            }
            AfterCapture::SaveAndCopy => {
                if let Some(dest) = saved {
                    platform::notify("Saved", &dest.display().to_string());
                }
                if show_after {
                    self.show_window(ctx);
                }
            }
            AfterCapture::CopyOnly => {
                if show_after {
                    self.show_window(ctx);
                }
            }
        }
        let _ = std::fs::remove_file(&path);
    }

    /// After each edit, put the finished picture on the clipboard (quietly, in the background).
    fn auto_copy_edits(&mut self, ctx: &egui::Context) {
        let Some(ed) = &self.ed else {
            self.clip_due = None;
            return;
        };
        if ed.version == self.clip_version || ed.is_busy() {
            return;
        }
        let now = Instant::now();
        let due = *self.clip_due.get_or_insert(now + Duration::from_millis(350));
        if now < due {
            ctx.request_repaint_after(due - now);
            return;
        }
        self.clip_due = None;
        self.clip_version = ed.version;
        let doc = ed.doc.clone();
        let saved = if self.settings.copy_path { ed.saved.clone() } else { None };
        let image = self.settings.copy_image;
        let tiff = self.settings.clip_format == crate::settings::ClipFormat::Tiff;
        let font = self.font.clone();
        let gen_id = self.clip_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let gen_now = self.clip_gen.clone();
        std::thread::spawn(move || {
            let img = crate::doc::render(&doc, &font, None);
            let tmp = platform::temp_file("autocopy", "png");
            // A newer edit wins: skip this one if another copy was started meanwhile.
            if img.save(&tmp).is_ok() && gen_now.load(std::sync::atomic::Ordering::SeqCst) == gen_id {
                platform::set_clipboard(if image { Some(&tmp) } else { None }, tiff, saved.as_deref());
            }
            let _ = std::fs::remove_file(tmp);
        });
    }

    /// Puts a PNG on the clipboard the way the settings say (image as PNG/TIFF, file path).
    fn copy_file_to_clipboard(&self, png: &Path, saved: Option<&Path>) -> bool {
        let image = if self.settings.copy_image { Some(png) } else { None };
        let path = if self.settings.copy_path { saved } else { None };
        let tiff = self.settings.clip_format == crate::settings::ClipFormat::Tiff;
        platform::set_clipboard(image, tiff, path)
    }

    fn save_picture(&mut self, save_as: bool) -> Option<PathBuf> {
        self.save_picture_ex(save_as, true)
    }

    fn save_picture_ex(&mut self, save_as: bool, close: bool) -> Option<PathBuf> {
        if save_as {
            let fmt = self.settings.format;
            let name = self.ed.as_ref()?.name.clone();
            let d = rfd::AsyncFileDialog::new()
                .set_directory(&self.settings.save_dir)
                .set_file_name(format!("{}.{}", crate::settings::sanitize(&name), fmt.ext()))
                .add_filter("Picture", &["png", "jpg", "jpeg"]);
            let ctx = self.egui_ctx.clone();
            self.spawn_dialog(&ctx, DialogKind::SaveAs, d.save_file());
            return None;
        }
        self.save_picture_at(None, close)
    }

    /// Saves the picture to `dest`, or to its usual place in the save folder.
    fn save_picture_at(&mut self, dest_override: Option<PathBuf>, close: bool) -> Option<PathBuf> {
        let font = self.font.clone();
        let fmt = self.settings.format;
        let q = self.settings.jpeg_quality;
        let ed = self.ed.as_mut()?;
        ed.commit_text();
        let dest = if let Some(d) = dest_override {
            d
        } else {
            match &ed.saved {
                Some(p) if ed.saved_name == ed.name && p.extension().is_some_and(|e| e == fmt.ext()) => p.clone(),
                _ => self.settings.free_path(&ed.name, fmt.ext()),
            }
        };
        let img = ed.flatten(&font);
        match save_image(&img, &dest, q) {
            Ok(()) => {
                ed.saved = Some(dest.clone());
                ed.saved_name = ed.name.clone();
                self.recent_dirty = true;
                self.tray_menu_dirty = true;
                self.toast(format!("Saved to {}", short_path(&dest)), Some(dest.clone()));
                // Like the original: saving finishes the picture and closes the editor.
                if close {
                    self.ed = None;
                    self.close_after_save = true;
                    platform::notify("Saved", &short_path(&dest));
                }
                Some(dest)
            }
            Err(e) => {
                self.toast(format!("Could not save: {e}"), None);
                None
            }
        }
    }

    fn copy_picture(&mut self) {
        let font = self.font.clone();
        let Some(ed) = self.ed.as_mut() else { return };
        ed.commit_text();
        let img = ed.flatten(&font);
        // The path needs a saved file; save first (the editor stays open).
        let saved = if self.settings.copy_path { self.save_picture_ex(false, false) } else { None };
        let tmp = platform::temp_file("copy", "png");
        if img.save(&tmp).is_ok() && self.copy_file_to_clipboard(&tmp, saved.as_deref()) {
            self.toast("Copied to clipboard", saved);
        } else {
            self.toast("Could not copy the picture", None);
        }
        let _ = std::fs::remove_file(tmp);
    }

    fn open_in_external_editor(&mut self) {
        let font = self.font.clone();
        let Some(ed) = self.ed.as_mut() else { return };
        ed.commit_text();
        let img = ed.flatten(&font);
        let tmp = platform::temp_file(&crate::settings::sanitize(&ed.name), "png");
        if img.save(&tmp).is_ok() {
            platform::open_with(&self.settings.external_editor, &tmp);
        }
    }

    fn save_video(&mut self, then_copy: bool) {
        let dir = self.settings.save_dir.clone();
        self.save_video_to(then_copy, dir);
    }

    fn save_video_to(&mut self, then_copy: bool, dir: PathBuf) {
        let Some(v) = &mut self.video else { return };
        if self.video_saving.is_some() {
            return;
        }
        v.stop();
        let fmt = self.settings.video_format;
        let src_ext = v.path.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_lowercase();
        // Without ffmpeg the video can only be copied as it is.
        let ext = if v.ffmpeg.is_some() { fmt.ext().to_string() } else { src_ext.clone() };
        if !v.is_trimmed() && dir == self.settings.save_dir {
            if let Some(saved) = v.saved.clone().filter(|p| p.extension().is_some_and(|e| e == ext.as_str())) {
                if then_copy {
                    platform::copy_file_ref(&saved);
                    self.toast("Copied the video file", Some(saved));
                }
                return;
            }
        }
        let _ = std::fs::create_dir_all(&dir);
        let mut dest = dir.join(format!("{}.{ext}", crate::settings::sanitize(&v.name)));
        let mut n = 2;
        while dest.exists() {
            dest = dir.join(format!("{} ({n}).{ext}", crate::settings::sanitize(&v.name)));
            n += 1;
        }
        let opts = crate::video::ExportOptions {
            trim: if v.is_trimmed() { Some(v.trim) } else { None },
            format: if v.ffmpeg.is_some() { fmt } else { crate::settings::VideoFormat::Mp4 },
            gif_fps: self.settings.gif_fps,
            gif_scale: self.settings.gif_quality as f32 / 100.0,
            same_format: ext == src_ext,
        };
        let (path, ff) = (v.path.clone(), v.ffmpeg.clone());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(VideoView::export(path, ff, opts, dest));
        });
        self.video_saving = Some(rx);
        self.copy_after_save = then_copy;
    }

    fn finish_recording(&mut self, ctx: &egui::Context) {
        let Some(r) = self.recording.take() else { return };
        self.set_tray_recording(false);
        let dest = platform::temp_file("screencast", "mp4");
        let (tx, rx) = channel();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let res = r.rec.finish(dest);
            let _ = tx.send(res);
            ctx2.request_repaint();
        });
        self.finishing = Some(rx);
    }

    fn set_tray_recording(&mut self, on: bool) {
        if self.tray_recording == on {
            return;
        }
        self.tray_recording = on;
        if let Some(t) = &self.tray {
            let (icon, template) = tray_image(on);
            let _ = t.set_icon_with_as_template(icon, template);
            t.set_menu(Some(Box::new(build_menu(on, &self.settings))));
            t.set_title(if on { Some("00:00") } else { None::<&str> });
        }
    }

    // ------------------------------------------------------------ per-frame logic

    fn handle_events(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.events.try_recv() {
            if let AppEvent::Dialog(kind, path) = &ev {
                let Some(p) = path.clone() else { continue };
                match kind {
                    DialogKind::OpenImage => {
                        self.open_file(ctx, &p);
                        self.show_window(ctx);
                    }
                    DialogKind::SaveAs => {
                        self.save_picture_at(Some(p), true);
                    }
                    DialogKind::SaveDir => {
                        self.settings.save_dir = p;
                        self.settings.save();
                        self.recent_dirty = true;
                        self.tray_menu_dirty = true;
                    }
                    DialogKind::EditorApp => {
                        self.settings.external_editor = p.display().to_string();
                        self.settings.save();
                    }
                }
                continue;
            }
            if let AppEvent::Menu(id) = &ev {
                if let Some(path) = id.strip_prefix("recent:") {
                    let p = PathBuf::from(path);
                    self.open_file(ctx, &p);
                    self.show_window(ctx);
                    continue;
                }
                if id == "help" {
                    self.prefs.open();
                    self.prefs.tab = crate::prefs::Tab::Hotkeys;
                    if !self.visible {
                        ctx.send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);
                    }
                    continue;
                }
            }
            let action = match ev {
                AppEvent::Menu(id) => action_from_id(&id, self.settings.delay_secs),
                AppEvent::Hotkey(id) => self.hotkey_map.iter().find(|(i, _)| *i == id).map(|(_, a)| *a),
                AppEvent::Dialog(..) => None,
            };
            if let Some(a) = action {
                // During a recording the record shortcut finishes it.
                let a = if a == Action::Record && self.recording.is_some() { Action::FinishRec } else { a };
                self.run_action(ctx, a);
            }
        }

        if let Some(job) = &self.job {
            if let Ok(res) = job.rx.try_recv() {
                let (for_record, show_after) = (job.for_record, job.show_after);
                self.job = None;
                match (res.first().cloned(), for_record) {
                    (Some(p), false) => self.handle_capture(ctx, p, show_after),
                    (Some(_), true) => {
                        let displays = platform::displays();
                        let mut screens = Vec::new();
                        for (i, p) in res.iter().enumerate() {
                            if let Ok(img) = image::open(p) {
                                let rgba = img.to_rgba8();
                                let d = displays.get(i).copied().unwrap_or(platform::Display {
                                    bounds: Rect::from_min_size(Pos2::ZERO, self.monitor),
                                    scale: rgba.width() as f32 / self.monitor.x.max(1.0),
                                });
                                screens.push(SelScreen {
                                    bounds: d.bounds,
                                    scale: d.scale,
                                    img: Some(ColorImage::from_rgba_unmultiplied(
                                        [rgba.width() as usize, rgba.height() as usize],
                                        rgba.as_raw(),
                                    )),
                                    tex: None,
                                });
                            }
                            let _ = std::fs::remove_file(p);
                        }
                        if screens.is_empty() {
                            self.show_window(ctx);
                            self.toast("Could not read the screen. Allow screen recording in System Settings › Privacy & Security.", None);
                        } else {
                            // Start with a frame of 75% of the screen under the mouse, centred.
                            let mouse = platform::mouse_position().unwrap_or(screens[0].bounds.center());
                            let si = screens.iter().position(|s| s.bounds.contains(mouse)).unwrap_or(0);
                            let b = screens[si].bounds;
                            let rect = Rect::from_center_size(b.center(), b.size() * 0.75);
                            self.select = Some(Select {
                                screens,
                                rect: Some(rect),
                                before_full: None,
                                drag: None,
                                settings_open: false,
                                focused: false,
                            });
                            ctx.send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);
                        }
                    }
                    (None, true) => {
                        self.show_window(ctx);
                        self.toast("Could not read the screen. Allow screen recording in System Settings › Privacy & Security.", None);
                    }
                    (None, false) => {
                        if show_after {
                            self.show_window(ctx);
                        }
                    }
                }
            }
        }

        if let Some(rx) = &self.finishing {
            if let Ok(res) = rx.try_recv() {
                self.finishing = None;
                match res {
                    Ok(path) => {
                        let mut v = VideoView::new(path, self.ffmpeg.clone(), self.rec_started_name.clone(), 0.0);
                        v.seek(ctx, 0.0);
                        if v.size[0] > 0 {
                            let sz = vec2(v.size[0] as f32, v.size[1] as f32) / self.native_ppp;
                            let want = vec2((sz.x + 60.0).max(900.0), (sz.y + 52.0 + 64.0 + 40.0).max(480.0));
                            self.pending_size = Some(want.min(self.monitor * 0.92));
                        }
                        self.ed = None;
                        self.video = Some(v);
                        if self.settings.autosave_movies {
                            let movies = dirs::video_dir()
                                .or_else(|| dirs::home_dir().map(|h| h.join("Movies")))
                                .unwrap_or_else(|| self.settings.save_dir.clone());
                            self.save_video_to(false, movies);
                        }
                    }
                    Err(e) => self.toast(e, None),
                }
                self.show_window(ctx);
            }
        }

        if let Some(rx) = &self.video_saving {
            if let Ok(res) = rx.try_recv() {
                self.video_saving = None;
                match res {
                    Ok(p) => {
                        if let Some(v) = &mut self.video {
                            if !v.is_trimmed() || true {
                                v.saved = Some(p.clone());
                            }
                        }
                        if self.copy_after_save {
                            platform::copy_file_ref(&p);
                            self.toast("Saved and copied the video file", Some(p));
                        } else {
                            self.toast(format!("Saved to {}", short_path(&p)), Some(p));
                        }
                        self.recent_dirty = true;
                self.tray_menu_dirty = true;
                    }
                    Err(e) => self.toast(e, None),
                }
            }
        }

        // Keep the menu-bar list of recent snaps current.
        if self.tray_menu_dirty && self.recording.is_none() {
            self.tray_menu_dirty = false;
            if let Some(t) = &self.tray {
                t.set_menu(Some(Box::new(build_menu(false, &self.settings))));
            }
        }

        // Recording timer in the menu bar, once per second.
        if let Some(r) = &mut self.recording {
            if let Some(err) = r.rec.poll_failure() {
                let r = self.recording.take().unwrap();
                r.rec.cancel();
                self.set_tray_recording(false);
                self.show_window(ctx);
                self.toast(err, None);
            } else {
                let secs = r.rec.elapsed().as_secs();
                if secs != r.last_secs {
                    r.last_secs = secs;
                    let paused = r.rec.is_paused();
                    let title = format!("{}{}", if paused { "❚❚ " } else { "" }, record::format_elapsed(r.rec.elapsed()));
                    if let Some(t) = &self.tray {
                        t.set_title(Some(title));
                    }
                }
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }
    }

    // ------------------------------------------------------------ UI pieces

    fn top_bar(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut action = None;
        let ctx = ui.ctx().clone();
        ui.spacing_mut().item_spacing.x = 10.0;
        centered_row(ui, BAR_ITEM_H, |ui| {
            ui.add_space(6.0);
            let has_ed = self.ed.is_some();
            if self.video.is_none() {
                group(ui, 2, TOOL_W, BAR_ITEM_H, |ui| {
                    let (cu, cr) = self.ed.as_ref().map(|e| (e.can_undo(), e.can_redo())).unwrap_or((false, false));
                    if icon_button(ui, Icon::Undo, false, cu, vec2(TOOL_W, BAR_ITEM_H - 2.0), "Undo (⌘Z)").clicked() {
                        if let Some(e) = &mut self.ed {
                            e.undo();
                        }
                    }
                    if icon_button(ui, Icon::Redo, false, cr, vec2(TOOL_W, BAR_ITEM_H - 2.0), "Redo (⇧⌘Z)").clicked() {
                        if let Some(e) = &mut self.ed {
                            e.redo();
                        }
                    }
                });
            }
            let cam = group(ui, 1, TOOL_W, BAR_ITEM_H, |ui| icon_button(ui, Icon::CameraPlus, false, !self.busy(), vec2(TOOL_W, BAR_ITEM_H - 2.0), "New capture"))
                .inner;
            egui::Popup::menu(&cam).show(|ui| {
                ui.set_min_width(230.0);
                if let Some(a) = capture_menu(ui, &self.settings) {
                    action = Some(a);
                }
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(6.0);
                // Copy + menu, joined like one control.
                ui.spacing_mut().item_spacing.x = 1.0;
                let menu_btn = colored_icon_button(ui, Icon::Menu, BLUE, vec2(32.0, BAR_ITEM_H), "More");
                egui::Popup::menu(&menu_btn).show(|ui| {
                    ui.set_min_width(200.0);
                    if let Some(a) = self.more_menu(ui) {
                        action = Some(a);
                    }
                });
                let can_copy = has_ed || self.video.is_some();
                let copy = text_button(ui, "Copy", BLUE, BLUE_HOVER, can_copy, vec2(62.0, BAR_ITEM_H));
                if copy.clicked() {
                    if has_ed {
                        self.copy_picture();
                    } else {
                        self.save_video(true);
                    }
                }
                copy.on_hover_text("Copy to the clipboard (⌘C). Nothing is uploaded.");
                ui.spacing_mut().item_spacing.x = 10.0;
                let saving = self.video_saving.is_some();
                let label = if saving { "Saving…" } else { "Save" };
                let save = text_button(ui, label, BTN, BTN_HOVER, can_copy && !saving, vec2(58.0, BAR_ITEM_H));
                if save.clicked() {
                    if has_ed {
                        self.save_picture(false);
                    } else {
                        self.save_video(false);
                    }
                }
                save.on_hover_text(format!("Save to {} (⌘S)", short_path(&self.settings.save_dir)));

                // File name with its extension.
                let ext = if let Some(v) = &self.video {
                    if v.is_trimmed() { "mp4".to_string() } else { v.path.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_string() }
                } else {
                    self.settings.format.ext().to_string()
                };
                ui.spacing_mut().item_spacing.x = 0.0;
                let field_bg = Color32::from_rgb(36, 36, 38);
                let ext_text = format!(".{ext}");
                let ext_w = 12.0 + 7.5 * ext_text.len() as f32;
                let (ext_rect, ext_resp) = ui.allocate_exact_size(vec2(ext_w, BAR_ITEM_H), Sense::click());
                let er = CornerRadius { nw: 0, sw: 0, ne: 5, se: 5 };
                ui.painter().rect_filled(ext_rect, er, field_bg);
                ui.painter().rect_stroke(ext_rect, er, Stroke::new(1.0, EDGE), StrokeKind::Inside);
                ui.painter().text(ext_rect.center(), Align2::CENTER_CENTER, ext_text, FontId::proportional(BAR_FONT), DIM_TEXT);
                if self.ed.is_some() {
                    let ext_resp = ext_resp.on_hover_text("Click to switch between PNG and JPG");
                    if ext_resp.clicked() {
                        self.settings.format = match self.settings.format {
                            ImageFormat::Png => ImageFormat::Jpg,
                            ImageFormat::Jpg => ImageFormat::Png,
                        };
                        self.settings.save();
                    }
                }
                let w = (ui.available_width() - 6.0).max(80.0);
                let (name_rect, _) = ui.allocate_exact_size(vec2(w, BAR_ITEM_H), Sense::hover());
                let nr = CornerRadius { nw: 5, sw: 5, ne: 0, se: 0 };
                ui.painter().rect_filled(name_rect, nr, field_bg);
                ui.painter().rect_stroke(name_rect, nr, Stroke::new(1.0, EDGE), StrokeKind::Inside);
                let name: Option<&mut String> = if let Some(e) = &mut self.ed {
                    Some(&mut e.name)
                } else if let Some(v) = &mut self.video {
                    Some(&mut v.name)
                } else {
                    None
                };
                match name {
                    Some(n) => {
                        ui.put(
                            Rect::from_center_size(name_rect.center() + vec2(0.0, 1.5), vec2(name_rect.width() - 20.0, 18.0)),
                            egui::TextEdit::singleline(n)
                                .frame(Frame::NONE)
                                .margin(Margin::ZERO)
                                .horizontal_align(Align::Center)
                                .font(FontId::proportional(BAR_FONT))
                                .text_color(Color32::from_rgb(200, 200, 205))
                                .desired_width(f32::INFINITY),
                        )
                        .on_hover_text("File name");
                    }
                    None => {
                        ui.painter().text(name_rect.center(), Align2::CENTER_CENTER, "LightSnap", FontId::proportional(BAR_FONT), DIM_TEXT);
                    }
                }
                let _ = &ctx;
            });
        });
        action
    }

    fn more_menu(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut action = None;
        if self.ed.is_some() && ui.button("Save As…").clicked() {
            self.save_picture(true);
        }
        if self.ed.is_some() {
            let app = crate::prefs::app_name(&self.settings.external_editor);
            if ui.button(format!("Open in {app}")).clicked() {
                self.open_in_external_editor();
            }
        }
        let saved = self.ed.as_ref().and_then(|e| e.saved.clone()).or_else(|| self.video.as_ref().and_then(|v| v.saved.clone()));
        if let Some(p) = saved {
            if ui.button("Show in Finder").clicked() {
                platform::reveal_path(&p);
            }
            if ui.button("Copy file path").clicked() {
                platform::copy_text(&p.display().to_string());
                self.toast("Copied the file path", None);
            }
        }
        if self.video.is_some() {
            if let Some(v) = &self.video {
                if ui.button("Open in QuickTime Player").clicked() {
                    platform::open_path(v.saved.as_ref().unwrap_or(&v.path));
                }
            }
        }
        if self.ed.is_some() || self.video.is_some() {
            if ui.add(egui::Button::new("Close").shortcut_text("⌘W")).clicked() {
                self.ed = None;
                self.video = None;
                self.recent_dirty = true;
                self.tray_menu_dirty = true;
            }
            ui.separator();
        }
        if ui.button("Open Picture…").clicked() {
            action = Some(Action::OpenImage);
        }
        if ui.button("Open Saved Snaps Folder").clicked() {
            action = Some(Action::OpenFolder);
        }
        if ui.button("Settings…").clicked() {
            action = Some(Action::Settings);
        }
        ui.separator();
        if ui.button("Quit LightSnap").clicked() {
            action = Some(Action::Quit);
        }
        action
    }

    fn bottom_bar(&mut self, ui: &mut Ui) {
        let enabled = self.ed.is_some();
        centered_row(ui, TOOL_H, |ui| {
            ui.add_space(14.0);
            ui.spacing_mut().item_spacing.x = 10.0;
            // Colour
            let [r, g, b, a] = self.settings.color;
            let mut color = Color32::from_rgba_unmultiplied(r, g, b, a);
            let (rect, swatch) = ui.allocate_exact_size(vec2(44.0, TOOL_H), if enabled { Sense::click() } else { Sense::hover() });
            ui.painter().rect_filled(rect, 5.0, BTN);
            ui.painter().rect_stroke(rect, 5.0, Stroke::new(1.0, EDGE), StrokeKind::Inside);
            ui.painter().rect_filled(rect.shrink(4.0), 3.0, if enabled { color } else { color.gamma_multiply(0.4) });
            let swatch = swatch.on_hover_text("Colour");
            let mut changed = false;
            egui::Popup::menu(&swatch).show(|ui| {
                ui.horizontal(|ui| {
                    for (i, c) in PALETTE.iter().enumerate() {
                        if i == 5 {
                            ui.end_row();
                        }
                        let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
                        ui.painter().rect_filled(r, 4.0, *c);
                        if *c == color {
                            ui.painter().rect_stroke(r.expand(2.0), 5.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
                        }
                        if resp.clicked() {
                            color = *c;
                            changed = true;
                            ui.close();
                        }
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Custom:");
                    if ui.color_edit_button_srgba(&mut color).changed() {
                        changed = true;
                    }
                });
            });

            // Thickness
            let mut width = self.settings.width;
            ui.add_enabled_ui(enabled, |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (r, _) = ui.allocate_exact_size(vec2(10.0, TOOL_H), Sense::hover());
                ui.painter().circle_filled(r.center(), 3.0, DIM_TEXT);
                ui.spacing_mut().slider_width = 100.0;
                if ui.add(egui::Slider::new(&mut width, 1.0..=20.0).show_value(false)).on_hover_text("Thickness and text size").changed() {
                    changed = true;
                }
                let (r, _) = ui.allocate_exact_size(vec2(18.0, TOOL_H), Sense::hover());
                ui.painter().circle_filled(r.center(), 7.5, DIM_TEXT);
            });
            if changed {
                self.settings.color = color.to_srgba_unmultiplied();
                self.settings.width = width;
                let style = self.style();
                if let Some(e) = &mut self.ed {
                    e.restyle_selected(style);
                }
            }

            let sz = vec2(TOOL_W, TOOL_H - 2.0);
            let mut tool_btn = |ui: &mut Ui, icon: Icon, tool: Tool, this: &mut Self| {
                if icon_button(ui, icon, this.tool == tool && enabled, enabled, sz, tool.tip()).clicked() {
                    this.tool = tool;
                    if let Some(e) = &mut this.ed {
                        e.commit_text();
                        if tool != Tool::Crop {
                            e.crop = None;
                        }
                    }
                }
            };
            group(ui, 5, sz.x, TOOL_H, |ui| {
                tool_btn(ui, Icon::Arrow, Tool::Arrow, self);
                tool_btn(ui, Icon::Rect, Tool::Rect, self);
                tool_btn(ui, Icon::Line, Tool::Line, self);
                tool_btn(ui, Icon::Ellipse, Tool::Ellipse, self);
                tool_btn(ui, Icon::Pen, Tool::Pen, self);
            });
            group(ui, 2, sz.x, TOOL_H, |ui| {
                tool_btn(ui, Icon::Text, Tool::Text, self);
                tool_btn(ui, Icon::Callout, Tool::Callout, self);
            });
            group(ui, 3, sz.x, TOOL_H, |ui| {
                tool_btn(ui, Icon::Blur, Tool::Blur, self);
                tool_btn(ui, Icon::Crop, Tool::Crop, self);
                if icon_button(ui, Icon::Resize, false, enabled, sz, "Resize picture").clicked() {
                    if let Some(e) = &mut self.ed {
                        e.resize = Some(ResizeDlg { w: e.doc.base.width(), h: e.doc.base.height(), keep: true });
                    }
                }
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(14.0);
                let wide = ui.available_width() > 130.0;
                if let Some(e) = self.ed.as_mut().filter(|_| wide) {
                    let r = ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!(
                                "{} × {}   {}%",
                                e.doc.base.width(),
                                e.doc.base.height(),
                                (e.zoom * e.ppp * 100.0).round()
                            ))
                            .color(DIM_TEXT),
                        )
                        .sense(Sense::click()),
                    );
                    if r.on_hover_text("Pinch to zoom, two fingers to move. Click or ⌘0 to fit the window.").clicked() {
                        e.reset_view();
                    }
                }
            });
        });
    }

    fn home(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut action = None;
        let rect = ui.max_rect();
        if let Some(ch) = &self.checker {
            ui.painter().image(
                ch.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(rect.width() / 16.0, rect.height() / 16.0)),
                Color32::WHITE,
            );
        }
        if self.recent_dirty {
            self.recent_dirty = false;
            self.recent = recent_files(&self.settings.save_dir, 6);
        }
        let card_w = 560.0f32.min(rect.width() - 40.0);
        let card = Rect::from_center_size(rect.center(), vec2(card_w, (rect.height() - 40.0).min(430.0)));
        ui.scope_builder(egui::UiBuilder::new().max_rect(card), |ui| {
            Frame::new()
                .fill(Color32::from_rgba_unmultiplied(30, 30, 32, 235))
                .stroke(Stroke::new(1.0, Color32::from_gray(64)))
                .corner_radius(10)
                .inner_margin(Margin::same(22))
                .show(ui, |ui| {
                    ui.set_width(card_w - 44.0);
                    ui.label(egui::RichText::new("Capture something").size(18.0).color(TEXT).strong());
                    ui.label(
                        egui::RichText::new("Screenshots and screen recordings stay on this Mac. Nothing is uploaded.")
                            .color(DIM_TEXT),
                    );
                    ui.add_space(14.0);
                    let items = [
                        ("Capture Area", self.settings.hotkey_short("area"), Action::Capture(CaptureKind::Area)),
                        ("Capture Full Screen", self.settings.hotkey_short("full"), Action::Capture(CaptureKind::FullScreen)),
                        ("Capture Window", self.settings.hotkey_short("window"), Action::Capture(CaptureKind::Window)),
                        ("Record Video", self.settings.hotkey_short("record"), Action::Record),
                    ];
                    let bw = (card_w - 44.0 - 10.0) / 2.0;
                    egui::Grid::new("home_grid").spacing(vec2(10.0, 10.0)).show(ui, |ui| {
                        for (i, (label, key, a)) in items.iter().enumerate() {
                            if big_button(ui, label, key, bw, !self.busy(), *a == Action::Record).clicked() {
                                action = Some(*a);
                            }
                            if i % 2 == 1 {
                                ui.end_row();
                            }
                        }
                        let timed = format!("Timed Capture ({} s)", self.settings.delay_secs);
                        if big_button(ui, &timed, &self.settings.hotkey_short("timed"), bw, !self.busy(), false).clicked() {
                            action = Some(Action::Capture(CaptureKind::Timed(self.settings.delay_secs)));
                        }
                        if big_button(ui, "Open Picture…", "", bw, true, false).clicked() {
                            action = Some(Action::OpenImage);
                        }
                        ui.end_row();
                    });
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Recent").color(DIM_TEXT).strong());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.link("Open folder").clicked() {
                                action = Some(Action::OpenFolder);
                            }
                        });
                    });
                    if self.recent.is_empty() {
                        ui.label(egui::RichText::new("Nothing saved yet.").color(DIM_TEXT).italics());
                    }
                    let mut open = None;
                    for p in &self.recent {
                        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                        if ui.add(egui::Button::new(egui::RichText::new(name).color(TEXT)).frame(false)).clicked() {
                            open = Some(p.clone());
                        }
                    }
                    if let Some(p) = open {
                        let ctx = ui.ctx().clone();
                        self.open_file(&ctx, &p);
                    }
                });
        });
        action
    }

    fn video_view(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some(v) = &mut self.video else { return };
        let rect = ui.max_rect();
        if let Some(ch) = &self.checker {
            ui.painter().image(
                ch.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(rect.width() / 16.0, rect.height() / 16.0)),
                Color32::WHITE,
            );
        }
        if let Some(f) = v.take_frame() {
            match &mut v.tex {
                Some(t) => t.set(f, TextureOptions::LINEAR),
                None => v.tex = Some(ctx.load_texture("video", f, TextureOptions::LINEAR)),
            }
        }
        if v.tick() {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        if let Some(t) = &v.tex {
            let size = t.size_vec2();
            let avail = rect.size() - vec2(24.0, 24.0);
            let z = (avail.x / size.x).min(avail.y / size.y).min(1.0 / self.native_ppp.max(1.0) * (v.size[0] as f32 / size.x).max(1.0));
            let r = Rect::from_center_size(rect.center(), size * z);
            ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        } else {
            let msg = if v.can_preview() {
                "Loading preview…"
            } else {
                "Preview and trimming need ffmpeg (brew install ffmpeg). You can still save the video or open it in QuickTime Player."
            };
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, msg, FontId::proportional(15.0), DIM_TEXT);
        }
    }

    fn video_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some(v) = &mut self.video else { return };
        let can = v.can_preview();
        // Same sizes as the picture editor's tool bar: 26-point controls on one centre line.
        centered_row(ui, TOOL_H, |ui| {
            ui.add_space(14.0);
            let icon = if v.is_playing() { Icon::Pause } else { Icon::Play };
            let tip = if v.is_playing() { "Pause (Space)" } else { "Play (Space)" };
            let resp = group(ui, 1, TOOL_W, TOOL_H, |ui| icon_button(ui, icon, false, can, vec2(TOOL_W, TOOL_H - 2.0), tip)).inner;
            if resp.clicked() || (can && ui.input(|i| i.key_pressed(Key::Space)) && !ctx.egui_wants_keyboard_input()) {
                if v.is_playing() {
                    v.stop();
                } else {
                    v.play(&ctx);
                }
            }
            ui.add_space(12.0);
            let time_w = 110.0;
            let tl_w = (ui.available_width() - time_w - 24.0).max(100.0);
            let (tl, tl_resp) = ui.allocate_exact_size(vec2(tl_w, TOOL_H), Sense::click_and_drag());
            let p = ui.painter();
            let x0 = tl.min.x + 8.0;
            let x1 = tl.max.x - 8.0;
            let dur = v.duration.max(0.001);
            let to_x = |t: f32| x0 + (x1 - x0) * (t / dur).clamp(0.0, 1.0);
            let to_t = |x: f32| ((x - x0) / (x1 - x0)).clamp(0.0, 1.0) * dur;
            let cy = tl.center().y;
            p.rect_filled(Rect::from_min_max(pos2(x0, cy - 2.0), pos2(x1, cy + 2.0)), 2.0, Color32::from_gray(80));
            p.rect_filled(
                Rect::from_min_max(pos2(to_x(v.trim.0), cy - 2.0), pos2(to_x(v.trim.1), cy + 2.0)),
                2.0,
                Color32::from_gray(185),
            );
            // Trim handles
            let handle = |t: f32| Rect::from_center_size(pos2(to_x(t), cy), vec2(7.0, TOOL_H - 4.0));
            for t in [v.trim.0, v.trim.1] {
                let h = handle(t);
                p.rect_filled(h, 2.0, Color32::from_gray(205));
                p.rect_stroke(h, 2.0, Stroke::new(1.0, Color32::from_gray(90)), StrokeKind::Inside);
                p.line_segment([h.center_top() + vec2(0.0, 6.0), h.center_bottom() - vec2(0.0, 6.0)], Stroke::new(1.0, Color32::from_gray(110)));
            }
            // Playhead
            let ph = pos2(to_x(v.pos), cy);
            p.circle_filled(ph, 8.0, Color32::WHITE);
            p.circle_filled(ph, 4.5, RED);

            let drag_id = Id::new("tl_drag");
            if can {
                if tl_resp.drag_started() {
                    let px = ui.input(|i| i.pointer.press_origin()).map(|q| q.x).unwrap_or(0.0);
                    let which = if (px - to_x(v.trim.0)).abs() < 10.0 {
                        1u8
                    } else if (px - to_x(v.trim.1)).abs() < 10.0 {
                        2
                    } else {
                        0
                    };
                    ui.data_mut(|d| d.insert_temp(drag_id, which));
                }
                if tl_resp.dragged() || tl_resp.clicked() {
                    if let Some(q) = tl_resp.interact_pointer_pos() {
                        let which: u8 = if tl_resp.clicked() { 0 } else { ui.data(|d| d.get_temp(drag_id).unwrap_or(0)) };
                        let t = to_t(q.x);
                        match which {
                            1 => {
                                v.trim.0 = t.min(v.trim.1 - 0.2).max(0.0);
                                v.seek(&ctx, v.trim.0);
                            }
                            2 => {
                                v.trim.1 = t.max(v.trim.0 + 0.2).min(dur);
                                v.seek(&ctx, v.trim.1);
                            }
                            _ => {
                                let t = t.clamp(v.trim.0, v.trim.1);
                                v.seek(&ctx, t);
                            }
                        }
                    }
                }
                tl_resp.on_hover_text("Drag the grey handles to choose where the video starts and ends");
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(20.0);
                let label = if v.is_trimmed() {
                    format!(
                        "{} / {}",
                        record::format_elapsed(Duration::from_secs_f32((v.pos - v.trim.0).max(0.0))),
                        record::format_elapsed(Duration::from_secs_f32(v.trim.1 - v.trim.0))
                    )
                } else {
                    format!(
                        "{} / {}",
                        record::format_elapsed(Duration::from_secs_f32(v.pos)),
                        record::format_elapsed(Duration::from_secs_f32(v.duration))
                    )
                };
                ui.label(egui::RichText::new(label).size(BAR_FONT).color(TEXT));
            });
        });
    }

    fn resize_window(&mut self, ctx: &egui::Context) {
        let font = self.font.clone();
        let Some(ed) = &mut self.ed else { return };
        let Some(dlg) = &mut ed.resize else { return };
        let (ow, oh) = (ed.doc.base.width() as f32, ed.doc.base.height() as f32);
        let mut apply = None;
        let mut close = false;
        let resp = egui::Modal::new(Id::new("resize")).show(ctx, |ui| {
            ui.heading("Resize picture");
            ui.add_space(6.0);
            egui::Grid::new("rs").num_columns(2).show(ui, |ui| {
                ui.label("Width");
                if ui.add(egui::DragValue::new(&mut dlg.w).range(1..=20000).suffix(" px")).changed() && dlg.keep {
                    dlg.h = ((dlg.w as f32) * oh / ow).round().max(1.0) as u32;
                }
                ui.end_row();
                ui.label("Height");
                if ui.add(egui::DragValue::new(&mut dlg.h).range(1..=20000).suffix(" px")).changed() && dlg.keep {
                    dlg.w = ((dlg.h as f32) * ow / oh).round().max(1.0) as u32;
                }
                ui.end_row();
            });
            ui.checkbox(&mut dlg.keep, "Keep proportions");
            ui.horizontal(|ui| {
                for pct in [25u32, 50, 75] {
                    if ui.button(format!("{pct}%")).clicked() {
                        dlg.w = (ow * pct as f32 / 100.0).round() as u32;
                        dlg.h = (oh * pct as f32 / 100.0).round() as u32;
                    }
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Resize").clicked() {
                    apply = Some((dlg.w, dlg.h));
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if let Some((w, h)) = apply {
            ed.resize = None;
            ed.apply_resize(&font, w, h);
        } else if close || resp.should_close() {
            ed.resize = None;
        }
    }

    fn editor_shortcuts(&mut self, ctx: &egui::Context) {
        if self.ed.is_none() {
            return;
        }
        let editing_text = self.ed.as_ref().is_some_and(|e| e.is_editing_text());
        let typing = ctx.egui_wants_keyboard_input() || editing_text;
        let (undo, redo, save, copy, del, esc) = ctx.input_mut(|i| {
            let redo = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
            let undo = i.consume_key(Modifiers::COMMAND, Key::Z);
            let save = i.consume_key(Modifiers::COMMAND, Key::S);
            let copy = !typing && i.events.iter().any(|e| matches!(e, egui::Event::Copy));
            let del = !typing && (i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace));
            let esc = !typing && i.key_pressed(Key::Escape);
            (undo, redo, save, copy, del, esc)
        });
        if !typing {
            let keys = [
                (Key::A, Tool::Arrow),
                (Key::R, Tool::Rect),
                (Key::L, Tool::Line),
                (Key::O, Tool::Ellipse),
                (Key::P, Tool::Pen),
                (Key::T, Tool::Text),
                (Key::C, Tool::Callout),
                (Key::B, Tool::Blur),
                (Key::X, Tool::Crop),
            ];
            for (k, t) in keys {
                if ctx.input(|i| i.key_pressed(k) && i.modifiers.is_none()) {
                    self.tool = t;
                }
            }
        }
        let (fit, zin, zout) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::Num0),
                i.consume_key(Modifiers::COMMAND, Key::Plus) || i.consume_key(Modifiers::COMMAND, Key::Equals),
                i.consume_key(Modifiers::COMMAND, Key::Minus),
            )
        });
        if let Some(e) = &mut self.ed {
            if fit {
                e.reset_view();
            }
            if zin {
                e.zoom_by(1.25);
            }
            if zout {
                e.zoom_by(0.8);
            }
        }
        if save {
            self.save_picture(false);
        }
        if copy {
            self.copy_picture();
        }
        let Some(ed) = &mut self.ed else { return };
        if undo && !typing {
            ed.undo();
        }
        if redo && !typing {
            ed.redo();
        }
        if del {
            ed.delete_selected();
        }
        if esc && ed.crop.is_none() {
            ed.selected = None;
        }
    }

    // ------------------------------------------------------------ recording overlays

    fn select_overlay(&mut self, ctx: &egui::Context) -> SelectOut {
        let Some(sel) = &mut self.select else { return SelectOut::None };
        let settings = &mut self.settings;
        let devices = &self.devices;
        let has_ff = self.ffmpeg.is_some();
        let Select { screens, rect, before_full, drag, settings_open, focused } = sel;
        let all = screens.iter().fold(Rect::NOTHING, |a, s| a.union(s.bounds));
        let bounds_list: Vec<Rect> = screens.iter().map(|s| s.bounds).collect();
        // The screen that holds the middle of the area gets the Record button.
        let main_i = rect
            .map(|r| {
                let c = r.center();
                bounds_list.iter().position(|b| b.contains(c)).unwrap_or_else(|| {
                    (0..bounds_list.len())
                        .min_by(|a, b| {
                            bounds_list[*a].distance_sq_to_pos(c).total_cmp(&bounds_list[*b].distance_sq_to_pos(c))
                        })
                        .unwrap_or(0)
                })
            })
            .unwrap_or(0);
        let mouse_i = platform::mouse_position()
            .and_then(|m| bounds_list.iter().position(|b| b.contains(m)))
            .unwrap_or(0);
        let mut out = SelectOut::None;
        let mut any_focus_sent = *focused;

        for (si, scr) in screens.iter_mut().enumerate() {
            let builder = ViewportBuilder::default()
                .with_title("Select area to record")
                .with_decorations(false)
                .with_position(scr.bounds.min)
                .with_inner_size(scr.bounds.size())
                .with_window_level(WindowLevel::AlwaysOnTop)
                .with_resizable(false)
                .with_taskbar(false)
                .with_has_shadow(false);
            let res = crate::show_immediate(ctx, ViewportId::from_hash_of(("lightsnap_select", si)), builder, |ui, _| {
                let ctx = ui.ctx().clone();
                if !any_focus_sent && si == mouse_i {
                    any_focus_sent = true;
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                if let Some(img) = scr.img.take() {
                    scr.tex = Some(ctx.load_texture(format!("select_snapshot_{si}"), img, TextureOptions::LINEAR));
                }
                let offset = ctx
                    .input(|i| i.viewport().inner_rect.map(|r| r.min.to_vec2()))
                    .unwrap_or(scr.bounds.min.to_vec2());
                let full = ui.max_rect();
                let painter = ui.painter_at(full);
                let screen_rect = scr.bounds.translate(-offset);
                if let Some(t) = &scr.tex {
                    painter.image(t.id(), screen_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                } else {
                    painter.rect_filled(full, 0.0, Color32::from_gray(30));
                }
                let to_local = |r: Rect| r.translate(-offset);
                let resp = ui.interact(full, Id::new(("sel_bg", si)), Sense::click_and_drag());
                let pointer = resp.interact_pointer_pos().map(|p| p + offset);

                if resp.drag_started() {
                    let p = ui.input(|i| i.pointer.press_origin()).unwrap_or_default() + offset;
                    *settings_open = false;
                    *drag = Some(match *rect {
                        Some(r) => {
                            let e = edges_near(r, p, 8.0);
                            if e != 0 {
                                SelDrag::Resize(e, r)
                            } else if r.contains(p) {
                                SelDrag::Move(p, r)
                            } else {
                                SelDrag::New(p)
                            }
                        }
                        None => SelDrag::New(p),
                    });
                }
                if resp.dragged() {
                    if let (Some(d), Some(p)) = (*drag, pointer) {
                        *rect = Some(match d {
                            SelDrag::New(s) => Rect::from_two_pos(s, p).intersect(all),
                            SelDrag::Move(s, r) => {
                                // The area can move across all screens.
                                let mut nr = r.translate(p - s);
                                let dx = (all.min.x - nr.min.x).max(0.0) + (all.max.x - nr.max.x).min(0.0);
                                let dy = (all.min.y - nr.min.y).max(0.0) + (all.max.y - nr.max.y).min(0.0);
                                nr = nr.translate(vec2(dx, dy));
                                nr
                            }
                            SelDrag::Resize(e, r) => {
                                let mut nr = r;
                                if e & 1 != 0 { nr.min.x = p.x.min(nr.max.x - 20.0); }
                                if e & 2 != 0 { nr.max.x = p.x.max(nr.min.x + 20.0); }
                                if e & 4 != 0 { nr.min.y = p.y.min(nr.max.y - 20.0); }
                                if e & 8 != 0 { nr.max.y = p.y.max(nr.min.y + 20.0); }
                                nr.intersect(all)
                            }
                        });
                        // The other screens show the area too.
                        ctx.request_repaint();
                    }
                }
                if resp.drag_stopped() {
                    *drag = None;
                    if rect.is_some_and(|r| r.width() < 16.0 || r.height() < 16.0) {
                        *rect = None;
                    }
                }
                if let (Some(r), Some(h)) = (*rect, resp.hover_pos().map(|p| p + offset)) {
                    let icon = match edges_near(r, h, 8.0) {
                        1 | 2 => egui::CursorIcon::ResizeHorizontal,
                        4 | 8 => egui::CursorIcon::ResizeVertical,
                        5 | 10 => egui::CursorIcon::ResizeNwSe,
                        6 | 9 => egui::CursorIcon::ResizeNeSw,
                        _ if r.contains(h) => egui::CursorIcon::Move,
                        _ => egui::CursorIcon::Crosshair,
                    };
                    ctx.set_cursor_icon(icon);
                } else {
                    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                }

                // Dim everything outside the area.
                let dim = Color32::from_black_alpha(120);
                let mut out = SelectOut::None;
                match *rect {
                    Some(r) => {
                        let lr = to_local(r);
                        for d in [
                            Rect::from_min_max(full.min, pos2(full.max.x, lr.min.y)),
                            Rect::from_min_max(pos2(full.min.x, lr.max.y), full.max),
                            Rect::from_min_max(pos2(full.min.x, lr.min.y), pos2(lr.min.x, lr.max.y)),
                            Rect::from_min_max(pos2(lr.max.x, lr.min.y), pos2(full.max.x, lr.max.y)),
                        ] {
                            let d = d.intersect(full);
                            if d.is_positive() {
                                painter.rect_filled(d, 0.0, dim);
                            }
                        }
                        painter.rect_stroke(lr, 0.0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Outside);
                        for c in [lr.left_top(), lr.right_top(), lr.left_bottom(), lr.right_bottom(), lr.center_top(), lr.center_bottom(), lr.left_center(), lr.right_center()] {
                            painter.circle_filled(c, 5.0, Color32::WHITE);
                            painter.circle_stroke(c, 5.0, Stroke::new(1.0, Color32::from_gray(60)));
                        }
                        if si == main_i && drag.is_none() {
                            // Top right: the size, and a button for a full-screen recording.
                            let fs = Rect::from_min_size(lr.right_top() + vec2(-8.0 - 26.0, 8.0), vec2(26.0, 26.0));
                            let fs_resp = ui.interact(fs, Id::new(("fullscreen_btn", si)), Sense::click());
                            let is_full = (r.min - scr.bounds.min).length() < 1.0 && (r.max - scr.bounds.max).length() < 1.0;
                            painter.rect_filled(fs, 4.0, if fs_resp.hovered() || is_full { BLUE } else { Color32::from_black_alpha(190) });
                            icons::draw(&painter, fs.shrink(5.0), Icon::Resize, Color32::WHITE);
                            if fs_resp
                                .on_hover_text(if is_full { "Back to the chosen area" } else { "Record the whole screen" })
                                .clicked()
                            {
                                if is_full {
                                    *rect = Some(before_full.take().unwrap_or_else(|| {
                                        Rect::from_center_size(scr.bounds.center(), scr.bounds.size() * 0.75)
                                    }));
                                } else {
                                    *before_full = Some(r);
                                    *rect = Some(scr.bounds);
                                }
                                ctx.request_repaint();
                            }
                            let label = format!("{} x {}", (r.width() * scr.scale).round(), (r.height() * scr.scale).round());
                            let galley = painter.layout_no_wrap(label, FontId::proportional(13.0), Color32::WHITE);
                            let lrect = Rect::from_min_size(
                                pos2(fs.min.x - galley.size().x - 16.0 - 4.0, fs.min.y),
                                vec2(galley.size().x + 16.0, 26.0),
                            );
                            painter.rect_filled(lrect, 4.0, Color32::from_black_alpha(190));
                            painter.galley(pos2(lrect.min.x + 8.0, lrect.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
                        }
                    }
                    None => {
                        painter.rect_filled(full, 0.0, dim);
                        if si == mouse_i {
                            let msg = "Drag to choose the area to record   ·   Enter: whole screen   ·   Esc: cancel";
                            let galley = painter.layout_no_wrap(msg.into(), FontId::proportional(13.0), Color32::WHITE);
                            let r = Rect::from_center_size(full.center(), galley.size() + vec2(32.0, 20.0));
                            painter.rect_filled(r, 10.0, Color32::from_black_alpha(200));
                            painter.galley(r.min + vec2(16.0, 10.0), galley, Color32::WHITE);
                        }
                    }
                }

                // Record button and options, on the screen that holds the area.
                if let (Some(r), None, true) = (*rect, *drag, si == main_i) {
                    let lr = to_local(r);
                    let c = lr.center();
                    let rec_rect = Rect::from_center_size(c - vec2(15.0, 0.0), vec2(84.0, 26.0));
                    let gear_rect = Rect::from_min_size(pos2(rec_rect.max.x + 4.0, rec_rect.min.y), vec2(26.0, 26.0));
                    let rec_resp = ui.interact(rec_rect, Id::new(("rec_btn", si)), Sense::click());
                    let fill = if rec_resp.hovered() { Color32::from_rgb(240, 58, 58) } else { RED };
                    painter.rect_filled(rec_rect, 4.0, fill);
                    painter.text(rec_rect.center(), Align2::CENTER_CENTER, "Record", FontId::proportional(13.0), Color32::WHITE);
                    if rec_resp.clicked() {
                        out = SelectOut::Record(r);
                    }
                    let gear_resp = ui.interact(gear_rect, Id::new(("gear_btn", si)), Sense::click());
                    painter.rect_filled(gear_rect, 4.0, if gear_resp.hovered() { Color32::from_gray(95) } else { Color32::from_gray(70) });
                    icons::draw(&painter, gear_rect.shrink(5.0), Icon::Gear, Color32::WHITE);
                    if gear_resp.clicked() {
                        *settings_open = !*settings_open;
                    }
                    let hint = "Drag to move or resize — also onto another screen";
                    let galley = painter.layout_no_wrap(hint.into(), FontId::proportional(13.0), Color32::WHITE);
                    let hr = Rect::from_center_size(pos2(c.x, rec_rect.min.y - 20.0), galley.size() + vec2(16.0, 8.0));
                    painter.rect_filled(hr, 6.0, Color32::from_black_alpha(170));
                    painter.galley(hr.min + vec2(8.0, 4.0), galley, Color32::WHITE);
                    if *settings_open {
                        let pos = pos2(gear_rect.center().x - 130.0, gear_rect.max.y + 8.0);
                        egui::Area::new(Id::new(("rec_opts", si)))
                            .fixed_pos(pos)
                            .order(egui::Order::Foreground)
                            .show(&ctx, |ui| {
                                Frame::popup(ui.style()).inner_margin(Margin::same(10)).show(ui, |ui| {
                                    ui.set_width(250.0);
                                    video_options(ui, settings, devices, has_ff);
                                });
                            });
                    }
                }
                let (enter, esc) = ctx.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                if esc || ctx.input(|i| i.viewport().close_requested()) {
                    out = SelectOut::Cancel;
                } else if enter {
                    out = SelectOut::Record(rect.unwrap_or(scr.bounds));
                }
                out
            });
            if !matches!(res, SelectOut::None) {
                out = res;
            }
        }
        *focused = any_focus_sent;
        out
    }

    fn start_recording(&mut self, ctx: &egui::Context, rect: Rect, screen: usize, screen_bounds: Rect, scale: f32) {
        // A recording covers one screen: keep the area inside the screen that holds most of it.
        let mut rect = rect;
        let dx = (screen_bounds.min.x - rect.min.x).max(0.0) + (screen_bounds.max.x - rect.max.x).min(0.0);
        let dy = (screen_bounds.min.y - rect.min.y).max(0.0) + (screen_bounds.max.y - rect.max.y).min(0.0);
        rect = rect.translate(vec2(dx, dy)).intersect(screen_bounds);
        let devices = self.devices.lock().ok().and_then(|d| d.clone()).unwrap_or_default();
        let audio = self.settings.audio.clone().filter(|a| devices.audio_inputs.contains(a) || self.ffmpeg.is_none());
        let opts = RecordOptions {
            rect_pt: [rect.min.x - screen_bounds.min.x, rect.min.y - screen_bounds.min.y, rect.width(), rect.height()],
            rect_global: [rect.min.x, rect.min.y, rect.width(), rect.height()],
            screen,
            scale,
            fps: self.settings.fps,
            quality: self.settings.quality,
            cursor: self.settings.capture_cursor,
            clicks: self.settings.capture_clicks,
            audio,
        };
        match Recorder::start(opts, self.ffmpeg.clone(), &devices) {
            Ok(rec) => {
                self.rec_started_name = self.settings.make_name("Screencast");
                self.recording = Some(RecState { rec, rect, screen: screen_bounds, last_secs: u64::MAX });
                self.set_tray_recording(true);
                ctx.send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);
            }
            Err(e) => {
                self.show_window(ctx);
                self.toast(e, None);
            }
        }
    }

    fn recording_overlays(&mut self, ctx: &egui::Context) -> Option<Action> {
        let r = self.recording.as_ref()?;
        let rect = r.rect;
        let screen = r.screen;
        let monitor = screen.size();
        let paused = r.rec.is_paused();
        let can_pause = r.rec.can_pause();
        let elapsed = record::format_elapsed(r.rec.elapsed());
        let full_screen = rect.width() >= monitor.x - 2.0 && rect.height() >= monitor.y - 2.0;
        let _ = monitor;

        // Red frame around the area: four thin solid bars just outside it, so nothing covers
        // the area itself (a see-through window would be recorded as black).
        if !full_screen {
            let (t, g) = (3.0, 2.0);
            let o = rect.expand(g + t);
            let bars = [
                Rect::from_min_size(o.min, vec2(o.width(), t)),
                Rect::from_min_size(pos2(o.min.x, o.max.y - t), vec2(o.width(), t)),
                Rect::from_min_size(pos2(o.min.x, o.min.y + t), vec2(t, o.height() - 2.0 * t)),
                Rect::from_min_size(pos2(o.max.x - t, o.min.y + t), vec2(t, o.height() - 2.0 * t)),
            ];
            let color = if paused { Color32::from_rgb(255, 170, 0) } else { RED };
            for (bi, b) in bars.iter().enumerate() {
                let builder = ViewportBuilder::default()
                    .with_title("Recording area")
                    .with_decorations(false)
                    .with_mouse_passthrough(true)
                    .with_window_level(WindowLevel::AlwaysOnTop)
                    .with_position(b.min)
                    .with_inner_size(b.size())
                    .with_min_inner_size(vec2(1.0, 1.0))
                    .with_resizable(false)
                    .with_taskbar(false)
                    .with_has_shadow(false)
                    .with_active(false);
                crate::show_immediate(ctx, ViewportId::from_hash_of(("lightsnap_frame", bi)), builder, |ui, _| {
                    ui.painter().rect_filled(ui.max_rect().expand(2.0), 0.0, color);
                });
            }
        }

        // Small control bar below the area.
        let bar_size = vec2(if can_pause { 256.0 } else { 214.0 }, 40.0);
        // The control bar goes below the area, or above it, on the same screen.
        let mut pos = pos2(rect.center().x - bar_size.x / 2.0, rect.max.y + 12.0);
        if pos.y + bar_size.y > screen.max.y - 4.0 {
            pos.y = rect.min.y - bar_size.y - 12.0;
            if pos.y < screen.min.y + 30.0 {
                pos.y = screen.max.y - bar_size.y - 70.0;
            }
        }
        pos.x = pos.x.clamp(screen.min.x + 4.0, (screen.max.x - bar_size.x - 4.0).max(screen.min.x + 4.0));
        let builder = ViewportBuilder::default()
            .with_title("Recording")
            .with_decorations(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_position(pos)
            .with_inner_size(bar_size)
            .with_resizable(false)
            .with_taskbar(false)
            .with_active(false);
        crate::show_immediate(ctx, ViewportId::from_hash_of("lightsnap_recbar"), builder, |ui, _| {
            let mut act = None;
            Frame::new().fill(Color32::from_rgb(236, 236, 238)).inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (dot, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 5.0, if paused { Color32::from_rgb(255, 170, 0) } else { RED });
                    ui.label(egui::RichText::new(elapsed.clone()).size(BAR_FONT).color(Color32::from_gray(30)));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let cancel = ui.add(
                            egui::Button::new(egui::RichText::new("Cancel").color(Color32::from_gray(40)))
                                .fill(Color32::from_gray(214))
                                .min_size(vec2(58.0, TOOL_H)),
                        );
                        if cancel.clicked() {
                            act = Some(Action::CancelRec);
                        }
                        if can_pause {
                            let (r, pr) = ui.allocate_exact_size(vec2(TOOL_W, TOOL_H), Sense::click());
                            ui.painter().rect_filled(r, 4.0, if pr.hovered() { Color32::from_gray(200) } else { Color32::from_gray(214) });
                            icons::draw(ui.painter(), r.shrink(7.0), if paused { Icon::Play } else { Icon::Pause }, Color32::from_gray(40));
                            if pr.on_hover_text(if paused { "Resume" } else { "Pause" }).clicked() {
                                act = Some(Action::PauseRec);
                            }
                        }
                        let stop = ui.add(
                            egui::Button::new(egui::RichText::new("Stop").color(Color32::WHITE).size(BAR_FONT))
                                .fill(RED)
                                .min_size(vec2(62.0, TOOL_H)),
                        );
                        if stop.clicked() {
                            act = Some(Action::FinishRec);
                        }
                    });
                });
            });
            act
        })
    }
}

impl eframe::App for LightSnap {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Remember the screen size while the main window is around.
        if let Some(m) = ctx.input(|i| i.viewport().monitor_size) {
            if m.x > 100.0 {
                self.monitor = m;
            }
        }
        if let Some(p) = ctx.input(|i| i.viewport().native_pixels_per_point) {
            self.native_ppp = p;
        }

        self.handle_events(ctx);
        if test_env("LS_TEST_AUTOREC") {
            if let Some(r) = &self.recording {
                if r.rec.elapsed() > Duration::from_secs(3) {
                    eprintln!("TEST rect={:?} screen={:?}", r.rect, r.screen);
                    self.run_action(ctx, Action::FinishRec);
                }
            }
            if let Some(v) = &self.video {
                if !self.test_dialog_done {
                    self.test_dialog_done = true;
                    eprintln!("TEST video={}", v.path.display());
                }
            }
        }
        if test_env("LS_TEST_RECORD") && !self.test_prefs_done {
            self.test_prefs_done = true;
            self.run_action(ctx, Action::Record);
        }
        if test_env("LS_TEST_EDIT") && !self.test_dialog_done {
            if let Some(ed) = &mut self.ed {
                self.test_dialog_done = true;
                let font = self.font.clone();
                ed.apply_resize(&font, 300, 200);
            }
        }
        self.auto_copy_edits(ctx);
        // Test hook: LS_TEST_PREFS=1 opens Preferences once, as the menu-bar item does.
        if test_env("LS_TEST_PREFS") && !self.prefs.open && !self.test_prefs_done {
            self.test_prefs_done = true;
            self.run_action(ctx, Action::Settings);
        }

        // Closing the window keeps LightSnap in the menu bar.
        if ctx.input(|i| i.viewport().close_requested()) && self.tray.is_some() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            if let Some(v) = &mut self.video {
                v.stop();
            }
            self.hide_window(ctx);
        }

        if let Some(t) = &self.toast {
            let now = Instant::now();
            if now >= t.until {
                self.toast = None;
            } else {
                ctx.request_repaint_after(t.until - now);
            }
        }

        if let Some(s) = &mut self.shot {
            s.frames += 1;
            if s.frames >= 120 && !s.sent {
                s.sent = true;
                let path = s.path.clone();
                ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                    let [w, h] = img.size;
                    let mut raw = Vec::with_capacity(w * h * 4);
                    for p in &img.pixels {
                        raw.extend_from_slice(&p.to_srgba_unmultiplied());
                    }
                    if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, raw) {
                        if let Some(d) = path.parent() {
                            let _ = std::fs::create_dir_all(d);
                        }
                        let _ = buf.save(&path);
                    }
                    std::process::exit(0);
                })));
            }
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.checker.is_none() {
            let a = Color32::from_gray(44);
            let b = Color32::from_gray(52);
            let img = ColorImage::new([2, 2], vec![a, b, b, a]);
            self.checker = Some(ctx.load_texture("checker", img, TextureOptions::NEAREST_REPEAT));
        }
        if let Some(sz) = self.pending_size.take() {
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(sz));
        }
        if let Some(p) = self.pending_pos.take() {
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(p));
        }

        // Pictures dropped onto the window open in the editor.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if let Some(p) = dropped.first() {
            self.open_file(&ctx, p);
        }

        let mut action = None;

        // Area selection before a recording.
        let mut sel_out = self.select_overlay(&ctx);
        if test_env("LS_TEST_AUTOREC") && self.select.is_some() {
            let t = *self.test_select_since.get_or_insert_with(Instant::now);
            if t.elapsed() > Duration::from_millis(1500) {
                if let Some(r) = self.select.as_ref().and_then(|s| s.rect) {
                    sel_out = SelectOut::Record(r);
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        match sel_out {
            SelectOut::None => {}
            SelectOut::Cancel => {
                self.select = None;
                self.show_window(&ctx);
            }
            SelectOut::Record(r) => {
                let (screen, bounds, scale) = self
                    .select
                    .as_ref()
                    .and_then(|s| {
                        let list: Vec<platform::Display> =
                            s.screens.iter().map(|x| platform::Display { bounds: x.bounds, scale: x.scale }).collect();
                        if list.is_empty() {
                            return None;
                        }
                        let i = platform::display_index_at(&list, r.center());
                        Some((i, list[i].bounds, list[i].scale))
                    })
                    .unwrap_or((0, Rect::from_min_size(Pos2::ZERO, self.monitor), self.native_ppp));
                self.select = None;
                self.start_recording(&ctx, r, screen, bounds, scale);
            }
        }
        if let Some(a) = self.recording_overlays(&ctx) {
            action = Some(a);
        }

        // Settings, in a window of their own.
        let out = self.prefs.show(&ctx, &mut self.settings);
        if out.capture_started {
            // Let the next key press reach the settings window instead of starting a capture.
            self.unregister_hotkeys();
        }
        if out.changed {
            self.settings.save();
            self.tray_menu_dirty = true;
        }
        if out.save_dir_changed {
            self.recent_dirty = true;
        }
        if test_env("LS_TEST_DIALOG") && self.prefs.open && !self.test_dialog_done {
            self.test_dialog_done = true;
            let d = rfd::AsyncFileDialog::new().set_directory(&self.settings.save_dir);
            self.spawn_dialog(&ctx, DialogKind::SaveDir, d.pick_folder());
        }
        if out.pick_save_dir {
            let d = rfd::AsyncFileDialog::new().set_directory(&self.settings.save_dir);
            self.spawn_dialog(&ctx, DialogKind::SaveDir, d.pick_folder());
        }
        if out.pick_editor_app {
            let d = rfd::AsyncFileDialog::new().set_directory("/Applications").add_filter("Application", &["app"]);
            self.spawn_dialog(&ctx, DialogKind::EditorApp, d.pick_file());
        }
        if (out.hotkeys_changed || out.capture_ended) && self.prefs.capturing.is_none() {
            self.register_hotkeys();
        }

        egui::Panel::top("top")
            .exact_size(40.0)
            .frame(Frame::new().fill(BAR).stroke(Stroke::new(1.0, EDGE)))
            .show(ui, |ui| {
                if let Some(a) = self.top_bar(ui) {
                    action = Some(a);
                }
            });
        egui::Panel::bottom("bottom")
            .exact_size(40.0)
            .frame(Frame::new().fill(BAR).stroke(Stroke::new(1.0, EDGE)))
            .show(ui, |ui| {
                if self.video.is_some() {
                    self.video_bar(ui);
                } else {
                    self.bottom_bar(ui);
                }
            });
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let font = self.font.clone();
            let style = self.style();
            let tool = self.tool;
            if self.video.is_some() {
                self.video_view(ui);
            } else if let (Some(ed), Some(ch)) = (&mut self.ed, &self.checker) {
                ed.canvas(ui, tool, style, &font, ch);
            } else if let Some(a) = self.home(ui) {
                action = Some(a);
            }

            // Toast
            if let Some(t) = &self.toast {
                let rect = ui.max_rect();
                let galley = ui.painter().layout_no_wrap(t.text.clone(), FontId::proportional(14.0), Color32::WHITE);
                let r = Rect::from_center_size(
                    pos2(rect.center().x, rect.max.y - 34.0),
                    galley.size() + vec2(28.0, 16.0),
                );
                ui.painter().rect_filled(r, 8.0, Color32::from_black_alpha(215));
                ui.painter().galley(r.min + vec2(14.0, 8.0), galley, Color32::WHITE);
                if let Some(p) = t.path.clone() {
                    if ui.interact(r, Id::new("toast"), Sense::click()).on_hover_text("Show in Finder").clicked() {
                        platform::reveal_path(&p);
                    }
                }
            }
        });

        // ⌘W closes the picture editor (or video) and puts the window away, like ⌘W in other Mac apps.
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::W)) {
            if let Some(v) = &mut self.video {
                v.stop();
            }
            if let Some(e) = &mut self.ed {
                e.commit_text();
            }
            self.ed = None;
            self.video = None;
            self.recent_dirty = true;
            self.close_after_save = true;
        }
        self.editor_shortcuts(&ctx);
        if self.close_after_save {
            self.close_after_save = false;
            if self.tray.is_some() {
                self.hide_window(&ctx);
            }
        }
        self.resize_window(&ctx);

        if let Some(a) = action {
            self.run_action(&ctx, a);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(r) = self.recording.take() {
            r.rec.cancel();
        }
        self.settings.save();
    }
}

// ---------------------------------------------------------------- helpers

/// A row of controls of height `h`, centred vertically in the bar.
fn centered_row<R>(ui: &mut Ui, h: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let full = ui.max_rect();
    let r = Rect::from_center_size(full.center(), vec2(full.width(), h));
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(r).layout(Layout::left_to_right(Align::Center)),
        add,
    )
    .inner
}

fn edges_near(r: Rect, p: Pos2, tol: f32) -> u8 {
    let inside_y = p.y >= r.min.y - tol && p.y <= r.max.y + tol;
    let inside_x = p.x >= r.min.x - tol && p.x <= r.max.x + tol;
    let mut e = 0;
    if inside_y && (p.x - r.min.x).abs() <= tol {
        e |= 1;
    }
    if inside_y && (p.x - r.max.x).abs() <= tol {
        e |= 2;
    }
    if inside_x && (p.y - r.min.y).abs() <= tol {
        e |= 4;
    }
    if inside_x && (p.y - r.max.y).abs() <= tol {
        e |= 8;
    }
    e
}

fn video_options(ui: &mut Ui, s: &mut Settings, devices: &Arc<Mutex<Option<Devices>>>, has_ff: bool) {
    let mut changed = false;
    ui.add_enabled_ui(has_ff, |ui| {
        egui::ComboBox::from_id_salt("fps")
            .width(240.0)
            .selected_text(format!("{} frames per second", s.fps))
            .show_ui(ui, |ui| {
                for f in [10, 15, 24, 30, 60] {
                    changed |= ui.selectable_value(&mut s.fps, f, format!("{f} frames per second")).changed();
                }
            });
        egui::ComboBox::from_id_salt("quality")
            .width(240.0)
            .selected_text(s.quality.label())
            .show_ui(ui, |ui| {
                for q in [Quality::High, Quality::Medium, Quality::Low] {
                    changed |= ui.selectable_value(&mut s.quality, q, q.label()).changed();
                }
            });
    });
    changed |= ui.checkbox(&mut s.capture_cursor, "Capture mouse cursor").changed();
    changed |= ui.checkbox(&mut s.capture_clicks, "Capture mouse clicks").changed();
    ui.label("Audio input:");
    let inputs = devices.lock().ok().and_then(|d| d.as_ref().map(|d| d.audio_inputs.clone())).unwrap_or_default();
    let current = s.audio.clone().unwrap_or_else(|| "Do Not Record Audio".into());
    egui::ComboBox::from_id_salt("audio").width(240.0).selected_text(current).show_ui(ui, |ui| {
        changed |= ui.selectable_value(&mut s.audio, None, "Do Not Record Audio").changed();
        if has_ff {
            for name in inputs {
                changed |= ui.selectable_value(&mut s.audio, Some(name.clone()), name).changed();
            }
        } else {
            changed |= ui
                .selectable_value(&mut s.audio, Some("Default".into()), "Default microphone")
                .changed();
        }
    });
    if !has_ff {
        ui.label(
            egui::RichText::new("Install ffmpeg for frame rate, quality, pause and trimming.")
                .small()
                .color(DIM_TEXT),
        );
    }
    if changed {
        s.save();
    }
}

fn capture_menu(ui: &mut Ui, st: &Settings) -> Option<Action> {
    let mut a = None;
    let delay = st.delay_secs;
    let row = |ui: &mut Ui, label: &str, id: &str| {
        ui.add(egui::Button::new(label).shortcut_text(st.hotkey_short(id)).min_size(vec2(220.0, 0.0)))
            .clicked()
    };
    if row(ui, "Capture Area", "area") {
        a = Some(Action::Capture(CaptureKind::Area));
    }
    if row(ui, "Capture Full Screen", "full") {
        a = Some(Action::Capture(CaptureKind::FullScreen));
    }
    if row(ui, "Capture Window", "window") {
        a = Some(Action::Capture(CaptureKind::Window));
    }
    if row(ui, &format!("Timed Capture ({delay} s)"), "timed") {
        a = Some(Action::Capture(CaptureKind::Timed(delay)));
    }
    if row(ui, "Record Video", "record") {
        a = Some(Action::Record);
    }
    ui.separator();
    if row(ui, "Open Picture…", "") {
        a = Some(Action::OpenImage);
    }
    if row(ui, "Open from Clipboard", "clipboard") {
        a = Some(Action::OpenClipboard);
    }
    if row(ui, "Open Latest Image", "latest") {
        a = Some(Action::OpenLatest);
    }
    a
}

/// The global shortcut for a settings hotkey.
fn to_global_hotkey(k: &crate::settings::Hotkey) -> Option<global_hotkey::hotkey::HotKey> {
    use global_hotkey::hotkey::{Code, HotKey, Modifiers as M};
    let code: Code = k.code.parse().ok()?;
    let mut m = M::empty();
    if k.cmd {
        m |= M::META;
    }
    if k.alt {
        m |= M::ALT;
    }
    if k.ctrl {
        m |= M::CONTROL;
    }
    if k.shift {
        m |= M::SHIFT;
    }
    Some(HotKey::new(if m.is_empty() { None } else { Some(m) }, code))
}

/// The same shortcut shown in the menu-bar menu.
fn to_accelerator(k: &crate::settings::Hotkey) -> Option<tray_icon::menu::accelerator::Accelerator> {
    use tray_icon::menu::accelerator::{Accelerator, Code, Modifiers as M};
    let code: Code = k.code.parse().ok()?;
    let mut m = M::empty();
    if k.cmd {
        m |= M::META;
    }
    if k.alt {
        m |= M::ALT;
    }
    if k.ctrl {
        m |= M::CONTROL;
    }
    if k.shift {
        m |= M::SHIFT;
    }
    Some(Accelerator::new(m, code))
}

/// A joined row of `n` buttons, each `w` wide; the group is exactly `h` tall.
fn group<R>(ui: &mut Ui, n: usize, w: f32, h: f32, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    let size = vec2(n as f32 * w + (n as f32 - 1.0) + 2.0, h);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, 5.0, EDGE);
    let inner = ui
        .scope_builder(
            egui::UiBuilder::new().max_rect(rect.shrink(1.0)).layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                add(ui)
            },
        )
        .inner;
    egui::InnerResponse::new(inner, resp)
}

fn icon_button(ui: &mut Ui, icon: Icon, selected: bool, enabled: bool, size: Vec2, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let bg = if selected {
        BLUE
    } else if enabled && resp.hovered() {
        BTN_HOVER
    } else {
        BTN
    };
    ui.painter().rect_filled(rect, 3.0, bg);
    let fg = if enabled { Color32::WHITE } else { Color32::from_gray(105) };
    let s = 22.0f32.min(size.y - 8.0);
    icons::draw(ui.painter(), Rect::from_center_size(rect.center(), vec2(s, s)), icon, fg);
    resp.on_hover_text(tip)
}

fn colored_icon_button(ui: &mut Ui, icon: Icon, fill: Color32, size: Vec2, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let bg = if resp.hovered() { BLUE_HOVER } else { fill };
    ui.painter().rect_filled(rect, CornerRadius { nw: 0, sw: 0, ne: 5, se: 5 }, bg);
    icons::draw(ui.painter(), Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), icon, Color32::WHITE);
    resp.on_hover_text(tip)
}

fn text_button(ui: &mut Ui, text: &str, fill: Color32, hover: Color32, enabled: bool, size: Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let bg = if enabled && resp.hovered() { hover } else { fill };
    let radius = if fill == BLUE { CornerRadius { nw: 5, sw: 5, ne: 0, se: 0 } } else { CornerRadius::same(5) };
    ui.painter().rect_filled(rect, radius, if enabled { bg } else { bg.gamma_multiply(0.6) });
    ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, EDGE), StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(BAR_FONT),
        if enabled { Color32::WHITE } else { Color32::from_gray(130) },
    );
    resp
}

fn big_button(ui: &mut Ui, label: &str, key: &str, w: f32, enabled: bool, red: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 34.0), if enabled { Sense::click() } else { Sense::hover() });
    let bg = if enabled && resp.hovered() { BTN_HOVER } else { BTN };
    let p = ui.painter();
    p.rect_filled(rect, 6.0, bg);
    p.rect_stroke(rect, 6.0, Stroke::new(1.0, EDGE), StrokeKind::Inside);
    if red {
        p.circle_filled(pos2(rect.min.x + 20.0, rect.center().y), 6.0, RED);
    }
    let x = rect.min.x + if red { 36.0 } else { 16.0 };
    p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(BAR_FONT), if enabled { TEXT } else { DIM_TEXT });
    if !key.is_empty() {
        p.text(pos2(rect.max.x - 14.0, rect.center().y), Align2::RIGHT_CENTER, key, FontId::proportional(12.0), DIM_TEXT);
    }
    resp
}

fn save_image(img: &image::RgbaImage, dest: &Path, jpeg_quality: u8) -> Result<(), String> {
    let ext = dest.extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
    if ext == "jpg" || ext == "jpeg" {
        let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
        let file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
        let mut w = std::io::BufWriter::new(file);
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, jpeg_quality.clamp(10, 100))
            .encode_image(&rgb)
            .map_err(|e| e.to_string())
    } else {
        img.save(dest).map_err(|e| e.to_string())
    }
}

fn short_path(p: &Path) -> String {
    let s = p.display().to_string();
    match dirs::home_dir() {
        Some(h) => {
            let h = h.display().to_string();
            if let Some(rest) = s.strip_prefix(&h) { format!("~{rest}") } else { s }
        }
        None => s,
    }
}

/// Test hooks (LS_TEST_*) only work in debug builds; a release build ignores them.
fn test_env(name: &str) -> bool {
    cfg!(debug_assertions) && std::env::var_os(name).is_some()
}

fn recent_files(dir: &Path, n: usize) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<(std::time::SystemTime, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            let ext = p.extension()?.to_str()?.to_lowercase();
            if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "mp4" | "mov") {
                return None;
            }
            Some((e.metadata().ok()?.modified().ok()?, p))
        })
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().take(n).map(|(_, p)| p).collect()
}

fn setup_style(ctx: &egui::Context, font_bytes: Vec<u8>) {
    // ⌘+ / ⌘- / ⌘0 zoom the picture, not the whole interface.
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("annot".into(), Arc::new(egui::FontData::from_owned(font_bytes)));
    fonts.families.insert(egui::FontFamily::Name("annot".into()), vec!["annot".into()]);
    // Small system font with the ⌥ ⌘ ⇧ key symbols, used as a fallback.
    if let Ok(sym) = std::fs::read(platform::SYMBOL_FONT) {
        fonts.font_data.insert("symbols".into(), Arc::new(egui::FontData::from_owned(sym)));
        for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(fam).or_default().push("symbols".into());
        }
    }
    ctx.set_fonts(fonts);

    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.visuals = egui::Visuals::dark();
        let v = &mut style.visuals;
        v.panel_fill = BAR;
        v.window_fill = Color32::from_rgb(38, 38, 40);
        v.extreme_bg_color = Color32::from_rgb(30, 30, 32);
        v.selection.bg_fill = BLUE;
        v.widgets.inactive.weak_bg_fill = BTN;
        v.widgets.inactive.bg_fill = BTN;
        v.widgets.hovered.weak_bg_fill = BTN_HOVER;
        v.widgets.hovered.bg_fill = BTN_HOVER;
        v.widgets.noninteractive.fg_stroke.color = TEXT;
        v.widgets.inactive.fg_stroke.color = TEXT;
        style.spacing.button_padding = vec2(8.0, 3.0);
        style.spacing.interact_size.y = 22.0;
        for (ts, f) in style.text_styles.iter_mut() {
            // Same text sizes as the settings window.
            f.size = match ts {
                egui::TextStyle::Small => 11.0,
                egui::TextStyle::Heading => 17.0,
                _ => 13.0,
            };
        }
    });
}

// ---------------------------------------------------------------- menu bar

fn tray_image(recording: bool) -> (Option<tray_icon::Icon>, bool) {
    use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};
    let n = 44u32;
    let Some(mut pm) = Pixmap::new(n, n) else { return (None, false) };
    let mut paint = Paint::default();
    paint.anti_alias = true;
    if recording {
        paint.set_color_rgba8(230, 40, 40, 255);
        if let Some(c) = PathBuilder::from_circle(22.0, 22.0, 13.0) {
            pm.fill_path(&c, &paint, FillRule::Winding, Transform::identity(), None);
        }
    } else {
        paint.set_color_rgba8(0, 0, 0, 255);
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
    let icon = tray_icon::Icon::from_rgba(pm.take(), n, n).ok();
    (icon, !recording)
}

/// Where the newest screenshot is kept (one file, replaced by each new capture).
fn latest_capture_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("LightSnap").join("latest-capture.png"))
}

fn is_picture(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_lowercase().as_str(), "png" | "jpg" | "jpeg"))
}

/// The menu-bar menu, laid out like the original's but without the cloud items:
/// "Latest Uploads" becomes a list of recent local snaps, "Storage" opens the save folder.
fn build_menu(recording: bool, st: &Settings) -> tray_icon::menu::Menu {
    use tray_icon::menu::accelerator::{Accelerator, Code, Modifiers as M};
    use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
    let menu = Menu::new();
    let item = |id: &str, text: &str| MenuItem::with_id(id, text, true, None);
    // Menu items show the shortcut chosen in Settings › Hotkeys.
    let keyed = |id: &str, key_id: &str, text: &str| {
        let acc = if st.hotkeys { st.hotkey(key_id).and_then(|k| to_accelerator(&k)) } else { None };
        MenuItem::with_id(id, text, true, acc)
    };
    if recording {
        let _ = menu.append(&keyed("finish", "rec_stop", "Finish Recording"));
        let _ = menu.append(&keyed("pause", "rec_pause", "Pause / Resume"));
        let _ = menu.append(&item("cancel", "Cancel Recording"));
        return menu;
    }
    let _ = menu.append(&keyed("area", "area", "Capture Area"));
    let _ = menu.append(&keyed("full", "full", "Capture Fullscreen"));
    let _ = menu.append(&keyed("record", "record", "Record Video"));
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&item("open", "Open…"));
    let _ = menu.append(&keyed("clipboard", "clipboard", "Open from Clipboard"));
    let _ = menu.append(&keyed("latest", "latest", "Open Latest Image"));
    let _ = menu.append(&PredefinedMenuItem::separator());
    let recent = Submenu::new("Latest Snaps", true);
    let files = recent_files(&st.save_dir, 10);
    if files.is_empty() {
        let _ = recent.append(&MenuItem::new("Nothing saved yet", false, None));
    }
    for p in files {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        let _ = recent.append(&MenuItem::with_id(format!("recent:{}", p.display()), name, true, None));
    }
    let _ = menu.append(&recent);
    let _ = menu.append(&item("folder", "Saved Snaps Folder"));
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&item("settings", "Preferences…"));
    let help = Submenu::new("Help", true);
    let _ = help.append(&item("help", "Keyboard Shortcuts…"));
    let _ = help.append(&MenuItem::new("Nothing is ever uploaded", false, None));
    let _ = menu.append(&help);
    let _ = menu.append(&MenuItem::with_id("quit", "Quit", true, Some(Accelerator::new(M::META, Code::KeyQ))));
    menu
}

fn build_tray(st: &Settings) -> Option<tray_icon::TrayIcon> {
    let (icon, template) = tray_image(false);
    let mut b = tray_icon::TrayIconBuilder::new()
        .with_menu(Box::new(build_menu(false, st)))
        .with_tooltip("LightSnap")
        .with_icon_as_template(template);
    if let Some(i) = icon {
        b = b.with_icon(i);
    }
    b.build().ok()
}
