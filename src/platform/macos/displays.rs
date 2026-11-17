//! The connected screens and the mouse position, read from CoreGraphics. Coordinates are global
//! points with the origin at the top-left of the main screen, like window positions.

use crate::platform::Display;
use egui::{Pos2, Rect, pos2, vec2};
use std::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGDisplayCopyDisplayMode(display: u32) -> *mut c_void;
    fn CGDisplayModeGetPixelWidth(mode: *mut c_void) -> usize;
    fn CGDisplayModeRelease(mode: *mut c_void);
    fn CGEventCreate(source: *mut c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

/// All active screens, the main screen first (the same order `screencapture` and
/// ffmpeg's "Capture screen N" use).
pub fn displays() -> Vec<Display> {
    let mut ids = [0u32; 16];
    let mut count = 0u32;
    let ok = unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) } == 0;
    if !ok || count == 0 {
        return Vec::new();
    }
    let main = unsafe { CGMainDisplayID() };
    let mut ids: Vec<u32> = ids[..count as usize].to_vec();
    ids.sort_by_key(|id| *id != main);
    ids.into_iter()
        .map(|id| {
            let b = unsafe { CGDisplayBounds(id) };
            let bounds = Rect::from_min_size(
                pos2(b.origin.x as f32, b.origin.y as f32),
                vec2(b.size.width as f32, b.size.height as f32),
            );
            let mut scale = 1.0;
            unsafe {
                let mode = CGDisplayCopyDisplayMode(id);
                if !mode.is_null() {
                    let px = CGDisplayModeGetPixelWidth(mode) as f32;
                    if b.size.width > 0.0 && px > 0.0 {
                        scale = px / b.size.width as f32;
                    }
                    CGDisplayModeRelease(mode);
                }
            }
            Display { bounds, scale }
        })
        .collect()
}

/// Where the mouse pointer is, in global points.
pub fn mouse_position() -> Option<Pos2> {
    unsafe {
        let ev = CGEventCreate(std::ptr::null_mut());
        if ev.is_null() {
            return None;
        }
        let p = CGEventGetLocation(ev);
        CFRelease(ev as *const c_void);
        Some(pos2(p.x as f32, p.y as f32))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lists_screens() {
        let d = super::displays();
        println!("{d:?} mouse={:?}", super::mouse_position());
        assert!(!d.is_empty());
        assert!(d[0].bounds.min == egui::pos2(0.0, 0.0));
    }
}
