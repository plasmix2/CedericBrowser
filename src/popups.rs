//! Native popups: the right-click menu, the address-bar suggestions and the tab hover card. Each is
//! a small owned window drawn by the ANVL engine (`anvil.rs`), like the toolbar.
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

use tao::{
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::EventLoopWindowTarget,
    platform::windows::{WindowBuilderExtWindows, WindowExtWindows},
    window::{CursorIcon, Window, WindowBuilder, WindowId},
};
use tiny_skia::{FillRule, Mask, Paint, Pixmap, PixmapPaint, Stroke, Transform};

use crate::{
    anvil::{Engine, Weight},
    gfx::{Brand, Icon, PersonaColor, Rgba, Theme, rrect_path},
};

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
const TIP_H: f32 = 28.0;
const TIP_FONT: f32 = 12.5;
/// Width of the form dialogs (persona editor, confirmations).
pub const FORM_W: f32 = 400.0;
pub const PERSONA_H: f32 = 428.0;
pub const CONFIRM_H: f32 = 150.0;
const MODAL_HEAD: f32 = 74.0;
/// Height of the persona editor's animated preview, and the size of a color dot.
const HERO_H: f32 = 160.0;
const DOT: f32 = 30.0;
/// Where the sliders' tracks start (their labels sit to the left).
const SLIDER_X: f32 = 92.0;
/// Frames bigger than this (in pixels) do not keep cached GPU buffers once they are gone: about
/// the size of the toolbar strip on a large screen.
pub const TRIM_PIXELS: u64 = 1_200_000;
/// Width of the switches dialog (`Content::Modal`).
pub const MODAL_W: f32 = 420.0;
/// Radius of a slider's knob, which stays inside the track.
const KNOB: f32 = 9.0;
const MODAL_ROW: f32 = 56.0;
const MODAL_FOOT: f32 = 62.0;

/// Space around a popup's card for its drop shadow (left, top, right, bottom), in logical pixels.
/// The shadow falls downward, so the top margin is small: tooltips and the suggestion list sit
/// just under the control they belong to.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Margins {
    l: f32,
    t: f32,
    r: f32,
    b: f32,
}

/// The parts of a popup frame that only change with its size: the shadow and the rounded clip.
struct Frame {
    key: (u32, u32, u32, u32, u32, u32, bool),
    shadow: Pixmap,
    mask: Mask,
}

const NO_MARGIN: Margins = Margins { l: 0.0, t: 0.0, r: 0.0, b: 0.0 };

/// Rounded, shadowed popups need a transparent window. `PERSONA_FLAT_POPUPS=1` falls back to plain
/// opaque rectangles, for systems where window transparency is not available.
fn rounded_popups() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("PERSONA_FLAT_POPUPS").is_none_or(|v| v.is_empty() || v == "0"))
}

fn margins(kind: Kind) -> Margins {
    if !rounded_popups() {
        return NO_MARGIN;
    }
    match kind {
        Kind::Scrim => NO_MARGIN,
        // A submenu sits flush against its parent menu: no transparent margin on that side, or the
        // window would cover the parent's rows and take their clicks.
        Kind::Sub => Margins { l: 0.0, t: 6.0, r: 20.0, b: 32.0 },
        // Small margins: the transparent margin is still part of the window and takes clicks away
        // from whatever is under it, so it only reaches as far as the shadow needs to.
        Kind::Tip => Margins { l: 12.0, t: 3.0, r: 12.0, b: 18.0 },
        _ => Margins { l: 20.0, t: 6.0, r: 20.0, b: 32.0 },
    }
}

fn corner_radius(kind: Kind) -> f32 {
    if !rounded_popups() {
        return 0.0;
    }
    match kind {
        Kind::Scrim => 0.0,
        Kind::Tip => 6.0,
        _ => 10.0,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Menu,
    Suggest,
    Preview,
    /// Toolbar button tooltips; its own window so it never fights the hover card.
    Tip,
    /// A centred dialog with switches (site permissions). Stays until its Done button is clicked.
    Modal,
    /// A submenu next to a menu row.
    Sub,
    /// The blurred, dimmed page behind a modal. Swallows clicks so the page cannot be used meanwhile.
    Scrim,
}

/// A round picture for a menu row: the photo at `url` once it has loaded, otherwise `letter` on `color`.
#[derive(Clone)]
pub struct Avatar {
    pub url: Option<String>,
    pub letter: String,
    pub color: Rgba,
}

#[derive(Default)]
pub struct MenuEntry {
    pub label: String,
    pub hint: Option<String>,
    pub disabled: bool,
    pub sep: bool,
    pub icon: Option<Icon>,
    pub avatar: Option<Avatar>,
    /// Opens a submenu (drawn with a chevron).
    pub sub: bool,
}

pub struct SuggestEntry {
    pub text: String,
    /// Opens a site rather than searching.
    pub url: bool,
}

pub struct ToggleEntry {
    pub label: String,
    pub desc: String,
    pub on: bool,
}

/// The persona editor: a name field, a grid of colors and Save / Cancel / Delete.
pub struct PersonaForm {
    pub title: String,
    pub name: String,
    /// Caret position in characters.
    pub caret: usize,
    /// The preset that is chosen, or `None` when the color comes from the sliders.
    pub selected: Option<usize>,
    pub colors: Vec<Rgba>,
    /// The color being built: its hue and vibrancy (0..1), and the resulting accent.
    pub hue: f32,
    pub vibrancy: f32,
    pub accent: Rgba,
    /// 1 right after any interaction, fading out: makes the orb flare.
    pub pulse: f32,
    /// Names of the colors, in the same order (the selected one is shown under its dot).
    pub color_names: Vec<String>,
    /// The persona's picture, shown in the preview, if it has one (a loaded photo URL).
    pub avatar_url: Option<String>,
    pub can_delete: bool,
    /// Labels of the two buttons on the right; an empty `cancel` hides that button.
    pub cancel: String,
    pub save: String,
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
    /// A dialog: title, a line of explanation, one switch row per entry, then a Done button.
    /// Rows: one per switch, then the finishing button (labelled `done`).
    Modal { title: String, note: String, items: Vec<ToggleEntry>, done: String },
    /// The persona editor dialog. Rows: 0 = name field, 1..=8 = colors, 9 = Delete, 10 = Cancel, 11 = Save.
    Persona(PersonaForm),
    /// A yes/no question. Rows: 0 = the cancelling button (hidden when `cancel` is empty), 1 = the
    /// confirming one, drawn in the accent when `primary` and in red otherwise.
    Confirm { title: String, note: String, ok: String, cancel: String, primary: bool },
    /// A blurred picture of the page (if there is one yet) and the height to cover, in logical px.
    Scrim { img: Option<Pixmap>, h: f32 },
}

/// A row laid out for drawing and hit-testing.
struct Row {
    y: f32,
    h: f32,
    /// Horizontal extent for rows that do not span the card (color swatches, buttons); `w` 0 = full width.
    x: f32,
    w: f32,
    /// A draggable track (reports a 0..1 position instead of a click).
    slider: bool,
    label: Option<String>,
    icon: Option<Icon>,
    avatar: Option<Avatar>,
    sub: bool,
    /// Text drawn right after `label` in a heavier weight (the part of a suggestion that was not typed).
    tail: String,
    hint: Option<String>,
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
    /// Whether the preview snapshot of the current content is on the GPU.
    shot_ready: bool,
}

pub struct Popup {
    pub kind: Kind,
    pub window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    surface_size: (u32, u32),
    /// The whole window: shadow, rounded card and border.
    pm: Pixmap,
    /// The card's content at its own size, before it is clipped to the rounded shape.
    card: Pixmap,
    frame: Option<Frame>,
    open: bool,
    /// Where to move the window once it has been repainted at its new size.
    pending_pos: Option<PhysicalPosition<i32>>,
    /// The window size this popup asked for, in physical pixels (re-applied after moving it).
    phys_size: (u32, u32),
    /// Where the card is on screen (logical), for placing a submenu beside a row.
    card_pos: (f64, f64),
    /// Last pointer position inside the card (logical px), to find the row under a press.
    cursor: (f32, f32),
    /// The pointer shape last set on the window.
    icon: CursorIcon,
    v: View,
}

/// What a click in a popup means.
pub enum Pick {
    Menu(usize),
    Suggest(usize),
    /// A row of the modal: a switch (0..n) or, last, the Done button.
    Modal(usize),
    /// A row of a submenu.
    Sub(usize),
    /// A slider row was pressed or dragged: (row, position 0..1).
    Slide(usize, f32),
    /// The pointer moved onto row `i` of the main menu (to open or close its submenu).
    Hover(usize),
}

pub struct Popups {
    /// Draws every popup; `None` only when the machine has no usable GPU.
    engine: Option<Engine>,
    theme: Theme,
    pub menu: Popup,
    pub suggest: Popup,
    pub preview: Popup,
    pub tip: Popup,
    pub modal: Popup,
    pub scrim: Popup,
    pub sub: Popup,
    /// Photos for menu avatars, by URL.
    images: std::collections::HashMap<String, Pixmap>,
    /// For shader animation time.
    t0: Instant,
    /// When the next animation frame may be drawn, and whether something was still animating.
    /// `tick` runs after every event of the event loop; without this it would repaint on each
    /// mouse move and key press, and the GPU frames would starve input.
    next_anim: Instant,
    animating: bool,
    /// The browser window's scale factor. Popups are sized and drawn with it rather than their own:
    /// a popup waiting off-screen can sit on another monitor with another scale.
    scale: std::cell::Cell<f64>,
    last_frame: Instant,
}

impl Popups {
    pub fn new<T>(target: &EventLoopWindowTarget<T>, owner: &Window, dark: bool, brand: Brand, engine: Option<Engine>) -> Self {
        let theme = Theme::for_brand(dark, brand);
        let make = |kind| Popup::new(target, owner, kind, theme.card);
        // The scrim is created first so the modal's window ends up above it.
        let scrim = make(Kind::Scrim);
        Self {
            engine,
            scrim,
            menu: make(Kind::Menu),
            suggest: make(Kind::Suggest),
            preview: make(Kind::Preview),
            tip: make(Kind::Tip),
            modal: make(Kind::Modal),
            sub: make(Kind::Sub),
            images: std::collections::HashMap::new(),
            t0: Instant::now(),
            next_anim: Instant::now(),
            animating: false,
            scale: std::cell::Cell::new(owner.scale_factor()),
            theme,
            last_frame: Instant::now(),
        }
    }

    #[allow(dead_code)]
    pub fn set_theme(&mut self, dark: bool, brand: Brand) {
        self.set_persona_theme(dark, PersonaColor::from(brand));
    }

    pub fn set_persona_theme(&mut self, dark: bool, color: PersonaColor) {
        self.theme = Theme::for_persona(dark, color);
        let c = self.theme.card;
        for p in [&self.menu, &self.suggest, &self.preview, &self.tip, &self.modal, &self.scrim, &self.sub] {
            p.window.set_background_color(Some(window_bg(c)));
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
            Kind::Tip => &mut self.tip,
            Kind::Modal => &mut self.modal,
            Kind::Scrim => &mut self.scrim,
            Kind::Sub => &mut self.sub,
        }
    }

    /// The browser window's scale factor (call it whenever it may have changed).
    pub fn set_scale(&self, scale: f64) {
        self.scale.set(scale);
    }

    /// A photo for a menu avatar arrived.
    pub fn image_loaded(&mut self, url: String, img: Option<Pixmap>) {
        if let Some(img) = img {
            self.images.insert(url, img);
            for kind in [Kind::Menu, Kind::Sub] {
                if self.get(kind).open {
                    self.redraw(kind);
                }
            }
        }
    }

    pub fn has_image(&self, url: &str) -> bool {
        self.images.contains_key(url)
    }

    /// Where row `i` of an open popup is on screen: (card left, row top, card width), logical px.
    pub fn row_anchor(&mut self, kind: Kind, i: usize) -> Option<(f64, f64, f32)> {
        let p = self.get(kind);
        let r = p.v.rows.get(i)?;
        Some((p.card_pos.0, p.card_pos.1 + r.y as f64, p.v.size.0))
    }

    /// Window width that fits a tooltip showing `text`, measured with the real font.
    pub fn tip_width(&mut self, text: &str) -> f32 {
        let s = self.scale.get() as f32;
        let w = match &mut self.engine {
            Some(e) => e.text_width(text, TIP_FONT, Weight::Regular, s),
            None => text.chars().count() as f32 * TIP_FONT * 0.55,
        };
        (w + 22.0).ceil()
    }

    pub fn is_open(&self, kind: Kind) -> bool {
        match kind {
            Kind::Menu => self.menu.open,
            Kind::Suggest => self.suggest.open,
            Kind::Preview => self.preview.open,
            Kind::Tip => self.tip.open,
            Kind::Modal => self.modal.open,
            Kind::Scrim => self.scrim.open,
            Kind::Sub => self.sub.open,
        }
    }

    /// Shows `content` with its top-left at `pos` (logical screen coordinates), `w` wide.
    /// Returns the height it needs, already applied.
    pub fn show(&mut self, kind: Kind, content: Content, pos: LogicalPosition<f64>, w: f32) -> f32 {
        let (rows, h) = layout(&content);
        let sc = self.scale.get();
        let p = self.get(kind);
        let was_open = p.open;
        let fresh = !p.open || p.v.size != (w, h);
        // Moving between hover cards or refreshing suggestions only swaps the content, without replaying the entrance.
        if !p.open {
            p.v.enter = 0.0;
            p.v.hover = None;
        }
        p.v.selected = 0;
        // A dialog that refreshes itself while a slider is being dragged keeps the drag going.
        if !was_open || kind != Kind::Modal {
            p.v.pressed = None;
        }
        // Refreshing the content of an open popup (a toggled switch, new suggestions) keeps the hover
        // fades going instead of snapping every row back to unhovered.
        if !was_open || p.v.hover_fade.len() != rows.len() {
            p.v.hover_fade = vec![0.0; rows.len()];
        }
        p.v.rows = rows;
        p.v.shot_ready = false;
        p.v.content = content;
        p.v.size = (w, h);
        p.open = true;
        p.card_pos = (pos.x, pos.y);
        // `pos` and `w`/`h` are the card's; the window adds room for the shadow around it.
        let m = margins(kind);
        // Physical pixels at the browser window's scale, so the popup matches it on any monitor.
        let pos = PhysicalPosition::new(((pos.x - m.l as f64) * sc).round() as i32, ((pos.y - m.t as f64) * sc).round() as i32);
        let size = ((((w + m.l + m.r) as f64) * sc).round().max(1.0) as u32, (((h + m.t + m.b) as f64) * sc).round().max(1.0) as u32);
        if fresh || p.phys_size != size {
            // Resizing exposes unpainted pixels. Keep the window off-screen while it is resized and
            // painted, and only then move it into place (`redraw` applies `pending_pos`).
            p.window.set_outer_position(PARKED);
            p.window.set_inner_size(PhysicalSize::new(size.0, size.1));
            p.phys_size = size;
            p.pending_pos = Some(pos);
        } else {
            p.window.set_outer_position(pos);
        }
        // Paint now rather than waiting for the OS to deliver a redraw request: a popup must never
        // appear with stale or missing content.
        self.redraw(kind);
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
            // The big ones (the blurred backdrop covers the window; dialogs and hover cards are
            // large) give their pixel buffers back while parked, and so does the GPU.
            if matches!(kind, Kind::Scrim | Kind::Modal | Kind::Preview) {
                p.pm = Pixmap::new(1, 1).expect("1x1 pixmap");
                p.card = Pixmap::new(1, 1).expect("1x1 pixmap");
                p.frame = None;
                p.window.set_inner_size(PhysicalSize::new(1, 1));
                p.phys_size = (1, 1);
                let one = NonZeroU32::new(1).expect("1 is not zero");
                if p.surface.resize(one, one).is_ok() {
                    p.surface_size = (1, 1);
                }
                if let Some(e) = &self.engine {
                    e.trim(TRIM_PIXELS);
                    if kind == Kind::Scrim {
                        let mut e = e.clone();
                        e.forget_texture("popup:scrim");
                    }
                }
            }
        }
    }

    /// Highlights a suggestion picked with the arrow keys in the address bar.
    pub fn select(&mut self, kind: Kind, index: usize) {
        let p = self.get(kind);
        p.v.selected = index;
        p.window.request_redraw();
    }

    /// Gives a popup the keyboard focus (only dialogs can take it).
    pub fn focus(&mut self, kind: Kind) {
        let p = self.get(kind);
        if p.open {
            p.window.set_focus();
        }
    }

    pub fn owns(&self, id: WindowId) -> Option<Kind> {
        [&self.menu, &self.suggest, &self.preview, &self.tip, &self.modal, &self.scrim, &self.sub].into_iter().find(|p| p.window.id() == id).map(|p| p.kind)
    }

    /// Mouse input on a popup window. Returns a pick when a row was clicked.
    pub fn window_event(&mut self, kind: Kind, event: &WindowEvent) -> Option<Pick> {
        let sc = self.scale.get();
        let p = self.get(kind);
        match event {
            // A popup is resized in place when its content changes; repaint at the new size so no
            // stale or blank pixels show around it.
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                p.window.request_redraw();
                None
            }
            WindowEvent::CursorMoved { position, .. } => {
                let m = margins(kind);
                let pt = position.to_logical::<f32>(sc);
                let (x, y) = (pt.x - m.l, pt.y - m.t);
                p.cursor = (x, y);
                // The shadow margin is part of the window but not of the card.
                let inside = x >= 0.0 && x < p.v.size.0;
                let hover = p.v.rows.iter().position(|r| inside && row_hit(r, x, y));
                // An I-beam over the name field, a hand over anything clickable.
                let icon = match hover {
                    Some(0) if matches!(p.v.content, Content::Persona(_)) => CursorIcon::Text,
                    Some(_) if matches!(kind, Kind::Modal | Kind::Menu | Kind::Sub) => CursorIcon::Hand,
                    _ => CursorIcon::Default,
                };
                if icon != p.icon {
                    p.icon = icon;
                    p.window.set_cursor_icon(icon);
                }
                if hover != p.v.hover {
                    p.v.hover = hover;
                    if kind == Kind::Suggest
                        && let Some(i) = hover
                    {
                        p.v.selected = i;
                    }
                    p.window.request_redraw();
                    if kind == Kind::Menu {
                        return hover.map(Pick::Hover);
                    }
                }
                // Dragging a slider follows the pointer even when it leaves the track.
                p.v.pressed.and_then(|i| slide_at(&p.v.rows, i, x))
            }
            WindowEvent::CursorLeft { .. } => {
                p.v.hover = None;
                p.v.pressed = None;
                p.window.request_redraw();
                None
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                // Where the pointer really is. A popup that appeared under a still pointer never
                // got a move event, and the last one can be stale: read it from the system.
                if let (Ok(at), Ok(origin)) = (p.window.cursor_position(), p.window.inner_position()) {
                    let m = margins(kind);
                    p.cursor = (((at.x - origin.x as f64) / sc) as f32 - m.l, ((at.y - origin.y as f64) / sc) as f32 - m.t);
                }
                let (x, y) = p.cursor;
                let inside = x >= 0.0 && x < p.v.size.0;
                let under = p.v.rows.iter().position(|r| inside && row_hit(r, x, y));
                if under != p.v.hover {
                    p.v.hover = under;
                    p.window.request_redraw();
                }
                match (kind, state) {
                // Suggestions act on press: the address bar may lose focus before release.
                (Kind::Suggest, ElementState::Pressed) => p.v.hover.map(Pick::Suggest),
                (Kind::Menu | Kind::Modal | Kind::Sub, ElementState::Pressed) => {
                    p.v.pressed = p.v.hover;
                    p.window.request_redraw();
                    // A slider reports a position right away and keeps reporting while dragged.
                    p.v.pressed.and_then(|i| slide_at(&p.v.rows, i, p.cursor.0))
                }
                (Kind::Menu | Kind::Modal | Kind::Sub, ElementState::Released) => {
                    if p.v.pressed.is_some_and(|i| p.v.rows.get(i).is_some_and(|r| r.slider)) {
                        p.v.pressed = None;
                        p.window.request_redraw();
                        return None;
                    }
                    // Pressed in this popup and released on a row: that row, even if the pointer
                    // slid from one row to the next while clicking (as native menus do).
                    // A dialog's buttons need press and release on the same one.
                    let pressed = p.v.pressed.take();
                    let picked = if kind == Kind::Modal { pressed.filter(|i| Some(*i) == p.v.hover) } else { pressed.and(p.v.hover) };
                    picked.map(match kind {
                        Kind::Modal => Pick::Modal,
                        Kind::Sub => Pick::Sub,
                        _ => Pick::Menu,
                    })
                }
                _ => None,
                }
            }
            _ => None,
        }
    }

    /// Advances entrance and hover animations. Returns when the next frame is needed.
    pub fn tick(&mut self, now: Instant) -> Option<Instant> {
        if now < self.next_anim {
            return self.animating.then_some(self.next_anim);
        }
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;
        let k = 1.0 - (-dt * 30.0).exp();
        let mut busy = false;
        let mut dirty = Vec::new();
        for p in [&mut self.menu, &mut self.suggest, &mut self.preview, &mut self.tip, &mut self.modal, &mut self.scrim, &mut self.sub] {
            if !p.open {
                continue;
            }
            let mut moved = false;
            if p.v.enter < 1.0 {
                p.v.enter = (p.v.enter + dt / 0.12).min(1.0);
                moved = true;
            }
            // The persona editor's orb is a live shader: keep drawing while it is open.
            if let Content::Persona(f) = &mut p.v.content {
                f.pulse = (f.pulse - dt * 1.4).max(0.0);
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
                dirty.push(p.kind);
                busy = true;
            }
        }
        // Animation frames are painted directly; OS redraw requests can be coalesced or dropped.
        for kind in dirty {
            self.redraw(kind);
        }
        // The persona editor's live shader only needs ~30 frames a second; everything else 60.
        let shader = matches!(self.modal.v.content, Content::Persona(_)) && self.modal.open;
        let frame = if shader { std::time::Duration::from_millis(33) } else { crate::pacer::FRAME };
        self.animating = busy;
        self.next_anim = Instant::now() + frame;
        busy.then_some(self.next_anim)
    }

    /// Draws a popup and puts it on screen.
    pub fn redraw(&mut self, kind: Kind) {
        let th = self.theme;
        let engine = &mut self.engine;
        let images = &self.images;
        let p = match kind {
            Kind::Menu => &mut self.menu,
            Kind::Suggest => &mut self.suggest,
            Kind::Preview => &mut self.preview,
            Kind::Tip => &mut self.tip,
            Kind::Modal => &mut self.modal,
            Kind::Scrim => &mut self.scrim,
            Kind::Sub => &mut self.sub,
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
        let s = self.scale.get() as f32;
        let (cw, ch) = (((p.v.size.0 * s).round() as u32).max(1), ((p.v.size.1 * s).round() as u32).max(1));
        if p.card.width() != cw || p.card.height() != ch {
            let Some(pm) = Pixmap::new(cw, ch) else { return };
            p.card = pm;
        }
        draw(&mut p.v, &mut p.card, engine, images, self.t0.elapsed().as_secs_f32(), &th, s);
        compose(&p.card, &mut p.pm, &mut p.frame, engine.as_mut(), margins(kind), corner_radius(kind), kind != Kind::Scrim, s, &th);
        let (w_u32, h_u32) = (w.get(), h.get());
        if p.surface_size != (w_u32, h_u32) {
            if p.surface.resize(w, h).is_err() {
                return;
            }
            p.surface_size = (w_u32, h_u32);
        }
        let Ok(mut buf) = p.surface.buffer_mut() else { return };
        // Premultiplied 0xAARRGGBB: with a transparent window, the system blends by this alpha.
        for (d, px) in buf.iter_mut().zip(p.pm.data().chunks_exact(4)) {
            *d = (px[3] as u32) << 24 | (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32;
        }
        let _ = buf.present();
        if let Some(pos) = p.pending_pos.take() {
            p.window.set_outer_position(pos);
            // Moving it onto a monitor with another scale lets Windows resize it; put its size back.
            if (p.window.inner_size().width, p.window.inner_size().height) != p.phys_size {
                p.window.set_inner_size(PhysicalSize::new(p.phys_size.0, p.phys_size.1));
            }
        }
    }
}

impl Popup {
    fn new<T>(target: &EventLoopWindowTarget<T>, owner: &Window, kind: Kind, card: Rgba) -> Self {
        let window = Rc::new(
            WindowBuilder::new()
                .with_decorations(false)
                // tao turns the shadow frame on by default for undecorated windows. That frame
                // reserves an unpainted strip on the left, right and bottom of the client area,
                // which showed as black edges and stale content around every popup.
                .with_undecorated_shadow(false)
                // Transparent so the rounded corners and the shadow can show what is behind them.
                .with_transparent(rounded_popups())
                .with_background_color(window_bg(card))
                .with_resizable(false)
                .with_visible(true)
                .with_focused(false)
                // Dialogs take the keyboard (the persona name field); every other popup leaves it
                // with the browser window.
                .with_focusable(kind == Kind::Modal)
                .with_position(PARKED)
                .with_skip_taskbar(true)
                .with_owner_window(owner.hwnd())
                .with_inner_size(LogicalSize::new(10.0, 10.0))
                .build(target)
                .expect("failed to create popup window"),
        );
        // Tooltips and hover cards are never clicked: let clicks fall through to what is under them.
        if matches!(kind, Kind::Tip | Kind::Preview) {
            let _ = window.set_ignore_cursor_events(true);
        }
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
            card: Pixmap::new(1, 1).expect("1x1 pixmap"),
            frame: None,
            open: false,
            pending_pos: None,
            phys_size: (1, 1),
            card_pos: (0.0, 0.0),
            cursor: (-1.0, -1.0),
            icon: CursorIcon::Default,
            v: View {
                content: Content::Empty,
                rows: Vec::new(),
                size: (0.0, 0.0),
                enter: 1.0,
                hover: None,
                hover_fade: Vec::new(),
                selected: 0,
                shot_ready: false,
                pressed: None,
            },
        }
    }
}

/// Lays out rows (with their text) and returns them with the total height.
fn layout(content: &Content) -> (Vec<Row>, f32) {
    let mut rows = Vec::new();
    let mut y = PAD;
    let row = |y: f32, h: f32, label: Option<String>, hint: Option<String>, enabled: bool| Row { y, h, x: 0.0, w: 0.0, slider: false, label, icon: None, avatar: None, sub: false, tail: String::new(), hint, enabled };
    match content {
        Content::Empty => return (rows, 1.0),
        Content::Menu(items) => {
            for it in items {
                if it.sep {
                    rows.push(row(y, MENU_SEP, None, None, false));
                    y += MENU_SEP;
                    continue;
                }
                let mut r = row(y, MENU_ROW, Some(it.label.clone()), it.hint.clone(), !it.disabled);
                r.icon = it.icon;
                r.avatar = it.avatar.clone();
                r.sub = it.sub;
                rows.push(r);
                y += MENU_ROW;
            }
        }
        Content::Suggest { items, query } => {
            let q = query.to_lowercase();
            for (i, it) in items.iter().enumerate() {
                // The part after what was typed is emphasised, as most browsers do.
                let typed = i > 0 && !q.is_empty() && it.text.to_lowercase().starts_with(&q) && it.text.is_char_boundary(q.len());
                let (head, tail) = if typed { it.text.split_at(q.len()) } else { (it.text.as_str(), "") };
                let hint = if it.url { "Open site" } else { "Search" };
                let mut r = row(y, SUGGEST_ROW, Some(head.to_owned()), Some(hint.to_owned()), true);
                r.tail = tail.to_owned();
                rows.push(r);
                y += SUGGEST_ROW;
            }
        }
        Content::Preview(card) => {
            let img_h = if card.img.is_some() { SHOT_H } else { 0.0 };
            let host = if card.host.is_empty() { "New tab" } else { &card.host };
            rows.push(row(img_h, CARD_TEXT_H, Some(card.title.clone()), Some(host.to_owned()), false));
            return (rows, img_h + CARD_TEXT_H);
        }
        Content::Tip(text) => {
            rows.push(row(0.0, TIP_H, Some(text.clone()), None, false));
            return (rows, TIP_H);
        }
        Content::Scrim { h, .. } => return (rows, *h),
        Content::Persona(f) => {
            // Preview area on top (HERO_H), then the name field, color dots, two sliders, buttons.
            let mut r = row(HERO_H + 12.0, 42.0, Some("name".into()), None, true);
            (r.x, r.w) = (22.0, FORM_W - 44.0);
            rows.push(r);
            let step = (FORM_W - 44.0 - DOT) / 7.0;
            for i in 0..8 {
                let mut r = row(HERO_H + 78.0, DOT, Some(String::new()), None, true);
                (r.x, r.w) = (22.0 + i as f32 * step, DOT);
                rows.push(r);
            }
            let by = HERO_H + 210.0;
            let (cw, sw) = (button_w(&f.cancel), button_w(&f.save));
            for (k, (x, w)) in [(22.0, 96.0), (FORM_W - 22.0 - sw - 8.0 - cw, cw), (FORM_W - 22.0 - sw, sw)].into_iter().enumerate() {
                let on = match k {
                    0 => f.can_delete,
                    1 => !f.cancel.is_empty(),
                    _ => true,
                };
                let mut r = row(by, 36.0, Some(String::new()), None, on);
                (r.x, r.w) = (x, w);
                rows.push(r);
            }
            // 12 = hue, 13 = vibrancy, 14 = shuffle
            for y in [HERO_H + 134.0, HERO_H + 168.0] {
                let mut r = row(y, 24.0, Some(String::new()), None, true);
                (r.x, r.w, r.slider) = (SLIDER_X, FORM_W - 22.0 - SLIDER_X, true);
                rows.push(r);
            }
            let mut r = row(12.0, 30.0, Some(String::new()), None, true);
            (r.x, r.w) = (FORM_W - 22.0 - 30.0, 30.0);
            rows.push(r);
            return (rows, by + 36.0 + 22.0);
        }
        Content::Confirm { ok, cancel, .. } => {
            let ok_w = button_w(ok);
            let cancel_w = button_w(cancel);
            for (k, (x, w)) in [(FORM_W - 22.0 - ok_w - 8.0 - cancel_w, cancel_w), (FORM_W - 22.0 - ok_w, ok_w)].into_iter().enumerate() {
                let mut r = row(94.0, 34.0, Some(String::new()), None, k == 1 || !cancel.is_empty());
                (r.x, r.w) = (x, w);
                rows.push(r);
            }
            return (rows, CONFIRM_H);
        }
        Content::Modal { items, done, .. } => {
            y = MODAL_HEAD;
            for it in items {
                rows.push(row(y, MODAL_ROW, Some(it.label.clone()), Some(it.desc.clone()), true));
                y += MODAL_ROW;
            }
            let mut r = row(y, MODAL_FOOT, Some(String::new()), None, true);
            // Only the button itself (bottom right) is clickable.
            let bw = button_w(done);
            (r.x, r.w) = (MODAL_W - 22.0 - bw, bw);
            rows.push(r);
            return (rows, y + MODAL_FOOT);
        }
    }
    (rows, y + PAD)
}

fn draw(p: &mut View, pm: &mut Pixmap, engine: &mut Option<Engine>, images: &std::collections::HashMap<String, Pixmap>, time: f32, th: &Theme, s: f32) {
    let bg = th.card;
    let Some(engine) = engine.as_mut() else {
        pm.fill(tiny_skia::Color::from_rgba8(bg.0, bg.1, bg.2, 255));
        return;
    };
    // The page snapshot goes to the GPU once per time the card is shown.
    if !p.shot_ready {
        match &p.content {
            Content::Preview(Card { img: Some(img), .. }) => {
                engine.forget_texture("popup:shot");
                engine.ensure_texture("popup:shot", img);
                p.shot_ready = true;
            }
            Content::Scrim { img: Some(img), .. } => {
                engine.forget_texture("popup:scrim");
                engine.ensure_texture("popup:scrim", img);
                p.shot_ready = true;
            }
            _ => {}
        }
    }
    // The persona editor's picture goes to the GPU too.
    if let Content::Persona(PersonaForm { avatar_url: Some(u), .. }) = &p.content
        && let Some(img) = images.get(u)
    {
        engine.ensure_texture(&format!("av:{u}"), img);
    }
    // Menu avatars: their photos go to the GPU once.
    for r in &p.rows {
        if let Some(Avatar { url: Some(u), .. }) = &r.avatar
            && let Some(img) = images.get(u)
        {
            engine.ensure_texture(&format!("av:{u}"), img);
        }
    }
    let dark = th.fg.0 > 128;
    let hover_bg = if dark { Rgba(255, 255, 255, 20) } else { Rgba(0, 0, 0, 14) };
    let (w, h) = p.size;
    // Entrance: content eases in from slightly above while fading in over the card.
    let t = 1.0 - (1.0 - p.enter).powi(3);
    // No vertical slide: moving content a few pixels per frame made dropdowns look jittery.
    let dy = 0.0;
    let a = t;
    // Paint the whole buffer: the window can briefly be larger than `p.size` while it resizes.
    let (pw, ph) = (pm.width(), pm.height());
    let frame = engine.frame(pw, ph, s, bg, |c| {
        c.rect(0.0, 0.0, w, h, bg);
        match &p.content {
            Content::Empty => {}
            Content::Menu(_) => {
                for (i, r) in p.rows.iter().enumerate() {
                    let y = r.y + dy;
                    let Some(label) = &r.label else {
                        c.hairline_h(PAD + 6.0, y + MENU_SEP / 2.0, w - 2.0 * (PAD + 6.0), th.border.with_alpha(a));
                        continue;
                    };
                    let f = p.hover_fade.get(i).copied().unwrap_or(0.0);
                    if f > 0.0 {
                        let bg = if p.pressed == Some(i) { hover_bg.with_alpha(1.6) } else { hover_bg.with_alpha(f) };
                        c.rrect(PAD, y, w - 2.0 * PAD, r.h, 7.0, bg.with_alpha(a));
                    }
                    let fg = if r.enabled { th.fg } else { th.dim.with_alpha(0.55) }.with_alpha(a);
                    // Rows with an icon or avatar get a leading column; the rest of the labels line up.
                    let lead = p.rows.iter().any(|r| r.icon.is_some() || r.avatar.is_some());
                    let tx = PAD + 10.0 + if lead { 26.0 } else { 0.0 };
                    let cy = y + r.h / 2.0;
                    if let Some(av) = &r.avatar {
                        let key = av.url.as_ref().map(|u| format!("av:{u}")).unwrap_or_default();
                        if !c.image_cover(&key, PAD + 10.0, cy - 9.0, 18.0, 18.0, 9.0) {
                            c.circle(PAD + 19.0, cy, 9.0, av.color.with_alpha(a));
                            let lw = c.text_width(&av.letter, 10.5, Weight::SemiBold);
                            let lh = c.line_height(10.5, Weight::SemiBold);
                            c.text(&av.letter, PAD + 19.0 - lw / 2.0, cy - lh / 2.0, 10.5, Weight::SemiBold, Rgba(255, 255, 255, 255).with_alpha(a), w);
                        }
                    } else if let Some(ic) = r.icon {
                        c.icon(ic, PAD + 10.0, cy - 8.0, 16.0, if r.enabled { th.dim } else { th.dim.with_alpha(0.5) }.with_alpha(a));
                    }
                    let chev = if r.sub { 18.0 } else { 0.0 };
                    let mut hint_w = r.hint.as_deref().map_or(0.0, |h| c.text_width(h, 11.5, Weight::Regular));
                    // The label wins: when label and hint do not both fit, the hint is left out
                    // rather than cutting the label down to a few letters.
                    let label_w = c.text_width(label, 13.0, Weight::Regular);
                    let show_hint = tx + label_w + 12.0 <= w - PAD - 16.0 - hint_w - chev;
                    if !show_hint {
                        hint_w = 0.0;
                    }
                    let ty = y + (r.h - c.line_height(13.0, Weight::Regular)) / 2.0;
                    c.text(label, tx, ty, 13.0, Weight::Regular, fg, w - PAD - 16.0 - hint_w - chev);
                    if show_hint && let Some(hint) = &r.hint {
                        let hy = y + (r.h - c.line_height(11.5, Weight::Regular)) / 2.0;
                        c.text(hint, w - PAD - 10.0 - hint_w - chev, hy, 11.5, Weight::Regular, th.dim.with_alpha(a), w);
                    }
                    if r.sub {
                        c.icon(Icon::ChevronRight, w - PAD - 8.0 - 14.0, cy - 7.0, 14.0, th.dim.with_alpha(a));
                    }
                }
            }
            Content::Suggest { items, .. } => {
                for (i, r) in p.rows.iter().enumerate() {
                    let y = r.y + dy;
                    let sel = i == p.selected;
                    if sel {
                        c.rrect(PAD, y, w - 2.0 * PAD, r.h, 9.0, th.accent.with_alpha(0.16 * a));
                    } else if let Some(f) = p.hover_fade.get(i).filter(|f| **f > 0.0) {
                        c.rrect(PAD, y, w - 2.0 * PAD, r.h, 9.0, hover_bg.with_alpha(*f * a));
                    }
                    let icon = if items.get(i).is_some_and(|it| it.url) { Icon::Globe } else { Icon::Search };
                    c.icon(icon, PAD + 11.0, y + (r.h - 18.0) / 2.0, 18.0, if sel { th.accent } else { th.fg.with_alpha(0.8) }.with_alpha(a));
                    let hint_w = r.hint.as_deref().map_or(0.0, |h| c.text_width(h, 11.5, Weight::Regular));
                    if let Some(label) = &r.label {
                        let ty = y + (r.h - c.line_height(13.5, Weight::Regular)) / 2.0;
                        let right = w - PAD - 20.0 - hint_w;
                        let mut x = PAD + 40.0;
                        x += c.text(label, x, ty, 13.5, Weight::Regular, th.fg.with_alpha(a), right);
                        if !r.tail.is_empty() && x < right {
                            c.text(&r.tail, x, ty, 13.5, Weight::SemiBold, th.fg.with_alpha(a), right);
                        }
                    }
                    if (sel || p.hover == Some(i))
                        && let Some(hint) = &r.hint
                    {
                        let hy = y + (r.h - c.line_height(11.5, Weight::Regular)) / 2.0;
                        c.text(hint, w - PAD - 12.0 - hint_w, hy, 11.5, Weight::Regular, th.dim.with_alpha(a), w);
                    }
                }
            }
            Content::Scrim { img, .. } => {
                c.rect(0.0, 0.0, w, h, Rgba(20, 22, 28, 255));
                if img.is_some() {
                    // The picture is assembled with the window's exact proportions: stretch it back.
                    c.image("popup:scrim", 0.0, 0.0, w, h, 1.0);
                }
                // Dim what is behind the dialog; fades in with the popup.
                c.rect(0.0, 0.0, w, h, Rgba(0, 0, 0, 255).with_alpha(if dark { 0.42 * a } else { 0.30 * a }));
            }
            Content::Persona(f) => {
                let accent = f.accent;
                // The preview: a shader-driven glowing orb in the chosen color that flares when
                // something changes, with the persona's picture (or initial) floating in it.
                c.orb(0.0, 0.0, w, HERO_H, 0.0, time, 1.0 + 0.9 * f.pulse, accent, bg);
                let (ax, ay, ad) = (w / 2.0 - 34.0, HERO_H / 2.0 - 34.0, 68.0);
                let ring = ad / 2.0 + 3.0 + 2.5 * f.pulse;
                c.circle(ax + ad / 2.0, ay + ad / 2.0, ring, accent.with_alpha(a));
                let shown = f.avatar_url.as_ref().is_some_and(|u| images.contains_key(u));
                if !(shown && c.image_cover(&format!("av:{}", f.avatar_url.clone().unwrap_or_default()), ax, ay, ad, ad, ad / 2.0)) {
                    c.circle(ax + ad / 2.0, ay + ad / 2.0, ad / 2.0, accent.mix(Rgba(0, 0, 0, 255), 0.25).with_alpha(a));
                    let letter = f.name.trim().chars().next().map_or("P".to_string(), |ch| ch.to_uppercase().to_string());
                    let (lw, lh) = (c.text_width(&letter, 28.0, Weight::SemiBold), c.line_height(28.0, Weight::SemiBold));
                    c.text(&letter, ax + (ad - lw) / 2.0, ay + (ad - lh) / 2.0, 28.0, Weight::SemiBold, Rgba(255, 255, 255, 255).with_alpha(a), w);
                }
                c.text(&f.title, 22.0, 20.0, 12.0, Weight::SemiBold, th.fg.with_alpha(0.75 * a), w - 80.0);
                // Shuffle: a round chip with sparkles, top right.
                if let Some(r) = p.rows.get(14) {
                    let hov = p.hover_fade.get(14).copied().unwrap_or(0.0);
                    c.circle(r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0, th.fg.with_alpha((0.08 + 0.12 * hov) * a));
                    c.icon(Icon::Sparkles, r.x + 6.0, r.y + 6.0, 18.0, th.fg.with_alpha(a));
                }
                // Name field: accent rim (it always has the keyboard), text and a caret.
                if let Some(r) = p.rows.first() {
                    c.rrect(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0, 11.0, accent.with_alpha(a));
                    c.rrect(r.x, r.y, r.w, r.h, 10.0, th.field.with_alpha(a));
                    let ty = r.y + (r.h - c.line_height(14.0, Weight::Regular)) / 2.0;
                    let prefix: String = f.name.chars().take(f.caret).collect();
                    let cx = r.x + 14.0 + c.text_width(&prefix, 14.0, Weight::Regular);
                    if f.name.is_empty() {
                        c.text("Persona name", r.x + 14.0, ty, 14.0, Weight::Regular, th.faint.with_alpha(a), r.x + r.w - 10.0);
                    } else {
                        c.text(&f.name, r.x + 14.0, ty, 14.0, Weight::Regular, th.fg.with_alpha(a), r.x + r.w - 10.0);
                    }
                    let lh = c.line_height(14.0, Weight::Regular);
                    c.rect(cx, ty, 1.5, lh, th.fg.with_alpha(a));
                }
                // Color dots, the chosen preset ringed.
                for (i, col) in f.colors.iter().enumerate() {
                    let Some(r) = p.rows.get(1 + i) else { break };
                    let hov = p.hover_fade.get(1 + i).copied().unwrap_or(0.0);
                    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
                    if Some(i) == f.selected {
                        c.circle(cx, cy, DOT / 2.0 + 5.0, col.with_alpha(a));
                        c.circle(cx, cy, DOT / 2.0 + 2.5, bg);
                    }
                    c.circle(cx, cy, DOT / 2.0 - 1.0 + 1.5 * hov, col.with_alpha(a));
                }
                // The name of the color (or its hex when it is a custom one) under the dots.
                let label = match f.selected {
                    Some(i) => f.color_names.get(i).cloned().unwrap_or_default(),
                    None => format!("Custom  #{:02x}{:02x}{:02x}", accent.0, accent.1, accent.2),
                };
                let tw = c.text_width(&label, 11.5, Weight::SemiBold);
                c.text(&label, (w - tw) / 2.0, HERO_H + 78.0 + DOT + 8.0, 11.5, Weight::SemiBold, accent.with_alpha(a), w);
                // Sliders: hue (a rainbow track) and vibrancy (grey to the full color).
                for (k, (name, frac)) in [("Hue", f.hue), ("Vibrancy", f.vibrancy)].into_iter().enumerate() {
                    let Some(r) = p.rows.get(12 + k) else { continue };
                    let ty = r.y + (r.h - c.line_height(12.0, Weight::Regular)) / 2.0;
                    c.text(name, 22.0, ty, 12.0, Weight::Regular, th.dim.with_alpha(a), SLIDER_X - 6.0);
                    // A slim track; the knob travels inside it, so it never pokes past either end.
                    let (tx, tw, th_) = (r.x, r.w, 6.0);
                    let cy = r.y + r.h / 2.0;
                    let ty = cy - th_ / 2.0;
                    let at = |frac: f32| if k == 0 { crate::gfx::hsl(frac, 0.85, 0.55) } else { crate::gfx::hsl(f.hue, 0.12 + 0.88 * frac, 0.55 - 0.05 * (1.0 - frac)) };
                    if k == 0 {
                        // Six square gradient segments between two round red caps (hue 0 and 1 are
                        // both red), so no segment edge shows a notch.
                        let cap = th_ / 2.0;
                        let red = at(0.0).with_alpha(a);
                        c.circle(tx + cap, ty + cap, cap, red);
                        c.circle(tx + tw - cap, ty + cap, cap, red);
                        let (sx, sw) = (tx + cap, tw - 2.0 * cap);
                        let seg = sw / 6.0;
                        for i in 0..6 {
                            let (c1, c2) = (at(i as f32 / 6.0), at((i + 1) as f32 / 6.0));
                            c.hgrad(sx + seg * i as f32, ty, seg + 0.6, th_, 0.0, c1.with_alpha(a), c2.with_alpha(a));
                        }
                    } else {
                        c.hgrad(tx, ty, tw, th_, th_ / 2.0, at(0.0).with_alpha(a), at(1.0).with_alpha(a));
                    }
                    let frac = frac.clamp(0.0, 1.0);
                    let kx = tx + KNOB + (tw - 2.0 * KNOB) * frac;
                    let grabbed = p.pressed == Some(12 + k);
                    let hot = grabbed || p.hover == Some(12 + k);
                    let kr = KNOB + if grabbed { 1.5 } else if hot { 0.5 } else { 0.0 };
                    // A white knob on a soft shadow, filled with the color under it.
                    c.shadow(kx - kr, cy - kr, 2.0 * kr, 2.0 * kr, kr, 6.0, 1.5, if dark { 0.45 } else { 0.22 } * a);
                    c.circle(kx, cy, kr + 0.75, Rgba(0, 0, 0, 255).with_alpha(if dark { 0.30 } else { 0.10 } * a));
                    c.circle(kx, cy, kr, Rgba(255, 255, 255, 255).with_alpha(a));
                    c.circle(kx, cy, kr - 3.0, at(frac).with_alpha(a));
                }
                let btn = |c: &mut crate::anvil::Painter, r: &Row, label: &str, fill: Rgba, fg: Rgba, f: f32| {
                    c.rrect(r.x, r.y, r.w, r.h, 10.0, fill.mix(Rgba(255, 255, 255, 255), 0.12 * f).with_alpha(a));
                    let tw = c.text_width(label, 13.0, Weight::SemiBold);
                    let ty = r.y + (r.h - c.line_height(13.0, Weight::SemiBold)) / 2.0;
                    c.text(label, r.x + (r.w - tw) / 2.0, ty, 13.0, Weight::SemiBold, fg.with_alpha(a), w);
                };
                let fade = |i: usize| p.hover_fade.get(i).copied().unwrap_or(0.0);
                if f.can_delete && let Some(r) = p.rows.get(9) {
                    btn(c, r, "Delete", th.close.with_alpha(0.14), th.close, fade(9));
                }
                if !f.cancel.is_empty() && let Some(r) = p.rows.get(10) {
                    btn(c, r, &f.cancel, th.field, th.fg, fade(10));
                }
                if let Some(r) = p.rows.get(11) {
                    btn(c, r, &f.save, accent, Rgba(255, 255, 255, 255), fade(11));
                }
            }
            Content::Confirm { title, note, ok, cancel, primary } => {
                c.text(title, 22.0, 20.0, 17.0, Weight::SemiBold, th.fg.with_alpha(a), w - 22.0);
                c.text(note, 22.0, 56.0, 12.5, Weight::Regular, th.dim.with_alpha(a), w - 22.0);
                for (i, r) in p.rows.iter().enumerate() {
                    if i == 0 && cancel.is_empty() {
                        continue;
                    }
                    let f = p.hover_fade.get(i).copied().unwrap_or(0.0);
                    let strong = if *primary { th.accent } else { th.close };
                    let (label, fill, fg) = if i == 0 { (cancel.as_str(), th.field, th.fg) } else { (ok.as_str(), strong, Rgba(255, 255, 255, 255)) };
                    c.rrect(r.x, r.y, r.w, r.h, 10.0, fill.mix(Rgba(255, 255, 255, 255), 0.12 * f).with_alpha(a));
                    let tw = c.text_width(label, 13.0, Weight::SemiBold);
                    let ty = r.y + (r.h - c.line_height(13.0, Weight::SemiBold)) / 2.0;
                    c.text(label, r.x + (r.w - tw) / 2.0, ty, 13.0, Weight::SemiBold, fg.with_alpha(a), w);
                }
            }
            Content::Modal { title, note, items, done } => {
                c.text(title, 22.0, 20.0, 17.0, Weight::SemiBold, th.fg.with_alpha(a), w - 22.0);
                c.text(note, 22.0, 46.0, 12.0, Weight::Regular, th.dim.with_alpha(a), w - 22.0);
                for (i, r) in p.rows.iter().enumerate() {
                    let y = r.y + dy;
                    let f = p.hover_fade.get(i).copied().unwrap_or(0.0);
                    if i == items.len() {
                        // The finishing button, bottom right (its row is just the button).
                        let (bw, bh) = (r.w, 34.0);
                        let (bx, by) = (r.x, y + (r.h - bh) / 2.0);
                        let col = if p.pressed == Some(i) { th.accent.mix(Rgba(0, 0, 0, 255), 0.15) } else { th.accent.mix(Rgba(255, 255, 255, 255), 0.12 * f) };
                        c.rrect(bx, by, bw, bh, 10.0, col.with_alpha(a));
                        let tw = c.text_width(done, 13.0, Weight::SemiBold);
                        let ty = by + (bh - c.line_height(13.0, Weight::SemiBold)) / 2.0;
                        c.text(done, bx + (bw - tw) / 2.0, ty, 13.0, Weight::SemiBold, Rgba(255, 255, 255, 255).with_alpha(a), w);
                        continue;
                    }
                    if f > 0.0 {
                        c.rrect(12.0, y, w - 24.0, r.h - 4.0, 10.0, hover_bg.with_alpha(f * a));
                    }
                    let (ly, dyy) = (y + 9.0, y + 29.0);
                    if let Some(l) = &r.label {
                        c.text(l, 22.0, ly, 13.5, Weight::Regular, th.fg.with_alpha(a), w - 90.0);
                    }
                    if let Some(d) = &r.hint {
                        c.text(d, 22.0, dyy, 11.5, Weight::Regular, th.dim.with_alpha(a), w - 90.0);
                    }
                    // Switch.
                    let on = items.get(i).is_some_and(|it| it.on);
                    let (sw, sh) = (38.0, 22.0);
                    let (sx, sy) = (w - 22.0 - sw, y + (r.h - 4.0 - sh) / 2.0);
                    c.rrect(sx, sy, sw, sh, sh / 2.0, if on { th.accent } else { th.border.mix(th.dim, 0.35) }.with_alpha(a));
                    let kx = if on { sx + sw - sh + 3.0 } else { sx + 3.0 };
                    c.circle(kx + (sh - 6.0) / 2.0, sy + sh / 2.0, (sh - 6.0) / 2.0, Rgba(255, 255, 255, 255).with_alpha(a));
                }
            }
            Content::Tip(_) => {
                if let Some(r) = p.rows.first()
                    && let Some(label) = &r.label
                {
                    let ty = r.y + dy + (TIP_H - c.line_height(TIP_FONT, Weight::Regular)) / 2.0;
                    c.text(label, 11.0, ty, TIP_FONT, Weight::Regular, th.fg.with_alpha(a), w - 6.0);
                }
            }
            Content::Preview(card) => {
                if let Some(img) = &card.img {
                    // Cover-fit the snapshot into the top of the card.
                    let k = (w / img.width() as f32 * s).max(SHOT_H / img.height() as f32 * s);
                    let (iw, ih) = (img.width() as f32 * k / s, img.height() as f32 * k / s);
                    c.rect(0.0, 0.0, w, SHOT_H, Rgba(0, 0, 0, 255));
                    c.image("popup:shot", (w - iw) / 2.0, 0.0, iw, ih, a);
                    // Anything taller than the slot is covered by the text area below it.
                    c.rect(0.0, SHOT_H, w, h - SHOT_H, bg);
                    c.hairline_h(0.0, SHOT_H, w, th.border);
                }
                if let Some(r) = p.rows.first() {
                    let y = r.y + dy;
                    if let Some(title) = &r.label {
                        c.text(title, 14.0, y + 11.0, 13.0, Weight::SemiBold, th.fg.with_alpha(a), w - 14.0);
                    }
                    c.icon(Icon::Globe, 14.0, y + 36.0, 12.0, th.dim.with_alpha(a));
                    if let Some(host) = &r.hint {
                        c.text(host, 32.0, y + 33.5, 12.0, Weight::Regular, th.dim.with_alpha(a), w - 14.0);
                    }
                }
            }
        }
    });
    crate::anvil::copy_to_pixmap(&frame, pm);
}

/// The 0..1 position of `x` along row `i` if it is a slider.
/// Width of a dialog button for `label`: room for the text at 13px semibold, at least 92.
fn button_w(label: &str) -> f32 {
    (label.chars().count() as f32 * 7.6 + 32.0).max(92.0)
}

fn slide_at(rows: &[Row], i: usize, x: f32) -> Option<Pick> {
    let r = rows.get(i).filter(|r| r.slider)?;
    // The knob's centre travels between KNOB from either end of the track.
    let span = (r.w - 2.0 * KNOB).max(1.0);
    Some(Pick::Slide(i, ((x - r.x - KNOB) / span).clamp(0.0, 1.0)))
}

/// Whether a point (card coordinates) is on an enabled row, honouring rows narrower than the card.
fn row_hit(r: &Row, x: f32, y: f32) -> bool {
    r.enabled && y >= r.y && y < r.y + r.h && (r.w == 0.0 || (x >= r.x && x < r.x + r.w))
}

/// Background the system paints before the first frame: transparent when popups are rounded.
fn window_bg(card: Rgba) -> (u8, u8, u8, u8) {
    if rounded_popups() { (0, 0, 0, 0) } else { (card.0, card.1, card.2, 255) }
}

/// Puts the card on the window pixmap: a soft drop shadow, the card clipped to a rounded
/// rectangle, and a 1px border. Everything outside the card and its shadow stays transparent.
/// The shadow and clip mask are cached in `frame` and only rebuilt when the size changes, so an
/// animation frame is a copy, a masked blit and a border stroke.
fn compose(card: &Pixmap, out: &mut Pixmap, frame: &mut Option<Frame>, engine: Option<&mut Engine>, m: Margins, radius: f32, border: bool, s: f32, th: &Theme) {
    let (x, y) = ((m.l * s).round(), (m.t * s).round());
    let (w, h) = (card.width() as f32, card.height() as f32);
    let r = radius * s;
    let dark = th.fg.0 > 128;
    let key = (out.width(), out.height(), card.width(), card.height(), r.to_bits(), (m.l * s).to_bits() ^ (m.b * s).to_bits(), dark);
    if frame.as_ref().is_none_or(|f| f.key != key) {
        let (Some(mut shadow), Some(mut mask)) = (Pixmap::new(out.width(), out.height()), Mask::new(out.width(), out.height())) else {
            return;
        };
        let soft = if m != NO_MARGIN {
            engine.and_then(|e| e.shadow_pixmap(out.width(), out.height(), s, (x, y, w, h), r, if dark { 0.85 } else { 0.5 }))
        } else {
            None
        };
        if let Some(pm) = soft {
            shadow = pm;
        } else if m != NO_MARGIN {
            // No GPU: tiny-skia has no blur: equal-alpha layers growing outward stack into a smooth falloff.
            // The shadow sits a little low, as if lit from above.
            let layers = 16;
            let spread = ((m.b - 2.0) * s).max(1.0);
            let drop = 2.0 * s;
            let strength = if dark { 0.42 } else { 0.18 };
            for i in 1..=layers {
                let e = spread * i as f32 / layers as f32;
                let alpha = strength / layers as f32;
                if let Some(path) = rrect_path(x - e * 0.55, y - e * 0.3 + drop, w + e * 1.1, h + e * 0.8, [r + e * 0.6; 4]) {
                    let mut paint = Paint::default();
                    paint.set_color_rgba8(0, 0, 0, (alpha * 255.0).round().clamp(1.0, 255.0) as u8);
                    paint.anti_alias = true;
                    shadow.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
                }
            }
        }
        if let Some(path) = rrect_path(x, y, w, h, [r; 4]) {
            mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
        }
        *frame = Some(Frame { key, shadow, mask });
    }
    let Some(f) = frame.as_ref() else { return };
    out.data_mut().copy_from_slice(f.shadow.data());
    out.draw_pixmap(x as i32, y as i32, card.as_ref(), &PixmapPaint::default(), Transform::identity(), Some(&f.mask));
    // Border, inset half a pixel so the 1px line lands on whole pixels.
    if !border {
        return;
    }
    if let Some(path) = rrect_path(x + 0.5, y + 0.5, w - 1.0, h - 1.0, [(r - 0.5).max(0.0); 4]) {
        let mut paint = Paint::default();
        let b = th.border;
        paint.set_color_rgba8(b.0, b.1, b.2, b.3);
        paint.anti_alias = true;
        out.stroke_path(&path, &paint, &Stroke { width: 1.0, ..Stroke::default() }, Transform::identity(), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persona_dialog_rows_and_hit_testing() {
        let form = |can_delete| {
            Content::Persona(PersonaForm {
                title: "New persona".into(),
                name: String::new(),
                caret: 0,
                selected: Some(0),
                hue: 0.0,
                vibrancy: 1.0,
                accent: Rgba(255, 0, 0, 255),
                pulse: 0.0,
                colors: vec![Rgba(255, 0, 0, 255); 8],
                color_names: vec![String::new(); 8],
                avatar_url: None,
                can_delete,
                cancel: "Cancel".into(),
                save: "Save".into(),
            })
        };
        let (rows, h) = layout(&form(false));
        // name field, 8 colors, delete / cancel / save, 2 sliders, shuffle
        assert_eq!(rows.len(), 15);
        assert!(rows[12].slider && rows[13].slider && !rows[14].slider);
        assert!(matches!(slide_at(&rows, 12, rows[12].x + rows[12].w / 2.0), Some(Pick::Slide(12, f)) if (f - 0.5).abs() < 0.01));
        assert!((h - PERSONA_H).abs() < 0.5);
        assert!(!rows[9].enabled, "Delete is off when the persona cannot be deleted");
        assert!(rows[10].enabled && rows[11].enabled);
        // A press inside the second color dot hits that dot and nothing else.
        let r = &rows[2];
        let hit = |x: f32, y: f32| rows.iter().position(|r| row_hit(r, x, y));
        assert_eq!(hit(r.x + 3.0, r.y + 3.0), Some(2));
        // The gap between two dots belongs to no row.
        assert_eq!(hit(rows[1].x + rows[1].w + 2.0, rows[1].y + 5.0), None);
        let (rows, _) = layout(&form(true));
        assert!(rows[9].enabled);
        let (rows, h) = layout(&Content::Confirm { title: "t".into(), note: "n".into(), ok: "Delete".into(), cancel: "Cancel".into(), primary: false });
        assert_eq!(rows.len(), 2);
        assert!((h - CONFIRM_H).abs() < 0.5);
    }

    /// Renders each popup off-screen: `CEDERIC_SHOT=<dir> cargo test -- --ignored render_popups`.
    #[test]
    #[ignore]
    fn render_popups() {
        let dir = std::path::PathBuf::from(std::env::var("CEDERIC_SHOT").expect("CEDERIC_SHOT"));
        let mut engine = Engine::new();
        for dark in [true, false] {
        let th = Theme::new(dark);
        let s = 2.0;
        let entry = |label: &str, hint: Option<&str>, disabled| MenuEntry { label: label.into(), hint: hint.map(Into::into), disabled, sep: false, ..Default::default() };
        let sep = || MenuEntry { sep: true, disabled: true, ..Default::default() };
        let mut thumb = Pixmap::new(320, 180).unwrap();
        thumb.fill(tiny_skia::Color::from_rgba8(40, 120, 90, 255));
        let icon_entry = |label: &str, icon: Icon, hint: Option<&str>| MenuEntry { label: label.into(), hint: hint.map(Into::into), icon: Some(icon), ..Default::default() };
        let av = |l: &str, c: Rgba| Avatar { url: None, letter: l.into(), color: c };
        let toggle = |label: &str, desc: &str, on| ToggleEntry { label: label.into(), desc: desc.into(), on };
        let persona = PersonaForm {
            title: "Edit persona".into(),
            name: "Work".into(),
            caret: 4,
            selected: None,
            colors: PersonaColor::ALL.iter().map(|c| Rgba::hex(u32::from_str_radix(c.hex_string().trim_start_matches('#'), 16).unwrap())).collect(),
            color_names: PersonaColor::ALL.iter().map(|c| c.label().to_string()).collect(),
            hue: 0.52,
            vibrancy: 0.8,
            accent: crate::gfx::hsl(0.52, 0.8, 0.55),
            pulse: 0.3,
            avatar_url: None,
            can_delete: true,
            cancel: "Cancel".into(),
            save: "Save".into(),
        };
        let cases = [
            ("popup-menu", 248.0, Kind::Menu, Content::Menu(vec![
                entry("Open link in new tab", None, false),
                entry("Copy link address", None, false),
                sep(),
                entry("Back", Some("Alt+←"), true),
                entry("Reload", Some("Ctrl+R"), false),
                sep(),
                entry("Inspect", Some("F12"), false),
            ])),
            ("popup-menu-icons", 248.0, Kind::Menu, Content::Menu(vec![
                MenuEntry { label: "Merik Venne".into(), hint: Some("merik@example.com".into()), avatar: Some(av("M", Rgba::hex(0xff6a3d))), sub: true, ..Default::default() },
                sep(),
                MenuEntry { label: "Personas".into(), hint: Some("Work".into()), icon: Some(Icon::Users), sub: true, ..Default::default() },
                sep(),
                icon_entry("Set background…", Icon::Photo, None),
                icon_entry("Site permissions…", Icon::ShieldLock, None),
                icon_entry("Reload", Icon::Reload, Some("Ctrl+R")),
            ])),
            ("popup-suggest", 600.0, Kind::Suggest, Content::Suggest {
                query: "rust bor".into(),
                items: vec![
                    SuggestEntry { text: "rust bor".into(), url: false },
                    SuggestEntry { text: "rust borrow checker".into(), url: false },
                    SuggestEntry { text: "rust borrow trait".into(), url: false },
                ],
            }),
            ("popup-card", CARD_W, Kind::Preview, Content::Preview(Card { title: "Rust Programming Language".into(), host: "rust-lang.org".into(), img: Some(thumb) })),
            ("popup-tip", 150.0, Kind::Tip, Content::Tip("Developer tools (F12)".into())),
            ("popup-settings", 420.0, Kind::Modal, Content::Modal {
                title: "Site permissions".into(),
                note: "On: sites may use it without asking again. Off: the request is blocked.".into(),
                items: vec![toggle("Camera & video", "Websites may use your webcam", true), toggle("Microphone & audio", "Websites may use your microphone", false), toggle("Notifications", "Sites may send desktop notifications", true), toggle("Location", "Websites may read your location", false)],
                done: "Done".into(),
            }),
            ("popup-setup-google", FORM_W, Kind::Modal, Content::Confirm { title: "Link your Google account".into(), note: "Signed in as Ada Lovelace (ada@example.com)".into(), ok: "Continue".into(), cancel: "Back".into(), primary: true }),
            ("popup-persona", FORM_W, Kind::Modal, Content::Persona(persona)),
            ("popup-confirm", FORM_W, Kind::Modal, Content::Confirm { title: "Delete persona?".into(), note: "“Work” and its bookmarks will be removed.".into(), ok: "Delete".into(), cancel: "Cancel".into(), primary: false }),
        ];
        for (name, w, kind, content) in cases {
            let (rows, h) = layout(&content);
            let mut v = View {
                hover_fade: (0..rows.len()).map(|i| if i == 1 { 1.0 } else { 0.0 }).collect(),
                rows,
                content,
                size: (w, h),
                enter: 1.0,
                hover: Some(1),
                selected: 2,
                pressed: None,
                shot_ready: false,
            };
            let mut pm = Pixmap::new((w * s) as u32, (h * s) as u32).unwrap();
            draw(&mut v, &mut pm, &mut engine, &Default::default(), 1.0, &th, s);
            let m = margins(kind);
            let mut out = Pixmap::new(((w + m.l + m.r) * s) as u32, ((h + m.t + m.b) * s) as u32).unwrap();
            compose(&pm, &mut out, &mut None, engine.as_mut(), m, corner_radius(kind), true, s, &th);
            // Over a light page, the way it usually appears.
            let mut page = Pixmap::new(out.width(), out.height()).unwrap();
            page.fill(tiny_skia::Color::from_rgba8(236, 236, 240, 255));
            page.draw_pixmap(0, 0, out.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            let out = page;
            out.save_png(dir.join(format!("{name}{}.png", if dark { "" } else { "-light" }))).unwrap();
        }
        }
    }

    /// Opens real windows and shows each popup in them, so a window-level artifact can be looked at:
    /// `cargo test -- --ignored live_popups` and screenshot the screen while it runs.
    #[test]
    #[ignore]
    fn live_popups() {
        use tao::{
            event::Event,
            event_loop::{ControlFlow, EventLoopBuilder},
            platform::{run_return::EventLoopExtRunReturn, windows::EventLoopBuilderExtWindows},
        };
        let mut el = EventLoopBuilder::new().with_any_thread(true).build();
        let main = WindowBuilder::new().with_decorations(false).with_inner_size(LogicalSize::new(900.0, 500.0)).with_position(LogicalPosition::new(0.0, 0.0)).build(&el).unwrap();
        let mut popups = Popups::new(&el, &main, true, Brand::Cederic, Engine::new());
        let entry = |label: &str, hint: Option<&str>| MenuEntry { label: label.into(), hint: hint.map(Into::into), disabled: false, sep: false, ..Default::default() };
        popups.show(Kind::Menu, Content::Menu(vec![entry("Bookmark this page", Some("Ctrl+D")), entry("No saved bookmarks yet", None)]), LogicalPosition::new(150.0, 100.0), 248.0);
        popups.show(Kind::Suggest, Content::Suggest { query: "dsda".into(), items: (0..4).map(|i| SuggestEntry { text: format!("suggestion {i}"), url: false }).collect() }, LogicalPosition::new(150.0, 260.0), 500.0);
        let tw = popups.tip_width("Developer tools (F12)");
        popups.show(Kind::Tip, Content::Tip("Developer tools (F12)".into()), LogicalPosition::new(500.0, 100.0), tw);
        for p in [&popups.menu, &popups.suggest, &popups.tip] {
            let w = &p.window;
            eprintln!("POPUP {:?} inner={:?} outer={:?} inner_pos={:?} outer_pos={:?} scale={} decorated={}", p.kind, w.inner_size(), w.outer_size(), w.inner_position(), w.outer_position(), w.scale_factor(), w.is_decorated());
        }
        let start = Instant::now();
        el.run_return(|event, _, flow| {
            *flow = ControlFlow::WaitUntil(Instant::now() + std::time::Duration::from_millis(50));
            popups.tick(Instant::now());
            if let Event::RedrawRequested(id) = event {
                if let Some(kind) = popups.owns(id) {
                    popups.redraw(kind);
                }
            }
            if start.elapsed() > std::time::Duration::from_secs(8) {
                *flow = ControlFlow::Exit;
            }
        });
    }

    /// Times one popup frame (draw + compose) at 2x: `cargo test --release bench_popup -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_popup() {
        let mut engine = Engine::new();
        let th = Theme::new(true);
        let s = 2.0;
        let items = (0..8).map(|i| MenuEntry { label: format!("Menu entry {i}"), hint: Some("Ctrl+X".into()), ..Default::default() }).collect();
        let content = Content::Menu(items);
        let (rows, h) = layout(&content);
        let w = 248.0;
        let mut v = View { hover_fade: vec![0.0; rows.len()], rows, content, size: (w, h), enter: 0.5, hover: Some(1), selected: 0, pressed: None, shot_ready: false };
        let m = margins(Kind::Menu);
        let mut card = Pixmap::new((w * s) as u32, (h * s) as u32).unwrap();
        let mut out = Pixmap::new(((w + m.l + m.r) * s) as u32, ((h + m.t + m.b) * s) as u32).unwrap();
        let mut cache = None;
        let start = Instant::now();
        for _ in 0..120 {
            draw(&mut v, &mut card, &mut engine, &Default::default(), 0.0, &th, s);
            compose(&card, &mut out, &mut cache, engine.as_mut(), m, corner_radius(Kind::Menu), true, s, &th);
        }
        println!("popup frame (draw + compose at 2x): {:?}", start.elapsed() / 120);
    }
}
