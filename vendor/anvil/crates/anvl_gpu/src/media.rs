//! Media: still images (PNG, JPEG, GIF, BMP, decoded by `crate::image`) and video (decoded by
//! the operating system, `crate::video`).
//!
//! Both produce straight-alpha RGBA8 [`Frame`]s. The renderer uploads them as textures
//! (`Renderer::upload`, and `Renderer::update` for each new video picture), and the UI draws
//! them with `Ui::image`. Video plays without sound.

use std::fmt;
use std::path::Path;

/// One decoded picture: RGBA8, rows from top to bottom, straight (not premultiplied) alpha.
#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Frame {
    /// A smooth test pattern, for demos and for media that has not loaded yet.
    pub fn gradient(width: u32, height: u32) -> Self {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let u = x as f32 / width.max(2).saturating_sub(1) as f32;
                let v = y as f32 / height.max(2).saturating_sub(1) as f32;
                rgba.extend_from_slice(&[
                    (u * 255.0) as u8,
                    (v * 255.0) as u8,
                    ((1.0 - u) * 255.0) as u8,
                    255,
                ]);
            }
        }
        Self {
            width,
            height,
            rgba,
        }
    }
}

#[derive(Debug)]
pub enum MediaError {
    /// The file is not a supported image, or the image is damaged.
    Image(crate::image::ImageError),
    /// The system could not open or decode the video.
    Video(String),
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaError::Image(err) => write!(f, "could not read the image: {err}"),
            MediaError::Video(message) => write!(f, "could not play the video: {message}"),
        }
    }
}

impl std::error::Error for MediaError {}

/// What kind of media a file holds, judged by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
}

impl MediaKind {
    /// Classifies a file by extension. Returns `None` for files the engine cannot show.
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "bmp" => Some(MediaKind::Image),
            "mp4" | "m4v" | "mov" | "mkv" | "webm" => Some(MediaKind::Video),
            _ => None,
        }
    }
}

/// Decodes a PNG, JPEG, GIF or BMP with Anvil's own decoders (`crate::image`). Animated GIFs
/// show their first frame only.
pub fn decode_image(bytes: &[u8]) -> Result<Frame, MediaError> {
    crate::image::decode(bytes).map_err(MediaError::Image)
}

pub use crate::video::Video;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_kind_follows_the_extension() {
        assert_eq!(MediaKind::from_path(Path::new("a/Photo.PNG")), Some(MediaKind::Image));
        assert_eq!(MediaKind::from_path(Path::new("clip.mp4")), Some(MediaKind::Video));
        assert_eq!(MediaKind::from_path(Path::new("notes.txt")), None);
        assert_eq!(MediaKind::from_path(Path::new("no_extension")), None);
    }

    #[test]
    fn damaged_image_bytes_are_an_error() {
        assert!(matches!(
            decode_image(b"not an image"),
            Err(MediaError::Image(_))
        ));
    }

    /// Plays a real video file for two seconds through the system decoder. Point
    /// `ANVIL_TEST_VIDEO` at a file and run `cargo test -p anvl_gpu -- --ignored`.
    #[test]
    #[ignore]
    fn plays_frames_from_a_real_video() {
        let path = std::env::var("ANVIL_TEST_VIDEO").expect("set ANVIL_TEST_VIDEO to a video file");
        let bytes = std::fs::read(&path).expect("read the video file");
        let mut video = Video::open(bytes).expect("open the video");
        let (w, h) = (video.frame().width, video.frame().height);
        assert!(w > 0 && h > 0);
        let mut changes = 0;
        for _ in 0..60 {
            if video.advance(1.0 / 30.0).expect("advance") {
                changes += 1;
            }
        }
        println!("{path}: {w}x{h}, {:.2}s, hardware {}, {changes} new pictures in 2 s", video.duration(), video.hardware());
        assert!(changes > 10, "too few pictures: {changes}");
    }

}
