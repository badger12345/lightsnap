//! The red frame around the record area while recording, as on macOS.
//!
//! A normal Wayland window can neither place itself nor stay on top. The wlr-layer-shell
//! protocol can do both: KDE Plasma, Sway, Hyprland and COSMIC support it, GNOME does not.
//! The frame is four thin bars just outside the area, each a layer surface on the
//! overlay layer that takes no clicks and no keys. The recording crops to the area, so
//! the bars are not in the video.
//!
//! The bars live on their own Wayland connection in their own thread. Without layer
//! shell `RecFrame::show` returns `None`, and the recording runs without a frame.

use egui::{Rect, pos2, vec2};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm, registry_handlers,
};
use std::os::fd::AsRawFd;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_output, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};

/// Bar thickness and gap to the area, in points. The same as on macOS.
const THICK: f32 = 3.0;
const GAP: f32 = 2.0;
/// Red while recording, orange while paused (ARGB).
const RED: u32 = 0xFF_E2_26_26;
const ORANGE: u32 = 0xFF_FF_AA_00;

enum Cmd {
    Paused(bool),
    Hide,
}

/// The frame on screen. Dropping it removes the frame.
pub struct RecFrame {
    tx: mpsc::Sender<Cmd>,
    thread: Option<JoinHandle<()>>,
}

impl RecFrame {
    /// Shows the frame around `area` (global points). `None` when the desktop has no
    /// layer shell, or the area fills its screen (the frame would be off screen).
    pub fn show(area: Rect) -> Option<RecFrame> {
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("rec-frame".into())
            .spawn(move || {
                if let Err(e) = run(area, rx, &ready_tx) {
                    log::info!(target: "lightsnap", "no recording frame: {e}");
                    let _ = ready_tx.try_send(false);
                }
            })
            .ok()?;
        // Setting up takes a few round trips to the compositor.
        if ready_rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false) {
            Some(RecFrame { tx, thread: Some(thread) })
        } else {
            let _ = tx.send(Cmd::Hide);
            None
        }
    }

    /// Orange while paused, red while recording.
    pub fn set_paused(&self, paused: bool) {
        let _ = self.tx.send(Cmd::Paused(paused));
    }
}

impl Drop for RecFrame {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Hide);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Whether the desktop has layer shell, so `RecFrame::show` can draw the frame. GNOME has
/// none; there a countdown shows the area before the recording starts. Asked once.
pub fn frame_supported() -> bool {
    static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        let Ok(conn) = Connection::connect_to_env() else { return false };
        let Ok((globals, _queue)) = registry_queue_init::<State>(&conn) else { return false };
        globals.contents().with_list(|list| list.iter().any(|g| g.interface == "zwlr_layer_shell_v1"))
    })
}

/// The four bars around `area`, in global points, cut to the screen `screen`. A bar that
/// would lie off screen (the area touches the screen edge) is left out.
fn bars(area: Rect, screen: Rect) -> Vec<Rect> {
    let o = area.expand(GAP + THICK);
    [
        Rect::from_min_size(o.min, vec2(o.width(), THICK)),
        Rect::from_min_size(pos2(o.min.x, o.max.y - THICK), vec2(o.width(), THICK)),
        Rect::from_min_size(pos2(o.min.x, o.min.y + THICK), vec2(THICK, o.height() - 2.0 * THICK)),
        Rect::from_min_size(pos2(o.max.x - THICK, o.min.y + THICK), vec2(THICK, o.height() - 2.0 * THICK)),
    ]
    .into_iter()
    .map(|b| b.intersect(screen))
    .filter(|b| b.width() >= 1.0 && b.height() >= 1.0)
    .collect()
}

struct Bar {
    layer: LayerSurface,
    size: (u32, u32),
    configured: bool,
    /// Kept until the next one is drawn, so the compositor can still read it.
    buffer: Option<Buffer>,
}

struct State {
    registry: RegistryState,
    outputs: OutputState,
    shm: Shm,
    pool: Option<SlotPool>,
    bars: Vec<Bar>,
    color: u32,
    closed: bool,
}

impl State {
    fn draw(&mut self, i: usize) {
        let color = self.color;
        let Some(pool) = &mut self.pool else { return };
        let bar = &mut self.bars[i];
        if !bar.configured {
            return;
        }
        let (w, h) = (bar.size.0 as i32, bar.size.1 as i32);
        let Ok((buffer, canvas)) = pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) else { return };
        for px in canvas.chunks_exact_mut(4) {
            px.copy_from_slice(&color.to_le_bytes());
        }
        let surface = bar.layer.wl_surface();
        surface.damage_buffer(0, 0, w, h);
        if buffer.attach_to(surface).is_ok() {
            bar.layer.commit();
            bar.buffer = Some(buffer);
        }
    }
}

fn run(area: Rect, rx: mpsc::Receiver<Cmd>, ready: &mpsc::SyncSender<bool>) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).map_err(|e| e.to_string())?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|_| "the desktop has no layer shell".to_string())?;
    let shm = Shm::bind(&globals, &qh).map_err(|e| e.to_string())?;
    let mut st = State {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        shm,
        pool: None,
        bars: Vec::new(),
        color: RED,
        closed: false,
    };
    // Two round trips: the outputs, then their details (xdg-output).
    for _ in 0..2 {
        queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
    }

    // The screen that holds the area, in the same global points as `displays()`.
    let screens: Vec<(wl_output::WlOutput, Rect)> = st
        .outputs
        .outputs()
        .filter_map(|o| {
            let info = st.outputs.info(&o)?;
            let (x, y) = info.logical_position.unwrap_or(info.location);
            let (w, h) = info.logical_size?;
            Some((o, Rect::from_min_size(pos2(x as f32, y as f32), vec2(w as f32, h as f32))))
        })
        .collect();
    let (output, screen) = screens
        .iter()
        .find(|(_, r)| r.contains(area.center()))
        .or(screens.first())
        .ok_or("no screen found")?;
    let bars = bars(area, *screen);
    if bars.is_empty() {
        return Err("the area fills the screen".into());
    }

    let mut bytes = 0;
    for b in &bars {
        let (w, h) = (b.width().round().max(1.0) as u32, b.height().round().max(1.0) as u32);
        bytes += (w * h * 4) as usize;
        let surface = compositor.create_surface(&qh);
        // Clicks go through the bars, to whatever is below.
        if let Ok(region) = Region::new(&compositor) {
            surface.set_input_region(Some(region.wl_region()));
        }
        let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("lightsnap-frame"), Some(output));
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        // Place it from the screen's corner, not moved aside by panels.
        layer.set_exclusive_zone(-1);
        let local = b.min - screen.min;
        layer.set_margin(local.y.round() as i32, 0, 0, local.x.round() as i32);
        layer.set_size(w, h);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        st.bars.push(Bar { layer, size: (w, h), configured: false, buffer: None });
    }
    // Room for two buffers per bar: one shown, one being drawn after a color change.
    st.pool = Some(SlotPool::new(bytes * 2, &st.shm).map_err(|e| e.to_string())?);
    queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
    let _ = ready.try_send(true);
    log::info!(target: "lightsnap", "recording frame shown: {} bars", st.bars.len());

    // Wait for commands, and keep answering the compositor (it may close the bars).
    loop {
        loop {
            match rx.try_recv() {
                Ok(Cmd::Paused(p)) => {
                    let color = if p { ORANGE } else { RED };
                    if color != st.color {
                        st.color = color;
                        for i in 0..st.bars.len() {
                            st.draw(i);
                        }
                    }
                }
                Ok(Cmd::Hide) | Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        if st.closed {
            return Ok(());
        }
        queue.dispatch_pending(&mut st).map_err(|e| e.to_string())?;
        conn.flush().map_err(|e| e.to_string())?;
        if let Some(guard) = queue.prepare_read() {
            let mut fd = libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
            // SAFETY: one valid pollfd; the fd stays open while the guard lives.
            let n = unsafe { libc::poll(&mut fd, 1, 100) };
            if n > 0 {
                guard.read().map_err(|e| e.to_string())?;
            }
        }
        queue.dispatch_pending(&mut st).map_err(|e| e.to_string())?;
    }
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.closed = true;
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, c: LayerSurfaceConfigure, _: u32) {
        let Some(i) = self.bars.iter().position(|b| &b.layer == layer) else { return };
        let bar = &mut self.bars[i];
        if c.new_size.0 > 0 && c.new_size.1 > 0 {
            bar.size = c.new_size;
        }
        bar.configured = true;
        self.draw(i);
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers!(OutputState);
}

delegate_compositor!(State);
delegate_output!(State);
delegate_shm!(State);
delegate_layer!(State);
delegate_registry!(State);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_sit_outside_the_area() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
        let area = Rect::from_min_size(pos2(100.0, 100.0), vec2(400.0, 300.0));
        let b = bars(area, screen);
        assert_eq!(b.len(), 4);
        for bar in &b {
            assert!(!bar.intersects(area), "{bar:?} covers the area");
        }
        // Top bar: 5 points above the area, as wide as the area plus both side bars.
        assert_eq!(b[0], Rect::from_min_size(pos2(95.0, 95.0), vec2(410.0, 3.0)));
    }

    #[test]
    fn bars_off_screen_are_left_out() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
        // Area at the top-left corner: no top and no left bar.
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
        assert_eq!(bars(area, screen).len(), 2);
        // Whole screen: nothing to draw.
        assert!(bars(screen, screen).is_empty());
    }
}
