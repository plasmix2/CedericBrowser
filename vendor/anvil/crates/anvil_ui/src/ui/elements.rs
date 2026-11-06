//! The interaction pass of an element tree (ANVIL-UI-2.md, Phase 2): scroll containers, hit
//! testing, then pointer, gesture and keyboard dispatch. `Ui::element` runs it between layout and
//! paint, so input acts on where elements are this frame.


use super::{Cursor, Ui, UiKey};
use crate::draw::RectStyle;
use crate::theme::with_alpha;
use crate::element::{Action, Element, Enter, Interact, Pointer};
use crate::gesture::{Axis, Gesture, Phase};
use crate::geom::Rect;
use crate::ElementId;

/// No parent: the root.
const ROOT: usize = usize::MAX;

/// Element states found by the pass, for painting.
pub(crate) const HOVER: u8 = 1;
pub(crate) const ACTIVE: u8 = 2;
pub(crate) const FOCUS: u8 = 4;
/// Disabled itself or inside a disabled element.
pub(crate) const DISABLED: u8 = 8;

/// One element of a tree, in tree order, with its handlers taken out of the tree.
pub(crate) struct Meta<'a> {
    pub id: ElementId,
    pub parent: usize,
    /// One past the last index of the element's subtree.
    pub end: usize,
    pub ix: Option<Box<Interact<'a>>>,
    pub focusable: bool,
    /// Children are clipped to the element (overflow hidden, or a scroll container).
    pub clip: bool,
    pub scroll: bool,
    /// Bottom padding, logical px: the room a scroll container keeps after its content.
    pub pad_bottom: f32,
    /// Shown or leaving, from `Element::presence`.
    pub presence: Option<(Enter, bool)>,
    /// Leaving or already gone (itself or an ancestor): takes no pointer or keyboard input.
    pub inert: bool,
}

/// Numbers the tree in the order layout does and moves each element's handlers into `out`. An
/// element without an id is keyed by `id`, its parent's id and its place among its siblings.
pub(crate) fn extract<'a>(el: &mut Element<'a>, id: ElementId, parent: usize, out: &mut Vec<Meta<'a>>) {
    let index = out.len();
    let id = el.id.unwrap_or(id);
    let ix = el.interact.take();
    let presence = ix.as_ref().and_then(|x| x.presence);
    let inert = presence.is_some_and(|(_, shown)| !shown) || (parent != ROOT && out[parent].inert);
    out.push(Meta {
        id,
        parent,
        end: 0,
        focusable: ix.as_ref().is_some_and(|x| x.is_focusable() || (x.pinned & FOCUS != 0 && !x.disabled)),
        presence,
        inert,
        ix,
        clip: el.style.clip || el.style.scroll,
        scroll: el.style.scroll,
        pad_bottom: el.style.padding.bottom,
    });
    for (k, child) in el.children.iter_mut().enumerate() {
        extract(child, id.key(k), index, out);
    }
    out[index].end = out.len();
}

/// `from`, then each of its ancestors up to the root.
fn ancestors(metas: &[Meta], from: usize) -> Vec<usize> {
    std::iter::successors(Some(from), |&i| (metas[i].parent != ROOT).then_some(metas[i].parent)).collect()
}

fn shift(rects: &mut [Rect], range: std::ops::Range<usize>, dy: f32) {
    if dy != 0.0 {
        for r in &mut rects[range] {
            *r = r.offset(0.0, dy);
        }
    }
}

/// `r` grown around its center to at least `min` each way.
fn at_least(r: Rect, min: f32) -> Rect {
    let (w, h) = (r.w().max(min), r.h().max(min));
    let c = r.center();
    Rect::new(c.x - w * 0.5, c.y - h * 0.5, w, h)
}

/// Sends an event along `path`, target first. The first enabled element whose handler `pick`
/// finds emits; the event goes on past it only if it bubbles. A disabled element, or one that
/// stops propagation, ends the walk. Returns whether anything was emitted.
fn bubble<'a>(
    metas: &mut [Meta<'a>],
    states: &[u8],
    path: &[usize],
    out: &mut Vec<Action>,
    mut pick: impl FnMut(&mut Interact<'a>) -> Option<Action>,
) -> bool {
    let mut emitted = false;
    for &i in path {
        if states[i] & DISABLED != 0 {
            break;
        }
        let Some(ix) = metas[i].ix.as_deref_mut() else { continue };
        if let Some(action) = pick(ix) {
            out.push(action);
            emitted = true;
            if !ix.bubble {
                break;
            }
        }
        if ix.stop {
            break;
        }
    }
    emitted
}

impl Ui {
    /// Runs scrolling and dispatch for a laid-out tree. Moves `rects` by scroll offsets and
    /// returns each element's state bits.
    pub(crate) fn interact(&mut self, metas: &mut [Meta], rects: &mut [Rect], layer: u8) -> Vec<u8> {
        let n = metas.len();
        let m = self.input.mouse;
        self.el_covered = self.el_covers.iter().chain(&self.el_covers_prev).any(|(l, r)| *l > layer && r.contains(m));
        let mut states = vec![0u8; n];
        for i in 0..n {
            let own = metas[i].ix.as_ref().is_some_and(|x| x.disabled);
            let parent = metas[i].parent;
            if own || (parent != ROOT && states[parent] & DISABLED != 0) {
                states[i] |= DISABLED;
            }
        }
        self.scroll_containers(metas, rects);
        let path = self.hit(metas, rects).map_or(Vec::new(), |t| ancestors(metas, t));
        for &i in &path {
            if states[i] & DISABLED == 0 {
                states[i] |= HOVER;
            }
        }
        self.hover_events(metas, &states);
        self.pointer_events(metas, rects, &path, &mut states);
        self.keyboard(metas, rects, &mut states, layer);
        for (i, meta) in metas.iter().enumerate() {
            if let Some(ix) = meta.ix.as_deref() {
                if states[i] & DISABLED == 0 {
                    states[i] |= ix.pinned;
                }
            }
        }
        if layer > 0 {
            self.el_covers.push((layer, rects[0]));
        }
        states
    }

    /// Hover enter and leave actions: an element with such a handler that is hovered now but was
    /// not last frame has been entered, and one hovered last frame but not now has been left.
    fn hover_events(&mut self, metas: &mut [Meta], states: &[u8]) {
        for (i, meta) in metas.iter_mut().enumerate() {
            let Some(ix) = meta.ix.as_deref_mut() else { continue };
            if ix.on_hover_enter.is_none() && ix.on_hover_leave.is_none() {
                continue;
            }
            let now = states[i] & HOVER != 0;
            let was = self.el_hover_prev.contains(&meta.id);
            if now {
                self.el_hover_now.insert(meta.id);
            }
            let action = match (was, now) {
                (false, true) => ix.on_hover_enter.take(),
                (true, false) => ix.on_hover_leave.take(),
                _ => None,
            };
            self.actions.extend(action);
        }
    }

    /// Scrolls every scroll container and moves its content to match. Only the innermost
    /// container under the pointer takes the wheel or a press.
    fn scroll_containers(&mut self, metas: &[Meta], rects: &mut [Rect]) {
        let scrolls: Vec<usize> = (0..metas.len()).filter(|&i| metas[i].scroll).collect();
        if scrolls.is_empty() {
            return;
        }
        // A touch press that began on something that takes drags is not a scroll.
        self.scroll_blocked = self.input.touch
            && self.el_press.iter().filter_map(|id| metas.iter().position(|m| m.id == *id)).any(|p| {
                metas[p].ix.as_deref().is_some_and(|x| x.on_drag.is_some() && x.drag_axis != Some(Axis::X))
            });
        // Content goes where last frame left it first, so the pointer finds what it is over.
        let mut old = Vec::with_capacity(scrolls.len());
        for &i in &scrolls {
            let (offset, _) = self.scroll_state(metas[i].id);
            old.push(offset);
            shift(rects, i + 1..metas[i].end, -offset);
        }
        let mouse = self.input.mouse;
        let owner = self.hit(metas, rects).and_then(|t| ancestors(metas, t).into_iter().find(|&i| metas[i].scroll));
        for (k, &i) in scrolls.iter().enumerate() {
            let viewport = rects[i];
            // The content's height. A container nested inside counts as its own box.
            let mut bottom = viewport.y();
            let mut j = i + 1;
            while j < metas[i].end {
                bottom = bottom.max(rects[j].bottom());
                j = if metas[j].scroll { metas[j].end } else { j + 1 };
            }
            let extent = bottom + old[k] - viewport.y() + self.s(metas[i].pad_bottom);
            self.set_scroll_max(metas[i].id, extent - viewport.h());
            let owns = owner == Some(i) && viewport.contains(mouse);
            let offset = self.scroll_physics(metas[i].id, viewport, owns);
            shift(rects, i + 1..metas[i].end, old[k] - offset);
        }
    }

    /// The topmost element under the pointer: the last in paint order whose box holds it, inside
    /// every clip above it. On touch, anything with a pointer handler is at least 48 dp. The
    /// immediate-mode gates (scroll areas, modals, popups) apply as they do to widgets.
    fn hit(&self, metas: &[Meta], rects: &[Rect]) -> Option<usize> {
        if self.el_covered {
            return None;
        }
        let m = self.input.mouse;
        let min = if self.input.touch { self.s(48.0) } else { 0.0 };
        let mut clip_in: Vec<Option<Rect>> = Vec::with_capacity(metas.len());
        let mut target = None;
        for (i, meta) in metas.iter().enumerate() {
            let clip = match meta.parent {
                ROOT => None,
                p if metas[p].clip => Some(clip_in[p].map_or(rects[p], |c| c.intersect(rects[p]))),
                p => clip_in[p],
            };
            clip_in.push(clip);
            let mut r = rects[i];
            if min > 0.0 && meta.ix.as_ref().is_some_and(|x| x.takes_pointer()) {
                r = at_least(r, min);
            }
            let through = meta.ix.as_deref().is_some_and(|x| x.passthrough);
            if !meta.inert && !through && r.contains(m) && clip.is_none_or(|c| c.contains(m)) {
                target = Some((i, r));
            }
        }
        let (i, r) = target?;
        self.hovered(r).then_some(i)
    }

    fn pointer_events(&mut self, metas: &mut [Meta], rects: &[Rect], path: &[usize], states: &mut [u8]) {
        let has = |metas: &[Meta], i: usize, f: fn(&Interact) -> bool| metas[i].ix.as_deref().is_some_and(f);
        let click_target = path.iter().copied().find(|&i| has(metas, i, |x| x.on_click.is_some()));
        if let Some(i) = click_target {
            self.want_cursor(if states[i] & DISABLED != 0 { Cursor::NotAllowed } else { Cursor::Pointer });
        }
        if self.input.pressed {
            if !path.is_empty() {
                self.el_press = path.iter().map(|&i| metas[i].id).collect();
            }
            // A press focuses what it lands on (without the ring), so Tab goes on from there.
            if let Some(&i) = path.iter().find(|&&i| states[i] & DISABLED == 0 && metas[i].focusable) {
                self.kb_focus = Some(metas[i].id);
            }
            if let Some(i) = click_target.filter(|&i| states[i] & DISABLED == 0) {
                self.active = Some(metas[i].id);
            }
            for i in 0..metas.len() {
                if path.contains(&i) || states[i] & DISABLED != 0 {
                    continue;
                }
                if let Some(action) = metas[i].ix.as_mut().and_then(|x| x.on_click_outside.take()) {
                    self.actions.push(action);
                }
            }
        }
        if self.el_press.is_empty() {
            return;
        }

        // Every element the press started on, wherever the pointer is now.
        let pressed: Vec<usize> =
            self.el_press.iter().filter_map(|id| metas.iter().position(|m| m.id == *id)).collect();
        let m = self.input.mouse;
        let held: Vec<usize> = path.iter().copied().filter(|&i| self.el_press.contains(&metas[i].id)).collect();
        // An element that reads the pointer's position follows it from the press to the release.
        let phase = if self.input.pressed {
            Some(Phase::Start)
        } else if self.input.released {
            Some(Phase::End)
        } else if self.input.down && m != self.mouse_last {
            Some(Phase::Move)
        } else {
            None
        };
        if let Some(p) = pressed.iter().copied().find(|&p| states[p] & DISABLED == 0 && metas[p].ix.as_deref().is_some_and(|x| x.on_pointer.is_some())) {
            let r = rects[p];
            if let Some(phase) = phase {
                let frac = |v: f32, lo: f32, size: f32| if size > 0.0 { ((v - lo) / size).clamp(0.0, 1.0) } else { 0.0 };
                let event = Pointer { phase, x: frac(m.x, r.x(), r.w()), y: frac(m.y, r.y(), r.h()), rect: r, pos: m };
                if let Some(f) = metas[p].ix.as_deref_mut().and_then(|x| x.on_pointer.as_mut()) {
                    self.actions.push(f(event));
                }
            }
            self.el_claimed = true;
            if self.input.down {
                states[p] |= ACTIVE;
            }
        }
        // A press that scrolled a container never also clicks, holds or drags what it began on.
        let scrolled = pressed.iter().any(|&i| metas[i].scroll && self.scroll_state(metas[i].id).1);

        let gestures = std::mem::take(&mut self.gestures);
        let touch = self.input.touch;
        let mut dragged = false;
        for g in &gestures {
            match *g {
                Gesture::LongPress(_) if !scrolled => {
                    if bubble(metas, states, &held, &mut self.actions, |x| x.on_long_press.take()) {
                        self.el_claimed = true;
                    }
                }
                Gesture::Drag(d) if !scrolled => {
                    dragged |= d.phase == Phase::End;
                    let pick = |x: &mut Interact| match x.drag_axis {
                        Some(a) if a != d.axis => None,
                        _ => x.on_drag.as_mut().map(|f| f(d)),
                    };
                    if bubble(metas, states, &pressed, &mut self.actions, pick) {
                        self.el_claimed = true;
                    }
                }
                Gesture::Swipe { dir, .. } if !scrolled => {
                    if bubble(metas, states, &pressed, &mut self.actions, |x| x.on_swipe.as_mut().map(|f| f(dir))) {
                        self.el_claimed = true;
                    }
                }
                _ => {}
            }
        }
        let cancelled = scrolled || self.el_claimed;
        if self.input.down && !cancelled {
            for &i in &held {
                if states[i] & DISABLED == 0 {
                    states[i] |= ACTIVE;
                }
            }
        }
        // A click is a release over an element the press started on.
        if self.input.released && !cancelled && !(touch && dragged) {
            bubble(metas, states, &held, &mut self.actions, |x| x.on_click.take());
            self.el_pulls.clear();
        }
        // After its click: the second tap clicks too.
        if !cancelled && gestures.iter().any(|g| matches!(g, Gesture::DoubleTap(_))) {
            bubble(metas, states, &held, &mut self.actions, |x| x.on_double_click.take());
        }
        self.gestures = gestures;
        if let Some(due) = self.gesture.long_press_due(self.time) {
            if held.iter().any(|&i| has(metas, i, |x| x.on_long_press.is_some())) {
                self.request_wake(due + 0.01);
            }
        }
    }

    /// Tab order, Enter and Space on the focused element, and Escape. While a modal is open,
    /// only elements inside it join the Tab order, as only they take the pointer.
    fn keyboard(&mut self, metas: &mut [Meta], rects: &[Rect], states: &mut [u8], layer: u8) {
        let mut focused = None;
        let modal = self.modal_prev;
        // Under a raised tree (a sheet, a menu) the Tab order belongs to the tree above.
        let under_layer = self.el_covers.iter().chain(&self.el_covers_prev).any(|(l, _)| *l > layer);
        for i in 0..metas.len() {
            let outside_modal = modal.is_some_and(|m| !m.contains(rects[i].center()));
            if metas[i].focusable && !metas[i].inert && !under_layer && states[i] & DISABLED == 0 && !outside_modal {
                let id = metas[i].id;
                self.register_focus(id, false);
                if self.kb_focused(id) {
                    states[i] |= FOCUS;
                    focused = Some(i);
                }
            }
        }
        if let Some(f) = focused {
            // Keys the element or something around it listens for.
            let pressed_keys = self.input.keys.clone();
            for key in pressed_keys {
                for i in ancestors(metas, f) {
                    let Some(ix) = metas[i].ix.as_deref_mut() else { continue };
                    if let Some(at) = ix.on_keys.iter().position(|(k, _)| *k == key) {
                        self.actions.push(ix.on_keys.remove(at).1);
                        break;
                    }
                }
            }
            let (held, clicked) = self.key_activation(metas[f].id, true);
            if held {
                states[f] |= ACTIVE;
            }
            if clicked {
                bubble(metas, states, &ancestors(metas, f), &mut self.actions, |x| x.on_click.take());
            }
        }
        // Behind an open modal, Escape belongs to the modal.
        let behind_modal = self.modal_prev.is_some() && self.modal_scoped_prev && !self.in_modal;
        if !self.escape_taken && !behind_modal && self.input.keys.contains(&UiKey::Escape) {
            // Escape goes to the nearest handler around focus, or else to the innermost one.
            let mut done = focused.is_some_and(|f| {
                bubble(metas, states, &ancestors(metas, f), &mut self.actions, |x| x.on_escape.take())
            });
            if !done {
                let deepest = (0..metas.len())
                    .rev()
                    .find(|&i| states[i] & DISABLED == 0 && metas[i].ix.as_ref().is_some_and(|x| x.on_escape.is_some()));
                if let Some(d) = deepest {
                    done = bubble(metas, states, &ancestors(metas, d), &mut self.actions, |x| x.on_escape.take());
                }
            }
            self.escape_taken |= done;
        }
    }

    /// The scroll thumb of an element scroll container, drawn like [`Ui::scroll_end`]'s.
    pub(crate) fn element_scroll_thumb(&mut self, id: ElementId, vp: Rect) {
        let Some(state) = self.scrolls.get(&id) else { return };
        let (offset, max) = (state.offset, state.max);
        if max <= 1.0 || vp.h() <= 24.0 {
            return;
        }
        let (track_pad, bar_w, min_thumb) = (self.s(4.0), self.s(4.0), self.s(24.0));
        let track_h = (vp.h() - track_pad * 2.0).max(16.0);
        let thumb_h = (vp.h() / (max + vp.h()) * track_h).max(min_thumb).min(track_h);
        let y = vp.y() + track_pad + (offset / max).clamp(0.0, 1.0) * (track_h - thumb_h);
        let thumb = Rect::new(vp.right() - bar_w - track_pad, y, bar_w, thumb_h);
        self.rect(thumb, RectStyle::fill(with_alpha(self.theme.muted, 0.45)).radius(thumb.w() * 0.5));
    }

    /// Clips drawing and the pointer to `r` for an element's children. Returns what to restore.
    pub(crate) fn push_element_clip(&mut self, r: Rect) -> Option<Rect> {
        let prev = self.clip;
        self.clip = Some(prev.map_or(r, |c| c.intersect(r)));
        self.push_clip(r);
        prev
    }

    pub(crate) fn pop_element_clip(&mut self, prev: Option<Rect>) {
        self.pop_clip();
        self.clip = prev;
    }
}
