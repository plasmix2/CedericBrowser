//! Form controls: `checkbox`, `toggle`, `slider`, `segmented`, and the text controls
//! (`text_input`, `search_field`, `text_editor`).

use super::feedback::Tone;
use super::{base_methods, boxed, Base};
use crate::editor::EditorCursor;
use crate::element::{canvas, div, icon, text, Action, Element, Enter, Tok};
use crate::geom::Rect;
use crate::icons::Icon;
use crate::input::InputKind;
use crate::theme::motion;
use crate::ui::anvl::EditorOutput;
use crate::ui::{Ui, UiKey};
use crate::ElementId;

// ---------------------------------------------------------------------- checkbox

/// A check box with an optional label, in a row that is the whole click target.
pub struct Checkbox {
    checked: bool,
    label: Option<String>,
    click: Option<Action>,
    base: Base,
}

pub fn checkbox(checked: bool) -> Checkbox {
    Checkbox { checked, label: None, click: None, base: Base::default() }
}

impl Checkbox {
    base_methods!();

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
    /// Emits `action` when the row is clicked or activated. The app flips its value.
    pub fn on_click<A: 'static>(mut self, action: A) -> Self {
        self.click = Some(boxed(action));
        self
    }
}

impl<'a> From<Checkbox> for Element<'a> {
    fn from(c: Checkbox) -> Element<'a> {
        let mark = if c.checked {
            div()
                .size(20.0)
                .flex_row()
                .items_center()
                .justify_center()
                .rounded_sm()
                .bg(Tok::Primary)
                .border_1()
                .border_color(Tok::Primary)
                .child(icon(Icon::Check).text_size(14.0).color(Tok::PrimaryFg))
        } else {
            div().size(20.0).rounded_sm().bg(Tok::Sunken).border_1().border_color(Tok::Muted)
        };
        let mut el = div()
            .flex_row()
            .items_center()
            .gap_3()
            .min_h(44.0)
            .px_2()
            .rounded_md()
            .transition(motion::INSTANT)
            .hover(|s| s.bg(Tok::CardHi))
            .active(|s| s.bg(Tok::CardHi))
            .child(mark);
        if let Some(label) = c.label {
            el = el.child(text(label).color(Tok::Text));
        }
        if let Some(action) = c.click {
            el = el.click_boxed(action);
        }
        c.base.apply(el)
    }
}

// ---------------------------------------------------------------------- toggle

/// An on/off switch, 44 × 26 px. The knob glides and the fill fades when `on` changes.
pub struct Toggle {
    on: bool,
    click: Option<Action>,
    base: Base,
}

pub fn toggle(on: bool) -> Toggle {
    Toggle { on, click: None, base: Base::default() }
}

impl Toggle {
    base_methods!();

    /// Emits `action` when the switch is clicked or activated. The app flips its value.
    pub fn on_click<A: 'static>(mut self, action: A) -> Self {
        self.click = Some(boxed(action));
        self
    }
}

impl<'a> From<Toggle> for Element<'a> {
    fn from(t: Toggle) -> Element<'a> {
        let track = div()
            .w(44.0)
            .h(26.0)
            .p(2.0)
            .rounded_full()
            .bg(Tok::CardHi)
            .border_1()
            .transition(motion::INSTANT)
            .hover(|s| s.border_color(Tok::Ring))
            // The fill fades in over the off track, starting from where it is on the first frame.
            .child(div().absolute().top(0.0).left(0.0).right(0.0).bottom(0.0).rounded_full().bg(Tok::Primary).presence(t.on, Enter::Fade))
            .child(
                div()
                    .size(20.0)
                    .rounded_full()
                    .border_1()
                    .bg(if t.on { Tok::PrimaryFg } else { Tok::Text })
                    .slide(if t.on { 18.0 } else { 0.0 }),
            );
        let mut el = track;
        if let Some(action) = t.click {
            el = el.click_boxed(action);
        }
        t.base.apply(el)
    }
}

// ---------------------------------------------------------------------- slider

/// A slider from 0 to 1. Drag or click the rail to set it; the arrow keys step by 5%, Home and
/// End jump to the ends. It emits `on_change(value)` while the pointer is down and on keys.
pub struct Slider {
    value: f32,
    tone: Tone,
    change: Option<Box<dyn Fn(f32) -> Action>>,
    base: Base,
}

pub fn slider(value: f32) -> Slider {
    Slider { value: value.clamp(0.0, 1.0), tone: Tone::Primary, change: None, base: Base::default() }
}

impl Slider {
    base_methods!();

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }
    pub fn on_change<A: 'static>(mut self, f: impl Fn(f32) -> A + 'static) -> Self {
        self.change = Some(Box::new(move |v| Box::new(f(v)) as Action));
        self
    }
}

impl<'a> From<Slider> for Element<'a> {
    fn from(s: Slider) -> Element<'a> {
        let v = s.value;
        let tok = s.tone.tok();
        let mut rail = div()
            .flex_1()
            .h(36.0)
            .rounded_full()
            .child(div().absolute().top(16.0).left(0.0).right(0.0).h(4.0).rounded_full().bg(Tok::Border))
            .child(div().absolute().top(16.0).left(0.0).h(4.0).w_frac(v).rounded_full().bg(tok))
            .child(
                div()
                    .absolute()
                    .top(0.0)
                    .left(0.0)
                    .bottom(0.0)
                    .w_frac(v)
                    .flex_row()
                    .justify_end()
                    .items_center()
                    .child(div().size(20.0).mr(-10.0).rounded_full().bg(Tok::Primary).border_1().border_color(Tok::Bg)),
            );
        if let Some(change) = s.change {
            let keys = [
                (UiKey::ArrowLeft, v - 0.05),
                (UiKey::ArrowDown, v - 0.05),
                (UiKey::ArrowRight, v + 0.05),
                (UiKey::ArrowUp, v + 0.05),
                (UiKey::Home, 0.0),
                (UiKey::End, 1.0),
            ];
            for (key, to) in keys {
                rail = rail.on_key_boxed(key, change(to.clamp(0.0, 1.0)));
            }
            rail = rail.on_pointer_raw(move |p| change(p.x));
        }
        s.base.apply(div().w_full().flex_row().items_center().px(10.0).rounded_md().child(rail))
    }
}

// ---------------------------------------------------------------------- segmented

/// A row of equal segments with one selected, whose highlight glides between them.
pub struct Segmented {
    labels: Vec<String>,
    selected: usize,
    seg_w: f32,
    select: Option<Box<dyn Fn(usize) -> Action>>,
    base: Base,
}

pub fn segmented<S: Into<String>>(labels: impl IntoIterator<Item = S>, selected: usize) -> Segmented {
    Segmented { labels: labels.into_iter().map(Into::into).collect(), selected, seg_w: 96.0, select: None, base: Base::default() }
}

impl Segmented {
    base_methods!();

    /// The width of every segment, logical px (default 96).
    pub fn segment_width(mut self, px: f32) -> Self {
        self.seg_w = px;
        self
    }
    pub fn on_select<A: 'static>(mut self, f: impl Fn(usize) -> A + 'static) -> Self {
        self.select = Some(Box::new(move |i| Box::new(f(i)) as Action));
        self
    }
}

impl<'a> From<Segmented> for Element<'a> {
    fn from(s: Segmented) -> Element<'a> {
        let w = s.seg_w;
        let mut el = div().flex_row().p(2.0).rounded_lg().bg(Tok::Sunken).border_1();
        if !s.labels.is_empty() {
            el = el.child(
                div()
                    .absolute()
                    .top(2.0)
                    .left(2.0)
                    .w(w)
                    .h(40.0)
                    .rounded_md()
                    .bg(Tok::CardHi)
                    .border_1()
                    .slide(s.selected.min(s.labels.len() - 1) as f32 * w),
            );
        }
        for (i, label) in s.labels.into_iter().enumerate() {
            let mut seg = div()
                .w(w)
                .h(40.0)
                .flex_row()
                .items_center()
                .justify_center()
                .rounded_md()
                .color(if i == s.selected { Tok::Text } else { Tok::TextDim })
                .transition(motion::INSTANT)
                .hover(|st| st.color(Tok::Text))
                .child(text(label).medium().nowrap());
            if let Some(select) = &s.select {
                seg = seg.click_boxed(select(i));
            }
            el = el.child(seg);
        }
        s.base.apply(el)
    }
}

// ---------------------------------------------------------------------- text controls

/// A single-line text field, 44 px: placeholder, optional icon, caret, selection, clipboard and
/// the focus ring, all from the immediate-mode `text_field` it wraps. It edits `value` in place.
pub struct TextInput<'a> {
    id: ElementId,
    value: &'a mut String,
    placeholder: String,
    icon: Option<Icon>,
    kind: InputKind,
    max_len: usize,
    disabled: bool,
}

pub fn text_input<'a>(id: impl Into<ElementId>, value: &'a mut String) -> TextInput<'a> {
    TextInput { id: id.into(), value, placeholder: String::new(), icon: None, kind: InputKind::Text, max_len: 256, disabled: false }
}

/// A [`text_input`] with a search icon and the placeholder "Search".
pub fn search_field<'a>(id: impl Into<ElementId>, value: &'a mut String) -> TextInput<'a> {
    text_input(id, value).icon(Icon::Search).placeholder("Search")
}

impl<'a> TextInput<'a> {
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = text.into();
        self
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    /// What may be typed (`InputKind::Text` by default).
    pub fn kind(mut self, kind: InputKind) -> Self {
        self.kind = kind;
        self
    }
    pub fn max_len(mut self, max_len: usize) -> Self {
        self.max_len = max_len;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
}

impl<'a> From<TextInput<'a>> for Element<'a> {
    fn from(t: TextInput<'a>) -> Element<'a> {
        if t.disabled {
            let shown = if t.value.is_empty() { t.placeholder.clone() } else { t.value.clone() };
            let mut el = div().w_full().h(44.0).flex_row().items_center().gap_2().px_3().rounded_md().bg(Tok::DisabledBg).border_1().color(Tok::Disabled);
            if let Some(i) = t.icon {
                el = el.child(icon(i).text_size(18.0));
            }
            return el.child(text(shown).nowrap());
        }
        let TextInput { id, value, placeholder, icon, kind, max_len, .. } = t;
        canvas(move |ui: &mut Ui, r: Rect| ui.text_field(id, r, value, max_len, kind, &placeholder, icon)).w_full().h(44.0)
    }
}

/// The multi-line Markdown editor (`Ui::text_editor`) as an element that fills the space it is
/// given. `cursor` holds the caret and scroll for this text, so keep one per note.
pub struct TextEditor<'a> {
    id: ElementId,
    value: &'a mut String,
    cursor: &'a mut EditorCursor,
    placeholder: String,
    out: Option<&'a mut EditorOutput>,
}

pub fn text_editor<'a>(id: impl Into<ElementId>, value: &'a mut String, cursor: &'a mut EditorCursor) -> TextEditor<'a> {
    TextEditor { id: id.into(), value, cursor, placeholder: String::new(), out: None }
}

impl<'a> TextEditor<'a> {
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = text.into();
        self
    }
    /// Receives what the editor reported this frame: changed, save requested, caret position.
    pub fn output(mut self, out: &'a mut EditorOutput) -> Self {
        self.out = Some(out);
        self
    }
}

impl<'a> From<TextEditor<'a>> for Element<'a> {
    fn from(t: TextEditor<'a>) -> Element<'a> {
        let TextEditor { id, value, cursor, placeholder, out } = t;
        canvas(move |ui: &mut Ui, r: Rect| {
            let got = ui.text_editor(id, r, value, cursor, &placeholder);
            if let Some(out) = out {
                *out = got;
            }
        })
        .flex_1()
        .w_full()
    }
}
