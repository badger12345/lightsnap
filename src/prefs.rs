//! The settings window: its own window with General, Advanced and Hotkeys tabs.
//! Cloud-only options of the original (upload, links, account) are left out on purpose.

use crate::icons::{self, Icon};
use crate::settings::{
    AfterCapture, ClipFormat, HOTKEY_ACTIONS, Hotkey, ImageFormat, Settings, VIDEO_HOTKEY_ACTIONS, VideoFormat,
};
use egui::{
    Align, Align2, Color32, FontId, Key, Layout, Rect, Sense, Stroke, Ui, Vec2, ViewportBuilder, ViewportCommand,
    ViewportId, pos2, vec2,
};

const BG: Color32 = Color32::from_rgb(34, 34, 36);
const HEADER: Color32 = Color32::from_rgb(42, 42, 44);
const TAB_SEL: Color32 = Color32::from_rgb(58, 58, 60);
const ACCENT: Color32 = Color32::from_rgb(70, 140, 255);
const TEXT: Color32 = Color32::from_rgb(232, 232, 236);
const DIM: Color32 = Color32::from_rgb(140, 140, 146);
const KEY_BTN: Color32 = Color32::from_rgb(88, 88, 92);
// Sizes measured from the original's settings screenshots (Retina pixels ÷ 2).
const WIN_W: f32 = 426.0;
const LABEL_W: f32 = 140.0;
const KEY_LABEL_W: f32 = 140.0;
const ROW_H: f32 = 24.0;
/// Every dropdown, slider and text field has this width, so their right edges line up.
const CTRL_W: f32 = 236.0;
const HEADER_H: f32 = 66.0;
const FONT: f32 = 13.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    General,
    Advanced,
    Hotkeys,
}

impl Tab {
    fn name(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Advanced => "Advanced",
            Tab::Hotkeys => "Hotkeys",
        }
    }
    fn icon(self) -> Icon {
        match self {
            Tab::General => Icon::Sliders,
            Tab::Advanced => Icon::Gear,
            Tab::Hotkeys => Icon::Command,
        }
    }
    fn size(self) -> Vec2 {
        match self {
            Tab::General => vec2(WIN_W, 330.0),
            Tab::Advanced => vec2(WIN_W, 600.0),
            Tab::Hotkeys => vec2(WIN_W, 600.0),
        }
    }
}

pub struct Prefs {
    pub open: bool,
    pub tab: Tab,
    /// The shortcut being recorded right now.
    pub capturing: Option<String>,
    about: bool,
    sized_for: Option<Tab>,
    focused: bool,
    /// Height of the current tab's contents, to fit the window to it.
    content_h: f32,
}

#[derive(Default)]
pub struct PrefsOut {
    pub changed: bool,
    pub hotkeys_changed: bool,
    /// Global shortcuts must be paused while a new one is recorded.
    pub capture_started: bool,
    pub capture_ended: bool,
    pub save_dir_changed: bool,
    /// Ask the app to show a folder picker for the save folder.
    pub pick_save_dir: bool,
    /// Ask the app to show an app picker for the external editor.
    pub pick_editor_app: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { open: false, tab: Tab::General, capturing: None, about: false, sized_for: None, focused: false, content_h: 0.0 }
    }
}

impl Prefs {
    pub fn open(&mut self) {
        self.open = true;
        self.focused = false;
    }

    pub fn show(&mut self, ctx: &egui::Context, s: &mut Settings) -> PrefsOut {
        let mut out = PrefsOut::default();
        if !self.open {
            return out;
        }
        let size = self.tab.size();
        let builder = ViewportBuilder::default()
            .with_title(format!("LightSnap | {}", self.tab.name()))
            .with_inner_size(vec2(size.x, 420.0))
            .with_resizable(false)
            .with_minimize_button(false)
            .with_maximize_button(false);
        let mut close = false;
        crate::show_immediate(ctx, ViewportId::from_hash_of("lightsnap_prefs"), builder, |ui, _| {
            let ctx = ui.ctx().clone();
            if !self.focused {
                self.focused = true;
                ctx.send_viewport_cmd(ViewportCommand::Focus);
            }
            if self.sized_for != Some(self.tab) {
                self.sized_for = Some(self.tab);
                self.content_h = 0.0;
                ctx.send_viewport_cmd(ViewportCommand::Title(format!("LightSnap | {}", self.tab.name())));
            }
            if ctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            if self.capturing.is_none() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::W)) {
                close = true;
            }
            let full = ui.max_rect();
            ui.painter().rect_filled(full, 0.0, BG);

            // Tab bar
            let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER_H));
            ui.painter().rect_filled(header, 0.0, HEADER);
            ui.painter().line_segment(
                [header.left_bottom(), header.right_bottom()],
                Stroke::new(1.0, Color32::from_gray(24)),
            );
            let tabs = [Tab::General, Tab::Advanced, Tab::Hotkeys];
            let tw = 66.0;
            let x0 = header.center().x - tw * tabs.len() as f32 / 2.0;
            for (i, t) in tabs.iter().enumerate() {
                let r = Rect::from_min_size(pos2(x0 + i as f32 * tw + 3.0, header.min.y + 6.0), vec2(tw - 6.0, 54.0));
                let resp = ui.interact(r, egui::Id::new(("tab", i)), Sense::click());
                let sel = *t == self.tab;
                if sel {
                    ui.painter().rect_filled(r, 6.0, TAB_SEL);
                }
                let col = if sel { ACCENT } else if resp.hovered() { TEXT } else { DIM };
                icons::draw(
                    ui.painter(),
                    Rect::from_center_size(pos2(r.center().x, r.min.y + 20.0), vec2(27.0, 27.0)),
                    t.icon(),
                    if sel { Color32::from_gray(200) } else { DIM },
                );
                ui.painter().text(
                    pos2(r.center().x, r.max.y - 9.0),
                    Align2::CENTER_CENTER,
                    t.name(),
                    FontId::proportional(12.0),
                    col,
                );
                if resp.clicked() && !sel {
                    self.tab = *t;
                    self.cancel_capture(&mut out);
                }
            }

            let body = Rect::from_min_max(pos2(full.min.x + 12.0, header.max.y + 12.0), full.max - vec2(12.0, 10.0));
            let used = ui.scope_builder(egui::UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Min)), |ui| {
                {
                    let st = ui.style_mut();
                    st.spacing.item_spacing = vec2(6.0, 5.0);
                    st.spacing.interact_size.y = 22.0;
                    st.spacing.button_padding = vec2(8.0, 2.0);
                    st.spacing.icon_width = 15.0;
                    st.spacing.icon_width_inner = 9.0;
                    for (ts, f) in st.text_styles.iter_mut() {
                        f.size = match ts {
                            egui::TextStyle::Small => 11.0,
                            egui::TextStyle::Heading => 15.0,
                            _ => FONT,
                        };
                    }
                }
                match self.tab {
                    Tab::General => self.general(ui, s, &mut out),
                    Tab::Advanced => self.advanced(ui, s, &mut out),
                    Tab::Hotkeys => self.hotkeys(ui, s, &mut out),
                }
                ui.min_rect().height()
            });
            // Fit the window height to the tab's contents.
            let want = HEADER_H + 12.0 + used.inner + 16.0;
            if (want - self.content_h).abs() > 1.0 {
                self.content_h = want;
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(size.x, want)));
            }
        });
        if close {
            self.open = false;
            self.sized_for = None;
            self.cancel_capture(&mut out);
            out.changed = true;
        }
        out
    }

    fn cancel_capture(&mut self, out: &mut PrefsOut) {
        if self.capturing.take().is_some() {
            out.capture_ended = true;
        }
    }

    // ------------------------------------------------------------ General

    fn general(&mut self, ui: &mut Ui, s: &mut Settings, out: &mut PrefsOut) {
        row(ui, "After screenshot:", |ui| {
            let label = match s.after_capture {
                AfterCapture::OpenEditor => "Open LightSnap editor",
                AfterCapture::SaveAndCopy => "Save to folder and copy",
                AfterCapture::CopyOnly => "Copy to clipboard only",
            };
            egui::ComboBox::from_id_salt("after").width(CTRL_W).selected_text(label).show_ui(ui, |ui| {
                for (a, l) in [
                    (AfterCapture::OpenEditor, "Open LightSnap editor"),
                    (AfterCapture::SaveAndCopy, "Save to folder and copy"),
                    (AfterCapture::CopyOnly, "Copy to clipboard only"),
                ] {
                    out.changed |= ui.selectable_value(&mut s.after_capture, a, l).changed();
                }
            });
        });
        row(ui, "", |ui| {
            out.changed |= ui.checkbox(&mut s.sound, "Sound effects").changed();
        });
        row(ui, "Timed capture after:", |ui| {
            ui.spacing_mut().slider_width = CTRL_W - 46.0;
            out.changed |= ui.add(egui::Slider::new(&mut s.delay_secs, 1..=30).suffix(" s")).changed();
        });
        separator(ui);
        row(ui, "External Image Editor:", |ui| {
            let name = app_name(&s.external_editor);
            egui::ComboBox::from_id_salt("ext_editor").width(CTRL_W).selected_text(name).show_ui(ui, |ui| {
                for app in ["Preview", "Photos"] {
                    out.changed |= ui.selectable_value(&mut s.external_editor, app.to_string(), app).changed();
                }
                if !matches!(s.external_editor.as_str(), "Preview" | "Photos") {
                    let cur = s.external_editor.clone();
                    ui.selectable_label(true, app_name(&cur));
                }
                if ui.selectable_label(false, "Choose…").clicked() {
                    out.pick_editor_app = true;
                }
            });
        });
        separator(ui);
        row(ui, "", |ui| {
            let mut on = crate::platform::login_item_enabled();
            if ui.checkbox(&mut on, "Launch at login").changed() {
                crate::platform::set_login_item(on);
            }
        });
        ui.vertical_centered(|ui| {
            if ui.add(egui::Button::new("About LightSnap").min_size(vec2(138.0, 24.0))).clicked() {
                self.about = !self.about;
            }
            if self.about {
                ui.label(
                    egui::RichText::new(
                        "LightSnap 0.1 — screenshots and screen recordings that stay on this Mac.\nNo account, no cloud, nothing is uploaded.",
                    )
                    .color(DIM),
                );
            }
        });
    }

    // ------------------------------------------------------------ Advanced

    fn advanced(&mut self, ui: &mut Ui, s: &mut Settings, out: &mut PrefsOut) {
        row(ui, "Image File Format:", |ui| {
            let label = match s.format {
                ImageFormat::Png => "PNG",
                ImageFormat::Jpg => "JPEG",
            };
            egui::ComboBox::from_id_salt("img_fmt").width(CTRL_W).selected_text(label).show_ui(ui, |ui| {
                out.changed |= ui.selectable_value(&mut s.format, ImageFormat::Png, "PNG").changed();
                out.changed |= ui.selectable_value(&mut s.format, ImageFormat::Jpg, "JPEG").changed();
            });
        });
        row(ui, &format!("JPEG Quality: {}%", s.jpeg_quality), |ui| {
            ui.spacing_mut().slider_width = CTRL_W;
            out.changed |= ui.add(egui::Slider::new(&mut s.jpeg_quality, 10..=100).show_value(false)).changed();
        });
        row(ui, "Save folder:", |ui| {
            // Folder and "Choose…" share the control column; the button ends where the dropdowns end.
            ui.allocate_ui_with_layout(vec2(CTRL_W, ROW_H), Layout::right_to_left(Align::Center), |ui| {
                ui.set_width(CTRL_W);
                if ui.button("Choose…").clicked() {
                    out.pick_save_dir = true;
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                    icons::draw(ui.painter(), r, Icon::Folder, Color32::from_rgb(110, 170, 240));
                    ui.add(egui::Label::new(egui::RichText::new(short_path(&s.save_dir)).color(TEXT)).truncate())
                        .on_hover_text(s.save_dir.display().to_string());
                });
            });
        });
        row(ui, "", |ui| {
            out.changed |= ui.checkbox(&mut s.autosave, "Autosave screenshots").changed();
        });
        row(ui, "", |ui| {
            out.changed |= ui.checkbox(&mut s.shrink_retina, "Shrink Retina snaps").changed();
        });
        separator(ui);
        row(ui, "Video File Format:", |ui| {
            egui::ComboBox::from_id_salt("vid_fmt").width(CTRL_W).selected_text(s.video_format.label()).show_ui(ui, |ui| {
                for f in [VideoFormat::Mp4, VideoFormat::Mov, VideoFormat::Gif] {
                    out.changed |= ui.selectable_value(&mut s.video_format, f, f.label()).changed();
                }
            });
        });
        row(ui, &format!("GIF FPS: {}", s.gif_fps), |ui| {
            ui.spacing_mut().slider_width = CTRL_W;
            out.changed |= ui.add(egui::Slider::new(&mut s.gif_fps, 1..=30).show_value(false)).changed();
        });
        row(ui, &format!("GIF Quality: {}%", s.gif_quality), |ui| {
            ui.spacing_mut().slider_width = CTRL_W;
            out.changed |= ui.add(egui::Slider::new(&mut s.gif_quality, 10..=100).show_value(false)).changed();
        });
        row(ui, "", |ui| {
            out.changed |= ui.checkbox(&mut s.autosave_movies, "Autosave to \"Movies\" folder").changed();
        });
        separator(ui);
        row(ui, "Copy to clipboard:", |ui| {
            out.changed |= ui.checkbox(&mut s.copy_path, "Path").changed();
            ui.add_space(16.0);
            out.changed |= ui.checkbox(&mut s.copy_image, "Image").changed();
            ui.add_space(16.0);
            let label = match s.clip_format {
                ClipFormat::Png => "PNG",
                ClipFormat::Tiff => "TIFF",
            };
            egui::ComboBox::from_id_salt("clip_fmt").width(70.0).selected_text(label).show_ui(ui, |ui| {
                out.changed |= ui.selectable_value(&mut s.clip_format, ClipFormat::Png, "PNG").changed();
                out.changed |= ui.selectable_value(&mut s.clip_format, ClipFormat::Tiff, "TIFF").changed();
            });
        });
        separator(ui);
        row(ui, "Filename template:", |ui| {
            out.changed |= ui
                .add(egui::TextEdit::singleline(&mut s.name_template).desired_width(CTRL_W))
                .changed();
        });
        row(ui, "", |ui| {
            out.changed |= ui.checkbox(&mut s.apply_template_reopened, "Apply to reopened files").changed();
        });
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for line in [
                "%Y — 4-digit year, %y — 2-digit year,",
                "%m — month, %d — day, %H — hour,",
                "%M — minutes, %S — seconds,",
                "%R — 5 random symbols, %T — file type",
            ] {
                ui.label(egui::RichText::new(line).color(DIM).size(12.0));
            }
        });
        separator(ui);
        ui.vertical_centered(|ui| {
            if ui.add(egui::Button::new("Reset all settings").min_size(vec2(190.0, 24.0))).clicked() {
                *s = Settings::default();
                out.changed = true;
                out.hotkeys_changed = true;
                out.save_dir_changed = true;
            }
            ui.label(egui::RichText::new("ALL settings in the application will be reset").color(DIM));
        });
    }

    // ------------------------------------------------------------ Hotkeys

    fn hotkeys(&mut self, ui: &mut Ui, s: &mut Settings, out: &mut PrefsOut) {
        // Record a new shortcut from the next key press.
        if let Some(id) = self.capturing.clone() {
            let pressed = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                    _ => None,
                })
            });
            if let Some((key, m)) = pressed {
                if key == Key::Escape && !m.any() {
                    self.cancel_capture(out);
                } else if let Some(code) = key_to_code(key) {
                    let is_fkey = code.starts_with('F') && code.len() > 1 && code[1..].chars().all(|c| c.is_ascii_digit());
                    if m.mac_cmd || m.command || m.alt || m.ctrl || is_fkey {
                        let hk = Hotkey { cmd: m.mac_cmd || m.command, alt: m.alt, ctrl: m.ctrl, shift: m.shift, code };
                        // One shortcut, one action.
                        let others: Vec<String> = HOTKEY_ACTIONS
                            .iter()
                            .chain(VIDEO_HOTKEY_ACTIONS.iter())
                            .map(|(i, _)| i.to_string())
                            .filter(|i| *i != id && s.hotkey(i).as_ref() == Some(&hk))
                            .collect();
                        for o in others {
                            s.keys.insert(o, None);
                        }
                        s.keys.insert(id.clone(), Some(hk));
                        out.changed = true;
                        out.hotkeys_changed = true;
                        self.cancel_capture(out);
                    }
                }
            }
            ui.ctx().request_repaint();
        }

        for (id, label) in HOTKEY_ACTIONS {
            let label = if id == "timed" { format!("Fullscreen after {}s:", s.delay_secs) } else { format!("{label}:") };
            self.key_row(ui, s, id, &label, out);
        }
        separator(ui);
        ui.vertical_centered(|ui| ui.label(egui::RichText::new("Video record hotkeys:").color(TEXT)));
        for (id, label) in VIDEO_HOTKEY_ACTIONS {
            self.key_row(ui, s, id, &format!("{label}:"), out);
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Click a field, then press the new shortcut. Esc cancels.").color(DIM));
    }

    fn key_row(&mut self, ui: &mut Ui, s: &mut Settings, id: &str, label: &str, out: &mut PrefsOut) {
        row_w(ui, label, KEY_LABEL_W, 10.0, |ui| {
            let capturing = self.capturing.as_deref() == Some(id);
            let text = if capturing {
                "Press keys…".to_string()
            } else {
                s.hotkey(id).map(|k| k.label()).unwrap_or_else(|| "Not set".into())
            };
            let (r, resp) = ui.allocate_exact_size(vec2(CTRL_W - 40.0, 27.0), Sense::click());
            let fill = if capturing {
                Color32::from_rgb(40, 76, 186)
            } else if resp.hovered() {
                Color32::from_gray(100)
            } else {
                KEY_BTN
            };
            ui.painter().rect_filled(r, 3.0, fill);
            ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, Color32::from_gray(30)), egui::StrokeKind::Inside);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, text, FontId::proportional(FONT), TEXT);
            if resp.clicked() {
                if !capturing {
                    let was = self.capturing.is_some();
                    self.capturing = Some(id.to_string());
                    if !was {
                        out.capture_started = true;
                    }
                } else {
                    self.cancel_capture(out);
                }
            }
            ui.add_space(14.0);
            let (xr, xresp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
            let col = if xresp.hovered() { TEXT } else { DIM };
            let c = xr.center();
            let d = 5.0;
            ui.painter().line_segment([c + vec2(-d, -d), c + vec2(d, d)], Stroke::new(2.0, col));
            ui.painter().line_segment([c + vec2(-d, d), c + vec2(d, -d)], Stroke::new(2.0, col));
            if xresp.on_hover_text("Clear this shortcut").clicked() {
                s.keys.insert(id.to_string(), None);
                out.changed = true;
                out.hotkeys_changed = true;
                if capturing {
                    self.cancel_capture(out);
                }
            }
        });
    }
}

/// A settings row: a right-aligned label column, then the controls.
fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    row_w(ui, label, LABEL_W, 6.0, add);
}

fn row_w(ui: &mut Ui, label: &str, label_w: f32, gap: f32, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(vec2(label_w, ROW_H), Layout::right_to_left(Align::Center), |ui| {
            ui.set_min_size(vec2(label_w, ROW_H));
            if !label.is_empty() {
                ui.label(egui::RichText::new(label).color(TEXT));
            }
        });
        ui.add_space(gap);
        add(ui);
    });
}

fn separator(ui: &mut Ui) {
    let r = ui.available_rect_before_wrap();
    let y = r.min.y + 1.0;
    ui.painter().line_segment([pos2(r.min.x, y), pos2(r.max.x, y)], Stroke::new(1.0, Color32::from_gray(120)));
    ui.add_space(4.0);
}

fn short_path(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    match dirs::home_dir() {
        Some(h) => {
            let h = h.display().to_string();
            if let Some(rest) = s.strip_prefix(&h) { format!("~{rest}") } else { s }
        }
        None => s,
    }
}

pub fn app_name(app: &str) -> String {
    std::path::Path::new(app)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(app)
        .to_string()
}

/// Maps a pressed key to a W3C key code ("KeyA", "Digit4", "F5", ...).
pub fn key_to_code(k: Key) -> Option<String> {
    let n = k.name();
    if n.len() == 1 {
        let c = n.chars().next()?;
        if c.is_ascii_alphabetic() {
            return Some(format!("Key{}", c.to_ascii_uppercase()));
        }
        if c.is_ascii_digit() {
            return Some(format!("Digit{c}"));
        }
    }
    if n.starts_with('F') && n.len() > 1 && n[1..].chars().all(|c| c.is_ascii_digit()) {
        return Some(n.to_string());
    }
    Some(
        match k {
            Key::Space => "Space",
            Key::Minus => "Minus",
            Key::Equals | Key::Plus => "Equal",
            Key::Comma => "Comma",
            Key::Period => "Period",
            Key::Slash => "Slash",
            Key::Backslash => "Backslash",
            Key::Semicolon => "Semicolon",
            Key::Quote => "Quote",
            Key::OpenBracket => "BracketLeft",
            Key::CloseBracket => "BracketRight",
            Key::Backtick => "Backquote",
            Key::ArrowUp => "ArrowUp",
            Key::ArrowDown => "ArrowDown",
            Key::ArrowLeft => "ArrowLeft",
            Key::ArrowRight => "ArrowRight",
            Key::Enter => "Enter",
            Key::Tab => "Tab",
            Key::Backspace => "Backspace",
            Key::Delete => "Delete",
            _ => return None,
        }
        .to_string(),
    )
}
