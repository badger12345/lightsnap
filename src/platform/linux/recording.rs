//! Screen recording on Wayland. An app may not read the screen itself, so:
//!
//! 1. The ScreenCast portal asks the person which screen to share (once: the answer is
//!    kept as a restore token) and hands out a PipeWire stream of it.
//! 2. A PipeWire thread keeps the newest frame of that stream.
//! 3. The first frame is the still picture the record area is chosen on.
//! 4. While recording, a writer thread feeds frames at a steady rate into a named pipe
//!    that ffmpeg reads as raw video; ffmpeg crops and encodes as on macOS.
//!
//! The session ends with `end_screen_session`, or when LightSnap quits.

use crate::platform::{Devices, temp_file};
use ashpd::desktop::PersistMode;
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use pipewire as pw;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- frames

/// One picture of the shared screen, rows packed without gaps.
#[derive(Clone)]
struct Frame {
    width: u32,
    height: u32,
    /// ffmpeg's name for the pixel layout ("bgr0", "rgba", ...).
    pix_fmt: &'static str,
    data: Arc<Vec<u8>>,
    /// Counts up with every frame the compositor sends.
    seq: u64,
}

struct Cast {
    session: ashpd::desktop::Session<Screencast>,
    frame: Arc<Mutex<Option<Frame>>>,
    stop: pw::channel::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Where the shared screen sits on the desktop, in points.
    position: Option<(i32, i32)>,
}

static CAST: Mutex<Option<Cast>> = Mutex::new(None);
/// Description → PulseAudio source name, filled by `list_devices`.
static SOURCES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Where the restore token lives, so the person picks the screen only once.
fn token_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("LightSnap").join("screencast-token"))
}

/// Starts sharing a screen: asks the portal (a dialog the first time), then connects to
/// the PipeWire stream. Blocks until the first frame is there.
fn start_cast(cursor: bool) -> Result<(), String> {
    end_screen_session();
    super::portal::register();
    let token = token_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| t.trim().to_string());
    let res: Result<_, ashpd::Error> = pollster::block_on(async {
        let proxy = Screencast::new().await?;
        let session = proxy.create_session(Default::default()).await?;
        let modes = proxy.available_cursor_modes().await.unwrap_or(CursorMode::HIDDEN);
        let cursor_mode = if cursor && modes.contains(CursorMode::EMBEDDED) { CursorMode::EMBEDDED } else { CursorMode::HIDDEN };
        proxy
            .select_sources(
                &session,
                SelectSourcesOptions::default()
                    .set_cursor_mode(cursor_mode)
                    .set_sources(SourceType::MONITOR)
                    .set_multiple(false)
                    .set_persist_mode(PersistMode::ExplicitlyRevoked)
                    .set_restore_token(token.as_deref().filter(|t| !t.is_empty())),
            )
            .await?
            .response()?;
        let streams = proxy.start(&session, None, Default::default()).await?.response()?;
        let fd = proxy.open_pipe_wire_remote(&session, Default::default()).await?;
        Ok((session, streams, fd))
    });
    let (session, streams, fd) = match res {
        Ok(v) => v,
        Err(e) => return Err(format!("{e}")),
    };
    if let (Some(t), Some(p)) = (streams.restore_token(), token_path()) {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, t);
    }
    let Some(stream) = streams.streams().first() else {
        let _ = pollster::block_on(session.close());
        return Err("no screen was shared".into());
    };
    let node = stream.pipe_wire_node_id();
    let position = stream.position();
    let frame: Arc<Mutex<Option<Frame>>> = Arc::new(Mutex::new(None));
    let (stop, stop_rx) = pw::channel::channel::<()>();
    let f = frame.clone();
    let thread = std::thread::Builder::new()
        .name("pipewire".into())
        .spawn(move || {
            if let Err(e) = run_stream(fd, node, f, stop_rx) {
                log::warn!(target: "lightsnap", "pipewire: {e}");
            }
        })
        .map_err(|e| e.to_string())?;
    *CAST.lock().map_err(|e| e.to_string())? = Some(Cast { session, frame: frame.clone(), stop, thread: Some(thread), position });
    // The compositor sends a frame right away.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if frame.lock().map(|f| f.is_some()).unwrap_or(false) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    end_screen_session();
    Err("the shared screen sent no picture".into())
}

/// Stops sharing the screen. Safe to call when nothing is shared.
pub fn end_screen_session() {
    let cast = CAST.lock().ok().and_then(|mut c| c.take());
    if let Some(mut c) = cast {
        let _ = c.stop.send(());
        if let Some(t) = c.thread.take() {
            let _ = t.join();
        }
        let _ = pollster::block_on(c.session.close());
    }
}

/// Where the shared screen sits on the desktop, if a screen is shared.
pub(super) fn shared_screen_position() -> Option<(i32, i32)> {
    CAST.lock().ok()?.as_ref()?.position
}

fn latest_frame() -> Option<Frame> {
    let cast = CAST.lock().ok()?;
    let f = cast.as_ref()?.frame.lock().ok()?.clone();
    f
}

/// The PipeWire side: connect to the portal's remote and keep the newest frame.
fn run_stream(fd: OwnedFd, node: u32, frame: Arc<Mutex<Option<Frame>>>, stop: pw::channel::Receiver<()>) -> Result<(), String> {
    use pw::spa;
    use pw::spa::param::video::VideoFormat;
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(|e| e.to_string())?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(|e| e.to_string())?;
    let core = context.connect_fd_rc(fd, None).map_err(|e| e.to_string())?;
    let ml = mainloop.clone();
    let _stop = stop.attach(mainloop.loop_(), move |_| ml.quit());

    struct Format {
        info: spa::param::video::VideoInfoRaw,
    }
    let mut seq = 0u64;
    let stream = pw::stream::StreamBox::new(
        &core,
        "lightsnap-screen",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|e| e.to_string())?;
    let _listener = stream
        .add_local_listener_with_user_data(Format { info: Default::default() })
        .param_changed(|_, fmt, id, param| {
            let Some(param) = param else { return };
            if id != pw::spa::param::ParamType::Format.as_raw() {
                return;
            }
            let _ = fmt.info.parse(param);
        })
        .process(move |stream, fmt| {
            seq += 1;
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let datas = buffer.datas_mut();
            let Some(d) = datas.first_mut() else { return };
            let (w, h) = (fmt.info.size().width, fmt.info.size().height);
            let pix_fmt = match fmt.info.format() {
                VideoFormat::BGRx => "bgr0",
                VideoFormat::BGRA => "bgra",
                VideoFormat::RGBx => "rgb0",
                VideoFormat::RGBA => "rgba",
                _ => return,
            };
            let stride = d.chunk().stride().max(0) as usize;
            let offset = d.chunk().offset() as usize;
            let size = d.chunk().size() as usize;
            let Some(bytes) = d.data() else { return };
            let row = w as usize * 4;
            let stride = if stride == 0 { row } else { stride };
            if w == 0 || h == 0 || size == 0 || bytes.len() < offset + stride * (h as usize - 1) + row {
                return; // an empty or partial buffer (for example only the cursor moved)
            }
            let mut packed = Vec::with_capacity(row * h as usize);
            for y in 0..h as usize {
                let start = offset + y * stride;
                packed.extend_from_slice(&bytes[start..start + row]);
            }
            if let Ok(mut f) = frame.lock() {
                *f = Some(Frame { width: w, height: h, pix_fmt, data: Arc::new(packed), seq });
            }
        })
        .register()
        .map_err(|e| e.to_string())?;

    let obj = pw::spa::pod::object!(
        pw::spa::utils::SpaTypes::ObjectParamFormat,
        pw::spa::param::ParamType::EnumFormat,
        pw::spa::pod::property!(pw::spa::param::format::FormatProperties::MediaType, Id, pw::spa::param::format::MediaType::Video),
        pw::spa::pod::property!(pw::spa::param::format::FormatProperties::MediaSubtype, Id, pw::spa::param::format::MediaSubtype::Raw),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            pw::spa::utils::Rectangle { width: 1920, height: 1080 },
            pw::spa::utils::Rectangle { width: 1, height: 1 },
            pw::spa::utils::Rectangle { width: 16384, height: 16384 }
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            pw::spa::utils::Fraction { num: 60, denom: 1 },
            pw::spa::utils::Fraction { num: 0, denom: 1 },
            pw::spa::utils::Fraction { num: 240, denom: 1 }
        ),
    );
    let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(obj),
    )
    .map_err(|e| format!("{e:?}"))?
    .0
    .into_inner();
    let mut params = [spa::pod::Pod::from_bytes(&values).ok_or("bad format")?];
    stream
        .connect(
            spa::utils::Direction::Input,
            Some(node),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(|e| e.to_string())?;
    mainloop.run();
    Ok(())
}

// ---------------------------------------------------------------- the still picture

/// Shares a screen and saves its first frame, for choosing the record area. The
/// screen stays shared for the recording (see `end_screen_session`).
pub fn capture_all_screens(_count: usize, cursor: bool) -> Vec<PathBuf> {
    if let Err(e) = start_cast(cursor) {
        log::warn!(target: "lightsnap", "screen cast: {e}");
        return Vec::new();
    }
    let Some(f) = latest_frame() else { return Vec::new() };
    let mut rgba = (*f.data).clone();
    if f.pix_fmt.starts_with("bgr") {
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
    }
    if f.pix_fmt.ends_with('0') {
        for px in rgba.chunks_exact_mut(4) {
            px[3] = 255;
        }
    }
    let Some(img) = image::RgbaImage::from_raw(f.width, f.height, rgba) else { return Vec::new() };
    let out = temp_file("screen0", "png");
    match img.save(&out) {
        Ok(()) => vec![out],
        Err(_) => Vec::new(),
    }
}

// ---------------------------------------------------------------- ffmpeg

/// Places to look for ffmpeg, in order: the system's. Only absolute PATH entries count:
/// an empty or relative one ("", ".") would run an ffmpeg from the current folder.
pub fn ffmpeg_candidates() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = vec!["/usr/bin/ffmpeg".into()];
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).filter(|d| d.is_absolute()).map(|d| d.join("ffmpeg")));
    }
    candidates
}

/// The screen being shared is input 0; microphones come from PulseAudio (or PipeWire's
/// Pulse server).
pub fn list_devices(ffmpeg: &Path) -> Devices {
    let mut devices = Devices { screen_index: Some(0), screens: vec![(0, 0)], audio_inputs: Vec::new() };
    let Ok(out) = Command::new(ffmpeg).args(["-hide_banner", "-sources", "pulse"]).output() else { return devices };
    // Lines look like: "* alsa_input.pci-0000_00_1f.3.analog-stereo [Built-in Audio Analog Stereo] (audio)"
    let mut map = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let line = line.trim_start_matches(['*', ' ']);
        let Some(open) = line.find(" [") else { continue };
        let name = line[..open].trim().to_string();
        if name.ends_with(".monitor") {
            continue; // the speakers' loopback, not a microphone
        }
        let desc = line[open + 2..].split(']').next().unwrap_or(&name).to_string();
        map.push((desc.clone(), name));
        devices.audio_inputs.push(desc);
    }
    if let Ok(mut s) = SOURCES.lock() {
        *s = map;
    }
    devices
}

/// Encode with x264 (the system ffmpeg has it). `quality` goes from 1 to 100.
pub fn video_encoder_args(quality: u32) -> Vec<String> {
    let crf = 40 - (quality.clamp(1, 100) * 22 / 100);
    ["-c:v", "libx264", "-preset", "veryfast", "-crf", &crf.to_string(), "-pix_fmt", "yuv420p"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

pub fn audio_encoder_args() -> Vec<String> {
    ["-c:a", "aac", "-b:a", "128k"].iter().map(|s| s.to_string()).collect()
}

/// Output options that make ffmpeg play sound on the default speakers.
pub fn sound_output_args() -> Vec<String> {
    ["-af", "aresample=48000,apad=pad_dur=0.3", "-f", "pulse", "-name", "LightSnap", "LightSnap"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// ffmpeg arguments for the shared screen (and microphone), up to and including `-i`.
/// Starts a writer that feeds the screen into a named pipe for this ffmpeg run.
pub fn ffmpeg_input_args(_screen_index: usize, audio: Option<&str>, fps: u32, _cursor: bool, _clicks: bool) -> Vec<String> {
    let Some(first) = latest_frame() else {
        // Nothing shared: let ffmpeg fail with a readable message.
        return vec!["-f".into(), "lavfi".into(), "-i".into(), "nullsrc=d=0:s=2x2".into(), "-frames:v".into(), "0".into()];
    };
    let fifo = temp_file("frames", "raw");
    let c = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap_or_default();
    if unsafe { libc::mkfifo(c.as_ptr(), 0o600) } != 0 {
        log::warn!(target: "lightsnap", "mkfifo failed: {}", std::io::Error::last_os_error());
    }
    let fps = fps.clamp(1, 60);
    let f2 = fifo.clone();
    let (w, h) = (first.width, first.height);
    std::thread::spawn(move || feed(&f2, w, h, fps));
    let mut args: Vec<String> = [
        "-f",
        "rawvideo",
        "-pixel_format",
        first.pix_fmt,
        "-video_size",
        &format!("{w}x{h}"),
        "-framerate",
        &fps.to_string(),
        "-thread_queue_size",
        "64",
        "-i",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.push(fifo.display().to_string());
    if let Some(a) = audio {
        let name = SOURCES
            .lock()
            .ok()
            .and_then(|s| s.iter().find(|(d, _)| d == a).map(|(_, n)| n.clone()))
            .unwrap_or_else(|| "default".into());
        args.extend(["-f", "pulse", "-thread_queue_size", "1024", "-i", &name].iter().map(|s| s.to_string()));
    }
    args
}

/// Writes the newest frame into the pipe `fps` times a second until ffmpeg stops
/// reading. Frames of another size (the screen changed) are skipped.
fn feed(fifo: &Path, w: u32, h: u32, fps: u32) {
    use std::os::unix::fs::OpenOptionsExt;
    // Opening a pipe for writing waits for the reader; poll so a failed ffmpeg can't hang us.
    let deadline = Instant::now() + Duration::from_secs(10);
    let file = loop {
        match std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(fifo) {
            Ok(f) => break Some(f),
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break None,
        }
    };
    let Some(mut file) = file else {
        let _ = std::fs::remove_file(fifo);
        return;
    };
    // From now on, writes wait until ffmpeg has room.
    unsafe {
        use std::os::fd::AsRawFd;
        let fd = file.as_raw_fd();
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
    }
    // The compositor sends frames only when the screen changes. The newest one may still
    // show LightSnap's area selection, which closes just now; wait briefly for the next.
    let start_seq = latest_frame().map(|f| f.seq).unwrap_or(0);
    let wait_until = Instant::now() + Duration::from_millis(700);
    while Instant::now() < wait_until && latest_frame().is_some_and(|f| f.seq == start_seq) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let period = Duration::from_secs_f64(1.0 / fps as f64);
    let mut next = Instant::now();
    let mut last: Option<Arc<Vec<u8>>> = None;
    loop {
        if let Some(f) = latest_frame() {
            if f.width == w && f.height == h {
                last = Some(f.data);
            }
        }
        let Some(data) = &last else { break };
        if file.write_all(data).is_err() {
            break; // ffmpeg stopped reading
        }
        next += period;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > Duration::from_secs(1) {
            next = now; // fell far behind (machine busy): don't try to catch up
        }
    }
    let _ = std::fs::remove_file(fifo);
}

/// Without ffmpeg there is no recorder on Linux; this command explains that.
pub fn fallback_record_command(_rect_global: [f32; 4], _cursor: bool, _clicks: bool, _audio: bool, _out: &Path) -> Command {
    let mut c = Command::new("sh");
    c.args(["-c", "echo 'Recording needs ffmpeg. Install it with your package manager.' >&2; exit 1"]);
    c
}

pub const FALLBACK_EXT: &str = "mp4";

pub fn stop_fallback(child: &mut Child) {
    let _ = child.kill();
}
