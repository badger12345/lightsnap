//! Screen video recording. Uses ffmpeg when it is installed (frame rate, quality,
//! pause and audio work then), otherwise macOS's own `screencapture -v`.
//! Recording runs in a separate process; the app only shows a timer.

use crate::settings::Quality;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub fn find_ffmpeg() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = vec![
        "/opt/homebrew/bin/ffmpeg".into(),
        "/usr/local/bin/ffmpeg".into(),
        "/opt/local/bin/ffmpeg".into(),
    ];
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join("ffmpeg")));
    }
    candidates.into_iter().find(|p| p.is_file())
}

#[derive(Clone, Debug, Default)]
pub struct Devices {
    /// avfoundation index of the main screen ("Capture screen 0").
    pub screen_index: Option<usize>,
    /// avfoundation index of "Capture screen N", for every screen N.
    pub screens: Vec<(usize, usize)>,
    pub audio_inputs: Vec<String>,
}

/// Asks ffmpeg which screens and microphones exist. Takes well under a second.
pub fn list_devices(ffmpeg: &Path) -> Devices {
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-f", "avfoundation", "-list_devices", "true", "-i", ""])
        .stdin(Stdio::null())
        .output();
    let mut devices = Devices::default();
    let Ok(out) = out else { return devices };
    let text = String::from_utf8_lossy(&out.stderr);
    let mut in_audio = false;
    for line in text.lines() {
        if line.contains("video devices:") {
            in_audio = false;
            continue;
        }
        if line.contains("audio devices:") {
            in_audio = true;
            continue;
        }
        // Lines look like: "[AVFoundation indev @ 0x..] [1] Capture screen 0"
        let Some(pos) = line.rfind("] [") else { continue };
        let rest = &line[pos + 3..];
        let Some(close) = rest.find(']') else { continue };
        let Ok(idx) = rest[..close].parse::<usize>() else { continue };
        let name = rest[close + 1..].trim().to_string();
        if in_audio {
            devices.audio_inputs.push(name);
        } else if let Some(n) = name.strip_prefix("Capture screen ").and_then(|n| n.trim().parse::<usize>().ok()) {
            devices.screens.push((n, idx));
            if n == 0 {
                devices.screen_index = Some(idx);
            }
        }
    }
    devices
}

#[derive(Clone, Debug)]
pub struct RecordOptions {
    /// Area in points (x, y, w, h), relative to the top-left of the screen being recorded.
    pub rect_pt: [f32; 4],
    /// The same area in global points (used by the macOS recorder fallback).
    pub rect_global: [f32; 4],
    /// Which screen (0 = main), in the order of `displays::list`.
    pub screen: usize,
    /// Screen pixels per point.
    pub scale: f32,
    pub fps: u32,
    pub quality: Quality,
    pub cursor: bool,
    pub clicks: bool,
    pub audio: Option<String>,
}

enum Backend {
    Ffmpeg { exe: PathBuf, screen_index: usize },
    ScreenCapture,
}

pub struct Recorder {
    opts: RecordOptions,
    backend: Backend,
    child: Option<Child>,
    segments: Vec<PathBuf>,
    started: Instant,
    paused_total: Duration,
    paused_at: Option<Instant>,
    pub error: Option<String>,
    log: PathBuf,
}

impl Recorder {
    pub fn start(opts: RecordOptions, ffmpeg: Option<PathBuf>, devices: &Devices) -> Result<Self, String> {
        let screen_index = devices
            .screens
            .iter()
            .find(|(n, _)| *n == opts.screen)
            .map(|(_, i)| *i)
            .or(if opts.screen == 0 { devices.screen_index } else { None });
        let backend = match (ffmpeg, screen_index) {
            (Some(exe), Some(screen_index)) => Backend::Ffmpeg { exe, screen_index },
            _ => Backend::ScreenCapture,
        };
        let mut rec = Recorder {
            opts,
            backend,
            child: None,
            segments: Vec::new(),
            started: Instant::now(),
            paused_total: Duration::ZERO,
            paused_at: None,
            error: None,
            log: crate::capture::temp_file("rec", "log"),
        };
        rec.start_segment()?;
        Ok(rec)
    }

    pub fn can_pause(&self) -> bool {
        matches!(self.backend, Backend::Ffmpeg { .. })
    }

    pub fn is_paused(&self) -> bool {
        self.paused_at.is_some()
    }

    pub fn elapsed(&self) -> Duration {
        let now = self.paused_at.unwrap_or_else(Instant::now);
        now.saturating_duration_since(self.started).saturating_sub(self.paused_total)
    }

    fn log_sink(&self) -> Stdio {
        std::fs::File::create(&self.log).map(Stdio::from).unwrap_or_else(|_| Stdio::null())
    }

    fn start_segment(&mut self) -> Result<(), String> {
        let o = &self.opts;
        let [x, y, w, h] = o.rect_pt;
        let child = match &self.backend {
            Backend::Ffmpeg { exe, screen_index } => {
                let seg = crate::capture::temp_file("rec", "mp4");
                let s = o.scale.max(1.0);
                let even = |v: f32| ((v * s).round() as i64 / 2 * 2).max(2);
                let (px, py, pw, ph) = (
                    ((x * s).round() as i64).max(0),
                    ((y * s).round() as i64).max(0),
                    even(w),
                    even(h),
                );
                let mut filter = format!("crop={pw}:{ph}:{px}:{py}");
                if o.quality == Quality::Low && pw >= 640 {
                    filter.push_str(",scale=trunc(iw/4)*2:trunc(ih/4)*2");
                }
                let input = match &o.audio {
                    Some(a) => format!("{screen_index}:{a}"),
                    None => format!("{screen_index}:none"),
                };
                let mut cmd = Command::new(exe);
                cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "avfoundation"])
                    .args(["-capture_cursor", if o.cursor { "1" } else { "0" }])
                    .args(["-capture_mouse_clicks", if o.clicks { "1" } else { "0" }])
                    .args(["-framerate", &o.fps.to_string()])
                    .args(["-pixel_format", "nv12"])
                    .args(["-i", &input])
                    .args(["-vf", &filter])
                    .args(["-r", &o.fps.to_string()])
                    .args(["-c:v", "libx264", "-preset", "veryfast"])
                    .args(["-crf", &o.quality.crf().to_string(), "-pix_fmt", "yuv420p"]);
                if o.audio.is_some() {
                    cmd.args(["-c:a", "aac", "-b:a", "128k"]);
                }
                cmd.arg(&seg);
                self.segments.push(seg);
                cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(self.log_sink()).spawn()
            }
            Backend::ScreenCapture => {
                let seg = crate::capture::temp_file("rec", "mov");
                let mut cmd = Command::new("/usr/sbin/screencapture");
                let [gx, gy, gw, gh] = o.rect_global;
                let _ = (x, y, w, h);
                cmd.args(["-v", "-x", "-R"]).arg(format!(
                    "{},{},{},{}",
                    gx.round() as i64,
                    gy.round() as i64,
                    gw.round() as i64,
                    gh.round() as i64
                ));
                if o.cursor {
                    cmd.arg("-C");
                }
                if o.clicks {
                    cmd.arg("-k");
                }
                if o.audio.is_some() {
                    cmd.arg("-g");
                }
                cmd.arg(&seg);
                self.segments.push(seg);
                cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(self.log_sink()).spawn()
            }
        };
        match child {
            Ok(c) => {
                self.child = Some(c);
                Ok(())
            }
            Err(e) => Err(format!("Could not start recording: {e}")),
        }
    }

    /// Checks whether the recorder process died on its own (e.g. no permission).
    pub fn poll_failure(&mut self) -> Option<String> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => {
                let msg: String = std::fs::read_to_string(&self.log)
                    .unwrap_or_default()
                    .lines()
                    .filter(|l| !l.starts_with("objc[") && !l.contains("pixel format"))
                    .collect::<Vec<_>>()
                    .join(" ");
                self.child = None;
                let msg = if msg.trim().is_empty() {
                    format!(
                        "Recording stopped unexpectedly ({status}). Check that screen recording is allowed in System Settings › Privacy & Security."
                    )
                } else {
                    format!("Recording stopped: {}", msg.trim())
                };
                Some(msg)
            }
            _ => None,
        }
    }

    fn stop_segment(&mut self) {
        let Some(mut child) = self.child.take() else { return };
        match self.backend {
            Backend::Ffmpeg { .. } => {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(b"q");
                    let _ = stdin.flush();
                }
            }
            Backend::ScreenCapture => {
                // screencapture stops on a key press or Ctrl-C.
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(b"\n");
                }
                let _ = Command::new("/bin/kill")
                    .args(["-INT", &child.id().to_string()])
                    .status();
            }
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(30))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
            }
        }
    }

    pub fn toggle_pause(&mut self) {
        if !self.can_pause() {
            return;
        }
        if let Some(at) = self.paused_at.take() {
            self.paused_total += at.elapsed();
            if let Err(e) = self.start_segment() {
                self.error = Some(e);
            }
        } else {
            self.stop_segment();
            self.paused_at = Some(Instant::now());
        }
    }

    /// Stops and throws the recording away.
    pub fn cancel(mut self) {
        self.stop_segment();
        for s in &self.segments {
            let _ = std::fs::remove_file(s);
        }
    }

    /// Stops and writes the video into `dest_dir`. Blocks for a moment.
    pub fn finish(mut self, dest_stem_path: PathBuf) -> Result<PathBuf, String> {
        self.stop_segment();
        let segments: Vec<PathBuf> = self.segments.iter().filter(|p| p.exists()).cloned().collect();
        if segments.is_empty() {
            return Err(
                "No video was recorded. Check that screen recording is allowed in System Settings › Privacy & Security."
                    .into(),
            );
        }
        let ext = segments[0].extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_string();
        let dest = dest_stem_path.with_extension(&ext);
        if let Some(d) = dest.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if segments.len() == 1 {
            move_file(&segments[0], &dest)?;
            return Ok(dest);
        }
        // Several pieces (after pause): join them without re-encoding.
        let Backend::Ffmpeg { exe, .. } = &self.backend else {
            move_file(&segments[0], &dest)?;
            return Ok(dest);
        };
        let list = crate::capture::temp_file("list", "txt");
        let body: String = segments
            .iter()
            .map(|p| format!("file '{}'\n", p.display().to_string().replace('\'', "'\\''")))
            .collect();
        std::fs::write(&list, body).map_err(|e| e.to_string())?;
        let ok = Command::new(exe)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "concat", "-safe", "0", "-i"])
            .arg(&list)
            .args(["-c", "copy"])
            .arg(&dest)
            .stdin(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        let _ = std::fs::remove_file(&list);
        if ok {
            for s in &segments {
                let _ = std::fs::remove_file(s);
            }
            Ok(dest)
        } else {
            Err("Could not join the recorded pieces.".into())
        }
    }
}

fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(from);
    Ok(())
}

pub fn format_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs screen-recording permission; run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn record_pause_finish() {
        let ff = find_ffmpeg().expect("ffmpeg");
        let devices = list_devices(&ff);
        assert!(devices.screen_index.is_some());
        let opts = RecordOptions {
            rect_pt: [50.0, 50.0, 300.0, 200.0],
            rect_global: [50.0, 50.0, 300.0, 200.0],
            screen: 0,
            scale: 2.0,
            fps: 15,
            quality: Quality::High,
            cursor: true,
            clicks: true,
            audio: None,
        };
        let mut r = Recorder::start(opts, Some(ff), &devices).unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        r.toggle_pause();
        std::thread::sleep(Duration::from_millis(500));
        r.toggle_pause();
        std::thread::sleep(Duration::from_millis(1500));
        assert!(r.poll_failure().is_none());
        let out = r.finish(std::env::temp_dir().join("lightsnap-test-rec")).unwrap();
        let meta = std::fs::metadata(&out).unwrap();
        assert!(meta.len() > 1000, "file too small");
        println!("recorded {} ({} bytes)", out.display(), meta.len());
    }
}
