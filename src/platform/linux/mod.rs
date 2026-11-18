//! Linux implementation of `crate::platform`, for Wayland desktops (GNOME, KDE).
//!
//! Wayland lets an app neither read the screen nor see the mouse or catch keys outside
//! its own windows. So screenshots, screen recording and global shortcuts go through the
//! xdg-desktop-portal, which asks the person once. The tray icon is a
//! StatusNotifierItem (GNOME shows it with the AppIndicator extension).

mod capture;
mod clipboard;
mod displays;
mod portal;
mod recording;
mod system;
mod tray;

pub use capture::*;
pub use clipboard::*;
pub use displays::*;
pub use recording::*;
pub use system::*;
pub use tray::*;

use super::APP_ID;
