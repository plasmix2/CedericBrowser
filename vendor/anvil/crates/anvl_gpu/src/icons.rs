//! Lucide icons, baked into one atlas (`assets/icons.png`) and built into the binary.
//!
//! Each icon is white; the alpha channel is the mask, and the shader applies the tint color.
//! The SVG sources below (Lucide's 24x24 grid, ISC license, https://lucide.dev) are what the
//! `bake_icons` example renders. At startup the engine only decodes the PNG, with its own
//! decoder, and a test fails if the PNG falls behind the sources.

/// Raster size of each icon slot in pixels. Sized for crisp downscaling to 16 to 48 px.
pub const ICON_PX: u32 = 96;
/// Icons per atlas row.
pub const COLUMNS: u32 = 4;

/// The baked atlas. Re-bake with `cargo run -p anvl_gpu --example bake_icons`.
static BAKED: &[u8] = include_bytes!("../assets/icons.png");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Scan,
    Check,
    Wifi,
    WifiOff,
    User,
    FileText,
    Cloud,
    Clock,
    ArrowLeft,
    ArrowRight,
    ShieldCheck,
    Activity,
    ChevronDown,
    Plus,
    Minus,
    Trash,
    Info,
    X,
    Upload,
    Search,
    Calendar,
    ChevronRight,
    Folder,
    File,
    Bold,
    Italic,
    Link,
    Image,
    PanelLeft,
    Command,
    Bell,
    Play,
    Pause,
    Tag,
    Undo,
    Redo,
    Heading,
    List,
    SquareCheck,
    Code,
    KeyboardOff,
    Copy,
    Clipboard,
    Scissors,
    Settings,
    GripVertical,
}

impl Icon {
    /// Every icon, in atlas order. The enum discriminant is the atlas index.
    pub const ALL: [Icon; 46] = [
        Icon::Scan,
        Icon::Check,
        Icon::Wifi,
        Icon::WifiOff,
        Icon::User,
        Icon::FileText,
        Icon::Cloud,
        Icon::Clock,
        Icon::ArrowLeft,
        Icon::ArrowRight,
        Icon::ShieldCheck,
        Icon::Activity,
        Icon::ChevronDown,
        Icon::Plus,
        Icon::Minus,
        Icon::Trash,
        Icon::Info,
        Icon::X,
        Icon::Upload,
        Icon::Search,
        Icon::Calendar,
        Icon::ChevronRight,
        Icon::Folder,
        Icon::File,
        Icon::Bold,
        Icon::Italic,
        Icon::Link,
        Icon::Image,
        Icon::PanelLeft,
        Icon::Command,
        Icon::Bell,
        Icon::Play,
        Icon::Pause,
        Icon::Tag,
        Icon::Undo,
        Icon::Redo,
        Icon::Heading,
        Icon::List,
        Icon::SquareCheck,
        Icon::Code,
        Icon::KeyboardOff,
        Icon::Copy,
        Icon::Clipboard,
        Icon::Scissors,
        Icon::Settings,
        Icon::GripVertical,
    ];

    /// SVG elements for the 24x24 Lucide grid: the source the atlas is baked from.
    pub fn svg_body(self) -> &'static str {
        match self {
            Icon::Scan => {
                r#"<path d="M3 7V5a2 2 0 0 1 2-2h2"/><path d="M17 3h2a2 2 0 0 1 2 2v2"/><path d="M21 17v2a2 2 0 0 1-2 2h-2"/><path d="M7 21H5a2 2 0 0 1-2-2v-2"/><path d="M7 12h10"/>"#
            }
            Icon::Check => r#"<path d="M20 6 9 17l-5-5"/>"#,
            Icon::Wifi => {
                r#"<path d="M12 20h.01"/><path d="M2 8.82a15 15 0 0 1 20 0"/><path d="M5 12.859a10 10 0 0 1 14 0"/><path d="M8.5 16.429a5 5 0 0 1 7 0"/>"#
            }
            Icon::WifiOff => {
                r#"<path d="M12 20h.01"/><path d="M8.5 16.429a5 5 0 0 1 7 0"/><path d="M5 12.859a10 10 0 0 1 5.17-2.69"/><path d="M19 12.859a10 10 0 0 0-2.007-1.523"/><path d="M2 8.82a15 15 0 0 1 4.177-2.643"/><path d="M22 8.82a15 15 0 0 0-11.288-3.764"/><path d="m2 2 20 20"/>"#
            }
            Icon::User => r#"<circle cx="12" cy="8" r="5"/><path d="M20 21a8 8 0 0 0-16 0"/>"#,
            Icon::FileText => {
                r#"<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M10 9H8"/><path d="M16 13H8"/><path d="M16 17H8"/>"#
            }
            Icon::Cloud => r#"<path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z"/>"#,
            Icon::Clock => r#"<circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/>"#,
            Icon::ArrowLeft => r#"<path d="m12 19-7-7 7-7"/><path d="M19 12H5"/>"#,
            Icon::ArrowRight => r#"<path d="M5 12h14"/><path d="m12 5 7 7-7 7"/>"#,
            Icon::ShieldCheck => {
                r#"<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/><path d="m9 12 2 2 4-4"/>"#
            }
            Icon::Activity => {
                r#"<path d="M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.25.25 0 0 1-.48 0L9.24 2.18a.25.25 0 0 0-.48 0l-2.35 8.36A2 2 0 0 1 4.49 12H2"/>"#
            }
            Icon::ChevronDown => r#"<path d="m6 9 6 6 6-6"/>"#,
            Icon::Plus => r#"<path d="M5 12h14"/><path d="M12 5v14"/>"#,
            Icon::Minus => r#"<path d="M5 12h14"/>"#,
            Icon::Trash => {
                r#"<path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/><path d="M10 11v6"/><path d="M14 11v6"/>"#
            }
            Icon::Info => r#"<circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/>"#,
            Icon::X => r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#,
            Icon::Upload => r#"<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m17 8-5-5-5 5"/><path d="M12 3v12"/>"#,
            Icon::Search => r#"<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>"#,
            Icon::Calendar => r#"<rect width="18" height="18" x="3" y="4" rx="2"/><path d="M16 2v4"/><path d="M8 2v4"/><path d="M3 10h18"/>"#,
            Icon::ChevronRight => r#"<path d="m9 18 6-6-6-6"/>"#,
            Icon::Folder => r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#,
            Icon::File => r#"<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/>"#,
            Icon::Bold => r#"<path d="M6 12h9a4 4 0 0 1 0 8H7a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h7a4 4 0 0 1 0 8"/>"#,
            Icon::Italic => r#"<line x1="19" x2="10" y1="4" y2="4"/><line x1="14" x2="5" y1="20" y2="20"/><line x1="15" x2="9" y1="4" y2="20"/>"#,
            Icon::Link => r#"<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>"#,
            Icon::Image => r#"<rect width="18" height="18" x="3" y="3" rx="2" ry="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>"#,
            Icon::PanelLeft => r#"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18"/>"#,
            Icon::Command => r#"<path d="M15 6v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3"/>"#,
            Icon::Bell => r#"<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>"#,
            Icon::Play => r#"<polygon points="6 3 20 12 6 21 6 3"/>"#,
            Icon::Pause => r#"<rect x="6" y="4" width="4" height="16"/><rect x="14" y="4" width="4" height="16"/>"#,
            Icon::Tag => r#"<path d="M12.586 2.586A2 2 0 0 0 11.172 2H4a2 2 0 0 0-2 2v7.172a2 2 0 0 0 .586 1.414l8.704 8.704a2.426 2.426 0 0 0 3.42 0l6.58-6.58a2.426 2.426 0 0 0 0-3.42z"/><circle cx="7.5" cy="7.5" r=".5"/>"#,
            Icon::Undo => r#"<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11"/>"#,
            Icon::Redo => r#"<path d="m15 14 5-5-5-5"/><path d="M20 9H9.5A5.5 5.5 0 0 0 4 14.5A5.5 5.5 0 0 0 9.5 20H13"/>"#,
            Icon::Heading => r#"<path d="M6 12h12"/><path d="M6 20V4"/><path d="M18 20V4"/>"#,
            Icon::List => r#"<path d="M3 12h.01"/><path d="M3 18h.01"/><path d="M3 6h.01"/><path d="M8 12h13"/><path d="M8 18h13"/><path d="M8 6h13"/>"#,
            Icon::SquareCheck => r#"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="m9 12 2 2 4-4"/>"#,
            Icon::Code => r#"<path d="m16 18 6-6-6-6"/><path d="m8 6-6 6 6 6"/>"#,
            Icon::KeyboardOff => r#"<path d="M20 4a2 2 0 0 1 2 2"/><path d="M22 6v8"/><path d="M14 4h6"/><path d="m2 2 20 20"/><path d="M20 20H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2"/><path d="M6 8h.01"/><path d="M6 16h12"/><path d="M10 8h.01"/><path d="M8 12h.01"/>"#,
            Icon::Copy => r#"<rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>"#,
            Icon::Clipboard => r#"<rect width="8" height="4" x="8" y="2" rx="1" ry="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/>"#,
            Icon::Scissors => r#"<circle cx="6" cy="6" r="3"/><path d="M8.12 8.12 12 12"/><path d="M20 4 8.12 15.88"/><circle cx="6" cy="18" r="3"/><path d="M14.8 14.8 20 20"/>"#,
            Icon::Settings => r#"<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>"#,
            Icon::GripVertical => r#"<circle cx="9" cy="12" r="1"/><circle cx="9" cy="5" r="1"/><circle cx="9" cy="19" r="1"/><circle cx="15" cy="12" r="1"/><circle cx="15" cy="5" r="1"/><circle cx="15" cy="19" r="1"/>"#,
        }
    }
}

/// White-on-transparent icon atlas, RGBA8. The shader reads only the alpha.
pub struct IconAtlas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    uvs: Vec<[f32; 4]>,
}

impl IconAtlas {
    /// Loads the baked atlas. Panics if it does not match the icon list, which means it needs
    /// re-baking (the `icons_baked` test says the same thing earlier).
    pub fn build() -> Self {
        let frame = crate::image::png::decode(BAKED).expect("assets/icons.png decodes");
        let count = Icon::ALL.len() as u32;
        let (width, height) = (COLUMNS * ICON_PX, count.div_ceil(COLUMNS) * ICON_PX);
        assert_eq!(
            (frame.width, frame.height),
            (width, height),
            "assets/icons.png is out of date: run `cargo run -p anvl_gpu --example bake_icons`"
        );
        let uvs = (0..count)
            .map(|i| {
                let (ox, oy) = ((i % COLUMNS) * ICON_PX, (i / COLUMNS) * ICON_PX);
                [
                    ox as f32 / width as f32,
                    oy as f32 / height as f32,
                    (ox + ICON_PX) as f32 / width as f32,
                    (oy + ICON_PX) as f32 / height as f32,
                ]
            })
            .collect();
        Self {
            width,
            height,
            pixels: frame.rgba,
            uvs,
        }
    }

    /// Atlas coordinates `[u0, v0, u1, v1]` for an icon.
    pub fn uv(&self, icon: Icon) -> [f32; 4] {
        self.uvs[icon as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_rasterizes_with_ink() {
        let atlas = IconAtlas::build();
        for icon in Icon::ALL {
            let uv = atlas.uv(icon);
            let x0 = (uv[0] * atlas.width as f32) as u32;
            let y0 = (uv[1] * atlas.height as f32) as u32;
            let mut ink = 0u32;
            for y in y0..y0 + ICON_PX {
                for x in x0..x0 + ICON_PX {
                    let i = ((y * atlas.width + x) * 4 + 3) as usize;
                    ink += (atlas.pixels[i] > 0) as u32;
                }
            }
            assert!(ink > 100, "{icon:?} drew too few pixels: {ink}");
            // An icon is a thin outline. A badly scaled bake fills most of its tile.
            assert!(ink < ICON_PX * ICON_PX * 2 / 5, "{icon:?} fills {ink} pixels: is the bake scaled?");
        }
    }
}
