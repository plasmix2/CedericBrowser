//! Feedback: `badge`, `progress` (bar and ring), `skeleton` and `empty_state`.

use super::{base_methods, Base};
use crate::element::{canvas, div, icon, text, Element, Tok};
use crate::geom::Rect;
use crate::icons::Icon;
use crate::ui::Ui;
use glam::Vec2;

/// The role a status component plays, which picks its color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    /// Secondary text color: no status.
    #[default]
    Neutral,
    Primary,
    Success,
    Warning,
    Danger,
}

impl Tone {
    pub fn tok(self) -> Tok {
        match self {
            Tone::Neutral => Tok::TextDim,
            Tone::Primary => Tok::Primary,
            Tone::Success => Tok::Success,
            Tone::Warning => Tok::Warning,
            Tone::Danger => Tok::Danger,
        }
    }
}

macro_rules! tone_methods {
    () => {
        pub fn tone(mut self, tone: Tone) -> Self {
            self.tone = tone;
            self
        }
        pub fn success(self) -> Self {
            self.tone(Tone::Success)
        }
        pub fn warning(self) -> Self {
            self.tone(Tone::Warning)
        }
        pub fn danger(self) -> Self {
            self.tone(Tone::Danger)
        }
    };
}

// ---------------------------------------------------------------------- badge

/// A status pill: a tinted fill, a 1 px tinted border, an optional icon and a short label.
pub struct Badge {
    label: String,
    icon: Option<Icon>,
    tone: Tone,
    base: Base,
}

pub fn badge(label: impl Into<String>) -> Badge {
    Badge { label: label.into(), icon: None, tone: Tone::Neutral, base: Base::default() }
}

impl Badge {
    base_methods!();
    tone_methods!();

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
}

impl<'a> From<Badge> for Element<'a> {
    fn from(b: Badge) -> Element<'a> {
        let tok = b.tone.tok();
        let mut el = div()
            .flex_row()
            .items_center()
            .gap(6.0)
            .h(28.0)
            .px_3()
            .rounded_full()
            .bg(tok.tint(0.12))
            .border_1()
            .border_color(tok.tint(0.35))
            .color(tok);
        if let Some(i) = b.icon {
            el = el.child(icon(i).text_size(14.0));
        }
        b.base.apply(el.child(text(b.label).text_size(13.0).medium().nowrap()))
    }
}

// ---------------------------------------------------------------------- progress

/// A progress bar (8 px) or, with [`Progress::ring`], a ring. `fraction` is clamped to 0..1.
pub struct Progress {
    fraction: f32,
    tone: Tone,
    ring: Option<f32>,
    base: Base,
}

pub fn progress(fraction: f32) -> Progress {
    Progress { fraction: fraction.clamp(0.0, 1.0), tone: Tone::Success, ring: None, base: Base::default() }
}

impl Progress {
    base_methods!();
    tone_methods!();

    /// A ring of `size` logical px across, starting at the top and running clockwise.
    pub fn ring(mut self, size: f32) -> Self {
        self.ring = Some(size);
        self
    }
}

impl<'a> From<Progress> for Element<'a> {
    fn from(p: Progress) -> Element<'a> {
        let tok = p.tone.tok();
        let el = match p.ring {
            Some(size) => {
                let f = p.fraction;
                canvas(move |ui: &mut Ui, r: Rect| {
                    let width = ui.s(5.0).min(r.w() * 0.2);
                    let radius = (r.w().min(r.h()) - width) * 0.5;
                    let color = tok.resolve(ui.theme());
                    ui.progress_ring(Vec2::new(r.center().x, r.center().y), radius, width, f, color);
                })
                .size(size)
            }
            None => div()
                .w_full()
                .h(8.0)
                .rounded_full()
                .bg(Tok::Border)
                .overflow_hidden()
                .child(div().w_frac(p.fraction).h_full().rounded_full().bg(tok)),
        };
        p.base.apply(el)
    }
}

// ---------------------------------------------------------------------- skeleton

/// A loading placeholder with a light band sweeping across it. Give it a size.
pub struct Skeleton {
    base: Base,
}

pub fn skeleton() -> Skeleton {
    Skeleton { base: Base::default() }
}

impl Skeleton {
    base_methods!();
}

impl<'a> From<Skeleton> for Element<'a> {
    fn from(s: Skeleton) -> Element<'a> {
        let el = canvas(|ui: &mut Ui, r: Rect| {
            if ui.reduced_motion() {
                ui.skeleton(r, 2.0);
            } else {
                ui.skeleton(r, ui.time * 0.8);
                ui.request_frame();
            }
        })
        .w_full()
        .h(16.0);
        s.base.apply(el)
    }
}

// ---------------------------------------------------------------------- empty state

/// What a list shows when it has nothing: an icon, a title, a line of help and maybe an action.
pub struct EmptyState<'a> {
    icon: Icon,
    title: String,
    body: Option<String>,
    action: Option<Element<'a>>,
    base: Base,
}

pub fn empty_state<'a>(icon: Icon, title: impl Into<String>) -> EmptyState<'a> {
    EmptyState { icon, title: title.into(), body: None, action: None, base: Base::default() }
}

impl<'a> EmptyState<'a> {
    base_methods!();

    /// A sentence of help under the title.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }
    /// The way out: usually a primary `button`.
    pub fn action(mut self, action: impl Into<Element<'a>>) -> Self {
        self.action = Some(action.into());
        self
    }
}

impl<'a> From<EmptyState<'a>> for Element<'a> {
    fn from(e: EmptyState<'a>) -> Element<'a> {
        let mut el = div()
            .w_full()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .p_6()
            .child(icon(e.icon).text_size(40.0).color(Tok::Muted))
            .child(text(e.title).heading().color(Tok::Text));
        if let Some(body) = e.body {
            el = el.child(text(body).body().color(Tok::TextDim).max_w(320.0));
        }
        if let Some(action) = e.action {
            el = el.child(div().mt(4.0).child(action));
        }
        e.base.apply(el)
    }
}
