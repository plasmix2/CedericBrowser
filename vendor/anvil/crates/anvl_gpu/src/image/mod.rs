//! Anvil's own image decoders: PNG, JPEG, GIF (first frame) and BMP, to straight-alpha RGBA8
//! [`Frame`]s. No outside crates: [`inflate`] does PNG's decompression, and each format is a
//! small module here.

pub mod bmp;
pub mod gif;
pub mod inflate;
pub mod jpeg;
pub mod png;

use std::fmt;

use crate::media::Frame;

/// Why an image did not decode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// The bytes are not an image Anvil reads, or use a feature it does not support.
    Format(&'static str),
    /// The file ends early.
    Truncated,
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageError::Format(why) => write!(f, "cannot decode the image: {why}"),
            ImageError::Truncated => write!(f, "the image file ends early"),
        }
    }
}

impl std::error::Error for ImageError {}

/// Decodes PNG, JPEG, GIF or BMP bytes, chosen by their signature, not the file name.
pub fn decode(bytes: &[u8]) -> Result<Frame, ImageError> {
    if png::is_png(bytes) {
        png::decode(bytes)
    } else if jpeg::is_jpeg(bytes) {
        jpeg::decode(bytes)
    } else if gif::is_gif(bytes) {
        gif::decode(bytes)
    } else if bmp::is_bmp(bytes) {
        bmp::decode(bytes)
    } else {
        Err(ImageError::Format("unknown image format (PNG, JPEG, GIF and BMP are supported)"))
    }
}
