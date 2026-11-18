//! The About window: version, what LightSnap is, links to the project and the licenses.
//! LightSnap makes no network connections itself; every link opens in the browser.

use egui::{Align, Color32, FontFamily, FontId, Key, Layout, RichText, TextureHandle, ViewportBuilder, ViewportCommand, ViewportId, vec2};
use std::path::{Path, PathBuf};

const BG: Color32 = Color32::from_rgb(34, 34, 36);
const TEXT: Color32 = Color32::from_rgb(232, 232, 236);
const DIM: Color32 = Color32::from_rgb(140, 140, 146);
const WIN_W: f32 = 380.0;

const REPO: &str = "https://github.com/badger12345/lightsnap";
const ICON_PNG: &[u8] = include_bytes!("../linux/io.github.badger12345.LightSnap.png");

pub struct About {
    pub open: bool,
    focused: bool,
    /// Frames to wait before the window is made (Linux, as for the settings window).
    wait: u32,
    content_h: f32,
    icon: Option<TextureHandle>,
}

impl Default for About {
    fn default() -> Self {
        Self { open: false, focused: false, wait: 0, content_h: 0.0, icon: None }
    }
}

impl About {
    pub fn open(&mut self) {
        // Wayland ignores "bring to front" for a window that is already open, so on
        // Linux it is made again.
        if cfg!(target_os = "linux") {
            self.wait = if self.open { 1 } else { 30 };
        }
        self.open = true;
        self.focused = false;
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        if self.wait > 0 {
            let main_focused = ctx.input(|i| i.viewport().focused) == Some(true);
            self.wait = if main_focused { self.wait.min(1) - 1 } else { self.wait - 1 };
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            return;
        }
        if self.icon.is_none() {
            self.icon = image::load_from_memory(ICON_PNG).ok().map(|img| {
                let img = img.to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                ctx.load_texture("about_icon", ci, egui::TextureOptions::LINEAR)
            });
        }
        let builder = ViewportBuilder::default()
            .with_app_id(crate::platform::APP_ID)
            .with_title("About LightSnap")
            .with_inner_size(vec2(WIN_W, 400.0))
            .with_resizable(false)
            .with_minimize_button(false)
            .with_maximize_button(false);
        let mut close = false;
        let icon = self.icon.clone();
        crate::show_immediate(ctx, ViewportId::from_hash_of("lightsnap_about"), builder, |ui, _| {
            let ctx = ui.ctx().clone();
            if !self.focused {
                self.focused = true;
                ctx.send_viewport_cmd(ViewportCommand::Focus);
            }
            if ctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::W) || i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
                close = true;
            }
            let full = ui.max_rect();
            ui.painter().rect_filled(full, 0.0, BG);
            let body = full.shrink2(vec2(24.0, 0.0));
            let used = ui.scope_builder(egui::UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Center)), |ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
                ui.add_space(22.0);
                if let Some(t) = &icon {
                    ui.add(egui::Image::new(t).fit_to_exact_size(vec2(88.0, 88.0)));
                }
                ui.add_space(8.0);
                ui.label(RichText::new("LightSnap").font(FontId::new(22.0, FontFamily::Name("annot".into()))).color(TEXT));
                ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(DIM));
                ui.add_space(12.0);
                ui.label(
                    RichText::new("A lightweight, free and open-source app for screenshots and screen recordings.")
                        .color(TEXT),
                );
                ui.add_space(2.0);
                ui.label(RichText::new("Nothing is ever uploaded.").color(DIM));
                ui.add_space(16.0);
                centered_row(ui, |ui| {
                    if ui.button("Website").clicked() {
                        open_url(REPO);
                    }
                    if ui.button("Check for Updates").clicked() {
                        open_url(&format!("{REPO}/releases"));
                    }
                    if ui.button("Report a Problem").clicked() {
                        open_url(&format!("{REPO}/issues"));
                    }
                });
                ui.add_space(18.0);
                ui.label(RichText::new("© 2026 badger12345 · MIT License").size(11.0).color(DIM));
                ui.add_space(2.0);
                if ui.button(RichText::new("Third-Party Licenses").size(11.0)).clicked() {
                    match notices_file() {
                        Some(p) => crate::platform::open_path(&p),
                        None => open_url(&format!("{REPO}/blob/main/THIRD_PARTY_NOTICES.txt")),
                    }
                }
                ui.add_space(4.0);
                ui.label(RichText::new("Provided as is, without warranty.").size(11.0).color(DIM));
                ui.add_space(18.0);
                ui.min_rect().height()
            });
            // Fit the window height to the contents (not when drawn inside the main window
            // for check pictures: the command would resize that one).
            let want = used.inner;
            if !ctx.embed_viewports() && (want - self.content_h).abs() > 1.0 {
                self.content_h = want;
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(WIN_W, want)));
            }
        });
        if close {
            self.open = false;
            self.content_h = 0.0;
        }
    }
}

/// Lays out a row of buttons centred in the available width.
fn centered_row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    // Measure the row once, invisibly, then draw it with the space split on both sides.
    let id = ui.id().with("row_w");
    let w: f32 = ui.ctx().data(|d| d.get_temp(id)).unwrap_or(0.0);
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
        let start = ui.cursor().min.x;
        add(ui);
        let new_w = ui.cursor().min.x - start - ui.spacing().item_spacing.x;
        if (new_w - w).abs() > 0.5 {
            ui.ctx().data_mut(|d| d.insert_temp(id, new_w));
            ui.ctx().request_repaint();
        }
    });
}

fn open_url(url: &str) {
    crate::platform::open_path(Path::new(url));
}

/// THIRD_PARTY_NOTICES.txt of this copy: in the app bundle (macOS), next to the program
/// (Linux package), or at the top of the source folder (run from source).
fn notices_file() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("../Resources"), dir.to_path_buf(), dir.join("../..")]
        .into_iter()
        .map(|d| d.join("THIRD_PARTY_NOTICES.txt"))
        .find(|p| p.is_file())
}
