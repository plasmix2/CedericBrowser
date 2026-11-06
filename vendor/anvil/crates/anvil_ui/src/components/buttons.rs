//! Buttons: the labeled `button` and the square `icon_button`.

use super::{base_methods, boxed, Base};
use crate::element::{div, icon, text, Action, Element, Tok};
use crate::icons::Icon;
use crate::theme::motion;

/// How loud a button is. One `Primary` per screen; the rest are quieter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Variant {
    /// The one main action: a solid `primary` fill.
    #[default]
    Primary,
    /// A raised neutral fill.
    Secondary,
    /// No fill until hovered.
    Ghost,
    /// Destructive actions: a solid `danger` fill.
    Destructive,
    /// Completion: a solid `success` fill.
    Success,
}

/// A button: 44 px tall (48 dp to touch), a label and an optional icon. Build it with
/// [`button`].
pub struct Button {
    label: String,
    icon: Option<Icon>,
    variant: Variant,
    full: bool,
    click: Option<Action>,
    base: Base,
}

/// A labeled button, `Primary` until told otherwise.
pub fn button(label: impl Into<String>) -> Button {
    Button { label: label.into(), icon: None, variant: Variant::Primary, full: false, click: None, base: Base::default() }
}

impl Button {
    base_methods!();

    pub fn variant(mut self, variant: Variant) -> Self {
        self.variant = variant;
        self
    }
    pub fn primary(self) -> Self {
        self.variant(Variant::Primary)
    }
    pub fn secondary(self) -> Self {
        self.variant(Variant::Secondary)
    }
    pub fn ghost(self) -> Self {
        self.variant(Variant::Ghost)
    }
    pub fn destructive(self) -> Self {
        self.variant(Variant::Destructive)
    }
    pub fn success(self) -> Self {
        self.variant(Variant::Success)
    }
    /// An icon before the label.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    /// Fills the width it is given.
    pub fn full_width(mut self) -> Self {
        self.full = true;
        self
    }
    /// Emits `action` when the button is clicked, tapped, or activated with Enter or Space.
    pub fn on_click<A: 'static>(mut self, action: A) -> Self {
        self.click = Some(boxed(action));
        self
    }
}

impl<'a> From<Button> for Element<'a> {
    fn from(b: Button) -> Element<'a> {
        let mut el = div()
            .flex_row()
            .items_center()
            .justify_center()
            .gap_2()
            .h(44.0)
            .min_w(44.0)
            .px_4()
            .rounded_md()
            .transition(motion::INSTANT);
        el = match b.variant {
            Variant::Primary => el
                .bg(Tok::Primary)
                .color(Tok::PrimaryFg)
                .hover(|s| s.opacity(0.9))
                .active(|s| s.opacity(0.8).scale(0.98)),
            Variant::Secondary => el
                .bg(Tok::CardHi)
                .color(Tok::Text)
                .border_1()
                .hover(|s| s.bg(Tok::Selection))
                .active(|s| s.bg(Tok::Selection).scale(0.98)),
            Variant::Ghost => el
                .color(Tok::TextDim)
                .hover(|s| s.bg(Tok::CardHi).color(Tok::Text))
                .active(|s| s.bg(Tok::Selection).color(Tok::Text).scale(0.98)),
            Variant::Destructive => el
                .bg(Tok::Danger)
                .color(Tok::OnColor)
                .hover(|s| s.opacity(0.9))
                .active(|s| s.opacity(0.8).scale(0.98)),
            Variant::Success => el
                .bg(Tok::Success)
                .color(Tok::OnColor)
                .hover(|s| s.opacity(0.9))
                .active(|s| s.opacity(0.8).scale(0.98)),
        };
        if b.full {
            el = el.w_full();
        }
        if let Some(i) = b.icon {
            el = el.child(icon(i));
        }
        if !b.label.is_empty() {
            el = el.child(text(b.label).medium().nowrap());
        }
        if let Some(action) = b.click {
            el = el.click_boxed(action);
        }
        b.base.apply(el)
    }
}

/// A square icon button, 44 px: toolbars, headers, row actions. Build it with [`icon_button`].
pub struct IconButton {
    icon: Icon,
    selected: bool,
    click: Option<Action>,
    base: Base,
}

pub fn icon_button(icon: Icon) -> IconButton {
    IconButton { icon, selected: false, click: None, base: Base::default() }
}

impl IconButton {
    base_methods!();

    /// Shows the button as on (a toolbar's Bold while the text is bold).
    pub fn selected(mut self, on: bool) -> Self {
        self.selected = on;
        self
    }
    pub fn on_click<A: 'static>(mut self, action: A) -> Self {
        self.click = Some(boxed(action));
        self
    }
}

impl<'a> From<IconButton> for Element<'a> {
    fn from(b: IconButton) -> Element<'a> {
        let mut el = div()
            .size(44.0)
            .flex_row()
            .items_center()
            .justify_center()
            .rounded_md()
            .transition(motion::INSTANT)
            .child(icon(b.icon));
        el = if b.selected {
            el.bg(Tok::CardHi).color(Tok::Text).hover(|s| s.bg(Tok::Selection)).active(|s| s.bg(Tok::Selection).scale(0.96))
        } else {
            el.color(Tok::TextDim).hover(|s| s.bg(Tok::CardHi).color(Tok::Text)).active(|s| s.bg(Tok::Selection).color(Tok::Text).scale(0.96))
        };
        if let Some(action) = b.click {
            el = el.click_boxed(action);
        }
        b.base.apply(el)
    }
}
