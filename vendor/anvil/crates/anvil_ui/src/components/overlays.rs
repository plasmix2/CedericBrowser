//! Overlays: `modal`, `sheet`, `tooltip` and `toast_stack`.
//!
//! `modal` and `sheet` are whole trees on layer 1: build them last, with
//! `ui.element(ui.screen_rect(), modal(open, content).on_dismiss(Msg::Close))`. The page behind
//! gets no pointer or Tab input while they are open. They stay in the tree and fade in and out
//! as `open` changes.

use super::{base_methods, boxed, Base};
use crate::element::{canvas, div, Action, Element, Enter, Paint, Tok};
use crate::geom::Rect;
use crate::ui::{Toasts, Ui};
use crate::ElementId;

/// A dimming layer over the page.
fn scrim<'a>() -> Element<'a> {
    div().absolute().top(0.0).left(0.0).right(0.0).bottom(0.0).bg(Paint::Fixed([0.0, 0.0, 0.0, 0.5])).focusable(false)
}

// ---------------------------------------------------------------------- modal

/// A dialog: a glass card centered over a scrim. A press on the scrim or Escape dismisses it.
pub struct Modal<'a> {
    open: bool,
    content: Element<'a>,
    width: f32,
    dismiss: Option<(Action, Action)>,
    base: Base,
}

pub fn modal<'a>(open: bool, content: impl Into<Element<'a>>) -> Modal<'a> {
    Modal { open, content: content.into(), width: 440.0, dismiss: None, base: Base::default() }
}

impl<'a> Modal<'a> {
    base_methods!();

    /// The card's largest width, logical px (default 440).
    pub fn width(mut self, px: f32) -> Self {
        self.width = px;
        self
    }
    pub fn on_dismiss<A: Clone + 'static>(mut self, action: A) -> Self {
        self.dismiss = Some((boxed(action.clone()), boxed(action)));
        self
    }
}

impl<'a> From<Modal<'a>> for Element<'a> {
    fn from(m: Modal<'a>) -> Element<'a> {
        let mut shade = scrim();
        let mut root = div().w_full().h_full().layer(1).flex_col().items_center().justify_center().p_4();
        if let Some((on_scrim, on_escape)) = m.dismiss {
            shade = shade.click_boxed(on_scrim);
            root = root.on_escape_boxed(on_escape);
        }
        let card = div().w_full().max_w(m.width).p_6().gap_4().glass().rounded_lg().bg(Tok::Card).color(Tok::Text).stop_propagation().child(m.content);
        m.base.apply(root.presence(m.open, Enter::Rise).child(shade).child(card))
    }
}

// ---------------------------------------------------------------------- sheet

/// A bottom sheet: a glass panel rising from the bottom edge over a scrim, with a grab handle.
pub struct Sheet<'a> {
    open: bool,
    content: Element<'a>,
    dismiss: Option<(Action, Action)>,
    base: Base,
}

pub fn sheet<'a>(open: bool, content: impl Into<Element<'a>>) -> Sheet<'a> {
    Sheet { open, content: content.into(), dismiss: None, base: Base::default() }
}

impl<'a> Sheet<'a> {
    base_methods!();

    pub fn on_dismiss<A: Clone + 'static>(mut self, action: A) -> Self {
        self.dismiss = Some((boxed(action.clone()), boxed(action)));
        self
    }
}

impl<'a> From<Sheet<'a>> for Element<'a> {
    fn from(s: Sheet<'a>) -> Element<'a> {
        let mut shade = scrim();
        let mut root = div().w_full().h_full().layer(1).flex_col().justify_end();
        if let Some((on_scrim, on_escape)) = s.dismiss {
            shade = shade.click_boxed(on_scrim);
            root = root.on_escape_boxed(on_escape);
        }
        let panel = div()
            .w_full()
            .p_4()
            .gap_4()
            .glass()
            .rounded_lg()
            .bg(Tok::Card)
            .color(Tok::Text)
            .stop_propagation()
            .child(div().self_center().w(40.0).h(4.0).rounded_full().bg(Tok::Border))
            .child(s.content);
        s.base.apply(root.presence(s.open, Enter::Rise).child(shade).child(panel))
    }
}

// ---------------------------------------------------------------------- tooltip

/// Gives `el` a tooltip: after the pointer rests on it, `text` appears beside it on glass.
/// Touch never shows one. `el` must be a container (it gets an invisible child); the components
/// do this for you with `.tooltip(..)`.
pub fn tooltip<'a>(el: Element<'a>, text: impl Into<String>) -> Element<'a> {
    let text = text.into();
    let id = el.id.unwrap_or_else(|| ElementId::new("anvil.tooltip")).part("tip").key(&text);
    el.child(canvas(move |ui: &mut Ui, r: Rect| ui.tooltip(id, r, &text)).absolute().top(0.0).left(0.0).right(0.0).bottom(0.0).passthrough())
}

// ---------------------------------------------------------------------- toasts

/// The toast stack (`Toasts`) drawn over the whole tree it is a child of. Put it at the end of
/// the root: it fills it without taking part in the layout. `on_action` hears about a toast's
/// action button ("Undo") with the toast's handle.
pub struct ToastStack<'a> {
    toasts: &'a mut Toasts,
    action: Option<Box<dyn Fn(u64) -> Action + 'a>>,
}

pub fn toast_stack<'a>(toasts: &'a mut Toasts) -> ToastStack<'a> {
    ToastStack { toasts, action: None }
}

impl<'a> ToastStack<'a> {
    pub fn on_action<A: 'static>(mut self, f: impl Fn(u64) -> A + 'a) -> Self {
        self.action = Some(Box::new(move |handle| Box::new(f(handle)) as Action));
        self
    }
}

impl<'a> From<ToastStack<'a>> for Element<'a> {
    fn from(t: ToastStack<'a>) -> Element<'a> {
        let ToastStack { toasts, action } = t;
        canvas(move |ui: &mut Ui, r: Rect| {
            if let Some(handle) = ui.toast_stack(toasts, r) {
                if let Some(f) = &action {
                    ui.emit_boxed(f(handle));
                }
            }
        })
        .absolute()
        .top(0.0)
        .left(0.0)
        .right(0.0)
        .bottom(0.0)
        .passthrough()
    }
}
