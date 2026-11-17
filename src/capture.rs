//! Screen capture through macOS's own `screencapture` tool.
//! It runs as a short-lived child process, so the app itself stays small and idle.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureKind {
    Area,
    Window,
    FullScreen,
    Timed(u32),
}

impl CaptureKind {
    pub fn label(self) -> String {
        match self {
            CaptureKind::Area => "Capture Area".into(),
            CaptureKind::Window => "Capture Window".into(),
            CaptureKind::FullScreen => "Capture Full Screen".into(),
            CaptureKind::Timed(s) => format!("Timed Capture ({s} s)"),
        }
    }
}

pub fn temp_file(prefix: &str, ext: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("lightsnap");
    let _ = std::fs::create_dir_all(&dir);
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    dir.join(format!("{prefix}-{n}.{ext}"))
}

/// Runs the capture and blocks until it is done. Returns the image file,
/// or None if the person cancelled (pressed Esc) or capture failed.
pub fn run_capture(kind: CaptureKind, sound: bool) -> Option<PathBuf> {
    let out = temp_file("shot", "png");
    let mut cmd = Command::new("/usr/sbin/screencapture");
    if !sound {
        cmd.arg("-x"); // no shutter sound
    }
    match kind {
        CaptureKind::Area => {
            cmd.arg("-i");
        }
        CaptureKind::Window => {
            cmd.args(["-i", "-w"]);
        }
        CaptureKind::FullScreen => {
            cmd.arg("-m");
        }
        CaptureKind::Timed(s) => {
            cmd.args(["-m", "-T", &s.to_string()]);
        }
    }
    cmd.arg(&out);
    let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
    if ok && out.exists() { Some(out) } else { None }
}

/// Takes one silent picture of every screen (main screen first).
pub fn capture_all_screens(count: usize) -> Vec<PathBuf> {
    let files: Vec<PathBuf> = (0..count.max(1)).map(|i| temp_file(&format!("screen{i}"), "png")).collect();
    let ok = Command::new("/usr/sbin/screencapture")
        .arg("-x")
        .args(&files)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Vec::new();
    }
    files.into_iter().filter(|f| f.exists()).collect()
}

/// Copies a PNG file to the clipboard as an image.
pub fn copy_png_to_clipboard(png: &Path) -> bool {
    let script = format!(
        "set the clipboard to (read (POSIX file \"{}\") as «class PNGf»)",
        png.display().to_string().replace('"', "\\\"")
    );
    Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Copies plain text to the clipboard.
pub fn copy_text(text: &str) {
    use std::io::Write;
    if let Ok(mut child) = Command::new("/usr/bin/pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

pub fn reveal_in_finder(path: &Path) {
    let _ = Command::new("/usr/bin/open").arg("-R").arg(path).spawn();
}

pub fn open_path(path: &Path) {
    let _ = Command::new("/usr/bin/open").arg(path).spawn();
}

/// Writes the picture on the clipboard to a PNG file. Returns false if there is none.
pub fn clipboard_png_to_file(dest: &Path) -> bool {
    let path = dest.display().to_string().replace('"', "\\\"");
    let script = format!(
        "set f to POSIX file \"{path}\"\nset fh to open for access f with write permission\nset eof fh to 0\ntry\nwrite (the clipboard as «class PNGf») to fh\non error\nclose access fh\nerror number -1\nend try\nclose access fh"
    );
    Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
        && dest.metadata().map(|m| m.len() > 0).unwrap_or(false)
}

/// Puts an image (PNG, or converted to TIFF) and/or a file path on the clipboard together.
pub fn set_clipboard(png: Option<&Path>, tiff: bool, path: Option<&Path>) -> bool {
    let esc = |p: &Path| p.display().to_string().replace('\\', "\\\\").replace('"', "\\\"");
    let mut parts: Vec<String> = Vec::new();
    let mut tmp_tiff = None;
    if let Some(png) = png {
        if tiff {
            let t = temp_file("copy", "tiff");
            let ok = Command::new("/usr/bin/sips")
                .args(["-s", "format", "tiff"])
                .arg(png)
                .arg("--out")
                .arg(&t)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                parts.push(format!("«class TIFF»:(read (POSIX file \"{}\") as «class TIFF»)", esc(&t)));
                tmp_tiff = Some(t);
            }
        } else {
            parts.push(format!("«class PNGf»:(read (POSIX file \"{}\") as «class PNGf»)", esc(png)));
        }
    }
    if let Some(p) = path {
        parts.push(format!("«class utf8»:\"{}\"", esc(p)));
    }
    if parts.is_empty() {
        return false;
    }
    let script = format!("set the clipboard to {{{}}}", parts.join(", "));
    let ok = Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if let Some(t) = tmp_tiff {
        let _ = std::fs::remove_file(t);
    }
    ok
}
