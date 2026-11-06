//! ANVL_GPU: everything Anvil draws, and the assets it draws with.
//!
//! The UI builds a [`DrawList`] each frame. [`Renderer`] turns it into GPU work: rounded shapes
//! with soft shadows and backdrop blur, icons from the baked atlas, pictures and video frames,
//! and text shaped by [`TextSystem`]. Nothing here knows about widgets or windows: a renderer
//! draws into any surface wgpu accepts.

pub mod color;
pub mod draw;
pub mod geom;
pub mod glyphs;
pub mod icons;
pub mod image;
pub mod media;
pub mod path;
pub mod raster;
pub mod render;
pub mod text;
pub mod video;

pub use color::{hex, lerp, with_alpha, Color};
pub use draw::{DrawList, Gradient, RectStyle, TextureId, LAYER_COUNT, NO_CLIP};
pub use path::Path;
pub use geom::Rect;
pub use icons::{Icon, IconAtlas};
pub use media::{decode_image, Frame, MediaError, MediaKind, Video};
pub use render::{FrameStatus, GpuError, RenderTimings, Renderer};
pub use text::{FontRole, RichSpan, TextSystem, WEIGHT_MEDIUM, WEIGHT_REGULAR, WEIGHT_SEMIBOLD};
