//! macOS implementation of `crate::platform`.

mod capture;
mod clipboard;
mod displays;
mod recording;
mod system;

pub use capture::*;
pub use clipboard::*;
pub use displays::*;
pub use recording::*;
pub use system::*;
