//! Components (ANVIL-UI-2.md, section 3.4): the controls apps are built from, written with the
//! element API. Each carries its ADL states (rest, hover, pressed, focus, disabled, selected)
//! and reads every color from the theme, so all of them look right in the dark, light and
//! high-contrast themes.
//!
//! A component is a small builder that turns into an [`Element`](crate::element::Element)
//! wherever one is wanted:
//!
//! ```ignore
//! div().flex_col().gap_3().p_4()
//!     .child(text("Battery").heading())
//!     .child(badge("94%").success())
//!     .child(button("Pair phone").primary().icon(Icon::Plus).on_click(Msg::StartPairing))
//!     .child(toggle(state.sync).on_click(Msg::ToggleSync))
//!     .child(slider(state.volume).on_change(Msg::Volume))
//! ```
//!
//! The app owns the state. A control shows the value it is given and emits an action (the app's
//! own message type, read after the frame with `ui.take_actions::<Msg>()`) when it is used.
//! Text inputs and the editor, which keep a caret and a selection, edit a `&mut String` in
//! place instead, through the immediate-mode widgets they wrap.
//!
//! Every component also takes `.id(..)` (give list items one), `.disabled(bool)`, `.tooltip(..)`
//! and `.pin(..)` (hold a state on, for showcases and screenshots).

use crate::element::{Action, Element, Pin};
use crate::ElementId;

mod buttons;
mod feedback;
mod forms;
mod nav;
mod overlays;

pub use buttons::{button, icon_button, Button, IconButton, Variant};
pub use feedback::{badge, empty_state, progress, skeleton, Badge, EmptyState, Progress, Skeleton, Tone};
pub use forms::{
    checkbox, search_field, segmented, slider, text_editor, text_input, toggle, Checkbox, Segmented, Slider, TextEditor, TextInput,
    Toggle,
};
pub use nav::{list_item, menu, nav_bar, tabs, toolbar, toolbar_divider, ListItem, Menu, MenuItem, NavBar, Tabs};
pub use overlays::{modal, sheet, toast_stack, tooltip, Modal, Sheet, ToastStack};

/// What every component shares: an optional id, the disabled flag, pinned states and a tooltip.
#[derive(Default)]
pub(crate) struct Base {
    pub id: Option<ElementId>,
    pub disabled: bool,
    pub pins: Vec<Pin>,
    pub tip: Option<String>,
}

impl Base {
    /// Applies the shared settings to the finished element.
    pub fn apply<'a>(self, mut el: Element<'a>) -> Element<'a> {
        if let Some(id) = self.id {
            el = el.id(id);
        }
        if self.disabled {
            el = el.disabled(true);
        }
        for p in self.pins {
            el = el.pin(p);
        }
        match self.tip {
            Some(text) => overlays::tooltip(el, text),
            None => el,
        }
    }
}

/// `.id`, `.disabled`, `.pin` and `.tooltip` for a component with a `base: Base` field.
macro_rules! base_methods {
    () => {
        /// Keys the component's state. List items need one.
        pub fn id(mut self, id: impl Into<$crate::ElementId>) -> Self {
            self.base.id = Some(id.into());
            self
        }
        /// Greys the component out and makes it ignore input.
        pub fn disabled(mut self, on: bool) -> Self {
            self.base.disabled = on;
            self
        }
        /// Holds a state (hover, pressed, focus) on, whatever the input is.
        pub fn pin(mut self, state: $crate::element::Pin) -> Self {
            self.base.pins.push(state);
            self
        }
        /// Shows `text` after the pointer rests on the component.
        pub fn tooltip(mut self, text: impl Into<String>) -> Self {
            self.base.tip = Some(text.into());
            self
        }
    };
}
pub(crate) use base_methods;

/// Boxes an action for storage until the component is built.
pub(crate) fn boxed<A: 'static>(action: A) -> Action {
    Box::new(action)
}
