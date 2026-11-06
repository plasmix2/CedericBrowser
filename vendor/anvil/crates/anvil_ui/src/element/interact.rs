//! What an element does when used (ANVIL-UI-2.md, Phase 2): the actions it emits, how it looks
//! while hovered, pressed or focused, and how it enters.
//!
//! An action is any `'static` value, usually the app's own message enum. `Ui::element` collects
//! the actions a frame's input caused and the app reads them after the frame:
//!
//! ```ignore
//! ui.element(area, div().p_3().rounded_md()
//!     .hover(|s| s.bg(Tok::CardHi)).transition(motion::INSTANT)
//!     .on_click(Msg::Open(id))
//!     .child(text(&note.title)));
//! for msg in ui.take_actions::<Msg>() { app.update(msg) }
//! ```
//!
//! An event goes to the topmost element under the pointer and then up through its ancestors.
//! The first one with a handler for it emits its action, and the event stops there unless that
//! element is `.bubble()`. `.stop_propagation()` stops events at an element without handlers,
//! and a disabled element swallows them.

use std::any::Any;

use super::style::Paint;
use super::Element;
use crate::geom::Rect;
use crate::gesture::{Axis, Dir, Drag, Phase};
use glam::Vec2;
use crate::ui::UiKey;

pub(crate) type Action = Box<dyn Any>;
pub(crate) type DragFn<'a> = Box<dyn FnMut(Drag) -> Action + 'a>;
pub(crate) type SwipeFn<'a> = Box<dyn FnMut(Dir) -> Action + 'a>;
pub(crate) type PointerFn<'a> = Box<dyn FnMut(Pointer) -> Action + 'a>;

/// The pointer inside one element while it is pressed, for controls that read a position:
/// sliders, color pickers, scrubbers. `x` and `y` are fractions (0 to 1) of the element's box,
/// clamped, so a drag that leaves the box keeps reporting its edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pointer {
    pub phase: Phase,
    pub x: f32,
    pub y: f32,
    /// The element's box in physical px and the pointer in window px, for anything finer.
    pub rect: Rect,
    pub pos: Vec2,
}

/// A state to hold an element in regardless of the pointer and keyboard: for showcases,
/// documentation screenshots and tests that must show every state at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pin {
    Hover,
    Pressed,
    Focus,
}

/// How an element leaves (and returns): see [`Element::presence`].
pub type Exit = Enter;

/// The paint an element takes on in a state (hovered, pressed, focused). Unset parts keep the
/// element's own. With `.transition(..)` the change blends over that duration.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StateStyle {
    pub bg: Option<Paint>,
    pub border_color: Option<Paint>,
    /// Text and icon color, inherited by the element's children like `.color(..)`.
    pub color: Option<Paint>,
    pub opacity: Option<f32>,
    pub scale: Option<f32>,
    pub y_offset: Option<f32>,
}

impl StateStyle {
    pub fn bg(mut self, paint: impl Into<Paint>) -> Self {
        self.bg = Some(paint.into());
        self
    }
    pub fn border_color(mut self, paint: impl Into<Paint>) -> Self {
        self.border_color = Some(paint.into());
        self
    }
    pub fn color(mut self, paint: impl Into<Paint>) -> Self {
        self.color = Some(paint.into());
        self
    }
    pub fn opacity(mut self, a: f32) -> Self {
        self.opacity = Some(a.clamp(0.0, 1.0));
        self
    }
    pub fn scale(mut self, s: f32) -> Self {
        self.scale = Some(s);
        self
    }
    pub fn y_offset(mut self, y: f32) -> Self {
        self.y_offset = Some(y);
        self
    }
}

/// How an element appears the first time it is shown (ADL-V3 4.4), over `motion::BASE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enter {
    /// Fades in.
    Fade,
    /// Fades in while rising 8 px into place.
    Rise,
    /// Grows smoothly from size 0 to full size with fluid ease.
    Pop,
    /// Scales smoothly from size 0 to full size.
    Scale,
}

/// Everything interactive about one element. Kept boxed and optional, so a plain element stays
/// small.
#[derive(Default)]
pub(crate) struct Interact<'a> {
    pub on_click: Option<Action>,
    pub on_double_click: Option<Action>,
    pub on_long_press: Option<Action>,
    pub on_click_outside: Option<Action>,
    pub on_escape: Option<Action>,
    pub on_drag: Option<DragFn<'a>>,
    pub on_swipe: Option<SwipeFn<'a>>,
    pub hover: Option<StateStyle>,
    pub active: Option<StateStyle>,
    pub focus: Option<StateStyle>,
    /// `None`: focusable when it has a click handler.
    pub focusable: Option<bool>,
    pub stop: bool,
    pub bubble: bool,
    pub disabled: bool,
    pub transition: Option<f32>,
    pub spring: Option<crate::theme::motion::Spring>,
    pub enter: Option<(Enter, usize)>,
    /// State bits held on (`ui::elements::HOVER` and friends).
    pub pinned: u8,
    pub on_hover_enter: Option<Action>,
    pub on_hover_leave: Option<Action>,
    pub on_pointer: Option<PointerFn<'a>>,
    /// Only drags locked to this axis go to `on_drag`; `None` takes any.
    pub drag_axis: Option<Axis>,
    /// On a tree's root: which tree is on top when trees overlap (`Element::layer`).
    pub layer: u8,
    /// Shown or leaving, and how it moves (`Element::presence`).
    pub presence: Option<(Enter, bool)>,
    /// Where the element rests, sideways from its laid-out place, logical px (`Element::slide`).
    pub slide: Option<f32>,
    /// Actions for keys pressed while focus is on the element or inside it.
    pub on_keys: Vec<(UiKey, Action)>,
    /// The pointer passes through to whatever is beneath (`Element::passthrough`).
    pub passthrough: bool,
}

impl Interact<'_> {
    /// True when the pointer can do something here, so the hit box grows to 48 dp on touch.
    pub fn takes_pointer(&self) -> bool {
        self.on_click.is_some()
            || self.on_double_click.is_some()
            || self.on_long_press.is_some()
            || self.on_drag.is_some()
            || self.on_swipe.is_some()
            || self.on_pointer.is_some()
    }

    pub fn is_focusable(&self) -> bool {
        !self.disabled && self.focusable.unwrap_or(self.on_click.is_some())
    }
}

impl<'a> Element<'a> {
    fn ix(&mut self) -> &mut Interact<'a> {
        self.interact.get_or_insert_with(Default::default)
    }

    /// `on_escape` with an action already boxed.
    pub(crate) fn on_escape_boxed(mut self, action: Action) -> Self {
        self.ix().on_escape = Some(action);
        self
    }

    /// `on_click_outside` with an action already boxed.
    pub(crate) fn on_click_outside_boxed(mut self, action: Action) -> Self {
        self.ix().on_click_outside = Some(action);
        self
    }

    /// `on_key` with an action already boxed.
    pub(crate) fn on_key_boxed(mut self, key: UiKey, action: Action) -> Self {
        let ix = self.ix();
        ix.on_keys.push((key, action));
        ix.focusable.get_or_insert(true);
        self
    }

    /// `on_pointer` for a handler that already returns boxed actions.
    pub(crate) fn on_pointer_raw(mut self, f: impl FnMut(Pointer) -> Action + 'a) -> Self {
        self.ix().on_pointer = Some(Box::new(f));
        self
    }

    /// `on_click` with an action already boxed, for components that store theirs until built.
    pub(crate) fn click_boxed(mut self, action: Action) -> Self {
        self.ix().on_click = Some(action);
        self
    }

    /// Emits `action` when the element is clicked or tapped (press and release inside it), or
    /// activated with Enter or Space while focused. Makes it focusable.
    pub fn on_click<A: 'static>(mut self, action: A) -> Self {
        self.ix().on_click = Some(Box::new(action));
        self
    }

    /// Emits `action` on the second of two quick taps or clicks. The first still clicks.
    pub fn on_double_click<A: 'static>(mut self, action: A) -> Self {
        self.ix().on_double_click = Some(Box::new(action));
        self
    }

    /// Emits `action` when a press is held still for half a second. The release does not click.
    pub fn on_long_press<A: 'static>(mut self, action: A) -> Self {
        self.ix().on_long_press = Some(Box::new(action));
        self
    }

    /// Emits `action` when a press lands anywhere outside the element: menus and popovers use
    /// it to close.
    pub fn on_click_outside<A: 'static>(mut self, action: A) -> Self {
        self.ix().on_click_outside = Some(Box::new(action));
        self
    }

    /// Emits `action` on Escape, when focus is inside the element or nothing nearer handles it.
    pub fn on_escape<A: 'static>(mut self, action: A) -> Self {
        self.ix().on_escape = Some(Box::new(action));
        self
    }

    /// Maps every step of a drag that started on the element to an action. The axis is locked
    /// once the press moves past the slop. A drag the element takes never clicks.
    pub fn on_drag<A: 'static>(mut self, mut f: impl FnMut(Drag) -> A + 'a) -> Self {
        self.ix().on_drag = Some(Box::new(move |d| Box::new(f(d)) as Action));
        self
    }

    /// Like [`Element::on_drag`], but only for drags that lock to `axis`. A drag along the
    /// other axis is left to whatever else wants it: a vertical `scroll()` container around a
    /// horizontally draggable row keeps scrolling, and the row ignores it. (Plain `on_drag`
    /// takes every drag, and a touch scroll container around it does not scroll.)
    pub fn on_drag_axis<A: 'static>(mut self, axis: Axis, mut f: impl FnMut(Drag) -> A + 'a) -> Self {
        let ix = self.ix();
        ix.on_drag = Some(Box::new(move |d| Box::new(f(d)) as Action));
        ix.drag_axis = Some(axis);
        self
    }

    /// Maps the pointer inside the element, from the press until the release, to an action:
    /// where in the box it is, as fractions. For sliders and other controls that read a
    /// position. A press that starts here never clicks.
    pub fn on_pointer<A: 'static>(mut self, mut f: impl FnMut(Pointer) -> A + 'a) -> Self {
        self.ix().on_pointer = Some(Box::new(move |p| Box::new(f(p)) as Action));
        self
    }

    /// Emits `action` on the frame the pointer moves onto the element or anything inside it.
    pub fn on_hover_enter<A: 'static>(mut self, action: A) -> Self {
        self.ix().on_hover_enter = Some(Box::new(action));
        self
    }

    /// Emits `action` on the frame the pointer leaves the element and everything inside it.
    /// An element that disappears from the tree while hovered emits nothing.
    pub fn on_hover_leave<A: 'static>(mut self, action: A) -> Self {
        self.ix().on_hover_leave = Some(Box::new(action));
        self
    }

    /// Holds the element in `state` whatever the input is: its state style is shown fully.
    pub fn pin(mut self, state: Pin) -> Self {
        use crate::ui::elements::{ACTIVE, FOCUS, HOVER};
        self.ix().pinned |= match state {
            Pin::Hover => HOVER,
            Pin::Pressed => ACTIVE,
            Pin::Focus => FOCUS,
        };
        self
    }

    /// Lets the pointer pass through the element to what is beneath it, like CSS
    /// `pointer-events: none`. A full-size overlay that only draws (a toast stack, a tooltip's
    /// canvas) needs it, or it would be the thing under the pointer wherever it covers the
    /// page. Its children still take the pointer.
    pub fn passthrough(mut self) -> Self {
        self.ix().passthrough = true;
        self
    }

    /// Moves the element sideways to `x` logical px from where layout put it, with a spring, so
    /// an indicator or a switch knob glides when `x` changes. The first frame it is at `x`.
    pub fn slide(mut self, x: f32) -> Self {
        self.ix().slide = Some(x);
        self
    }

    /// Emits `action` when `key` is pressed while focus is on the element or inside it. Makes
    /// the element focusable.
    pub fn on_key<A: 'static>(mut self, key: UiKey, action: A) -> Self {
        let ix = self.ix();
        ix.on_keys.push((key, Box::new(action)));
        ix.focusable.get_or_insert(true);
        self
    }

    /// Marks the root of a tree as sitting above trees with a lower layer (default 0): the
    /// tree draws on that layer (1 floats, with glass blur; 2 is for tooltips and toasts), and
    /// the pointer and Tab reach a lower tree only where no higher tree's root covers it. So a
    /// menu or sheet built as its own `ui.element(..)` call blocks the page behind it. Give a
    /// covering tree a root that fills the area it blocks (`w_full().h_full()`).
    pub fn layer(mut self, layer: u8) -> Self {
        self.ix().layer = layer;
        self
    }

    /// Keeps the element in the tree and fades it in or out as `shown` changes (`Enter::Rise`
    /// also moves it 8 px). Once fully out it takes no space, and while it is leaving it takes
    /// no input. Give it an id. This is exit motion without keeping anything alive: the app just
    /// keeps building the element and flips `shown`.
    pub fn presence(mut self, shown: bool, motion: Enter) -> Self {
        self.ix().presence = Some((motion, shown));
        self
    }

    /// Maps a fast flick that started on the element to an action.
    pub fn on_swipe<A: 'static>(mut self, mut f: impl FnMut(Dir) -> A + 'a) -> Self {
        self.ix().on_swipe = Some(Box::new(move |d| Box::new(f(d)) as Action));
        self
    }

    /// How the element looks while the pointer is over it or anything inside it.
    pub fn hover(mut self, f: impl FnOnce(StateStyle) -> StateStyle) -> Self {
        self.ix().hover = Some(f(StateStyle::default()));
        self
    }

    /// How the element looks while it is pressed.
    pub fn active(mut self, f: impl FnOnce(StateStyle) -> StateStyle) -> Self {
        self.ix().active = Some(f(StateStyle::default()));
        self
    }

    /// How the element looks while it has keyboard focus. The focus ring draws either way.
    pub fn focus(mut self, f: impl FnOnce(StateStyle) -> StateStyle) -> Self {
        self.ix().focus = Some(f(StateStyle::default()));
        self
    }

    /// Puts the element in the Tab order (or, with `false`, takes a clickable one out of it).
    pub fn focusable(mut self, on: bool) -> Self {
        self.ix().focusable = Some(on);
        self
    }

    /// Lets events this element handles go on to its ancestors as well.
    pub fn bubble(mut self) -> Self {
        self.ix().bubble = true;
        self
    }

    /// Stops events here, even if this element does not handle them.
    pub fn stop_propagation(mut self) -> Self {
        self.ix().stop = true;
        self
    }

    /// Greys the element and everything inside it out and makes it ignore input.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.ix().disabled = disabled;
        self
    }

    /// Blends state styles over `seconds`: use the V3 tokens, `motion::INSTANT` for hover
    /// and press, `motion::FAST` for focus.
    pub fn transition(mut self, seconds: f32) -> Self {
        self.ix().transition = Some(seconds);
        self
    }

    /// Transitions state styles (hover, active, focus) using Apple-grade fluid springs.
    pub fn spring(mut self, spring: crate::theme::motion::Spring) -> Self {
        self.ix().spring = Some(spring);
        self
    }

    /// Apple tactile press feedback: smooth shrink on press and fluid spring overshoot on release.
    pub fn press_scale(self, scale: f32) -> Self {
        self.active(move |s| s.scale(scale)).spring(crate::theme::motion::Spring::SNAPPY)
    }

    /// Plays `enter` the first time the element is shown. Give it an id, so it plays once per
    /// item and not again when the list around it changes.
    pub fn enter(self, enter: Enter) -> Self {
        self.enter_at(enter, 0)
    }

    /// Plays `enter` delayed by `index` stagger steps (`motion::STAGGER`), for lists.
    pub fn enter_at(mut self, enter: Enter, index: usize) -> Self {
        self.ix().enter = Some((enter, index));
        self
    }

    /// Makes the element a vertical scroll container: its content keeps its own height and
    /// scrolls with the wheel, a drag, and touch fling and rubber band. Give it a height (or
    /// `flex_1()`) and an id.
    pub fn scroll(mut self) -> Self {
        self.style.scroll = true;
        self
    }
}
