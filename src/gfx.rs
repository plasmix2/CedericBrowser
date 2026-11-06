//! Shared types for the native UI: colors, the persona and brand themes, the icon names, and the
//! rounded-rectangle path the popups clip with. Drawing itself is done by the ANVL engine (`anvil.rs`).

use tiny_skia::{Path, PathBuilder};

/// An sRGB color with straight (not premultiplied) alpha.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const fn hex(v: u32) -> Self {
        Self((v >> 16) as u8, (v >> 8) as u8, v as u8, 255)
    }
    pub fn with_alpha(self, a: f32) -> Self {
        Self(self.0, self.1, self.2, (self.3 as f32 * a.clamp(0.0, 1.0)).round() as u8)
    }
    /// Linear blend from `self` to `o` by `t`.
    pub fn mix(self, o: Self, t: f32) -> Self {
        let l = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t.clamp(0.0, 1.0)).round() as u8;
        Self(l(self.0, o.0), l(self.1, o.1), l(self.2, o.2), l(self.3, o.3))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaColor {
    #[default]
    Orange,
    Slate,
    Emerald,
    Ruby,
    Violet,
    Rose,
    Cyan,
    Amber,
    /// Any color the user picked, as 0xRRGGBB. The chrome's palette is derived from it.
    Custom(u32),
}

impl PersonaColor {
    pub const ALL: [PersonaColor; 8] = [
        PersonaColor::Orange,
        PersonaColor::Slate,
        PersonaColor::Emerald,
        PersonaColor::Ruby,
        PersonaColor::Violet,
        PersonaColor::Rose,
        PersonaColor::Cyan,
        PersonaColor::Amber,
    ];

    pub fn from_str_name(s: &str) -> Self {
        // "#rrggbb" is a custom color.
        if let Some(hex) = s.trim().strip_prefix('#')
            && hex.len() == 6
            && let Ok(v) = u32::from_str_radix(hex, 16)
        {
            return Self::Custom(v);
        }
        match s.trim().to_ascii_lowercase().as_str() {
            "orange" | "cederic" => Self::Orange,
            "slate" | "uzaki" | "blue" | "slate_blue" | "indigo" => Self::Slate,
            "emerald" | "green" => Self::Emerald,
            "ruby" | "red" => Self::Ruby,
            "violet" | "purple" => Self::Violet,
            "rose" | "pink" => Self::Rose,
            "cyan" | "teal" | "sky" => Self::Cyan,
            "amber" | "gold" | "yellow" => Self::Amber,
            _ => Self::Orange,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Orange => "orange",
            Self::Slate => "slate",
            Self::Emerald => "emerald",
            Self::Ruby => "ruby",
            Self::Violet => "violet",
            Self::Rose => "rose",
            Self::Cyan => "cyan",
            Self::Amber => "amber",
            Self::Custom(_) => "custom",
        }
    }

    /// What is stored in a persona's `color`: the preset's name, or `#rrggbb` for a custom color.
    pub fn to_key(self) -> String {
        match self {
            Self::Custom(v) => format!("#{:06x}", v & 0xffffff),
            c => c.name().to_string(),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Orange => "Orange (Cederic)",
            Self::Slate => "Slate Blue (Uzaki)",
            Self::Emerald => "Emerald Green",
            Self::Ruby => "Ruby Red",
            Self::Violet => "Violet Purple",
            Self::Rose => "Rose Pink",
            Self::Cyan => "Cyan Blue",
            Self::Amber => "Amber Gold",
            Self::Custom(_) => "Custom",
        }
    }

    /// The accent as `#rrggbb`.
    pub fn hex_string(self) -> String {
        match self {
            Self::Custom(v) => format!("#{:06x}", v & 0xffffff),
            c => c.preset_hex().to_string(),
        }
    }

    const fn preset_hex(self) -> &'static str {
        match self {
            Self::Orange => "#ff6a3d",
            Self::Slate => "#7366e6",
            Self::Emerald => "#10b981",
            Self::Ruby => "#ef4444",
            Self::Violet => "#8b5cf6",
            Self::Rose => "#f43f5e",
            Self::Cyan => "#06b6d4",
            Self::Amber => "#f59e0b",
            Self::Custom(_) => "#808080",
        }
    }

    pub fn accents(self, dark: bool) -> (Rgba, Rgba) {
        if let Self::Custom(v) = self {
            let (h, s, l) = to_hsl(Rgba::hex(v));
            let l = if dark { l.clamp(0.5, 0.64) } else { l.clamp(0.36, 0.5) };
            let s = s.max(0.35);
            return (hsl(h, s, l), hsl((h + 0.06).fract(), s, (l + 0.1).min(0.8)));
        }
        match (self, dark) {
            (Self::Orange, _) => (Rgba::hex(0xff6a3d), Rgba::hex(0xffb03d)),
            (Self::Slate, true) => (Rgba::hex(0x7366e6), Rgba::hex(0x94a3f8)),
            (Self::Slate, false) => (Rgba::hex(0x6a5acd), Rgba::hex(0x94a3f8)),
            (Self::Emerald, true) => (Rgba::hex(0x10b981), Rgba::hex(0x34d399)),
            (Self::Emerald, false) => (Rgba::hex(0x059669), Rgba::hex(0x34d399)),
            (Self::Ruby, true) => (Rgba::hex(0xef4444), Rgba::hex(0xf87171)),
            (Self::Ruby, false) => (Rgba::hex(0xdc2626), Rgba::hex(0xf87171)),
            (Self::Violet, true) => (Rgba::hex(0x8b5cf6), Rgba::hex(0xa78bfa)),
            (Self::Violet, false) => (Rgba::hex(0x7c3aed), Rgba::hex(0xa78bfa)),
            (Self::Rose, true) => (Rgba::hex(0xf43f5e), Rgba::hex(0xfb7185)),
            (Self::Rose, false) => (Rgba::hex(0xe11d48), Rgba::hex(0xfb7185)),
            (Self::Cyan, true) => (Rgba::hex(0x06b6d4), Rgba::hex(0x38bdf8)),
            (Self::Cyan, false) => (Rgba::hex(0x0891b2), Rgba::hex(0x38bdf8)),
            (Self::Amber, true) => (Rgba::hex(0xf59e0b), Rgba::hex(0xfbbf24)),
            (Self::Amber, false) => (Rgba::hex(0xd97706), Rgba::hex(0xfbbf24)),
            (Self::Custom(_), _) => (Rgba::hex(0xff6a3d), Rgba::hex(0xffb03d)),
        }
    }
    pub fn surface_tints(self, dark: bool) -> (Rgba, Rgba, Rgba) {
        // (strip, toolbar, border)
        if let Self::Custom(v) = self {
            let (h, s, _) = to_hsl(Rgba::hex(v));
            let t = (s * 0.3).clamp(0.12, 0.28);
            return if dark {
                (hsl(h, t, 0.085), hsl(h, t + 0.02, 0.135), hsl(h, t, 0.22))
            } else {
                (hsl(h, t + 0.02, 0.88), hsl(h, t + 0.04, 0.965), hsl(h, t, 0.82))
            };
        }
        match (self, dark) {
            (Self::Orange, true) => (Rgba::hex(0x18191d), Rgba::hex(0x26272c), Rgba::hex(0x34363d)),
            (Self::Orange, false) => (Rgba::hex(0xdfe1e6), Rgba::hex(0xf7f7f9), Rgba::hex(0xd2d4da)),
            (Self::Slate, true) => (Rgba::hex(0x151926), Rgba::hex(0x1f2537), Rgba::hex(0x2e3750)),
            (Self::Slate, false) => (Rgba::hex(0xd8deed), Rgba::hex(0xeef1f8), Rgba::hex(0xc5cee3)),
            (Self::Emerald, true) => (Rgba::hex(0x141d19), Rgba::hex(0x1c2923), Rgba::hex(0x2a3e35)),
            (Self::Emerald, false) => (Rgba::hex(0xd9ebe2), Rgba::hex(0xf0f7f4), Rgba::hex(0xc4ded2)),
            (Self::Ruby, true) => (Rgba::hex(0x1f1616), Rgba::hex(0x2c1f1f), Rgba::hex(0x422d2d)),
            (Self::Ruby, false) => (Rgba::hex(0xedd9d9), Rgba::hex(0xf8f0f0), Rgba::hex(0xe3c5c5)),
            (Self::Violet, true) => (Rgba::hex(0x1a1624), Rgba::hex(0x241e33), Rgba::hex(0x382f4d)),
            (Self::Violet, false) => (Rgba::hex(0xe1d9f0), Rgba::hex(0xf3eff9), Rgba::hex(0xd0c4e8)),
            (Self::Rose, true) => (Rgba::hex(0x20151a), Rgba::hex(0x2d1d24), Rgba::hex(0x442b36)),
            (Self::Rose, false) => (Rgba::hex(0xeed8e1), Rgba::hex(0xf9eff3), Rgba::hex(0xe4c3d2)),
            (Self::Cyan, true) => (Rgba::hex(0x131d22), Rgba::hex(0x1a2930), Rgba::hex(0x273e49)),
            (Self::Cyan, false) => (Rgba::hex(0xd7e8ee), Rgba::hex(0xeff6f8), Rgba::hex(0xc1dbe4)),
            (Self::Amber, true) => (Rgba::hex(0x1d1a14), Rgba::hex(0x29241b), Rgba::hex(0x3e3729)),
            (Self::Amber, false) => (Rgba::hex(0xece5d7), Rgba::hex(0xf8f5ef), Rgba::hex(0xdfd4be)),
            (Self::Custom(_), _) => (Rgba::hex(0x18191d), Rgba::hex(0x26272c), Rgba::hex(0x34363d)),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Brand {
    #[default]
    Cederic,
    Uzaki,
}

impl Brand {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Cederic => "Cederic",
            Self::Uzaki => "Uzaki",
        }
    }
    pub const fn badge_letter(self) -> &'static str {
        match self {
            Self::Cederic => "C",
            Self::Uzaki => "U",
        }
    }
    pub const fn search_host(self) -> &'static str {
        match self {
            Self::Cederic => "search.cederic.com",
            Self::Uzaki => "search.uzaki.com",
        }
    }
}

impl AsRef<str> for Brand {
    fn as_ref(&self) -> &str {
        self.name()
    }
}

impl From<Brand> for PersonaColor {
    fn from(b: Brand) -> Self {
        match b {
            Brand::Cederic => PersonaColor::Orange,
            Brand::Uzaki => PersonaColor::Slate,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub strip: Rgba,
    pub toolbar: Rgba,
    pub tab_hover: Rgba,
    pub fg: Rgba,
    pub dim: Rgba,
    pub faint: Rgba,
    pub field: Rgba,
    pub field_hover: Rgba,
    pub border: Rgba,
    pub card: Rgba,
    pub accent: Rgba,
    pub accent2: Rgba,
    pub btn_hover: Rgba,
    pub btn_press: Rgba,
    pub close: Rgba,
    pub warn: Rgba,
    pub selection: Rgba,
}

impl Theme {
    #[cfg(test)]
    pub fn new(dark: bool) -> Self {
        Self::for_persona(dark, PersonaColor::Orange)
    }

    pub fn for_brand(dark: bool, brand: Brand) -> Self {
        Self::for_persona(dark, PersonaColor::from(brand))
    }

    pub fn for_persona(dark: bool, color: PersonaColor) -> Self {
        let (accent, accent2) = color.accents(dark);
        if let PersonaColor::Custom(_) = color {
            // Start from the neutral palette and tint the surfaces with the chosen color.
            let mut t = Self::for_persona(dark, PersonaColor::Orange);
            let (strip, toolbar, border) = color.surface_tints(dark);
            t.strip = strip;
            t.toolbar = toolbar;
            t.border = border;
            t.tab_hover = strip.mix(toolbar, 0.5);
            t.field = if dark { strip } else { strip.mix(toolbar, 0.55) };
            t.field_hover = t.field.mix(toolbar, 0.3);
            t.card = if dark { toolbar.mix(Rgba(255, 255, 255, 255), 0.03) } else { Rgba(255, 255, 255, 255) };
            t.accent = accent;
            t.accent2 = accent2;
            t.selection = accent.with_alpha(0.4);
            return t;
        }
        match (color, dark) {
            (PersonaColor::Orange, true) => Self {
                strip: Rgba::hex(0x18191d),
                toolbar: Rgba::hex(0x26272c),
                tab_hover: Rgba::hex(0x222328),
                fg: Rgba::hex(0xececf0),
                dim: Rgba::hex(0x9a9ca6),
                faint: Rgba::hex(0x5d606a),
                field: Rgba::hex(0x18191d),
                field_hover: Rgba::hex(0x1d1e22),
                border: Rgba::hex(0x34363d),
                card: Rgba::hex(0x2b2c32),
                accent,
                accent2,
                btn_hover: Rgba(255, 255, 255, 20),
                btn_press: Rgba(255, 255, 255, 33),
                close: Rgba::hex(0xe81123),
                warn: Rgba::hex(0xf0a03c),
                selection: Rgba(255, 106, 61, 102),
            },
            (PersonaColor::Orange, false) => Self {
                strip: Rgba::hex(0xdfe1e6),
                toolbar: Rgba::hex(0xf7f7f9),
                tab_hover: Rgba::hex(0xe9eaee),
                fg: Rgba::hex(0x1d1e22),
                dim: Rgba::hex(0x5c5f68),
                faint: Rgba::hex(0x9fa2ab),
                field: Rgba::hex(0xe9eaee),
                field_hover: Rgba::hex(0xe2e3e8),
                border: Rgba::hex(0xd2d4da),
                card: Rgba(255, 255, 255, 255),
                accent,
                accent2,
                btn_hover: Rgba(0, 0, 0, 15),
                btn_press: Rgba(0, 0, 0, 26),
                close: Rgba::hex(0xe81123),
                warn: Rgba::hex(0xd9822b),
                selection: Rgba(255, 106, 61, 90),
            },
            (PersonaColor::Slate, true) => Self {
                strip: Rgba::hex(0x151926),
                toolbar: Rgba::hex(0x1f2537),
                tab_hover: Rgba::hex(0x1a1f2e),
                fg: Rgba::hex(0xeceff7),
                dim: Rgba::hex(0x949eb8),
                faint: Rgba::hex(0x5b647d),
                field: Rgba::hex(0x151926),
                field_hover: Rgba::hex(0x1a1f2e),
                border: Rgba::hex(0x2e3750),
                card: Rgba::hex(0x23293d),
                accent,
                accent2,
                btn_hover: Rgba(255, 255, 255, 20),
                btn_press: Rgba(255, 255, 255, 33),
                close: Rgba::hex(0xe81123),
                warn: Rgba::hex(0xf0a03c),
                selection: Rgba(115, 102, 230, 102),
            },
            (PersonaColor::Slate, false) => Self {
                strip: Rgba::hex(0xd8deed),
                toolbar: Rgba::hex(0xeef1f8),
                tab_hover: Rgba::hex(0xe2e7f3),
                fg: Rgba::hex(0x181d2c),
                dim: Rgba::hex(0x525c75),
                faint: Rgba::hex(0x8c96b0),
                field: Rgba::hex(0xe2e7f3),
                field_hover: Rgba::hex(0xd8deed),
                border: Rgba::hex(0xc5cee3),
                card: Rgba::hex(0xf7f9fc),
                accent,
                accent2,
                btn_hover: Rgba(0, 0, 0, 15),
                btn_press: Rgba(0, 0, 0, 26),
                close: Rgba::hex(0xe81123),
                warn: Rgba::hex(0xd9822b),
                selection: Rgba(106, 90, 205, 90),
            },
            (_, true) => {
                let (strip, toolbar, border) = match color {
                    PersonaColor::Emerald => (Rgba::hex(0x141d19), Rgba::hex(0x1c2923), Rgba::hex(0x2a3e35)),
                    PersonaColor::Ruby => (Rgba::hex(0x1f1616), Rgba::hex(0x2c1f1f), Rgba::hex(0x422d2d)),
                    PersonaColor::Violet => (Rgba::hex(0x1a1624), Rgba::hex(0x241e33), Rgba::hex(0x382f4d)),
                    PersonaColor::Rose => (Rgba::hex(0x20151a), Rgba::hex(0x2d1d24), Rgba::hex(0x442b36)),
                    PersonaColor::Cyan => (Rgba::hex(0x131d22), Rgba::hex(0x1a2930), Rgba::hex(0x273e49)),
                    PersonaColor::Amber => (Rgba::hex(0x1d1a14), Rgba::hex(0x29241b), Rgba::hex(0x3e3729)),
                    _ => (Rgba::hex(0x18191d), Rgba::hex(0x26272c), Rgba::hex(0x34363d)),
                };
                Self {
                    strip,
                    toolbar,
                    tab_hover: Rgba(toolbar.0.saturating_sub(4), toolbar.1.saturating_sub(4), toolbar.2.saturating_sub(4), 255),
                    fg: Rgba::hex(0xececf0),
                    dim: Rgba::hex(0x9a9ca6),
                    faint: Rgba::hex(0x5d606a),
                    field: strip,
                    field_hover: Rgba(strip.0.saturating_add(6), strip.1.saturating_add(6), strip.2.saturating_add(6), 255),
                    border,
                    card: toolbar,
                    accent,
                    accent2,
                    btn_hover: Rgba(255, 255, 255, 20),
                    btn_press: Rgba(255, 255, 255, 33),
                    close: Rgba::hex(0xe81123),
                    warn: Rgba::hex(0xf0a03c),
                    selection: Rgba(accent.0, accent.1, accent.2, 102),
                }
            }
            (_, false) => {
                let (strip, toolbar, border) = match color {
                    PersonaColor::Emerald => (Rgba::hex(0xd9ebe2), Rgba::hex(0xf0f7f4), Rgba::hex(0xc4ded2)),
                    PersonaColor::Ruby => (Rgba::hex(0xedd9d9), Rgba::hex(0xf8f0f0), Rgba::hex(0xe3c5c5)),
                    PersonaColor::Violet => (Rgba::hex(0xe1d9f0), Rgba::hex(0xf3eff9), Rgba::hex(0xd0c4e8)),
                    PersonaColor::Rose => (Rgba::hex(0xeed8e1), Rgba::hex(0xf9eff3), Rgba::hex(0xe4c3d2)),
                    PersonaColor::Cyan => (Rgba::hex(0xd7e8ee), Rgba::hex(0xeff6f8), Rgba::hex(0xc1dbe4)),
                    PersonaColor::Amber => (Rgba::hex(0xece5d7), Rgba::hex(0xf8f5ef), Rgba::hex(0xdfd4be)),
                    _ => (Rgba::hex(0xdfe1e6), Rgba::hex(0xf7f7f9), Rgba::hex(0xd2d4da)),
                };
                Self {
                    strip,
                    toolbar,
                    tab_hover: Rgba(strip.0.saturating_sub(10), strip.1.saturating_sub(10), strip.2.saturating_sub(10), 255),
                    fg: Rgba::hex(0x1d1e22),
                    dim: Rgba::hex(0x5c5f68),
                    faint: Rgba::hex(0x9fa2ab),
                    field: Rgba::hex(0xffffff),
                    field_hover: Rgba(strip.0.saturating_sub(5), strip.1.saturating_sub(5), strip.2.saturating_sub(5), 255),
                    border,
                    card: Rgba(255, 255, 255, 255),
                    accent,
                    accent2,
                    btn_hover: Rgba(0, 0, 0, 15),
                    btn_press: Rgba(0, 0, 0, 26),
                    close: Rgba::hex(0xe81123),
                    warn: Rgba::hex(0xd9822b),
                    selection: Rgba(accent.0, accent.1, accent.2, 90),
                }
            }
        }
    }
}

/// The vector icons, drawn in a 24-unit grid (window controls use a 10-unit grid).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Back,
    Forward,
    Reload,
    Stop,
    Close,
    Plus,
    Lock,
    Info,
    Search,
    Globe,
    Star,
    StarFilled,
    Bookmarks,
    Pip,
    History,
    Code,
    WinMin,
    WinMax,
    WinRestore,
    WinClose,
    ExternalLink,
    Copy,
    Link,
    Photo,
    Printer,
    Maximize,
    UserCircle,
    Users,
    Settings,
    ChevronRight,
    Check,
    Logout,
    ShieldLock,
    Trash,
    Sparkles,
    Menu,
}

/// HSL (all 0..1) to a color.
pub fn hsl(h: f32, s: f32, l: f32) -> Rgba {
    let (h, s, l) = (h.rem_euclid(1.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h * 6.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h * 6.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let q = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgba(q(r), q(g), q(b), 255)
}

/// A color's (hue, saturation, lightness), all 0..1.
pub fn to_hsl(c: Rgba) -> (f32, f32, f32) {
    let (r, g, b) = (c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    let d = mx - mn;
    if d < 1e-6 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if mx == r { ((g - b) / d).rem_euclid(6.0) } else if mx == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 } / 6.0;
    (h, s, l)
}

pub fn rrect_path(x: f32, y: f32, w: f32, h: f32, r: [f32; 4]) -> Option<Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let lim = (w.min(h)) / 2.0;
    let [tl, tr, br, bl] = r.map(|v| v.min(lim));
    let k = 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(x + tl, y);
    pb.line_to(x + w - tr, y);
    pb.cubic_to(x + w - tr + tr * k, y, x + w, y + tr - tr * k, x + w, y + tr);
    pb.line_to(x + w, y + h - br);
    pb.cubic_to(x + w, y + h - br + br * k, x + w - br + br * k, y + h, x + w - br, y + h);
    pb.line_to(x + bl, y + h);
    pb.cubic_to(x + bl - bl * k, y + h, x, y + h - bl + bl * k, x, y + h - bl);
    pb.line_to(x, y + tl);
    pb.cubic_to(x, y + tl - tl * k, x + tl - tl * k, y, x + tl, y);
    pb.close();
    pb.finish()
}

#[cfg(test)]
mod custom_color_tests {
    use super::*;

    #[test]
    fn hsl_round_trips() {
        for c in [Rgba(255, 106, 61, 255), Rgba(16, 185, 129, 255), Rgba(139, 92, 246, 255), Rgba(30, 30, 30, 255)] {
            let (h, s, l) = to_hsl(c);
            let back = hsl(h, s, l);
            for (a, b) in [(c.0, back.0), (c.1, back.1), (c.2, back.2)] {
                assert!((a as i32 - b as i32).abs() <= 1, "{c:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn custom_colors_are_stored_as_hex_and_make_a_theme() {
        let c = PersonaColor::from_str_name("#12ab9f");
        assert_eq!(c, PersonaColor::Custom(0x12ab9f));
        assert_eq!(c.to_key(), "#12ab9f");
        assert_eq!(PersonaColor::from_str_name("ruby").to_key(), "ruby");
        assert_eq!(PersonaColor::from_str_name("#zzz"), PersonaColor::Orange);
        for dark in [true, false] {
            let t = Theme::for_persona(dark, c);
            // Surfaces are tinted (not the neutral grey) and the accent stays visible against them.
            assert_ne!(t.toolbar, Theme::for_persona(dark, PersonaColor::Orange).toolbar);
            let lum = |x: Rgba| x.0 as i32 + x.1 as i32 + x.2 as i32;
            assert!((lum(t.accent) - lum(t.toolbar)).abs() > 120);
        }
    }
}
