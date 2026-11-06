//! The element tree (ANVIL-UI-2.md, Phase 1): describe a screen as nested elements with
//! Tailwind-like style builders, and let Taffy lay it out.
//!
//! ```ignore
//! ui.element(area, div()
//!     .flex_col().gap_4().p_4()
//!     .bg(Tok::Card).rounded_lg().border_1()
//!     .child(div().flex_row().items_center().justify_between()
//!         .child(text("Battery").heading())
//!         .child(text("94%").color(Tok::Success)))
//!     .child(text("Paragraphs wrap to the width they are given.").body()));
//! ```
//!
//! A tree is built, laid out and painted in the same frame (`Ui::element`). `canvas` hands its
//! laid-out rectangle to ordinary immediate-mode code, so old widgets and new elements mix
//! freely while screens move over. What elements do when used (actions, state styles, focus,
//! scrolling, enter motion) is in `interact`.

mod interact;
mod list;
mod render;
mod style;

pub use interact::{Enter, Exit, Pin, Pointer, StateStyle};
pub use list::OVERSCAN;
pub(crate) use interact::{Action, Interact};
pub(crate) use list::{fill_lists, lower_lists};
pub use render::LaidOut;
pub use style::{Align, Display, Edges, Justify, Length, Paint, Radius, Style, Tok};

use crate::geom::Rect;
use crate::icons::Icon;
use crate::text::{FontRole, WEIGHT_MEDIUM, WEIGHT_REGULAR, WEIGHT_SEMIBOLD};
use crate::ui::{Fit, Ui};
use crate::{ElementId, TextureId};

/// How a text element looks. Unset parts inherit from the nearest parent that sets them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextLook {
    pub size: Option<f32>,
    pub weight: u16,
    pub role: FontRole,
    pub italic: bool,
    pub color: Option<Paint>,
    /// One line, ellipsized to fit, instead of wrapping.
    pub nowrap: bool,
}

/// The drawing a `canvas` element does, inside its laid-out rectangle.
pub type CanvasFn<'a> = Box<dyn FnOnce(&mut Ui, Rect) + 'a>;

/// What an element is.
pub enum Kind<'a> {
    Div,
    Text { content: String, look: TextLook },
    Icon { icon: Icon, size: f32, color: Option<Paint> },
    Img { texture: TextureId, fit: Fit },
    /// Draws itself with ordinary `Ui` calls inside its laid-out rectangle.
    Canvas(CanvasFn<'a>),
    /// A virtualized list (`uniform_list`). `Ui::element` swaps it for a scroll container with
    /// only the rows in view before the tree is laid out.
    List(list::ListSpec<'a>),
}

/// One node of the tree: what it is, its style, its children, and an optional id.
pub struct Element<'a> {
    pub kind: Kind<'a>,
    pub style: Style,
    pub children: Vec<Element<'a>>,
    /// Keys the element's state (hover blend, scroll offset, enter motion). Without one, an
    /// element is keyed by its parent's id and its place among its siblings.
    pub id: Option<ElementId>,
    pub(crate) interact: Option<Box<Interact<'a>>>,
}

fn element(kind: Kind) -> Element {
    Element { kind, style: Style::default(), children: Vec::new(), id: None, interact: None }
}

/// A container. Children stack in a column unless `.flex_row()`.
pub fn div<'a>() -> Element<'a> {
    element(Kind::Div)
}

/// Text: 15 px Inter by default, wrapping to the width it gets. See [`Element::body`],
/// [`Element::heading`] and friends.
pub fn text<'a>(content: impl Into<String>) -> Element<'a> {
    let look = TextLook { size: None, weight: WEIGHT_REGULAR, role: FontRole::Ui, italic: false, color: None, nowrap: false };
    element(Kind::Text { content: content.into(), look })
}

/// An icon from the baked atlas: 20 px, the inherited text color.
pub fn icon<'a>(icon: Icon) -> Element<'a> {
    element(Kind::Icon { icon, size: 20.0, color: None })
}

/// A picture at its own size unless sized. `Fit` says how it meets the box.
pub fn img<'a>(texture: TextureId) -> Element<'a> {
    element(Kind::Img { texture, fit: Fit::Cover })
}

/// Custom drawing with ordinary `Ui` calls inside the element's rectangle (physical px). This
/// is the bridge that lets immediate-mode widgets sit in an element tree. Give it a size, or
/// `flex_1()`: it has no content to measure.
pub fn canvas<'a>(draw: impl FnOnce(&mut Ui, Rect) + 'a) -> Element<'a> {
    element(Kind::Canvas(Box::new(draw)))
}

/// A virtualized vertical list: `count` rows of a fixed `row_h` logical px, of which only the
/// ones in view (and a few either side) are built, by calling `row(index)`. It scrolls like
/// `.scroll()`, so give it a height (or `flex_1()`) and an `id`; the id also keys its rows, so
/// a row without an id of its own is keyed by its index. A list inside a row of another list is
/// not supported.
///
/// ```ignore
/// uniform_list("notes", notes.len(), 64.0, |i| note_row(&notes[i])).flex_1()
/// ```
pub fn uniform_list<'a, R: Into<Element<'a>>>(id: impl Into<ElementId>, count: usize, row_h: f32, row: impl Fn(usize) -> R + 'a) -> Element<'a> {
    let mut el = element(Kind::List(list::ListSpec::new(count, row_h, move |i| row(i).into())));
    el.id = Some(id.into());
    el.style.scroll = true;
    el
}

/// Generates a spacing method per ADL step (4 px each): `gap_1` is 4, `p_4` is 16, ...
macro_rules! steps {
    ($($name:ident => $method:ident($px:expr);)*) => {
        $(pub fn $name(self) -> Self { self.$method($px) })*
    };
}

impl<'a> Element<'a> {
    // ---------------------------------------------------------------- tree

    pub fn child(mut self, child: impl Into<Element<'a>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = Element<'a>>) -> Self {
        self.children.extend(children);
        self
    }

    /// Gives the element an id. List items need one, so their state follows them when the list
    /// changes.
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    // ---------------------------------------------------------------- layout

    pub fn flex_row(mut self) -> Self {
        self.style.display = Display::Flex;
        self.style.row = true;
        self
    }

    pub fn flex_col(mut self) -> Self {
        self.style.display = Display::Flex;
        self.style.row = false;
        self
    }

    /// Lets children flow onto more lines when they run out of room.
    pub fn flex_wrap(mut self) -> Self {
        self.style.wrap = true;
        self
    }

    /// A grid of `n` equal columns.
    pub fn grid_cols(mut self, n: u16) -> Self {
        self.style.display = Display::Grid;
        self.style.grid_cols = Some(n.max(1));
        self
    }

    pub fn col_span(mut self, n: u16) -> Self {
        self.style.col_span = n.max(1);
        self
    }

    pub fn hidden(mut self) -> Self {
        self.style.display = Display::None;
        self
    }

    pub fn items_start(mut self) -> Self {
        self.style.align_items = Some(Align::Start);
        self
    }
    pub fn items_center(mut self) -> Self {
        self.style.align_items = Some(Align::Center);
        self
    }
    pub fn items_end(mut self) -> Self {
        self.style.align_items = Some(Align::End);
        self
    }
    pub fn items_stretch(mut self) -> Self {
        self.style.align_items = Some(Align::Stretch);
        self
    }
    pub fn items_baseline(mut self) -> Self {
        self.style.align_items = Some(Align::Baseline);
        self
    }
    pub fn self_center(mut self) -> Self {
        self.style.align_self = Some(Align::Center);
        self
    }
    pub fn justify_start(mut self) -> Self {
        self.style.justify = Some(Justify::Start);
        self
    }
    pub fn justify_center(mut self) -> Self {
        self.style.justify = Some(Justify::Center);
        self
    }
    pub fn justify_end(mut self) -> Self {
        self.style.justify = Some(Justify::End);
        self
    }
    pub fn justify_between(mut self) -> Self {
        self.style.justify = Some(Justify::Between);
        self
    }
    pub fn justify_around(mut self) -> Self {
        self.style.justify = Some(Justify::Around);
        self
    }

    /// Space between children, both ways, logical px.
    pub fn gap(mut self, px: f32) -> Self {
        self.style.gap = (px, px);
        self
    }
    pub fn gap_x(mut self, px: f32) -> Self {
        self.style.gap.0 = px;
        self
    }
    pub fn gap_y(mut self, px: f32) -> Self {
        self.style.gap.1 = px;
        self
    }

    pub fn p(mut self, px: f32) -> Self {
        self.style.padding = Edges { top: px, right: px, bottom: px, left: px };
        self
    }
    pub fn px(mut self, px: f32) -> Self {
        self.style.padding.left = px;
        self.style.padding.right = px;
        self
    }
    pub fn py(mut self, px: f32) -> Self {
        self.style.padding.top = px;
        self.style.padding.bottom = px;
        self
    }
    pub fn pt(mut self, px: f32) -> Self {
        self.style.padding.top = px;
        self
    }
    pub fn pb(mut self, px: f32) -> Self {
        self.style.padding.bottom = px;
        self
    }
    pub fn m(mut self, px: f32) -> Self {
        self.style.margin = Edges { top: px, right: px, bottom: px, left: px };
        self
    }
    pub fn mt(mut self, px: f32) -> Self {
        self.style.margin.top = px;
        self
    }
    /// Top and bottom margin.
    pub fn my(mut self, px: f32) -> Self {
        self.style.margin.top = px;
        self.style.margin.bottom = px;
        self
    }
    pub fn my_1(self) -> Self {
        self.my(4.0)
    }
    pub fn mb(mut self, px: f32) -> Self {
        self.style.margin.bottom = px;
        self
    }
    /// Margins may be negative: a knob that overhangs the end of a slider's fill.
    pub fn ml(mut self, px: f32) -> Self {
        self.style.margin.left = px;
        self
    }
    pub fn mr(mut self, px: f32) -> Self {
        self.style.margin.right = px;
        self
    }

    steps! {
        gap_1 => gap(4.0); gap_2 => gap(8.0); gap_3 => gap(12.0); gap_4 => gap(16.0); gap_6 => gap(24.0); gap_8 => gap(32.0);
        p_1 => p(4.0); p_2 => p(8.0); p_3 => p(12.0); p_4 => p(16.0); p_6 => p(24.0); p_8 => p(32.0);
        px_2 => px(8.0); px_3 => px(12.0); px_4 => px(16.0);
        py_1 => py(4.0); py_2 => py(8.0); py_3 => py(12.0); py_4 => py(16.0);
    }

    /// Width in logical px (ADL "Fixed").
    pub fn w(mut self, px: f32) -> Self {
        self.style.width = Length::Px(px);
        self
    }
    pub fn h(mut self, px: f32) -> Self {
        self.style.height = Length::Px(px);
        self
    }
    /// Width and height in logical px.
    pub fn size(self, px: f32) -> Self {
        self.w(px).h(px)
    }
    pub fn w_full(mut self) -> Self {
        self.style.width = Length::Frac(1.0);
        self
    }
    pub fn h_full(mut self) -> Self {
        self.style.height = Length::Frac(1.0);
        self
    }
    /// A fraction (0 to 1) of the parent's width.
    pub fn w_frac(mut self, f: f32) -> Self {
        self.style.width = Length::Frac(f);
        self
    }
    pub fn min_w(mut self, px: f32) -> Self {
        self.style.min_width = Length::Px(px);
        self
    }
    pub fn max_w(mut self, px: f32) -> Self {
        self.style.max_width = Length::Px(px);
        self
    }
    pub fn min_h(mut self, px: f32) -> Self {
        self.style.min_height = Length::Px(px);
        self
    }
    pub fn max_h(mut self, px: f32) -> Self {
        self.style.max_height = Length::Px(px);
        self
    }

    /// Takes a share of the free space, starting from zero: ADL "Fill".
    pub fn flex_1(mut self) -> Self {
        self.style.grow = 1.0;
        self.style.shrink = 1.0;
        self.style.basis = Some(Length::Px(0.0));
        self
    }
    /// Grows into free space with `weight`, from its content size.
    pub fn grow(mut self, weight: f32) -> Self {
        self.style.grow = weight;
        self
    }
    /// Never shrinks below its content.
    pub fn shrink_0(mut self) -> Self {
        self.style.shrink = 0.0;
        self
    }

    /// Takes the element out of the flow, placed against its parent's edges with `top()`,
    /// `left()` and friends.
    pub fn absolute(mut self) -> Self {
        self.style.absolute.get_or_insert([None; 4]);
        self
    }
    pub fn top(mut self, px: f32) -> Self {
        self.style.absolute.get_or_insert([None; 4])[0] = Some(px);
        self
    }
    pub fn right(mut self, px: f32) -> Self {
        self.style.absolute.get_or_insert([None; 4])[1] = Some(px);
        self
    }
    pub fn bottom(mut self, px: f32) -> Self {
        self.style.absolute.get_or_insert([None; 4])[2] = Some(px);
        self
    }
    pub fn left(mut self, px: f32) -> Self {
        self.style.absolute.get_or_insert([None; 4])[3] = Some(px);
        self
    }

    // ---------------------------------------------------------------- paint

    pub fn bg(mut self, paint: impl Into<Paint>) -> Self {
        self.style.bg = Some(paint.into());
        self
    }
    /// A 1 px border in `BORDER` (or `.border_color(..)`). It takes up layout space, like CSS.
    pub fn border_1(mut self) -> Self {
        self.style.border = 1.0;
        self
    }
    pub fn border_color(mut self, paint: impl Into<Paint>) -> Self {
        self.style.border_color = paint.into();
        self
    }
    pub fn rounded(mut self, px: f32) -> Self {
        self.style.radius = Radius::Px(px);
        self
    }
    pub fn rounded_sm(mut self) -> Self {
        self.style.radius = Radius::Sm;
        self
    }
    pub fn rounded_md(mut self) -> Self {
        self.style.radius = Radius::Md;
        self
    }
    pub fn rounded_lg(mut self) -> Self {
        self.style.radius = Radius::Lg;
        self
    }
    pub fn rounded_full(mut self) -> Self {
        self.style.radius = Radius::Full;
        self
    }
    /// The overlay shadow. For floating surfaces only (ADL-V3 3.3).
    pub fn shadow(mut self) -> Self {
        self.style.shadow = true;
        self
    }
    /// Glass: shadow, blurred backdrop, and the background tinted at the theme's glass alpha
    /// (ADL-V3 3.5). Only on a floating layer; on layer 0 it draws opaque.
    pub fn glass(mut self) -> Self {
        self.style.glass = true;
        self
    }
    pub fn opacity(mut self, a: f32) -> Self {
        self.style.opacity = a.clamp(0.0, 1.0);
        self
    }
    /// Clips children to this element's box.
    pub fn overflow_hidden(mut self) -> Self {
        self.style.clip = true;
        self
    }

    // ---------------------------------------------------------------- text and icons

    /// Text color: on a text or icon, its own; on a div, inherited by text and icons inside.
    pub fn color(mut self, paint: impl Into<Paint>) -> Self {
        let paint = paint.into();
        match &mut self.kind {
            Kind::Text { look, .. } => look.color = Some(paint),
            Kind::Icon { color, .. } => *color = Some(paint),
            _ => self.style.text_color = Some(paint),
        }
        self
    }

    /// Text size in logical px: on a text, its own; on an icon, its size; on a div, inherited.
    pub fn text_size(mut self, px: f32) -> Self {
        match &mut self.kind {
            Kind::Text { look, .. } => look.size = Some(px),
            Kind::Icon { size, .. } => *size = px,
            _ => self.style.text_size = Some(px),
        }
        self
    }

    fn look(mut self, f: impl FnOnce(&mut TextLook)) -> Self {
        if let Kind::Text { look, .. } = &mut self.kind {
            f(look);
        }
        self
    }

    pub fn medium(self) -> Self {
        self.look(|l| l.weight = WEIGHT_MEDIUM)
    }
    pub fn semibold(self) -> Self {
        self.look(|l| l.weight = WEIGHT_SEMIBOLD)
    }
    pub fn italic(self) -> Self {
        self.look(|l| l.italic = true)
    }
    pub fn mono(self) -> Self {
        self.look(|l| l.role = FontRole::Mono)
    }
    /// One line, ellipsized to fit.
    pub fn nowrap(self) -> Self {
        self.look(|l| l.nowrap = true)
    }
    /// Paragraph text: 15 px Noto Sans on a 1.5 line (ADL-V3 2.3).
    pub fn body(self) -> Self {
        self.look(|l| {
            l.role = FontRole::Body;
            l.size = Some(15.0);
        })
    }
    /// A card title: 20 px Medium.
    pub fn heading(self) -> Self {
        self.look(|l| {
            l.size = Some(20.0);
            l.weight = WEIGHT_MEDIUM;
        })
    }
    /// A screen title: 26 px SemiBold.
    pub fn title(self) -> Self {
        self.look(|l| {
            l.size = Some(26.0);
            l.weight = WEIGHT_SEMIBOLD;
        })
    }
    /// A caption: 13 px in `MUTED` unless colored.
    pub fn caption(self) -> Self {
        self.look(|l| {
            l.size = Some(13.0);
            l.color.get_or_insert(Paint::Theme(Tok::Muted));
        })
    }
    /// A small all-caps field label: 13 px Medium, `MUTED` (ADL 2.3).
    pub fn label(self) -> Self {
        self.look(|l| {
            l.size = Some(13.0);
            l.weight = WEIGHT_MEDIUM;
            l.color.get_or_insert(Paint::Theme(Tok::Muted));
        })
    }

    /// How a picture meets its box.
    pub fn fit(mut self, f: Fit) -> Self {
        if let Kind::Img { fit, .. } = &mut self.kind {
            *fit = f;
        }
        self
    }
}
