//! Video playback through the operating system's own decoders: Media Foundation on Windows,
//! MediaCodec on Android. They use the GPU's hardware decoder where there is one, play every
//! codec the system has, and add nothing to the binary.
//!
//! [`Video`] plays a file from memory without sound, loops at the end, and hands out each
//! picture as an RGBA [`Frame`] for `Renderer::upload` and `Renderer::update`.

#[cfg(target_os = "android")]
mod android;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "android")]
use android::Decoder;
#[cfg(windows)]
use windows::Decoder;

use crate::media::{Frame, MediaError};

/// Platforms without a video backend yet get a decoder that always refuses.
#[cfg(not(any(windows, target_os = "android")))]
struct Decoder {
    width: u32,
    height: u32,
    duration: f64,
    hardware: bool,
}

#[cfg(not(any(windows, target_os = "android")))]
impl Decoder {
    fn open(_: &[u8]) -> Result<Self, String> {
        Err("video playback needs Windows or Android for now".into())
    }
    fn next(&mut self) -> Result<Option<(f64, Frame)>, String> {
        Ok(None)
    }
    fn rewind(&mut self) -> Result<(), String> {
        Ok(())
    }
}

/// A video file, decoded as it plays. It loops when it reaches the end.
pub struct Video {
    decoder: Decoder,
    frame: Frame,
    /// The next picture and its time, decoded ahead of the clock.
    pending: Option<(f64, Frame)>,
    /// Playback position in seconds.
    clock: f64,
    duration: f64,
    frames_shown: u64,
}

impl Video {
    /// Opens a video from memory (MP4, MOV, MKV or WebM, as the system supports) and decodes its
    /// first picture.
    pub fn open(bytes: Vec<u8>) -> Result<Self, MediaError> {
        let mut decoder = Decoder::open(&bytes).map_err(MediaError::Video)?;
        let (_, first) = decoder
            .next()
            .map_err(MediaError::Video)?
            .ok_or_else(|| MediaError::Video("the video has no pictures".into()))?;
        let pending = decoder.next().map_err(MediaError::Video)?;
        let duration = if decoder.duration > 0.0 { decoder.duration } else { 1.0 };
        Ok(Self { decoder, frame: first, pending, clock: 0.0, duration, frames_shown: 1 })
    }

    /// The picture for the current playback position.
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// Length of the video in seconds.
    pub fn duration(&self) -> f64 {
        self.duration
    }

    /// Playback position in seconds.
    pub fn position(&self) -> f64 {
        self.clock
    }

    /// True when the GPU's hardware decoder is in use, false for the system's software decoder.
    pub fn hardware(&self) -> bool {
        self.decoder.hardware
    }

    /// Pictures shown since the video opened, across loops.
    pub fn frames_shown(&self) -> u64 {
        self.frames_shown
    }

    /// Moves playback on by `dt` seconds. Returns true if the picture changed, so the caller
    /// knows to upload it again.
    pub fn advance(&mut self, dt: f32) -> Result<bool, MediaError> {
        self.clock += dt.max(0.0) as f64;
        let mut changed = false;
        loop {
            match self.pending.take() {
                Some((t, frame)) if t <= self.clock => {
                    self.frame = frame;
                    self.frames_shown += 1;
                    changed = true;
                    self.pending = self.decoder.next().map_err(MediaError::Video)?;
                }
                Some(later) => {
                    self.pending = Some(later);
                    return Ok(changed);
                }
                None => {
                    // The end: start over.
                    self.decoder.rewind().map_err(MediaError::Video)?;
                    self.clock = (self.clock - self.duration).max(0.0);
                    self.pending = self.decoder.next().map_err(MediaError::Video)?;
                    if self.pending.is_none() {
                        return Ok(changed);
                    }
                }
            }
        }
    }
}
