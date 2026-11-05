//! Design tokens from `ADL.md`: a neutral zinc palette, corner radii, and 1 px borders.
//! Colors are straight-alpha RGBA in 0..1. Shadows belong to floating surfaces only, and there
//! are no glows.

pub use anvl_gpu::color::{hex, lerp, with_alpha, Color};

/// The design tokens a `Ui` draws with, swappable at runtime (`Ui::set_theme`). Every widget
/// reads its colors and radii from here, never from the `palette` or `anvl` constants, which stay
/// only as the values of [`Theme::adl_dark`] and for code that has not moved over yet (the kiosk
/// demo).
///
/// One name per role, so the old palettes' clashes are gone: `sunken` is the input well
/// (`palette::SURFACE`), `surface` is bars and sidebars (`anvl::SURFACE`), `text_dim` is readable
/// secondary text (`palette::TEXT_DIM`, `anvl::MUTED`), and `muted` is the dimmer caption color
/// (`palette::MUTED`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub name: &'static str,
    pub dark: bool,
    /// The canvas behind everything.
    pub bg: Color,
    /// Input wells, which sit below the surface around them.
    pub sunken: Color,
    /// Bars, headers and sidebars.
    pub surface: Color,
    pub card: Color,
    /// Hover fills, secondary buttons, selected rows.
    pub card_hi: Color,
    pub border: Color,
    /// Focus and selection outlines.
    pub ring: Color,
    pub text: Color,
    /// Secondary text, readable at body sizes.
    pub text_dim: Color,
    /// Captions and hints, 13 px and up only.
    pub muted: Color,
    /// Text and icons on disabled controls, and placeholders.
    pub disabled: Color,
    pub disabled_bg: Color,
    /// The one main action on a screen.
    pub primary: Color,
    pub primary_fg: Color,
    /// Text on a status color fill (Delete, Done).
    pub on_color: Color,
    pub selection: Color,
    pub code: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    pub radius: Radii,
    /// How opaque a glass surface's tint is over its blurred backdrop.
    pub glass_alpha: f32,
}

/// Corner radii in logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radii {
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub pill: f32,
}

impl Theme {
    /// ADL v3 dark zinc: Anvil's default, exactly the look before themes existed.
    pub const fn adl_dark() -> Self {
        Self {
            name: "ADL dark",
            dark: true,
            bg: hex(0x09090b),
            sunken: hex(0x09090b),
            surface: hex(0x121215),
            card: hex(0x18181b),
            card_hi: hex(0x27272a),
            border: hex(0x27272a),
            ring: hex(0xa1a1aa),
            text: hex(0xf4f4f5),
            text_dim: hex(0xa1a1aa),
            muted: hex(0x71717a),
            disabled: hex(0x71717a),
            disabled_bg: hex(0x141417),
            primary: hex(0xfafafa),
            primary_fg: hex(0x18181b),
            on_color: hex(0xffffff),
            selection: hex(0x3f3f46),
            code: hex(0xd4d4d8),
            success: hex(0x10b981),
            warning: hex(0xf59e0b),
            danger: hex(0xef4444),
            radius: Radii { sm: 4.0, md: 8.0, lg: 12.0, pill: 9999.0 },
            glass_alpha: 0.8,
        }
    }

    /// ADL light: the same roles on zinc-50 and white. Text is zinc-900 (16:1 on the canvas),
    /// secondary text zinc-600 (7.6:1) and captions zinc-500 (4.8:1). Status fills are one
    /// step darker than the dark theme's so white text on them stays above 4.5:1.
    pub const fn adl_light() -> Self {
        Self {
            name: "ADL light",
            dark: false,
            bg: hex(0xfafafa),
            sunken: hex(0xf4f4f5),
            surface: hex(0xffffff),
            card: hex(0xffffff),
            card_hi: hex(0xf4f4f5),
            border: hex(0xe4e4e7),
            ring: hex(0x52525b),
            text: hex(0x18181b),
            text_dim: hex(0x52525b),
            muted: hex(0x71717a),
            disabled: hex(0x8a8a92),
            disabled_bg: hex(0xf4f4f5),
            primary: hex(0x18181b),
            primary_fg: hex(0xfafafa),
            on_color: hex(0xffffff),
            selection: hex(0xd4d4d8),
            code: hex(0x3f3f46),
            success: hex(0x047857),
            warning: hex(0xb45309),
            danger: hex(0xdc2626),
            radius: Radii { sm: 4.0, md: 8.0, lg: 12.0, pill: 9999.0 },
            glass_alpha: 0.8,
        }
    }

    /// High contrast: black canvas, white 1 px borders and text, a yellow focus ring, and bright
    /// status colors with black text on them. Every text role is 7:1 or better on the canvas
    /// and on cards, and glass is opaque.
    pub const fn high_contrast() -> Self {
        Self {
            name: "High contrast",
            dark: true,
            bg: hex(0x000000),
            sunken: hex(0x000000),
            surface: hex(0x0a0a0a),
            card: hex(0x0a0a0a),
            card_hi: hex(0x2a2a2a),
            border: hex(0xffffff),
            ring: hex(0xffd60a),
            text: hex(0xffffff),
            text_dim: hex(0xe5e5e5),
            muted: hex(0xd4d4d4),
            disabled: hex(0x9a9a9a),
            disabled_bg: hex(0x1a1a1a),
            primary: hex(0xffffff),
            primary_fg: hex(0x000000),
            on_color: hex(0x000000),
            selection: hex(0x1d4ed8),
            code: hex(0xffffff),
            success: hex(0x4ade80),
            warning: hex(0xfbbf24),
            danger: hex(0xf87171),
            radius: Radii { sm: 4.0, md: 8.0, lg: 12.0, pill: 9999.0 },
            glass_alpha: 1.0,
        }
    }

    /// Every built-in theme, dark first.
    pub const fn all() -> [Theme; 3] {
        [Self::adl_dark(), Self::adl_light(), Self::high_contrast()]
    }

    /// The theme `t` of the way from `self` to `to` (0 to 1). Colors and the glass alpha
    /// blend; the name and `dark` flag switch over at the halfway point.
    pub fn lerp(&self, to: &Theme, t: f32) -> Theme {
        let c = |a: Color, b: Color| lerp(a, b, t);
        let near = if t < 0.5 { self } else { to };
        Theme {
            name: near.name,
            dark: near.dark,
            bg: c(self.bg, to.bg),
            sunken: c(self.sunken, to.sunken),
            surface: c(self.surface, to.surface),
            card: c(self.card, to.card),
            card_hi: c(self.card_hi, to.card_hi),
            border: c(self.border, to.border),
            ring: c(self.ring, to.ring),
            text: c(self.text, to.text),
            text_dim: c(self.text_dim, to.text_dim),
            muted: c(self.muted, to.muted),
            disabled: c(self.disabled, to.disabled),
            disabled_bg: c(self.disabled_bg, to.disabled_bg),
            primary: c(self.primary, to.primary),
            primary_fg: c(self.primary_fg, to.primary_fg),
            on_color: c(self.on_color, to.on_color),
            selection: c(self.selection, to.selection),
            code: c(self.code, to.code),
            success: c(self.success, to.success),
            warning: c(self.warning, to.warning),
            danger: c(self.danger, to.danger),
            radius: near.radius,
            glass_alpha: self.glass_alpha + (to.glass_alpha - self.glass_alpha) * t,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::adl_dark()
    }
}

/// Corner radii in design pixels. Shapes clamp to half their height, so `PILL` is a capsule.
pub mod radius {
    pub const SM: f32 = 4.0;
    pub const MD: f32 = 8.0;
    pub const LG: f32 = 12.0;
    pub const PILL: f32 = 9999.0;
}

/// Neutral zinc palette (shadcn/ui dark theme). Values match `ADL.md` section 1.
pub mod palette {
    use super::{hex, Color};

    /// Page canvas.
    pub const BG: Color = hex(0x09090b);
    /// Card and popover surface.
    pub const CARD: Color = hex(0x18181b);
    /// Hover surface and secondary fill.
    pub const CARD_HI: Color = hex(0x27272a);
    /// Input surface. Sits on the canvas, so it reads as sunken.
    pub const SURFACE: Color = hex(0x09090b);
    /// The one border color. Every border is 1 px.
    pub const BORDER: Color = hex(0x27272a);
    /// Hairline dividers. Same value as `BORDER` by design.
    pub const BORDER_SOFT: Color = hex(0x27272a);

    /// Primary text.
    pub const TEXT: Color = hex(0xf4f4f5);
    /// Secondary text.
    pub const TEXT_DIM: Color = hex(0xa1a1aa);
    /// Muted text: labels, hints, captions, placeholders.
    pub const MUTED: Color = hex(0x71717a);
    /// Text and icons on disabled controls. 3:1 or better on CARD, CARD_HI, and DISABLED_BG.
    pub const DISABLED_TEXT: Color = hex(0x71717a);
    pub const WHITE: Color = hex(0xffffff);

    /// Solid primary fill for the main action.
    pub const PRIMARY: Color = hex(0xfafafa);
    /// Text on a primary fill.
    pub const PRIMARY_FG: Color = hex(0x18181b);
    /// Focus and selection outline.
    pub const RING: Color = hex(0xa1a1aa);
    /// Text selection highlight.
    pub const SELECTION: Color = hex(0x3f3f46);

    /// Success and online.
    pub const SUCCESS: Color = hex(0x10b981);
    /// Pending and caution.
    pub const WARNING: Color = hex(0xf59e0b);
    /// Destructive and error.
    pub const DANGER: Color = hex(0xef4444);
    /// Fill of a disabled control.
    pub const DISABLED_BG: Color = hex(0x141417);

    /// Floating surface (Layer 1): modals, dropdown menus, sheets, sticky bars. CARD at 80%.
    pub const CARD_GLASS: Color = super::with_alpha(hex(0x18181b), 0.80);
}

/// Layer 1 drop shadows (ADL section 3.3). Each entry is a vertical offset, a blur size, and an
/// opacity. They apply to floating surfaces only, never to Layer 0.
pub mod shadow {
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Shadow {
        pub offset_y: f32,
        pub blur: f32,
        pub alpha: f32,
    }

    /// `SHADOW_OVERLAY`: 0 8px 24px rgba(0,0,0,0.45) and 0 2px 6px rgba(0,0,0,0.30).
    pub const OVERLAY: [Shadow; 2] = [
        Shadow {
            offset_y: 8.0,
            blur: 24.0,
            alpha: 0.45,
        },
        Shadow {
            offset_y: 2.0,
            blur: 6.0,
            alpha: 0.30,
        },
    ];
}

/// Motion tokens (ADL-V3 section 4). Every animation picks a duration and a curve from here.
/// Durations are seconds. An exponential ease reaches about 95% of its distance in `3 / rate`
/// seconds, so [`rate`] turns a duration into the rate `Ui::animate` takes.
pub mod motion {
    /// Hover and press feedback, color changes.
    pub const INSTANT: f32 = 0.09;
    /// Tooltips, small fades, focus rings.
    pub const FAST: f32 = 0.16;
    /// Toasts, menus, list items entering.
    pub const BASE: f32 = 0.24;
    /// Screen transitions.
    pub const SCREEN: f32 = 0.28;
    /// Sheets and modals.
    pub const SLOW: f32 = 0.32;
    /// Delay between items in a staggered entrance. At most [`STAGGER_MAX`] items are delayed.
    pub const STAGGER: f32 = 0.04;
    pub const STAGGER_MAX: usize = 5;
    /// How long the pointer rests on a control before its tooltip shows.
    pub const TOOLTIP_DELAY: f32 = 0.5;
    /// How long a toast stays when nothing holds it. A toast with an action stays longer.
    pub const TOAST_TTL: f32 = 3.2;
    pub const TOAST_ACTION_TTL: f32 = 6.0;

    /// The exponential rate that settles in about `duration` seconds.
    pub fn rate(duration: f32) -> f32 {
        3.0 / duration.max(1e-3)
    }

    /// A damped spring: stiffness and damping for `Ui::spring`.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Spring {
        pub stiffness: f32,
        pub damping: f32,
    }

    impl Spring {
        /// Creates an Apple-style fluid spring specified by oscillation `response` (in seconds)
        /// and `damping_ratio` (1.0 = smooth critically damped, 0.70-0.80 = organic micro-overshoot).
        pub fn apple(response: f32, damping_ratio: f32) -> Self {
            let omega = std::f32::consts::TAU / response.max(1e-3);
            Self {
                stiffness: omega * omega,
                damping: 2.0 * damping_ratio * omega,
            }
        }

        /// Apple snappy spring for buttons, keys, and toggles (response: 0.28s, damping: 0.76).
        pub const SNAPPY: Spring = Spring { stiffness: 503.7, damping: 34.1 };
        /// Apple smooth spring for sheets, modals, screen transitions (response: 0.40s, damping: 0.92).
        pub const SMOOTH: Spring = Spring { stiffness: 246.7, damping: 28.9 };
        /// Apple playful bouncy spring for badges, celebrations, icons (response: 0.38s, damping: 0.62).
        pub const BOUNCY: Spring = Spring { stiffness: 273.4, damping: 20.5 };
    }

    /// About 4% overshoot, settles in 400 ms. Indicators, sheets, releases.
    pub const SPRING: Spring = Spring { stiffness: 260.0, damping: 23.0 };
    /// No overshoot, settles in about 300 ms. Things that must not bounce, like a stack of toasts.
    pub const SPRING_CALM: Spring = Spring { stiffness: 300.0, damping: 35.0 };

    /// Entrances and moves: fast start, gentle stop.
    pub fn ease_out_cubic(t: f32) -> f32 {
        let u = 1.0 - t.clamp(0.0, 1.0);
        1.0 - u * u * u
    }

    /// Apple fluid ease-out: responsive immediate start with luxurious cushion (iOS cubic-bezier(0.16, 1.0, 0.3, 1.0)).
    pub fn ease_out_apple(t: f32) -> f32 {
        let u = 1.0 - t.clamp(0.0, 1.0);
        1.0 - u * u * u * u
    }

    /// Exits: gentle start, fast stop.
    pub fn ease_in_cubic(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        t * t * t
    }

    /// Back-and-forth offset for an error shake, in units of its amplitude. `t` is 0 to 1 over
    /// the shake. Three swings that die away, ending at rest.
    pub fn shake(t: f32) -> f32 {
        if !(0.0..1.0).contains(&t) {
            return 0.0;
        }
        (t * std::f32::consts::TAU * 3.0).sin() * (1.0 - t)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn curves_start_and_end_on_their_targets() {
            for f in [ease_out_cubic, ease_in_cubic] {
                assert_eq!(f(0.0), 0.0);
                assert_eq!(f(1.0), 1.0);
                assert_eq!(f(-1.0), 0.0);
                assert_eq!(f(2.0), 1.0);
            }
            assert!(ease_out_cubic(0.5) > 0.5);
            assert!(ease_in_cubic(0.5) < 0.5);
        }

        #[test]
        fn shake_rests_outside_its_window() {
            assert_eq!(shake(-0.1), 0.0);
            assert_eq!(shake(1.0), 0.0);
            assert!(shake(1.0 / 12.0) > 0.5);
        }

        #[test]
        fn rate_settles_within_the_duration() {
            let r = rate(SCREEN);
            let left = (-r * SCREEN).exp();
            assert!(left < 0.06);
        }
    }
}

/// ADL v2.1 tokens for ANVL Note. They sit beside `palette` instead of replacing it, because the
/// kiosk app still reads `palette` (there `SURFACE` is the sunken input color and `MUTED` is the
/// dimmer zinc). Here `SURFACE` is the sidebar and header fill, and `MUTED` is the readable zinc.
pub mod anvl {
    use super::{hex, Color};

    // Surfaces and borders.
    /// Root window and editor canvas.
    pub const BG: Color = hex(0x09090b);
    /// Sidebar and header.
    pub const SURFACE: Color = hex(0x121215);
    /// Note list items and floating status cards.
    pub const CARD: Color = hex(0x18181b);
    /// Hovered or active note item, secondary buttons.
    pub const CARD_HI: Color = hex(0x27272a);
    /// The one stroke color, always 1 logical px.
    pub const BORDER: Color = hex(0x27272a);
    /// Active input focus border.
    pub const RING: Color = hex(0xa1a1aa);

    // Foreground.
    /// Editor text and active headings.
    pub const TEXT: Color = hex(0xf4f4f5);
    /// Timestamps, character counts, inactive note titles.
    pub const MUTED: Color = hex(0xa1a1aa);
    /// Empty placeholders.
    pub const DISABLED_TEXT: Color = hex(0x71717a);
    /// Selected text in the editor.
    pub const SELECTION: Color = hex(0x3f3f46);
    /// Code in the editor: inline and in blocks. A neutral a step under `TEXT`, set apart by
    /// its panel rather than by hue.
    pub const CODE: Color = hex(0xd4d4d8);

    // Semantic accents.
    /// Synced badge, connected peer dot.
    pub const SUCCESS: Color = hex(0x10b981);
    /// Unsaved local changes, syncing.
    pub const WARNING: Color = hex(0xf59e0b);
    /// Delete action, offline error.
    pub const DANGER: Color = hex(0xef4444);

    // Corner radii in logical px.
    pub const SM: f32 = 6.0;
    pub const MD: f32 = 8.0;
    pub const LG: f32 = 12.0;
    pub const PILL: f32 = 999.0;

    /// Decay rate of the exponential ease, per second.
    pub const EASE_RATE: f32 = 14.0;

    /// Frame-rate-independent exponential decay. A value moves toward its target by
    /// `value += (target - value) * ease(dt)`, which covers the same distance per second at 30,
    /// 60, or 144 fps.
    pub fn ease(dt: f32) -> f32 {
        1.0 - (-EASE_RATE * dt).exp()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn ease_is_frame_rate_independent() {
            // Two 8 ms steps cover the same ground as one 16 ms step.
            let one = ease(0.016);
            let two = 1.0 - (1.0 - ease(0.008)).powi(2);
            assert!((one - two).abs() < 1e-5);
            assert_eq!(ease(0.0), 0.0);
            assert!(ease(10.0) > 0.999);
        }
    }
}

#[cfg(test)]
mod theme_tests {
    use super::*;

    #[test]
    fn the_dark_theme_is_exactly_the_old_palettes() {
        let t = Theme::adl_dark();
        assert_eq!((t.bg, t.sunken, t.surface), (palette::BG, palette::SURFACE, anvl::SURFACE));
        assert_eq!((t.card, t.card_hi, t.border, t.ring), (palette::CARD, palette::CARD_HI, palette::BORDER, palette::RING));
        assert_eq!((t.text, t.text_dim, t.muted, t.disabled), (palette::TEXT, palette::TEXT_DIM, palette::MUTED, palette::DISABLED_TEXT));
        assert_eq!((t.text_dim, t.disabled), (anvl::MUTED, anvl::DISABLED_TEXT));
        assert_eq!((t.primary, t.primary_fg, t.on_color, t.disabled_bg), (palette::PRIMARY, palette::PRIMARY_FG, palette::WHITE, palette::DISABLED_BG));
        assert_eq!((t.selection, t.code), (anvl::SELECTION, anvl::CODE));
        assert_eq!((t.success, t.warning, t.danger), (anvl::SUCCESS, anvl::WARNING, anvl::DANGER));
        assert_eq!((t.radius.sm, t.radius.md, t.radius.lg), (radius::SM, radius::MD, radius::LG));
        assert_eq!((anvl::MD, anvl::LG), (t.radius.md, t.radius.lg));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance of an sRGB color.
    fn luminance(c: Color) -> f32 {
        let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
    }

    fn contrast(a: Color, b: Color) -> f32 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn text_roles_are_readable_on_the_surfaces_they_sit_on() {
        for t in Theme::all() {
            let big = if t.name == "High contrast" { 7.0 } else { 4.5 };
            for (on, bg) in [("bg", t.bg), ("surface", t.surface), ("card", t.card), ("sunken", t.sunken), ("card_hi", t.card_hi)] {
                assert!(contrast(t.text, bg) >= big, "{}: text on {on} is {:.1}", t.name, contrast(t.text, bg));
                assert!(contrast(t.text_dim, bg) >= 4.5, "{}: text_dim on {on} is {:.1}", t.name, contrast(t.text_dim, bg));
            }
            // Captions are 13 px and up (ADL section 1.1); ADL dark has always been 4.1 on the canvas and 3.7 on cards, so 3.5.
            for (on, bg) in [("bg", t.bg), ("card", t.card)] {
                assert!(contrast(t.muted, bg) >= 3.5, "{}: muted on {on} is {:.1}", t.name, contrast(t.muted, bg));
            }
            assert!(contrast(t.primary_fg, t.primary) >= 7.0, "{}: primary label {:.1}", t.name, contrast(t.primary_fg, t.primary));
            // ADL dark keeps its original status fills (white on emerald is 2.5), so only the
            // newer themes are held to AA for labels on them.
            for (name, fill) in [("success", t.success), ("warning", t.warning), ("danger", t.danger)].into_iter().filter(|_| t.name != "ADL dark") {
                assert!(contrast(t.on_color, fill) >= 4.5, "{}: label on {name} is {:.1}", t.name, contrast(t.on_color, fill));
            }
            assert!(contrast(t.disabled, t.disabled_bg) >= 3.0, "{}: disabled text {:.1}", t.name, contrast(t.disabled, t.disabled_bg));
            // The focus ring must stand out from the canvas it is drawn on.
            assert!(contrast(t.ring, t.bg) >= 3.0, "{}: focus ring {:.1}", t.name, contrast(t.ring, t.bg));
            assert!(contrast(t.ring, t.card) >= 3.0, "{}: focus ring on cards {:.1}", t.name, contrast(t.ring, t.card));
        }
    }

    #[test]
    fn high_contrast_has_visible_borders() {
        let t = Theme::high_contrast();
        assert!(contrast(t.border, t.bg) >= 7.0 && contrast(t.border, t.card) >= 7.0);
        assert_eq!(t.glass_alpha, 1.0, "glass is opaque");
    }

    #[test]
    fn lerp_runs_from_one_theme_to_the_other() {
        let (a, b) = (Theme::adl_dark(), Theme::adl_light());
        assert_eq!(a.lerp(&b, 0.0).bg, a.bg);
        assert_eq!(a.lerp(&b, 1.0).bg, b.bg);
        assert_eq!(a.lerp(&b, 1.0).name, "ADL light");
        let mid = a.lerp(&b, 0.5);
        assert!(luminance(mid.bg) > luminance(a.bg) && luminance(mid.bg) < luminance(b.bg));
    }

    #[test]
    fn adl_dark_is_unchanged() {
        let d = Theme::adl_dark();
        assert_eq!((d.bg, d.card, d.text), (palette::BG, palette::CARD, palette::TEXT));
    }
}
