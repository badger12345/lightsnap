//! Screen capture through macOS's own `screencapture` tool.
//! It runs as a short-lived child process, so the app itself stays small and idle.

use std::path::{Path, PathBuf};
use std::process::Command;

use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardItem, NSPasteboardTypePNG, NSPasteboardTypeString,
    NSPasteboardTypeTIFF,
};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSString, NSURL};

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
/// Accepts PNG or TIFF on the clipboard (TIFF is converted to PNG).
pub fn clipboard_png_to_file(dest: &Path) -> bool {
    objc2::rc::autoreleasepool(|_| read_png(&NSPasteboard::generalPasteboard(), dest))
}

/// Puts an image (PNG, or converted to TIFF) and/or a file path on the clipboard together.
pub fn set_clipboard(png: Option<&Path>, tiff: bool, path: Option<&Path>) -> bool {
    objc2::rc::autoreleasepool(|_| write_items(&NSPasteboard::generalPasteboard(), png, tiff, path))
}

/// Puts a file reference on the clipboard (paste in Finder copies the file).
pub fn copy_file_ref(p: &Path) -> bool {
    objc2::rc::autoreleasepool(|_| write_file_ref(&NSPasteboard::generalPasteboard(), p))
}

// The pasteboard is passed in so tests can use a private one instead of the person's clipboard.
// Paths and data go to the pasteboard as plain values: nothing is turned into script text.

fn read_png(pb: &NSPasteboard, dest: &Path) -> bool {
    let png = match pb.dataForType(unsafe { NSPasteboardTypePNG }) {
        Some(d) => Some(d.to_vec()),
        None => pb
            .dataForType(unsafe { NSPasteboardTypeTIFF })
            .and_then(|d| NSBitmapImageRep::imageRepWithData(&d))
            .and_then(|rep| unsafe {
                rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
            })
            .map(|d| d.to_vec()),
    };
    match png {
        Some(bytes) if !bytes.is_empty() => std::fs::write(dest, bytes).is_ok(),
        _ => false,
    }
}

fn write_items(pb: &NSPasteboard, png: Option<&Path>, tiff: bool, path: Option<&Path>) -> bool {
    let item = NSPasteboardItem::new();
    let mut any = false;
    if let Some(png) = png {
        if let Ok(bytes) = std::fs::read(png) {
            let data = NSData::from_vec(bytes);
            if tiff {
                if let Some(t) = NSBitmapImageRep::imageRepWithData(&data).and_then(|rep| rep.TIFFRepresentation()) {
                    any |= item.setData_forType(&t, unsafe { NSPasteboardTypeTIFF });
                }
            } else {
                any |= item.setData_forType(&data, unsafe { NSPasteboardTypePNG });
            }
        }
    }
    if let Some(p) = path {
        let s = NSString::from_str(&p.display().to_string());
        any |= item.setString_forType(&s, unsafe { NSPasteboardTypeString });
    }
    if !any {
        return false;
    }
    pb.clearContents();
    pb.writeObjects(&NSArray::from_retained_slice(&[ProtocolObject::from_retained(item)]))
}

fn write_file_ref(pb: &NSPasteboard, p: &Path) -> bool {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&p.display().to_string()));
    pb.clearContents();
    pb.writeObjects(&NSArray::from_retained_slice(&[ProtocolObject::from_retained(url)]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_app_kit::NSPasteboardTypeFileURL;

    /// Frees the private pasteboard (not bound by objc2-app-kit, so sent directly).
    fn release(pb: &NSPasteboard) {
        let _: () = unsafe { objc2::msg_send![pb, releaseGlobally] };
    }

    fn tmp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("lightsnap-cliptest-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn small_png(dir: &Path) -> PathBuf {
        let p = dir.join("in.png");
        image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 200, 30, 255])).save(&p).unwrap();
        p
    }

    #[test]
    fn png_and_path_round_trip() {
        let dir = tmp_dir();
        let png = small_png(&dir);
        let pb = NSPasteboard::pasteboardWithUniqueName();
        let path = Path::new("/Users/x/Snap \"quoted\" \\ back.png");
        assert!(write_items(&pb, Some(&png), false, Some(path)));
        let s = pb.stringForType(unsafe { NSPasteboardTypeString }).unwrap();
        assert_eq!(s.to_string(), path.display().to_string());
        let out = dir.join("out.png");
        assert!(read_png(&pb, &out));
        let img = image::open(&out).unwrap().to_rgba8();
        assert_eq!((img.width(), img.height()), (3, 2));
        assert_eq!(img.get_pixel(1, 1).0, [10, 200, 30, 255]);
        release(&pb);
    }

    #[test]
    fn tiff_is_read_back_as_png() {
        let dir = tmp_dir();
        let png = small_png(&dir);
        let pb = NSPasteboard::pasteboardWithUniqueName();
        assert!(write_items(&pb, Some(&png), true, None));
        assert!(pb.dataForType(unsafe { NSPasteboardTypePNG }).is_none());
        assert!(pb.dataForType(unsafe { NSPasteboardTypeTIFF }).is_some());
        let out = dir.join("out-tiff.png");
        assert!(read_png(&pb, &out));
        assert_eq!(image::open(&out).unwrap().width(), 3);
        release(&pb);
    }

    /// A file name built to break out of an AppleScript string must end up on the
    /// clipboard unchanged, and nothing in it may run.
    #[test]
    fn hostile_file_name_is_just_data() {
        let dir = tmp_dir();
        let marker = dir.join("PWNED");
        let _ = std::fs::remove_file(&marker);
        let cmd = format!("touch {}", marker.display());
        let ids: Vec<String> = cmd.chars().map(|c| (c as u32).to_string()).collect();
        let name = format!("a\\\"&(do shell script (character id {{{}}})))--.mov", ids.join(","));
        let p = PathBuf::from("/Users/x/Movies").join(&name);
        let pb = NSPasteboard::pasteboardWithUniqueName();
        assert!(write_file_ref(&pb, &p));
        let url = pb.stringForType(unsafe { NSPasteboardTypeFileURL }).unwrap().to_string();
        let back = NSURL::URLWithString(&NSString::from_str(&url)).and_then(|u| u.path()).unwrap();
        assert_eq!(back.to_string(), p.display().to_string());
        assert!(!marker.exists());
        release(&pb);
    }
}
