//! Native popups: the right-click menu, the address-bar suggestions and the tab hover card. Each is
//! a small owned window drawn with tiny-skia, like the toolbar.
//!
//! The windows are created once, shown without taking focus, and then only moved: hidden ones are
//! parked off-screen. tao can show a window without activating it only at creation, and an active
//! window would get the accent-colored border. Since there is no WebView inside, they can stay
//! non-focusable and still receive clicks.

use std::{
    num::NonZeroU32,
    rc::Rc,
    time::Instant,
};

use cosmic_text::{Buffer, Shaping, Weight};
use tao::{
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::EventLoopWindowTarget,
    platform::windows::{WindowBuilderExtWindows, WindowExtWindows},
    window::{Window, WindowBuilder, WindowId},
};
use tiny_skia::Pixmap;

use crate::gfx::{Brand, Canvas, Fonts, Icon, Rgba, Theme, attrs, text_width};

/// Where hidden popups wait. Physical pixels: a logical position this far out would be scaled past
/// what Windows accepts at high DPI, and the window would land at the screen's top-left instead.
const PARKED: PhysicalPosition<i32> = PhysicalPosition::new(-30000, -30000);
const MENU_ROW: f32 = 30.0;
const MENU_SEP: f32 = 9.0;
const SUGGEST_ROW: f32 = 36.0;
const PAD: f32 = 5.0;
pub const CARD_W: f32 = 280.0;
const SHOT_H: f32 = 158.0;
const CARD_TEXT_H: f32 = 62.0;
/// Height of a toolbar tooltip.
pub const TIP_H: f32 = 28.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Menu,
    Suggest,
    Preview,
}

pub struct MenuEntry {
    pub label: String,
    pub hint: Option<String>,
    pub disabled: bool,
    pub sep: bool,
}

pub struct SuggestEntry {
    pub text: String,
    /// Opens a site rather than searching.
    pub url: bool,
}

pub struct Card {
    pub title: String,
    pub host: String,
    pub img: Option<Pixmap>,
}

pub enum Content {
    Empty,
    Menu(Vec<MenuEntry>),
    Suggest { items: Vec<SuggestEntry>, query: String },
    Preview(Card),
    /// One-line tooltip under a toolbar button.
    Tip(String),
}

/// A row laid out for drawing and hit-testing.
struct Row {
    y: f32,
    h: f32,
    label: Option<Buffer>,
    hint: Option<Buffer>,
    enabled: bool,
}

/// What a popup shows and its interaction state, independent of the window it is drawn into.
struct View {
    content: Content,
    rows: Vec<Row>,
    size: (f32, f32),
    /// 0..1 entrance animation.
    enter: f32,
    hover: Option<usize>,
    hover_fade: Vec<f32>,
    selected: usize,
    pressed: Option<usize>,
}

pub struct Popup {
    pub kind: Kind,
    pub window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    surface_size: (u32, u32),
    pm: Pixmap,
    open: bool,
    /// Where to move the window once it has been repainted at its new size.
    pending_pos: Option<LogicalPosition<f64>>,
    v: View,
}

/// What a click in a popup means.
pub enum Pick {
    Menu(usize),
    Suggest(usize),
}

pub struct Popups {
    fonts: Fonts,
    theme: Theme,
    pub menu: Popup,
    pub suggest: Popup,
    pub preview: Popup,
    last_frame: Instant,
}

impl Popups {
    pub fn new<T>(target: &EventLoopWindowTarget<T>, owner: &Window, dark: bool, brand: Brand) -> Self {
        let theme = Theme::for_brand(dark, brand);
        let make = |kind| Popup::new(target, owner, kind, theme.card);
        Self {
            fonts: Fonts::load(),
            menu: make(Kind::Menu),
            suggest: make(Kind::Suggest),
            preview: make(Kind::Preview),
            theme,
            last_frame: Instant::now(),
        }
    }

    pub fn set_theme(&mut self, dark: bool, brand: Brand) {
        self.theme = Theme::for_brand(dark, brand);
        let c = self.theme.card;
        for p in [&self.menu, &self.suggest, &self.preview] {
            p.window.set_background_color(Some((c.0, c.1, c.2, 255)));
            if p.open {
                p.window.request_redraw();
            }
        }
    }

    fn get(&mut self, kind: Kind) -> &mut Popup {
        match kind {
            Kind::Menu => &mut self.menu,
            Kind::Suggest => &mut self.suggest,
            Kind::Preview => &mut self.preview,
        }
    }

    pub fn is_open(&self, kind: Kind) -> bool {
        match kind {
            Kind::Menu => self.menu.open,
            Kind::Suggest => self.suggest.open,
            Kind::Preview => self.preview.open,
        }
    }

    /// Shows `content` with its top-left at `pos` (logical screen coordinates), `w` wide.
    /// Returns the height it needs, already applied.
    pub fn show(&mut self, kind: Kind, content: Content, pos: LogicalPosition<f64>, w: f32) -> f32 {
        let s = self.get(kind).window.scale_factor() as f32;
        let (rows, h) = layout(&mut self.fonts, &content, s);
        let p = self.get(kind);
        let fresh = !p.open || p.v.size != (w, h);
        // Moving between hover cards or refreshing suggestions only swaps the content, without replaying the entrance.
        if !p.open {
            p.v.enter = 0.0;
            p.v.hover = None;
        }
        p.v.selected = 0;
        p.v.pressed = None;
        p.v.hover_fade = vec![0.0; rows.len()];
        p.v.rows = rows;
        p.v.content = content;
        p.v.size = (w, h);
        p.open = true;
        if fresh {
            // Resizing exposes unpainted pixels (white slivers, stale content). Keep the window
            // off-screen until `redraw` has painted it at the new size, then move it into place.
            p.window.set_outer_position(PARKED);
            p.window.set_inner_size(LogicalSize::new(w as f64, h as f64));
            p.pending_pos = Some(pos);
        } else {
            p.window.set_outer_position(pos);
        }
        p.window.request_redraw();
        h
    }

    pub fn hide(&mut self, kind: Kind) {
        let p = self.get(kind);
        if p.open {
            p.open = false;
            p.v.content = Content::Empty;
            p.v.rows.clear();
            p.pending_pos = None;
            p.window.set_outer_position(PARKED);
        }
    }

    /// Highlights a suggestion picked with the arrow keys in the address bar.
    pub fn select(&mut self, kind: Kind, index: usize) {
        let p = self.get(kind);
        p.v.selected = index;
        p.window.request_redraw();
    }

    pub fn owns(&self, id: WindowId) -> Option<Kind> {
        [&self.menu, &self.suggest, &self.preview].into_iter().find(|p| p.window.id() == id).map(|p| p.kind)
    }

    /// Mouse input on a popup window. Returns a pick when a row was clicked.
    pub fn window_event(&mut self, kind: Kind, event: &WindowEvent) -> Option<Pick> {
        let p = self.get(kind);
        match event {
            // A popup is resized in place when its content changes; repaint at the new size so no
            // stale or blank pixels show around it.
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                p.window.request_redraw();
                None
            }
            WindowEvent::CursorMoved { position, .. } => {
                let y = position.to_logical::<f32>(p.window.scale_factor()).y;
                let hover = p.v.rows.iter().position(|r| r.enabled && y >= r.y && y < r.y + r.h);
                if hover != p.v.hover {
                    p.v.hover = hover;
                    if kind == Kind::Suggest
                        && let Some(i) = hover
                    {
                        p.v.selected = i;
                    }
                    p.window.request_redraw();
                }
                None
            }
            WindowEvent::CursorLeft { .. } => {
                p.v.hover = None;
                p.v.pressed = None;
                p.window.request_redraw();
                None
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match (kind, state) {
                // Suggestions act on press: the address bar may lose focus before release.
                (Kind::Suggest, ElementState::Pressed) => p.v.hover.map(Pick::Suggest),
                (Kind::Menu, ElementState::Pressed) => {
                    p.v.pressed = p.v.hover;
                    p.window.request_redraw();
                    None
                }
                (Kind::Menu, ElementState::Released) => {
                    let picked = p.v.pressed.take().filter(|i| Some(*i) == p.v.hover);
                    picked.map(Pick::Menu)
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Advances entrance and hover animations. Returns when the next frame is needed.
    pub fn tick(&mut self, now: Instant) -> Option<Instant> {
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;
        let k = 1.0 - (-dt * 30.0).exp();
        let mut busy = false;
        for p in [&mut self.menu, &mut self.suggest, &mut self.preview] {
            if !p.open {
                continue;
            }
            let mut moved = false;
            if p.v.enter < 1.0 {
                p.v.enter = (p.v.enter + dt / 0.14).min(1.0);
                moved = true;
            }
            for (i, f) in p.v.hover_fade.iter_mut().enumerate() {
                let target = if Some(i) == p.v.hover { 1.0 } else { 0.0 };
                if (*f - target).abs() > 0.01 {
                    *f += (target - *f) * k;
                    moved = true;
                } else {
                    *f = target;
                }
            }
            if moved {
                p.window.request_redraw();
                busy = true;
            }
        }
        busy.then(|| now + crate::pacer::FRAME)
    }

    /// Draws a popup and puts it on screen.
    pub fn redraw(&mut self, kind: Kind) {
        let th = self.theme;
        let fonts = &mut self.fonts;
        let p = match kind {
            Kind::Menu => &mut self.menu,
            Kind::Suggest => &mut self.suggest,
            Kind::Preview => &mut self.preview,
        };
        if !p.open {
            return;
        }
        let size = p.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if p.pm.width() != w.get() || p.pm.height() != h.get() {
            let Some(pm) = Pixmap::new(w.get(), h.get()) else { return };
            p.pm = pm;
        }
        let s = p.window.scale_factor() as f32;
        draw(&mut p.v, &mut p.pm, fonts, &th, s);
        if p.surface_size != (w.get(), h.get()) {
            if p.surface.resize(w, h).is_err() {
                return;
            }
            p.surface_size = (w.get(), h.get());
        }
        let Ok(mut buf) = p.surface.buffer_mut() else { return };
        for (d, px) in buf.iter_mut().zip(p.pm.data().chunks_exact(4)) {
            *d = (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32;
        }
        let _ = buf.present();
        if let Some(pos) = p.pending_pos.take() {
            p.window.set_outer_position(pos);
        }
    }
}

impl Popup {
    fn new<T>(target: &EventLoopWindowTarget<T>, owner: &Window, kind: Kind, card: Rgba) -> Self {
        let window = Rc::new(
            WindowBuilder::new()
                .with_decorations(false)
                .with_background_color((card.0, card.1, card.2, 255))
                .with_resizable(false)
                .with_visible(true)
                .with_focused(false)
                .with_focusable(false)
                .with_position(PARKED)
                .with_skip_taskbar(true)
                .with_owner_window(owner.hwnd())
                .with_inner_size(LogicalSize::new(10.0, 10.0))
                .build(target)
                .expect("failed to create popup window"),
        );
        // The creation-time position gets clamped on-screen; move it away explicitly.
        window.set_outer_position(PARKED);
        let ctx = softbuffer::Context::new(window.clone()).expect("failed to create drawing context");
        let surface = softbuffer::Surface::new(&ctx, window.clone()).expect("failed to create drawing surface");
        Self {
            kind,
            window,
            surface,
            surface_size: (0, 0),
            pm: Pixmap::new(1, 1).expect("1x1 pixmap"),
            open: false,
            pending_pos: None,
            v: View {
                content: Content::Empty,
                rows: Vec::new(),
                size: (0.0, 0.0),
                enter: 1.0,
                hover: None,
                hover_fade: Vec::new(),
                selected: 0,
                pressed: None,
            },
        }
    }
}

/// Lays out rows (with their text) and returns them with the total height.
fn layout(fonts: &mut Fonts, content: &Content, s: f32) -> (Vec<Row>, f32) {
    let mut rows = Vec::new();
    let mut y = PAD;
    match content {
        Content::Empty => return (rows, 1.0),
        Content::Menu(items) => {
            for it in items {
                if it.sep {
                    rows.push(Row { y, h: MENU_SEP, label: None, hint: None, enabled: false });
                    y += MENU_SEP;
                    continue;
                }
                let label = fonts.line(&it.label, 13.0, Weight::NORMAL, s);
                let hint = it.hint.as_deref().map(|h| fonts.line(h, 11.5, Weight::NORMAL, s));
                rows.push(Row { y, h: MENU_ROW, label: Some(label), hint, enabled: !it.disabled });
                y += MENU_ROW;
            }
        }
        Content::Suggest { items, query } => {
            let q = query.to_lowercase();
            for (i, it) in items.iter().enumerate() {
                let mut label = fonts.line("", 13.5, Weight::NORMAL, s);
                // The part after what was typed is emphasised, as most browsers do.
                let typed = i > 0 && !q.is_empty() && it.text.to_lowercase().starts_with(&q) && it.text.is_char_boundary(q.len());
                let (head, tail) = if typed { it.text.split_at(q.len()) } else { (it.text.as_str(), "") };
                label.set_rich_text(
                    [(head, attrs(Weight::NORMAL)), (tail, attrs(Weight::SEMIBOLD))],
                    &attrs(Weight::NORMAL),
                    Shaping::Advanced,
                    None,
                );
                label.shape_until_scroll(&mut fonts.system, false);
                let hint = fonts.line(if it.url { "Open site" } else { "Search" }, 11.5, Weight::NORMAL, s);
                rows.push(Row { y, h: SUGGEST_ROW, label: Some(label), hint: Some(hint), enabled: true });
                y += SUGGEST_ROW;
            }
        }
        Content::Preview(card) => {
            let img_h = if card.img.is_some() { SHOT_H } else { 0.0 };
            let title = fonts.line(&card.title, 13.0, Weight::SEMIBOLD, s);
            let host = fonts.line(if card.host.is_empty() { "New tab" } else { &card.host }, 12.0, Weight::NORMAL, s);
            rows.push(Row { y: img_h, h: CARD_TEXT_H, label: Some(title), hint: Some(host), enabled: false });
            return (rows, img_h + CARD_TEXT_H);
        }
        Content::Tip(text) => {
            let label = fonts.line(text, 12.5, Weight::NORMAL, s);
            rows.push(Row { y: 0.0, h: TIP_H, label: Some(label), hint: None, enabled: false });
            return (rows, TIP_H);
        }
    }
    (rows, y + PAD)
}

fn draw(p: &mut View, pm: &mut Pixmap, fonts: &mut Fonts, th: &Theme, s: f32) {
    let dark = th.fg.0 > 128;
    let bg = th.card;
    let hover_bg = if dark { Rgba(255, 255, 255, 20) } else { Rgba(0, 0, 0, 14) };
    let (w, h) = p.size;
    // Entrance: content eases in from slightly above while fading in over the card.
    let t = 1.0 - (1.0 - p.enter).powi(3);
    let dy = -3.0 * (1.0 - t);
    let a = t;
    // Paint the whole buffer: the window can briefly be larger than `p.size` while it resizes.
    pm.fill(tiny_skia::Color::from_rgba8(bg.0, bg.1, bg.2, 255));
    let mut c = Canvas { pm, s };
    c.rect(0.0, 0.0, w, h, bg);
    match &p.content {
        Content::Empty => {}
        Content::Menu(_) => {
            for (i, r) in p.rows.iter_mut().enumerate() {
                let y = r.y + dy;
                if r.label.is_none() {
                    c.hairline_h(PAD + 6.0, y + MENU_SEP / 2.0, w - 2.0 * (PAD + 6.0), th.border.with_alpha(a));
                    continue;
                }
                let f = p.hover_fade.get(i).copied().unwrap_or(0.0);
                if f > 0.0 {
                    let bg = if p.pressed == Some(i) { hover_bg.with_alpha(1.6) } else { hover_bg.with_alpha(f) };
                    c.rrect(PAD, y, w - 2.0 * PAD, r.h, [7.0; 4], bg.with_alpha(a));
                }
                let fg = if r.enabled { th.fg } else { th.dim.with_alpha(0.55) }.with_alpha(a);
                let ty = y + (r.h - 13.0 * 1.35) / 2.0;
                let hint_w = r.hint.as_ref().map_or(0.0, |b| text_width(b, s));
                if let Some(b) = &mut r.label {
                    c.text(fonts, b, PAD + 10.0, ty, fg, w - PAD - 16.0 - hint_w, 12.0);
                }
                if let Some(b) = &mut r.hint {
                    c.text(fonts, b, w - PAD - 10.0 - hint_w, y + (r.h - 11.5 * 1.35) / 2.0, th.dim.with_alpha(a), w, 0.0);
                }
            }
        }
        Content::Suggest { items, .. } => {
            for (i, r) in p.rows.iter_mut().enumerate() {
                let y = r.y + dy;
                let sel = i == p.selected;
                if sel {
                    c.rrect(PAD, y, w - 2.0 * PAD, r.h, [9.0; 4], th.accent.with_alpha(0.16 * a));
                } else if let Some(f) = p.hover_fade.get(i).filter(|f| **f > 0.0) {
                    c.rrect(PAD, y, w - 2.0 * PAD, r.h, [9.0; 4], hover_bg.with_alpha(*f * a));
                }
                let icon = if items.get(i).is_some_and(|it| it.url) { Icon::Globe } else { Icon::Search };
                c.icon(icon, PAD + 11.0, y + (r.h - 18.0) / 2.0, 18.0, if sel { th.accent } else { th.fg.with_alpha(0.8) }.with_alpha(a));
                let hint_w = r.hint.as_ref().map_or(0.0, |b| text_width(b, s));
                if let Some(b) = &mut r.label {
                    c.text(fonts, b, PAD + 40.0, y + (r.h - 13.5 * 1.35) / 2.0, th.fg.with_alpha(a), w - PAD - 20.0 - hint_w, 16.0);
                }
                if (sel || p.hover == Some(i))
                    && let Some(b) = &mut r.hint
                {
                    c.text(fonts, b, w - PAD - 12.0 - hint_w, y + (r.h - 11.5 * 1.35) / 2.0, th.dim.with_alpha(a), w, 0.0);
                }
            }
        }
        Content::Tip(_) => {
            if let Some(r) = p.rows.first_mut() {
                let y = r.y + dy;
                if let Some(b) = &mut r.label {
                    c.text(fonts, b, 10.0, y + 5.5, th.fg.with_alpha(a), w - 10.0, 18.0);
                }
            }
        }
        Content::Preview(card) => {
            if let Some(img) = &card.img {
                // Cover-fit the snapshot into the top of the card.
                let k = (w / img.width() as f32 * s).max(SHOT_H / img.height() as f32 * s);
                let (iw, ih) = (img.width() as f32 * k / s, img.height() as f32 * k / s);
                c.rect(0.0, 0.0, w, SHOT_H, Rgba(0, 0, 0, 255));
                c.image(img, (w - iw) / 2.0, 0.0, iw, ih, a);
                // Anything taller than the slot is covered by the text area below it.
                c.rect(0.0, SHOT_H, w, h - SHOT_H, bg);
                c.hairline_h(0.0, SHOT_H, w, th.border);
            }
            if let Some(r) = p.rows.first_mut() {
                let y = r.y + dy;
                if let Some(b) = &mut r.label {
                    c.text(fonts, b, 14.0, y + 11.0, th.fg.with_alpha(a), w - 14.0, 18.0);
                }
                c.icon(Icon::Globe, 14.0, y + 36.0, 12.0, th.dim.with_alpha(a));
                if let Some(b) = &mut r.hint {
                    c.text(fonts, b, 32.0, y + 33.5, th.dim.with_alpha(a), w - 14.0, 18.0);
                }
            }
        }
    }
    // Own 1px border: the popups have no OS frame (the OS shadow frame left unpainted edges).
    c.hairline_h(0.0, 0.0, w, th.border);
    c.hairline_h(0.0, h - 1.0, w, th.border);
    c.hairline_v(0.0, 0.0, h, th.border);
    c.hairline_v(w - 1.0, 0.0, h, th.border);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders each popup off-screen: `CEDERIC_SHOT=<dir> cargo test -- --ignored render_popups`.
    #[test]
    #[ignore]
    fn render_popups() {
        let dir = std::path::PathBuf::from(std::env::var("CEDERIC_SHOT").expect("CEDERIC_SHOT"));
        let mut fonts = Fonts::load();
        let th = Theme::new(true);
        let s = 2.0;
        let entry = |label: &str, hint: Option<&str>, disabled| MenuEntry { label: label.into(), hint: hint.map(Into::into), disabled, sep: false };
        let sep = || MenuEntry { label: String::new(), hint: None, disabled: true, sep: true };
        let mut thumb = Pixmap::new(320, 180).unwrap();
        thumb.fill(tiny_skia::Color::from_rgba8(40, 120, 90, 255));
        let cases = [
            ("popup-menu", 248.0, Content::Menu(vec![
                entry("Open link in new tab", None, false),
                entry("Copy link address", None, false),
                sep(),
                entry("Back", Some("Alt+←"), true),
                entry("Reload", Some("Ctrl+R"), false),
                sep(),
                entry("Inspect", Some("F12"), false),
            ])),
            ("popup-suggest", 600.0, Content::Suggest {
                query: "rust bor".into(),
                items: vec![
                    SuggestEntry { text: "rust bor".into(), url: false },
                    SuggestEntry { text: "rust borrow checker".into(), url: false },
                    SuggestEntry { text: "rust borrow trait".into(), url: false },
                ],
            }),
            ("popup-card", CARD_W, Content::Preview(Card { title: "Rust Programming Language".into(), host: "rust-lang.org".into(), img: Some(thumb) })),
        ];
        for (name, w, content) in cases {
            let (rows, h) = layout(&mut fonts, &content, s);
            let mut v = View {
                hover_fade: (0..rows.len()).map(|i| if i == 1 { 1.0 } else { 0.0 }).collect(),
                rows,
                content,
                size: (w, h),
                enter: 1.0,
                hover: Some(1),
                selected: 2,
                pressed: None,
            };
            let mut pm = Pixmap::new((w * s) as u32, (h * s) as u32).unwrap();
            draw(&mut v, &mut pm, &mut fonts, &th, s);
            pm.save_png(dir.join(format!("{name}.png"))).unwrap();
        }
    }
}
