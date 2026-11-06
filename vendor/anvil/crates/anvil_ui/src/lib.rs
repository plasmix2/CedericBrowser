//! Anvil UI: the immediate-mode widget layer, layout, motion and the ADL design tokens. Drawing,
//! text, icons and media are ANVL_GPU (`anvl_gpu`), re-exported here.
//!
//! Frame loop: build widgets with [`Ui`] between [`Ui::begin`] and [`Ui::end`], then pass
//! `ui.draw` and `ui.text` to [`Renderer::render`]. The design rules are in `ADL-V3.md`.

// Rendering, text, icons and media live in anvl_gpu. They are re-exported under their old
// paths, so `anvil_ui::draw::RectStyle` and friends keep working.
pub use anvl_gpu::{color, draw, geom, icons, media, path, raster, render, text};

pub mod components;
pub mod crash;
pub mod editor;
pub mod element;
pub mod events;
pub mod flex;
pub mod gesture;
pub mod id;
pub mod input;
pub mod layout;
pub mod logic;
pub mod logo;
pub mod runner;
pub mod markdown;
pub mod platform;
pub mod theme;
pub mod ui;

pub use draw::{DrawList, RectStyle, TextureId, LAYER_COUNT};
pub use geom::Rect;
pub use icons::{Icon, IconAtlas};
pub use editor::{EditorCursor, VisualLines};
pub use element::{canvas, div, icon, img, text, Element, Enter, LaidOut, Paint, Radius as RadiusRef, Tok};
pub use id::ElementId;
pub use input::{Editor, InputKind};
pub use layout::{Column, CompactView, LayoutMode, Row, ShellLayout, Size};
pub use logo::LogoPart;
pub use ui::anvl::{LogoTextures, EditorOutput};
pub use media::{decode_image, Frame, MediaError, MediaKind, Video};
pub use render::{FrameStatus, GpuError, Renderer};
pub use text::{FontRole, RichSpan, TextSystem, WEIGHT_MEDIUM, WEIGHT_REGULAR, WEIGHT_SEMIBOLD};
pub use theme::{hex, lerp, motion, palette, radius, with_alpha, Color, Radii, Theme};
pub use theme::motion::Spring;
pub use ui::{
    Breakpoint, ButtonVariant, Cursor, CursorImage, Fit, Input, Insets, Preedit, TabState, TextStyle, Toast, Toasts, Ui, NavItem, Swipe,
    UiKey,
};
