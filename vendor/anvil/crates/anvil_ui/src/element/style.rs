//! What an element looks like and how it lays out: everything the builder methods set.
//!
//! Lengths are logical px (ADL). They become physical px once, when the Taffy tree is built.
//! Colors and radii may be theme tokens, resolved when the element is painted, so swapping the
//! theme recolors a tree without rebuilding it.

use crate::theme::{Color, Theme};

/// A color: fixed, or a role in the current theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Paint {
    Fixed(Color),
    Theme(Tok),
    /// A theme color at a fraction of its opacity: status tints and scrims that follow the theme.
    Tint(Tok, f32),
}

/// The theme's color roles, by name (`Theme` has one field for each).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    Bg,
    Sunken,
    Surface,
    Card,
    CardHi,
    Border,
    Ring,
    Text,
    TextDim,
    Muted,
    Disabled,
    DisabledBg,
    Primary,
    PrimaryFg,
    OnColor,
    Selection,
    Code,
    Success,
    Warning,
    Danger,
}

impl Tok {
    pub fn resolve(self, t: &Theme) -> Color {
        match self {
            Tok::Bg => t.bg,
            Tok::Sunken => t.sunken,
            Tok::Surface => t.surface,
            Tok::Card => t.card,
            Tok::CardHi => t.card_hi,
            Tok::Border => t.border,
            Tok::Ring => t.ring,
            Tok::Text => t.text,
            Tok::TextDim => t.text_dim,
            Tok::Muted => t.muted,
            Tok::Disabled => t.disabled,
            Tok::DisabledBg => t.disabled_bg,
            Tok::Primary => t.primary,
            Tok::PrimaryFg => t.primary_fg,
            Tok::OnColor => t.on_color,
            Tok::Selection => t.selection,
            Tok::Code => t.code,
            Tok::Success => t.success,
            Tok::Warning => t.warning,
            Tok::Danger => t.danger,
        }
    }
}

impl Paint {
    pub fn resolve(self, t: &Theme) -> Color {
        match self {
            Paint::Fixed(c) => c,
            Paint::Theme(tok) => tok.resolve(t),
            Paint::Tint(tok, a) => {
                let c = tok.resolve(t);
                [c[0], c[1], c[2], c[3] * a]
            }
        }
    }
}

impl From<Color> for Paint {
    fn from(c: Color) -> Self {
        Paint::Fixed(c)
    }
}

impl Tok {
    /// This role at `alpha` of its opacity.
    pub fn tint(self, alpha: f32) -> Paint {
        Paint::Tint(self, alpha)
    }
}

impl From<Tok> for Paint {
    fn from(t: Tok) -> Self {
        Paint::Theme(t)
    }
}

/// A corner radius: logical px, or one of the theme's radii.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Radius {
    Px(f32),
    Sm,
    Md,
    Lg,
    /// Half the shorter side: a pill or a circle.
    Full,
}

impl Radius {
    /// In logical px.
    pub fn resolve(self, t: &Theme) -> f32 {
        match self {
            Radius::Px(r) => r,
            Radius::Sm => t.radius.sm,
            Radius::Md => t.radius.md,
            Radius::Lg => t.radius.lg,
            Radius::Full => t.radius.pill,
        }
    }
}

/// A size along one axis.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Length {
    /// Sized by content: ADL "Hug".
    #[default]
    Auto,
    /// Logical px: ADL "Fixed".
    Px(f32),
    /// A fraction (0 to 1) of the parent's inner size.
    Frac(f32),
}

impl Length {
    /// For min and max sizes, which Taffy 0.14 types separately.
    fn lpa(self, scale: f32) -> taffy::LengthPercentageAuto {
        match self {
            Length::Auto => taffy::LengthPercentageAuto::auto(),
            Length::Px(v) => taffy::LengthPercentageAuto::length(v * scale),
            Length::Frac(f) => taffy::LengthPercentageAuto::percent(f),
        }
    }

    fn dimension(self, scale: f32) -> taffy::Dimension {
        match self {
            Length::Auto => taffy::Dimension::auto(),
            Length::Px(v) => taffy::Dimension::length(v * scale),
            Length::Frac(f) => taffy::Dimension::percent(f),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Display {
    #[default]
    Flex,
    Grid,
    Block,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
    Stretch,
    Baseline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Justify {
    Start,
    Center,
    End,
    Between,
    Around,
    Evenly,
}

/// Four sides in logical px.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

/// Everything about an element that is not its content.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    // Layout.
    pub display: Display,
    /// Lay children out in a row (true) or a column (false). Divs default to a column.
    pub row: bool,
    pub wrap: bool,
    pub align_items: Option<Align>,
    pub align_self: Option<Align>,
    pub justify: Option<Justify>,
    /// Gap between columns and between rows, logical px.
    pub gap: (f32, f32),
    pub padding: Edges,
    pub margin: Edges,
    pub width: Length,
    pub height: Length,
    pub min_width: Length,
    pub min_height: Length,
    pub max_width: Length,
    pub max_height: Length,
    pub grow: f32,
    pub shrink: f32,
    /// `None` is auto.
    pub basis: Option<Length>,
    /// Grid: this many equal columns.
    pub grid_cols: Option<u16>,
    /// Grid: span this many columns.
    pub col_span: u16,
    /// Placed relative to the parent instead of in the flow. Insets top, right, bottom, left in
    /// logical px; a side left `None` is free.
    pub absolute: Option<[Option<f32>; 4]>,
    // Paint.
    pub bg: Option<Paint>,
    pub border: f32,
    pub border_color: Paint,
    pub radius: Radius,
    pub shadow: bool,
    pub glass: bool,
    pub opacity: f32,
    /// Children are clipped to this element's box.
    pub clip: bool,
    /// A vertical scroll container: children keep their height and scroll (`Element::scroll`).
    pub scroll: bool,
    // Inherited by text and icons inside, unless they set their own.
    pub text_color: Option<Paint>,
    pub text_size: Option<f32>,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            display: Display::Flex,
            row: false,
            wrap: false,
            align_items: None,
            align_self: None,
            justify: None,
            gap: (0.0, 0.0),
            padding: Edges::default(),
            margin: Edges::default(),
            width: Length::Auto,
            height: Length::Auto,
            min_width: Length::Auto,
            min_height: Length::Auto,
            max_width: Length::Auto,
            max_height: Length::Auto,
            grow: 0.0,
            shrink: 1.0,
            basis: None,
            grid_cols: None,
            col_span: 1,
            absolute: None,
            bg: None,
            border: 0.0,
            border_color: Paint::Theme(Tok::Border),
            radius: Radius::Px(0.0),
            shadow: false,
            glass: false,
            opacity: 1.0,
            clip: false,
            scroll: false,
            text_color: None,
            text_size: None,
        }
    }
}

fn align(a: Align) -> taffy::AlignItems {
    match a {
        Align::Start => taffy::AlignItems::FLEX_START,
        Align::Center => taffy::AlignItems::CENTER,
        Align::End => taffy::AlignItems::FLEX_END,
        Align::Stretch => taffy::AlignItems::STRETCH,
        Align::Baseline => taffy::AlignItems::BASELINE,
    }
}

fn lp(v: f32, scale: f32) -> taffy::LengthPercentage {
    taffy::LengthPercentage::length(v * scale)
}

impl Style {
    /// The Taffy style for this element at `scale` physical px per logical px.
    pub fn to_taffy(&self, scale: f32) -> taffy::Style {
        let edges_lp = |e: Edges| taffy::Rect {
            left: lp(e.left, scale),
            right: lp(e.right, scale),
            top: lp(e.top, scale),
            bottom: lp(e.bottom, scale),
        };
        let mut s = taffy::Style {
            display: match self.display {
                Display::Flex => taffy::Display::Flex,
                Display::Grid => taffy::Display::Grid,
                Display::Block => taffy::Display::Block,
                Display::None => taffy::Display::None,
            },
            flex_direction: if self.row { taffy::FlexDirection::Row } else { taffy::FlexDirection::Column },
            flex_wrap: if self.wrap { taffy::FlexWrap::Wrap } else { taffy::FlexWrap::NoWrap },
            align_items: self.align_items.map(align),
            align_self: self.align_self.map(align),
            justify_content: self.justify.map(|j| match j {
                Justify::Start => taffy::JustifyContent::FLEX_START,
                Justify::Center => taffy::JustifyContent::CENTER,
                Justify::End => taffy::JustifyContent::FLEX_END,
                Justify::Between => taffy::JustifyContent::SPACE_BETWEEN,
                Justify::Around => taffy::JustifyContent::SPACE_AROUND,
                Justify::Evenly => taffy::JustifyContent::SPACE_EVENLY,
            }),
            gap: taffy::Size { width: lp(self.gap.0, scale), height: lp(self.gap.1, scale) },
            padding: edges_lp(self.padding),
            margin: taffy::Rect {
                left: taffy::LengthPercentageAuto::length(self.margin.left * scale),
                right: taffy::LengthPercentageAuto::length(self.margin.right * scale),
                top: taffy::LengthPercentageAuto::length(self.margin.top * scale),
                bottom: taffy::LengthPercentageAuto::length(self.margin.bottom * scale),
            },
            border: taffy::Rect {
                left: lp(self.border, scale),
                right: lp(self.border, scale),
                top: lp(self.border, scale),
                bottom: lp(self.border, scale),
            },
            size: taffy::Size { width: self.width.dimension(scale), height: self.height.dimension(scale) },
            min_size: taffy::Size { width: self.min_width.lpa(scale), height: self.min_height.lpa(scale) },
            max_size: taffy::Size { width: self.max_width.lpa(scale), height: self.max_height.lpa(scale) },
            flex_grow: self.grow,
            flex_shrink: self.shrink,
            flex_basis: self.basis.map_or(taffy::Dimension::auto(), |b| b.dimension(scale)),
            ..Default::default()
        };
        if self.scroll {
            // Scrolled content may be taller than the box, and the box does not grow to fit it.
            s.overflow = taffy::Point { x: taffy::Overflow::Hidden, y: taffy::Overflow::Scroll };
            s.scrollbar_width = 0.0;
        }
        if let Some(n) = self.grid_cols {
            s.grid_template_columns = (0..n).map(|_| taffy::prelude::fr(1.0)).collect();
        }
        if self.col_span > 1 {
            s.grid_column = taffy::Line { start: taffy::GridPlacement::Auto, end: taffy::prelude::span(self.col_span) };
        }
        if let Some([top, right, bottom, left]) = self.absolute {
            let side = |v: Option<f32>| v.map_or(taffy::LengthPercentageAuto::auto(), |v| taffy::LengthPercentageAuto::length(v * scale));
            s.position = taffy::Position::Absolute;
            s.inset = taffy::Rect { left: side(left), right: side(right), top: side(top), bottom: side(bottom) };
        }
        s
    }
}
