//! The clipboard through plain Wayland (wl_data_device), for GNOME.
//!
//! arboard needs a data-control protocol to set the clipboard from the background. KDE,
//! Sway, Hyprland and COSMIC have one; GNOME has none, and there arboard falls back to X11
//! through XWayland. Without XWayland nothing is copied.
//!
//! Every Wayland app may set the clipboard while one of its windows has the keyboard
//! focus, with the serial of a recent input event. LightSnap copies a screenshot right
//! when the mouse is let go on its area selection, which has the focus then. This module
//! listens to the seat on the main window's own Wayland connection (`attach`), so it
//! has that serial, and serves the copied data until something else is copied.

use std::io::Write;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wayland_backend::client::Backend;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source, wl_keyboard, wl_pointer, wl_registry, wl_seat,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, event_created_child};

/// What to put on the clipboard: the same bytes under several types.
pub struct Offer {
    pub mimes: Vec<String>,
    pub data: Arc<Vec<u8>>,
}

struct Request {
    offer: Offer,
    done: mpsc::SyncSender<bool>,
}

struct Attached {
    display: usize,
    tx: mpsc::Sender<Request>,
    /// The compositor has a data-control protocol: arboard can copy without focus there.
    data_control: bool,
}

static ATTACHED: Mutex<Option<Attached>> = Mutex::new(None);

/// Starts listening on the main window's Wayland connection. Called every frame; does
/// work only the first time (eframe keeps one connection for the whole run).
pub fn attach(window: &winit::window::Window) {
    use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
    let Ok(handle) = window.display_handle() else { return };
    let RawDisplayHandle::Wayland(h) = handle.as_raw() else { return };
    let display = h.display.as_ptr() as usize;
    let Ok(mut g) = ATTACHED.lock() else { return };
    if g.as_ref().is_some_and(|a| a.display == display) {
        return;
    }
    // SAFETY: the display belongs to winit's event loop, which eframe keeps for the whole
    // run of the app (a thread-local in eframe's `with_event_loop`).
    let backend = unsafe { Backend::from_foreign_display(display as *mut _) };
    let conn = Connection::from_backend(backend);
    let (tx, rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let spawned = std::thread::Builder::new().name("wl-clipboard".into()).spawn(move || {
        if let Err(e) = run(conn, rx, &ready_tx) {
            log::warn!(target: "lightsnap", "wayland clipboard: {e}");
            let _ = ready_tx.try_send(None);
        }
    });
    if spawned.is_err() {
        return;
    }
    if let Ok(Some(data_control)) = ready_rx.recv_timeout(Duration::from_secs(1)) {
        log::info!(target: "lightsnap", "wayland clipboard attached (data-control: {data_control})");
        *g = Some(Attached { display, tx, data_control });
    }
}

/// Sets the clipboard through Wayland, if that is the way on this desktop (no data-control)
/// and LightSnap has the focus. False means: use arboard.
pub fn set(offer: Offer) -> bool {
    let tx = {
        let Ok(g) = ATTACHED.lock() else { return false };
        match g.as_ref() {
            Some(a) if !a.data_control => a.tx.clone(),
            _ => return false,
        }
    };
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    if tx.send(Request { offer, done: done_tx }).is_err() {
        return false;
    }
    done_rx.recv_timeout(Duration::from_millis(500)).unwrap_or(false)
}

struct State {
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    device: Option<wl_data_device::WlDataDevice>,
    /// A surface of this app has the keyboard focus.
    focused: bool,
    /// The serial of the newest input event on this app's surfaces.
    serial: Option<u32>,
    /// The data being served, with its source.
    current: Option<(wl_data_source::WlDataSource, Arc<Vec<u8>>)>,
}

fn run(conn: Connection, rx: mpsc::Receiver<Request>, ready: &mpsc::SyncSender<Option<bool>>) -> Result<(), String> {
    let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let data_control = globals.contents().with_list(|list| {
        list.iter().any(|g| g.interface == "ext_data_control_manager_v1" || g.interface == "zwlr_data_control_manager_v1")
    });
    let manager: wl_data_device_manager::WlDataDeviceManager = globals.bind(&qh, 1..=3, ()).map_err(|e| e.to_string())?;
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=5, ()).map_err(|e| e.to_string())?;
    let mut st = State {
        device: Some(manager.get_data_device(&seat, &qh, ())),
        seat: Some(seat),
        keyboard: None,
        pointer: None,
        focused: false,
        serial: None,
        current: None,
    };
    queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
    let _ = ready.try_send(Some(data_control));
    loop {
        while let Ok(req) = rx.try_recv() {
            // Take in input events that winit read already, so the serial is current.
            let _ = queue.roundtrip(&mut st);
            let ok = match (st.focused, st.serial, &st.device) {
                (true, Some(serial), Some(device)) => {
                    let source = manager.create_data_source(&qh, ());
                    for m in &req.offer.mimes {
                        source.offer(m.clone());
                    }
                    device.set_selection(Some(&source), serial);
                    if let Some((old, _)) = st.current.replace((source, req.offer.data)) {
                        old.destroy();
                    }
                    conn.flush().is_ok()
                }
                _ => false,
            };
            let _ = req.done.send(ok);
        }
        queue.dispatch_pending(&mut st).map_err(|e| e.to_string())?;
        conn.flush().map_err(|e| e.to_string())?;
        // Wait for events, but not long: winit may read them for us on its thread.
        if let Some(guard) = queue.prepare_read() {
            let mut fd = libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
            // SAFETY: one valid pollfd; the fd stays open while the guard lives.
            if unsafe { libc::poll(&mut fd, 1, 50) } > 0 {
                let _ = guard.read();
            }
        }
    }
}

/// Writes the copied bytes into the pipe of the app that pastes. In a thread: the other
/// app may read slowly, and LightSnap must keep answering.
fn serve(fd: OwnedFd, data: Arc<Vec<u8>>) {
    std::thread::spawn(move || {
        // Mutter hands out a non-blocking pipe: a full pipe would end the write after the
        // first 64 KB. Wait for the reader instead.
        // SAFETY: fcntl on an fd we own.
        unsafe {
            let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
            if flags >= 0 {
                libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags & !libc::O_NONBLOCK);
            }
        }
        let mut f = std::fs::File::from(fd);
        let _ = f.write_all(&data);
    });
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(st: &mut Self, seat: &wl_seat::WlSeat, ev: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(caps) } = ev {
            if caps.contains(wl_seat::Capability::Keyboard) && st.keyboard.is_none() {
                st.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if caps.contains(wl_seat::Capability::Pointer) && st.pointer.is_none() {
                st.pointer = Some(seat.get_pointer(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(st: &mut Self, _: &wl_keyboard::WlKeyboard, ev: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            wl_keyboard::Event::Enter { serial, .. } => {
                st.focused = true;
                st.serial = Some(serial);
            }
            wl_keyboard::Event::Leave { .. } => st.focused = false,
            wl_keyboard::Event::Key { serial, .. } => st.serial = Some(serial),
            // The keymap's fd is closed when the event is dropped.
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(st: &mut Self, _: &wl_pointer::WlPointer, ev: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            wl_pointer::Event::Enter { serial, .. } | wl_pointer::Event::Button { serial, .. } => st.serial = Some(serial),
            _ => {}
        }
    }
}

impl Dispatch<wl_data_device_manager::WlDataDeviceManager, ()> for State {
    fn event(_: &mut Self, _: &wl_data_device_manager::WlDataDeviceManager, _: wl_data_device_manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for State {
    fn event(_: &mut Self, _: &wl_data_device::WlDataDevice, ev: wl_data_device::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        // Offers from other apps are not read here (arboard reads the clipboard).
        if let wl_data_device::Event::Selection { id: Some(offer) } = ev {
            offer.destroy();
        }
    }

    event_created_child!(State, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for State {
    fn event(_: &mut Self, _: &wl_data_offer::WlDataOffer, _: wl_data_offer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_data_source::WlDataSource, ()> for State {
    fn event(st: &mut Self, source: &wl_data_source::WlDataSource, ev: wl_data_source::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            wl_data_source::Event::Send { fd, .. } => {
                if let Some((s, data)) = &st.current {
                    if s == source {
                        serve(fd, data.clone());
                    }
                }
            }
            wl_data_source::Event::Cancelled => {
                // Something else was copied.
                if st.current.as_ref().is_some_and(|(s, _)| s == source) {
                    st.current = None;
                }
                source.destroy();
            }
            _ => {}
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(d) = self.device.take() {
            if d.version() >= 2 {
                d.release();
            }
        }
        let _ = self.seat.take();
    }
}
