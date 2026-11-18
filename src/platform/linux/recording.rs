//! The system-specific parts of screen recording. Not done yet on Linux.

use crate::platform::Devices;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// Places to look for ffmpeg, in order: the system's.
pub fn ffmpeg_candidates() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = vec!["/usr/bin/ffmpeg".into()];
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join("ffmpeg")));
    }
    candidates
}

pub fn list_devices(_ffmpeg: &Path) -> Devices {
    Devices::default()
}

pub fn video_encoder_args(quality: u32) -> Vec<String> {
    // quality 1..100 → x264 CRF 40..18.
    let crf = 40 - (quality.clamp(1, 100) * 22 / 100);
    ["-c:v", "libx264", "-preset", "veryfast", "-crf", &crf.to_string(), "-pix_fmt", "yuv420p"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

pub fn audio_encoder_args() -> Vec<String> {
    ["-c:a", "aac", "-b:a", "128k"].iter().map(|s| s.to_string()).collect()
}

/// Output options that make ffmpeg play sound on the default speakers (PulseAudio or
/// PipeWire's Pulse server).
pub fn sound_output_args() -> Vec<String> {
    ["-af", "aresample=48000,apad=pad_dur=0.3", "-f", "pulse", "-name", "LightSnap", "LightSnap"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

pub fn ffmpeg_input_args(_screen_index: usize, _audio: Option<&str>, _fps: u32, _cursor: bool, _clicks: bool) -> Vec<String> {
    Vec::new()
}

pub fn fallback_record_command(_rect_global: [f32; 4], _cursor: bool, _clicks: bool, _audio: bool, _out: &Path) -> Command {
    Command::new("false")
}

pub const FALLBACK_EXT: &str = "mp4";

pub fn stop_fallback(child: &mut Child) {
    let _ = child.kill();
}
