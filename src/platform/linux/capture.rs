//! Screenshots through the Screenshot portal. Area and window use the desktop's own
//! picker (interactive); full screen and timed captures take the picture right away.

use crate::platform::{CaptureKind, temp_file};
use ashpd::desktop::screenshot::Screenshot;
use std::path::{Path, PathBuf};

/// Runs the capture and blocks until it is done. Returns the image file,
/// or None if the person cancelled or capture failed.
pub fn run_capture(kind: CaptureKind, _sound: bool) -> Option<PathBuf> {
    let interactive = match kind {
        CaptureKind::Area | CaptureKind::Window => true,
        CaptureKind::FullScreen => false,
        CaptureKind::Timed(s) => {
            std::thread::sleep(std::time::Duration::from_secs(s as u64));
            false
        }
    };
    let src = portal_screenshot(interactive)?;
    take_file(&src, "shot")
}

/// Takes one picture of every screen, in the order of `displays()`.
pub fn capture_all_screens(_count: usize) -> Vec<PathBuf> {
    let Some(src) = portal_screenshot(false) else { return Vec::new() };
    let Some(file) = take_file(&src, "desktop") else { return Vec::new() };
    let displays = super::displays();
    if displays.len() <= 1 {
        return vec![file];
    }
    // The portal gives one picture of the whole desktop: cut it into screens.
    let Ok(img) = image::open(&file) else { return Vec::new() };
    let _ = std::fs::remove_file(&file);
    let all = displays.iter().fold(egui::Rect::NOTHING, |a, d| a.union(d.bounds));
    let f = img.width() as f32 / all.width().max(1.0);
    displays
        .iter()
        .enumerate()
        .filter_map(|(i, d)| {
            let r = d.bounds.translate(-all.min.to_vec2());
            let (x, y) = ((r.min.x * f).round() as u32, (r.min.y * f).round() as u32);
            let (w, h) = ((r.width() * f).round() as u32, (r.height() * f).round() as u32);
            let part = img.crop_imm(x, y, w.min(img.width().saturating_sub(x)), h.min(img.height().saturating_sub(y)));
            let out = temp_file(&format!("screen{i}"), "png");
            part.save(&out).ok().map(|_| out)
        })
        .collect()
}

fn portal_screenshot(interactive: bool) -> Option<PathBuf> {
    super::portal::register();
    let res = pollster::block_on(async {
        Screenshot::request().interactive(interactive).modal(false).send().await?.response()
    });
    match res {
        Ok(r) => file_uri_to_path(r.uri().as_str()),
        Err(e) => {
            // Cancelled is normal; anything else goes to the log.
            if !matches!(e, ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) {
                log::warn!(target: "lightsnap", "screenshot portal: {e}");
            }
            None
        }
    }
}

/// Moves the portal's file into LightSnap's temp folder. The desktop may have saved it
/// in the person's Pictures folder; that copy was made for us and goes away.
fn take_file(src: &Path, prefix: &str) -> Option<PathBuf> {
    let out = temp_file(prefix, "png");
    if std::fs::rename(src, &out).is_err() {
        std::fs::copy(src, &out).ok()?;
        let _ = std::fs::remove_file(src);
    }
    Some(out)
}

/// "file:///home/a/Bild%201.png" → "/home/a/Bild 1.png".
fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let mut out = Vec::with_capacity(rest.len());
    let b = rest.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).ok()?, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_file_uris() {
        assert_eq!(super::file_uri_to_path("file:///home/a/Bild%201.png").unwrap(), std::path::Path::new("/home/a/Bild 1.png"));
        assert_eq!(super::file_uri_to_path("file:///tmp/x%").unwrap(), std::path::Path::new("/tmp/x%"));
        assert!(super::file_uri_to_path("https://x").is_none());
    }
}

/// Whether captures that don't ask first (full screen, timed, record) may run: the
/// desktop remembers in its permission store that the person allowed it.
pub fn screenshot_allowed() -> bool {
    let r: zbus::Result<bool> = pollster::block_on(async {
        let c = zbus::Connection::session().await?;
        let reply = c
            .call_method(
                Some("org.freedesktop.impl.portal.PermissionStore"),
                "/org/freedesktop/impl/portal/PermissionStore",
                Some("org.freedesktop.impl.portal.PermissionStore"),
                "Lookup",
                &("screenshot", "screenshot"),
            )
            .await?;
        let (perms, _data): (std::collections::HashMap<String, Vec<String>>, zbus::zvariant::OwnedValue) =
            reply.body().deserialize()?;
        Ok(perms.get(super::APP_ID).is_some_and(|v| v.iter().any(|p| p == "yes")))
    });
    r.unwrap_or(false)
}

/// Asks the desktop for that permission by taking one picture and throwing it away.
/// GNOME only asks while one of LightSnap's windows is in front.
pub fn ask_screenshot_permission() -> bool {
    match portal_screenshot(false) {
        Some(p) => {
            let _ = std::fs::remove_file(p);
            true
        }
        None => false,
    }
}
