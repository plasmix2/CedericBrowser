//! Laying out and painting an element tree: build a Taffy tree (physical px), measure text and
//! pictures for it, read back every element's rectangle, run the interaction pass
//! (`ui::elements`), then paint in tree order into the frame's `DrawList`.

use taffy::{AvailableSpace, LayoutInput, LayoutOutput, NodeId, TaffyTree, TraversePartialTree};

use super::interact::{Enter, StateStyle};
use super::style::{Display, Length, Paint};
use super::{Element, Kind};
use crate::draw::{RectStyle, TextCmd};
use crate::geom::Rect;
use crate::text::FontRole;
use crate::theme::{lerp, motion, with_alpha, Color};
use crate::ui::elements::{extract, Meta, ACTIVE, DISABLED, FOCUS, HOVER};
use crate::ui::{Ui, CH_FOCUS, CH_HOVER, CH_PRESS, CH_SLIDE};
use crate::ElementId;
use super::{fill_lists, lower_lists};

/// A pixel of slack for text, in layout and in paint alike, so a line that fits never wraps on
/// rounding: Taffy rounds both edges of a box to whole pixels, which can take almost a pixel
/// off its width.
const TEXT_SLACK: f32 = 1.0;

/// Where every element landed, in tree order (parent before children), physical px, after
/// scrolling.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LaidOut {
    pub rects: Vec<Rect>,
}

/// What the measure function needs for one leaf, with inherited text settings resolved.
enum Measure<'e> {
    Text { content: &'e str, px: f32, weight: u16, role: FontRole, italic: bool, nowrap: bool },
    Square(f32),
    Size(f32, f32),
    Empty,
}

/// Per element, in tree order: what to measure, and its text size in physical px with
/// inheritance resolved.
struct Node<'e> {
    measure: Measure<'e>,
    px: f32,
}

/// What painting reads per element, in tree order.
struct PaintCx<'m, 'a> {
    rects: &'m [Rect],
    sizes: &'m [f32],
    metas: &'m [Meta<'a>],
    states: &'m [u8],
}

impl Ui {
    /// Lays out `el` inside `area` (physical px), runs this frame's input through it, and
    /// paints it. The root fills `area`'s width; its height follows its content unless styled.
    /// Actions it emitted wait in [`Ui::take_actions`]. Returns where everything landed.
    pub fn element(&mut self, area: Rect, mut el: Element) -> LaidOut {
        let base = ElementId::new("anvil.element").key(self.element_calls);
        self.element_calls += 1;
        let layer = el.interact.as_ref().map_or(0, |x| x.layer);
        self.expand_lists(area, &mut el);
        let mut metas = Vec::new();
        extract(&mut el, base, usize::MAX, &mut metas);
        self.hide_exited(&mut el, &mut 0, &metas);
        let (mut laid, sizes) = self.layout_resolved(area, &el);
        let states = self.interact(&mut metas, &mut laid.rects, layer);
        let cx = PaintCx { rects: &laid.rects, sizes: &sizes, metas: &metas, states: &states };
        let mut index = 0;
        let text = self.theme.text;
        // A raised tree draws on its own layer, so its glass has something to blur.
        let drawn_on = self.draw.current;
        if layer > 0 {
            self.set_layer((layer as usize).min(crate::draw::LAYER_COUNT - 1));
        }
        self.paint_element(el, &cx, &mut index, text, 0.0, 0.0);
        self.set_layer(drawn_on);
        laid
    }

    /// Turns each `uniform_list` into a scroll container holding the rows in view.
    fn expand_lists(&mut self, area: Rect, el: &mut Element) {
        let mut jobs = Vec::new();
        lower_lists(el, &mut 0, &mut jobs);
        if jobs.is_empty() {
            return;
        }
        let rects = self.layout_resolved(area, el).0.rects;
        let scale = self.scale;
        fill_lists(el, &mut 0, &mut jobs.into_iter().peekable(), &rects, scale, &|id| self.scroll_state(id).0);
    }

    /// Takes elements that have faded out (`Element::presence`) out of the layout.
    fn hide_exited(&self, el: &mut Element, index: &mut usize, metas: &[Meta]) {
        let i = *index;
        *index += 1;
        if let Some((_, false)) = metas[i].presence {
            if self.presence_value(metas[i].id).is_none_or(|p| p <= 0.002) {
                el.style.display = Display::None;
            }
        }
        for child in &mut el.children {
            self.hide_exited(child, index, metas);
        }
    }

    /// Lays out `el` inside `area` without painting it or giving it input.
    pub fn layout_element(&mut self, area: Rect, el: &Element) -> LaidOut {
        self.layout_resolved(area, el).0
    }

    fn layout_resolved(&mut self, area: Rect, el: &Element) -> (LaidOut, Vec<f32>) {
        let scale = self.scale;
        let mut tree = std::mem::replace(&mut self.layout_tree, TaffyTree::new());
        tree.clear();
        let mut nodes: Vec<Node> = Vec::new();
        let mut ids: Vec<NodeId> = Vec::new();
        let root = build(&mut tree, el, scale, 15.0, &mut nodes, &mut ids, true, false);

        let text = &mut self.text;
        // Taffy asks a text leaf for its size several times per pass (min-content, max-content,
        // final width). Its one-line width and widest word are found once and reused; only a
        // width that forces wrapping shapes the text again.
        let mut one_line_w: Vec<Option<f32>> = vec![None; nodes.len()];
        let mut widest_word: Vec<Option<f32>> = vec![None; nodes.len()];
        // Taffy's leaf layout applies the style (fixed sizes, min and max, padding, border); the
        // closure inside it only sizes the content.
        let measure = |input: LayoutInput, _: NodeId, ctx: Option<&mut usize>, style: &taffy::Style| -> LayoutOutput {
            let Some(&mut i) = ctx else { return LayoutOutput::HIDDEN };
            taffy::compute_leaf_layout(input, style, |_, _| 0.0, |known, space| match &nodes[i].measure {
                Measure::Empty => taffy::Size { width: 0.0, height: 0.0 },
                Measure::Square(s) => taffy::Size { width: *s, height: *s },
                Measure::Size(w, h) => taffy::Size { width: *w, height: *h },
                Measure::Text { content, px, weight, role, italic, nowrap } => {
                    let line_h = px * role.line_height();
                    let width_of = |text: &mut crate::text::TextSystem, s: &str| {
                        let k = text.layout_rich(s, *px, *weight, *role, *italic, None, &[]);
                        text.width(&k)
                    };
                    let full = *one_line_w[i].get_or_insert_with(|| width_of(text, content));
                    let avail = known.width.or(match space.width {
                        AvailableSpace::Definite(w) => Some(w),
                        _ => None,
                    });
                    let min_content = matches!(space.width, AvailableSpace::MinContent) && known.width.is_none();
                    if *nowrap {
                        let w = if min_content { 0.0 } else { avail.map_or(full, |a| full.min(a)) };
                        taffy::Size { width: w, height: line_h }
                    } else if min_content {
                        let widest = *widest_word[i]
                            .get_or_insert_with(|| content.split_whitespace().map(|word| width_of(text, word)).fold(0.0, f32::max));
                        let k = text.layout_rich(content, *px, *weight, *role, *italic, Some(widest.max(1.0)), &[]);
                        taffy::Size { width: widest, height: text.lines(&k) as f32 * line_h }
                    } else if avail.is_none_or(|a| full <= a + TEXT_SLACK) {
                        // It fits on one line: no wrapping to work out.
                        taffy::Size { width: full, height: line_h * content.lines().count().max(1) as f32 }
                    } else {
                        let k = text.layout_rich(content, *px, *weight, *role, *italic, avail.map(|a| a + TEXT_SLACK), &[]);
                        let a = avail.unwrap_or(full);
                        taffy::Size { width: text.width(&k).min(a), height: text.lines(&k) as f32 * line_h }
                    }
                }
            })
        };
        let space = taffy::Size { width: AvailableSpace::Definite(area.w()), height: AvailableSpace::Definite(area.h()) };
        let ok = tree.compute_layout_with_measure(root, space, measure).is_ok();

        let mut rects = Vec::with_capacity(ids.len());
        if ok {
            collect(&tree, root, area.x(), area.y(), &mut rects);
        } else {
            rects.resize(ids.len(), Rect::new(area.x(), area.y(), 0.0, 0.0));
        }
        let sizes = nodes.iter().map(|n| n.px).collect();
        self.layout_tree = tree;
        (LaidOut { rects }, sizes)
    }

    /// Paints `el` and its children, consuming the tree in the same order layout numbered it.
    /// `inherited` is the text color from above; `dy` moves the subtree (an entering parent).
    fn paint_element(&mut self, el: Element, cx: &PaintCx, index: &mut usize, inherited: Color, dx: f32, dy: f32) {
        let i = *index;
        *index += 1;
        let theme = self.theme;
        let s = &el.style;
        let meta = &cx.metas[i];
        let ix = meta.ix.as_deref();
        let state = cx.states[i];

        // How far each state style is blended in: hovered, pressed, focused.
        let mut blend = [0.0f32; 3];
        let mut styles: [Option<StateStyle>; 3] = [None; 3];
        if let Some(ix) = ix {
            styles = [ix.hover, ix.active, ix.focus];
            let spring = ix.spring;
            let rate = ix.transition.map(motion::rate);
            for (k, (bit, channel)) in [(HOVER, CH_HOVER), (ACTIVE, CH_PRESS), (FOCUS, CH_FOCUS)].into_iter().enumerate() {
                let target = if state & bit != 0 { 1.0 } else { 0.0 };
                blend[k] = if let Some(sp) = spring {
                    self.spring(meta.id, channel, target, sp.stiffness, sp.damping)
                } else {
                    match rate {
                        Some(rate) => self.animate(meta.id, channel, target, rate),
                        None => target,
                    }
                };
            }
        }
        let mix = |base: Option<Color>, part: fn(&StateStyle) -> Option<Paint>| {
            let mut c = base;
            for (style, t) in styles.iter().zip(blend) {
                if let Some(to) = style.as_ref().and_then(part) {
                    let to = to.resolve(&theme);
                    if t > 0.0 {
                        c = Some(lerp(c.unwrap_or(with_alpha(to, 0.0)), to, t));
                    }
                }
            }
            c
        };
        let mut fill = mix(s.bg.map(|p| p.resolve(&theme)), |st| st.bg);
        let mut border = mix(Some(s.border_color.resolve(&theme)), |st| st.border_color).unwrap_or(theme.border);
        let own = match &el.kind {
            Kind::Text { look, .. } => look.color,
            Kind::Icon { color, .. } => *color,
            _ => s.text_color,
        };
        let mut color = mix(Some(own.map_or(inherited, |p| p.resolve(&theme))), |st| st.color).unwrap_or(inherited);
        let mut opacity = s.opacity;
        for (style, t) in styles.iter().zip(blend) {
            if let Some(o) = style.and_then(|st| st.opacity) {
                opacity += (o - opacity) * t;
            }
        }
        if state & DISABLED != 0 {
            fill = fill.map(|_| theme.disabled_bg);
            border = theme.border;
            color = theme.disabled;
        }

        let mut scale = 1.0f32;
        let mut y_shift = 0.0f32;
        for (style, t) in styles.iter().zip(blend) {
            if let Some(st) = style {
                if let Some(sc) = st.scale {
                    scale += (sc - 1.0) * t;
                }
                if let Some(yo) = st.y_offset {
                    y_shift += yo * self.scale * t;
                }
            }
        }

        let slide = match ix.and_then(|x| x.slide) {
            Some(x) => self.spring(meta.id, CH_SLIDE, x * self.scale, 380.0, 30.0),
            None => 0.0,
        };
        let mut dy = dy + y_shift;
        let dx = dx + slide;
        if let Some((enter, shown)) = ix.and_then(|x| x.presence) {
            if self.presence_value(meta.id).is_none() {
                self.set_presence(meta.id, shown);
            }
            let p = self.presence(meta.id, shown, motion::BASE, motion::FAST);
            opacity *= p;
            if enter == Enter::Rise {
                dy += (1.0 - p) * self.s(8.0);
            } else if enter == Enter::Pop || enter == Enter::Scale {
                scale *= p;
            }
        }
        if let Some((enter, at)) = ix.and_then(|x| x.enter) {
            let p = self.entrance(meta.id, at);
            opacity *= p;
            if enter == Enter::Rise {
                dy += (1.0 - p) * self.s(8.0);
            } else if enter == Enter::Pop || enter == Enter::Scale {
                scale *= p;
            }
        }
        let base_r = cx.rects[i].offset(dx, dy);
        let r = if (scale - 1.0).abs() > 0.001 {
            let center = base_r.center();
            let w = base_r.w() * scale;
            let h = base_r.h() * scale;
            Rect::new(center.x - w * 0.5, center.y - h * 0.5, w, h)
        } else {
            base_r
        };

        let prev_opacity = self.set_opacity(self.opacity() * opacity);
        let radius = self.s(s.radius.resolve(&theme));
        let line = self.s(1.0).max(1.0);

        if s.glass {
            self.glass(r, radius, fill.unwrap_or(theme.card));
        } else {
            if s.shadow {
                self.shadow(r, radius);
            }
            if fill.is_some() || s.border > 0.0 {
                let mut style = RectStyle::fill(fill.unwrap_or([0.0; 4])).radius(radius);
                if s.border > 0.0 {
                    style = style.border(border, (s.border * self.scale).max(line));
                }
                self.rect(r, style);
            }
        }
        if meta.focusable && blend[2] > 0.01 {
            let prev = self.set_opacity(self.opacity() * blend[2]);
            self.focus_ring(r, radius);
            self.set_opacity(prev);
        }

        match el.kind {
            Kind::Div | Kind::List(_) => {}
            Kind::Text { content, look } => {
                let px = cx.sizes[i] * scale;
                let (weight, role, italic) = (look.weight, look.role, look.italic);
                if px > 0.5 {
                    if look.nowrap {
                        let style = crate::ui::TextStyle { size: px / self.scale, weight, color, role, italic };
                        let shown = self.fit_text(&content, style, r.w() + TEXT_SLACK);
                        let key = self.text.layout_rich(&shown, px, weight, role, italic, None, &[]);
                        self.draw.push_text(TextCmd { key, x: r.x(), y: r.y(), color, clip: self.screen_rect() });
                    } else {
                        // A pixel of slack, so a line that fits never wraps on rounding: Taffy rounds both
                        // edges of a box to whole pixels, which can take almost a pixel off its width.
                        let key = self.text.layout_rich(&content, px, weight, role, italic, Some(r.w() + TEXT_SLACK), &[]);
                        self.draw.push_text(TextCmd { key, x: r.x(), y: r.y(), color, clip: self.screen_rect() });
                    }
                }
            }
            Kind::Icon { icon, .. } => self.icon(r, icon, color),
            Kind::Img { texture, fit } => {
                let logical_radius = s.radius.resolve(&theme);
                self.image(r, texture, fit, logical_radius);
            }
            Kind::Canvas(draw) => draw(self, r),
        }

        let clip = meta.clip.then(|| self.push_element_clip(r));
        for child in el.children {
            self.paint_element(child, cx, index, color, dx, dy);
        }
        if s.scroll {
            self.element_scroll_thumb(meta.id, r);
        }
        if let Some(prev) = clip {
            self.pop_element_clip(prev);
        }
        self.set_opacity(prev_opacity);
    }
}

/// Adds `el` and its subtree to `tree` (pre-order into `nodes` and `ids`), resolving the
/// inherited text size. Returns its Taffy node.
#[allow(clippy::too_many_arguments)]
fn build<'e>(
    tree: &mut TaffyTree<usize>,
    el: &'e Element,
    scale: f32,
    text_size: f32,
    nodes: &mut Vec<Node<'e>>,
    ids: &mut Vec<NodeId>,
    root: bool,
    in_scroll: bool,
) -> NodeId {
    let mut style = el.style.to_taffy(scale);
    if root && el.style.width == Length::Auto {
        // The root fills the area it is given, like a block.
        style.size.width = taffy::Dimension::percent(1.0);
    }
    if in_scroll {
        // Scrolled content keeps its height instead of squeezing into the viewport.
        style.flex_shrink = 0.0;
    }
    let text_size = el.style.text_size.unwrap_or(text_size);
    let index = nodes.len();
    let mut px = text_size * scale;
    let measure = match &el.kind {
        Kind::Div | Kind::Canvas(_) | Kind::List(_) => Measure::Empty,
        Kind::Text { content, look } => {
            px = look.size.unwrap_or(text_size) * scale;
            Measure::Text { content, px, weight: look.weight, role: look.role, italic: look.italic, nowrap: look.nowrap }
        }
        Kind::Icon { size, .. } => Measure::Square(size * scale),
        Kind::Img { texture, .. } => Measure::Size(texture.width as f32 * scale, texture.height as f32 * scale),
    };
    nodes.push(Node { measure, px });
    ids.push(NodeId::from(0u64));
    let node = if el.children.is_empty() {
        tree.new_leaf_with_context(style, index).expect("taffy leaf")
    } else {
        let kids: Vec<NodeId> = el
            .children
            .iter()
            .map(|c| build(tree, c, scale, text_size, nodes, ids, false, el.style.scroll))
            .collect();
        let n = tree.new_with_children(style, &kids).expect("taffy node");
        tree.set_node_context(n, Some(index)).expect("taffy context");
        n
    };
    ids[index] = node;
    node
}

/// Absolute rectangles in pre-order. Taffy locations are relative to the parent's border box.
fn collect(tree: &TaffyTree<usize>, node: NodeId, ox: f32, oy: f32, out: &mut Vec<Rect>) {
    let l = tree.layout(node).expect("laid out");
    let (x, y) = (ox + l.location.x, oy + l.location.y);
    out.push(Rect::new(x, y, l.size.width, l.size.height));
    // An iterator: `children()` would allocate a Vec at every node.
    for child in tree.child_ids(node) {
        collect(tree, child, x, y, out);
    }
}
