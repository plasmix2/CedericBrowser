//! `uniform_list`: a vertical list that builds only the rows in view.
//!
//! A tree is rebuilt every frame, but the rows of a long list should not be. `Ui::element`
//! lowers each list to an ordinary scroll container holding one spacer as tall as all the rows
//! (so scrolling and the thumb work from the full height), lays that out once to learn the
//! viewport, then fills the spacer with the rows in view as absolutely placed children and
//! lays the tree out again. Scrolling is read from the previous frame, so a few rows are built
//! beyond each edge to cover a frame of movement.

use super::{div, Element, Kind};
use crate::geom::Rect;
use crate::ElementId;

/// Rows built beyond each edge of the viewport.
pub const OVERSCAN: usize = 3;

/// The rows come from a closure that returns `Element<'static>`, with the real lifetime erased
/// on the way in (`ListSpec::new`) and restored on the way out (`ListSpec::build`). Naming
/// `Element<'a>` in the closure's return type would make `Element` invariant in `'a`, and a
/// tree that holds a `'static` child could no longer sit in a shorter-lived parent.
pub struct ListSpec<'a> {
    pub count: usize,
    /// Logical px.
    pub row_h: f32,
    row: Box<dyn Fn(usize) -> Element<'static> + 'a>,
}

impl<'a> ListSpec<'a> {
    pub fn new(count: usize, row_h: f32, row: impl Fn(usize) -> Element<'a> + 'a) -> Self {
        Self {
            count,
            row_h,
            // SAFETY: only the lifetime parameter changes. The closure runs only through
            // `build`, which gives the element back its lifetime (shortened to that of the
            // spec itself), so no `'static` element is ever seen from outside.
            row: Box::new(move |i| unsafe { std::mem::transmute::<Element<'a>, Element<'static>>(row(i)) }),
        }
    }

    /// Builds row `i`.
    pub fn build(&self, i: usize) -> Element<'a> {
        // SAFETY: undoes the erasure in `new`; `'a` is the spec's own lifetime parameter, which
        // the captured data outlives.
        unsafe { std::mem::transmute::<Element<'static>, Element<'a>>((self.row)(i)) }
    }
}

/// A list found in a tree: where its node is in tree order after lowering, and what builds its
/// rows.
pub(crate) struct ListJob<'a> {
    index: usize,
    id: ElementId,
    spec: ListSpec<'a>,
}

/// Replaces every `Kind::List` below `el` with a scroll container holding an empty spacer,
/// remembering the list in `jobs` (in tree order). `index` counts nodes in tree order.
pub(crate) fn lower_lists<'a>(el: &mut Element<'a>, index: &mut usize, jobs: &mut Vec<ListJob<'a>>) {
    let i = *index;
    *index += 1;
    if matches!(el.kind, Kind::List(_)) {
        let Kind::List(spec) = std::mem::replace(&mut el.kind, Kind::Div) else { unreachable!() };
        let id = el.id.unwrap_or_else(|| ElementId::new("anvil.list").key(i));
        el.id = Some(id);
        el.style.scroll = true;
        el.children = vec![div().h(spec.count as f32 * spec.row_h)];
        *index += 1;
        jobs.push(ListJob { index: i, id, spec });
        return;
    }
    for child in &mut el.children {
        lower_lists(child, index, jobs);
    }
}

/// Builds the rows in view for each lowered list, given the first layout (`rects`, tree order)
/// and each list's scroll offset from last frame (`offset_of`, physical px).
pub(crate) fn fill_lists<'a>(
    el: &mut Element<'a>,
    index: &mut usize,
    jobs: &mut std::iter::Peekable<std::vec::IntoIter<ListJob<'a>>>,
    rects: &[Rect],
    scale: f32,
    offset_of: &dyn Fn(ElementId) -> f32,
) {
    let i = *index;
    *index += 1;
    if jobs.peek().is_some_and(|j| j.index == i) {
        let job = jobs.next().expect("peeked");
        *index += 1;
        let row_h = job.spec.row_h * scale;
        let viewport = rects[i].h();
        let offset = offset_of(job.id).max(0.0);
        let first = ((offset / row_h).floor() as usize).saturating_sub(OVERSCAN);
        let last = (((offset + viewport) / row_h).ceil() as usize + OVERSCAN).min(job.spec.count);
        let spacer = &mut el.children[0];
        for k in first..last.max(first) {
            let mut row = job.spec.build(k);
            if row.id.is_none() {
                row.id = Some(job.id.key(k));
            }
            spacer.children.push(row.absolute().top(k as f32 * job.spec.row_h).left(0.0).right(0.0).h(job.spec.row_h).shrink_0());
        }
        return;
    }
    for child in &mut el.children {
        fill_lists(child, index, jobs, rects, scale, offset_of);
    }
}
