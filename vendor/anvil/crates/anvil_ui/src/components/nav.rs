//! Navigation and lists: `tabs`, `nav_bar`, `toolbar`, `list_item` and `menu`.

use super::{base_methods, boxed, Base};
use crate::element::{div, icon, text, Action, Element, Enter, Tok};
use crate::icons::Icon;
use crate::theme::motion;
use crate::ui::NavItem;

// ---------------------------------------------------------------------- tabs

/// A row of text tabs over a hairline, the selected one underlined.
pub struct Tabs {
    labels: Vec<String>,
    selected: usize,
    select: Option<Box<dyn Fn(usize) -> Action>>,
    base: Base,
}

pub fn tabs<S: Into<String>>(labels: impl IntoIterator<Item = S>, selected: usize) -> Tabs {
    Tabs { labels: labels.into_iter().map(Into::into).collect(), selected, select: None, base: Base::default() }
}

impl Tabs {
    base_methods!();

    pub fn on_select<A: 'static>(mut self, f: impl Fn(usize) -> A + 'static) -> Self {
        self.select = Some(Box::new(move |i| Box::new(f(i)) as Action));
        self
    }
}

impl<'a> From<Tabs> for Element<'a> {
    fn from(t: Tabs) -> Element<'a> {
        let mut el = div().w_full().flex_row().child(div().absolute().left(0.0).right(0.0).bottom(0.0).h(1.0).bg(Tok::Border));
        for (i, label) in t.labels.into_iter().enumerate() {
            let on = i == t.selected;
            let mut tab = div()
                .h(44.0)
                .px_4()
                .flex_row()
                .items_center()
                .justify_center()
                .color(if on { Tok::Text } else { Tok::Muted })
                .transition(motion::INSTANT)
                .hover(|s| s.color(Tok::Text).bg(Tok::CardHi))
                .active(|s| s.bg(Tok::Selection))
                .rounded_md()
                .child(text(label).medium().nowrap())
                .child(div().absolute().left(16.0).right(16.0).bottom(0.0).h(2.0).rounded_full().bg(Tok::Primary).presence(on, Enter::Fade));
            if let Some(select) = &t.select {
                tab = tab.click_boxed(select(i));
            }
            el = el.child(tab);
        }
        t.base.apply(el)
    }
}

// ---------------------------------------------------------------------- nav bar

/// The phone's bottom navigation: icon-over-label destinations, and at most one raised primary
/// action (a `NavItem` with `primary` set) in its slot.
pub struct NavBar {
    items: Vec<(Icon, String, bool)>,
    selected: usize,
    select: Option<Box<dyn Fn(usize) -> Action>>,
    base: Base,
}

pub fn nav_bar(items: &[NavItem], selected: usize) -> NavBar {
    NavBar {
        items: items.iter().map(|i| (i.icon, i.label.to_string(), i.primary)).collect(),
        selected,
        select: None,
        base: Base::default(),
    }
}

impl NavBar {
    base_methods!();

    /// Emits `f(index)` when a slot is tapped, including the primary one.
    pub fn on_select<A: 'static>(mut self, f: impl Fn(usize) -> A + 'static) -> Self {
        self.select = Some(Box::new(move |i| Box::new(f(i)) as Action));
        self
    }
}

impl<'a> From<NavBar> for Element<'a> {
    fn from(n: NavBar) -> Element<'a> {
        let mut el = div().w_full().h(68.0).flex_row().bg(Tok::Surface).child(div().absolute().top(0.0).left(0.0).right(0.0).h(1.0).bg(Tok::Border));
        for (i, (glyph, label, primary)) in n.items.into_iter().enumerate() {
            let on = i == n.selected && !primary;
            let mut slot = div().flex_1().h_full().flex_col().items_center().justify_center().gap_1().transition(motion::INSTANT);
            if primary {
                slot = slot.child(
                    div()
                        .size(48.0)
                        .rounded_full()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .bg(Tok::Primary)
                        .color(Tok::PrimaryFg)
                        .shadow()
                        .child(icon(glyph)),
                );
            } else {
                slot = slot
                    .color(if on { Tok::Text } else { Tok::Muted })
                    .hover(|s| s.color(Tok::Text))
                    .active(|s| s.bg(Tok::CardHi))
                    .child(div().absolute().top(0.0).h(3.0).w(24.0).rounded_full().bg(Tok::Primary).presence(on, Enter::Fade))
                    .child(icon(glyph).text_size(22.0))
                    .child(text(label).text_size(13.0).medium().nowrap());
            }
            if let Some(select) = &n.select {
                slot = slot.click_boxed(select(i));
            }
            el = el.child(slot);
        }
        n.base.apply(el)
    }
}

// ---------------------------------------------------------------------- toolbar

/// A floating strip of icon buttons on the surface color.
pub fn toolbar<'a, C: Into<Element<'a>>>(items: impl IntoIterator<Item = C>) -> Element<'a> {
    div().flex_row().items_center().gap_1().p_1().bg(Tok::Surface).border_1().rounded_lg().children(items.into_iter().map(Into::into))
}

/// A vertical hairline between groups of toolbar buttons.
pub fn toolbar_divider<'a>() -> Element<'a> {
    div().w(1.0).h(24.0).bg(Tok::Border)
}

// ---------------------------------------------------------------------- list item

/// A row for lists: optional leading icon, a title, a subtitle, trailing content, the whole
/// row clickable. `selected` marks the open item.
pub struct ListItem<'a> {
    title: String,
    subtitle: Option<String>,
    icon: Option<Icon>,
    trailing: Option<Element<'a>>,
    selected: bool,
    click: Option<Action>,
    base: Base,
}

pub fn list_item<'a>(title: impl Into<String>) -> ListItem<'a> {
    ListItem { title: title.into(), subtitle: None, icon: None, trailing: None, selected: false, click: None, base: Base::default() }
}

impl<'a> ListItem<'a> {
    base_methods!();

    pub fn subtitle(mut self, text: impl Into<String>) -> Self {
        self.subtitle = Some(text.into());
        self
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    /// Something at the end of the row: a timestamp, a badge, a chevron.
    pub fn trailing(mut self, trailing: impl Into<Element<'a>>) -> Self {
        self.trailing = Some(trailing.into());
        self
    }
    pub fn selected(mut self, on: bool) -> Self {
        self.selected = on;
        self
    }
    pub fn on_click<A: 'static>(mut self, action: A) -> Self {
        self.click = Some(boxed(action));
        self
    }
}

impl<'a> From<ListItem<'a>> for Element<'a> {
    fn from(l: ListItem<'a>) -> Element<'a> {
        let mut el = div()
            .w_full()
            .min_h(56.0)
            .flex_row()
            .items_center()
            .gap_3()
            .px_4()
            .py_2()
            .rounded_md()
            .transition(motion::INSTANT)
            .active(|s| s.bg(Tok::Selection));
        el = if l.selected { el.bg(Tok::CardHi).hover(|s| s.bg(Tok::Selection)) } else { el.hover(|s| s.bg(Tok::CardHi)) };
        if let Some(i) = l.icon {
            el = el.child(icon(i).color(Tok::TextDim));
        }
        let mut body = div().flex_1().flex_col().child(text(l.title).medium().nowrap().color(Tok::Text));
        if let Some(sub) = l.subtitle {
            body = body.child(text(sub).caption().nowrap());
        }
        el = el.child(body);
        if let Some(t) = l.trailing {
            el = el.child(t);
        }
        if let Some(action) = l.click {
            el = el.click_boxed(action);
        }
        l.base.apply(el)
    }
}

// ---------------------------------------------------------------------- menu

/// One entry of a [`menu`]: a row that acts, or a separator.
pub struct MenuItem {
    label: String,
    icon: Option<Icon>,
    shortcut: Option<String>,
    danger: bool,
    disabled: bool,
    separator: bool,
    click: Option<Action>,
}

impl MenuItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), icon: None, shortcut: None, danger: false, disabled: false, separator: false, click: None }
    }
    pub fn separator() -> Self {
        Self { separator: true, ..Self::new("") }
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    /// The keys shown at the row's end ("Ctrl+N").
    pub fn shortcut(mut self, keys: impl Into<String>) -> Self {
        self.shortcut = Some(keys.into());
        self
    }
    /// Destructive: drawn in the danger color.
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn on_click<A: 'static>(mut self, action: A) -> Self {
        self.click = Some(boxed(action));
        self
    }
}

/// A floating list of actions on glass. Close it from the app by no longer building it: give it
/// `on_dismiss` to hear about a press outside or Escape. Place it with [`Menu::at`].
pub struct Menu {
    items: Vec<MenuItem>,
    width: f32,
    dismiss: Option<(Action, Action)>,
    base: Base,
}

pub fn menu(items: impl IntoIterator<Item = MenuItem>) -> Menu {
    Menu { items: items.into_iter().collect(), width: 220.0, dismiss: None, base: Base::default() }
}

impl Menu {
    base_methods!();

    pub fn width(mut self, px: f32) -> Self {
        self.width = px;
        self
    }
    /// Emits `action` on a press outside the menu and on Escape.
    pub fn on_dismiss<A: Clone + 'static>(mut self, action: A) -> Self {
        self.dismiss = Some((boxed(action.clone()), boxed(action)));
        self
    }

    /// The menu as its own raised tree with its top left at (`x`, `y`) logical px: build it
    /// last with `ui.element(ui.screen_rect(), menu.at(x, y))`. The page behind gets no input
    /// while it is open.
    pub fn at<'a>(self, x: f32, y: f32) -> Element<'a> {
        div().w_full().h_full().layer(1).child(Element::from(self).absolute().left(x).top(y))
    }
}

impl<'a> From<Menu> for Element<'a> {
    fn from(m: Menu) -> Element<'a> {
        let mut el = div().w(m.width).p_1().glass().rounded_lg().bg(Tok::Card).color(Tok::Text);
        for item in m.items {
            if item.separator {
                el = el.child(div().h(1.0).my_1().bg(Tok::Border));
                continue;
            }
            let tone = if item.danger { Tok::Danger } else { Tok::Text };
            let mut row = div()
                .h(40.0)
                .flex_row()
                .items_center()
                .gap_3()
                .px_3()
                .rounded_md()
                .color(tone)
                .transition(motion::INSTANT)
                .hover(|s| s.bg(Tok::CardHi))
                .active(|s| s.bg(Tok::Selection))
                .disabled(item.disabled);
            if let Some(i) = item.icon {
                row = row.child(icon(i).text_size(18.0));
            }
            row = row.child(div().flex_1().child(text(item.label).nowrap()));
            if let Some(keys) = item.shortcut {
                row = row.child(text(keys).caption().nowrap());
            }
            if let Some(action) = item.click {
                row = row.click_boxed(action);
            }
            el = el.child(row);
        }
        if let Some((outside, escape)) = m.dismiss {
            el = el.on_click_outside_boxed(outside).on_escape_boxed(escape);
        }
        m.base.apply(el)
    }
}
