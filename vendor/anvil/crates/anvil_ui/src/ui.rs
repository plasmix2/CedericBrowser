//! Immediate-mode UI. Each frame, widgets draw into a [`DrawList`] and report interaction.
//!
//! Widgets are identified by [`ElementId`]s (a string literal works). The id keys their animation
//! state, so every
//! interactive widget needs a unique id. Animations are frame-rate independent: each value
//! moves toward its target by `1 - exp(-rate * dt)` per frame.
//!
//! Overflow rules (`ADL.md` section 5):
//!   * Single-line text that does not fit ends in an ellipsis (`fit_text`).
//!   * Paragraphs wrap at word boundaries. Words too long for a line break by glyph.
//!   * Every text run is clipped to its container, so nothing draws outside it.
//!
//! Dropdown menus are a global overlay. [`Ui::select`] handles input and records the menu.
//! [`Ui::end`] draws every recorded menu in one pass after the rest of the frame, so a menu
//! always sits above every card and panel.

use std::collections::HashMap;

use glam::Vec2;

use crate::draw::{
    DrawList, IconInstance, ImageInstance, RectInstance, RectStyle, TextCmd, TextureId, LAYER_COUNT,
};
use crate::geom::Rect;
use crate::icons::{Icon, IconAtlas};
use crate::input::{Editor, InputKind};

pub use crate::id::ElementId;

pub mod anvl;
pub(crate) mod elements;
mod mobile;
mod motion;
mod paint;
mod toast;
mod widgets;
pub use mobile::{NavItem, Swipe};
pub use toast::{Toast, Toasts};
use crate::text::{FontRole, TextKey, TextSystem, WEIGHT_MEDIUM, WEIGHT_REGULAR};
use crate::theme::{lerp, with_alpha, Color, Theme};

/// Animation channels. Each widget keeps one animated value per channel.
pub const CH_HOVER: u8 = 0;
pub const CH_PRESS: u8 = 1;
pub const CH_FOCUS: u8 = 2;
pub const CH_TOGGLE: u8 = 3;
pub const CH_HOT: u8 = 4;
/// Popup lists (dropdowns, combo boxes, context menus) use this for their open animation.
pub const CH_POPUP: u8 = 5;
pub const CH_PULL_X: u8 = 14;
pub const CH_SLIDE: u8 = 16;
pub const CH_PULL_Y: u8 = 15;
pub const CH_IND_X: u8 = 10;
pub const CH_IND_Y: u8 = 11;
pub const CH_IND_W: u8 = 12;
pub const CH_IND_H: u8 = 13;
/// Dropdown rows use `CH_ITEM + row index`.
pub const CH_ITEM: u8 = 32;

const ELLIPSIS: &str = "\u{2026}";

/// Keys the UI reacts to, translated from window events by the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiKey {
    Backspace,
    Enter,
    Escape,
    /// Tab: move focus forward.
    Tab,
    /// Shift+Tab: move focus backward.
    BackTab,
    /// Space bar: activates the focused control.
    Space,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    Delete,
    /// Ctrl+A: select all text in the focused field.
    SelectAll,
    /// Ctrl+C: copy the selection.
    Copy,
    /// Ctrl+X: cut the selection.
    Cut,
    /// Ctrl+K: open the command palette.
    Command,
    /// Ctrl+S: save.
    Save,
    /// Ctrl+Z: undo the last edit.
    Undo,
    /// Ctrl+Y or Ctrl+Shift+Z: redo.
    Redo,
}

/// Text an input method is composing and has not committed yet: the underlined run while typing
/// Chinese, Japanese or Korean, or after a dead key. The focused text field shows it at the caret.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preedit {
    pub text: String,
    /// The input method's cursor or selection inside `text`, in bytes.
    pub cursor: Option<(usize, usize)>,
}

/// Everything the UI learns from the window during one frame.
#[derive(Default)]
pub struct Input {
    /// What an input method is composing now. It stays set across frames until it changes;
    /// the finished text arrives in `typed`.
    pub preedit: Option<Preedit>,
    /// Pointer position in window pixels.
    pub mouse: Vec2,
    /// Primary button went down this frame.
    pub pressed: bool,
    /// Primary button came up this frame.
    pub released: bool,
    /// Primary button is held down.
    pub down: bool,
    /// Printable characters typed this frame.
    pub typed: String,
    /// Keys pressed this frame.
    pub keys: Vec<UiKey>,
    /// Activation keys (Enter, Space) released this frame.
    pub key_releases: Vec<UiKey>,
    /// Shift is held.
    pub shift: bool,
    /// Ctrl is held.
    pub ctrl: bool,
    /// Clipboard text for a paste this frame (Ctrl+V). The app reads the clipboard.
    pub paste: Option<String>,
    /// Secondary (right) button went down this frame.
    pub secondary: bool,
    /// Scroll wheel movement this frame in pixels. Positive scrolls down.
    pub scroll: f32,
    /// A file is being dragged over the window.
    pub file_hover: bool,
    /// File names dropped on the window this frame.
    pub dropped: Vec<String>,
    /// The pointer is a finger. Text areas then scroll on a drag instead of selecting.
    pub touch: bool,
}

/// How a picture fits its box. Stretch fills the box, Contain keeps the whole picture, and Cover
/// fills the box and crops what sticks out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fit {
    #[default]
    Stretch,
    Contain,
    Cover,
}

/// A picture drawn in place of the OS pointer, and its height in design pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorImage {
    pub texture: TextureId,
    pub height: f32,
}

/// The pointer shape the window shows. Widgets ask for one while the pointer is over them, and
/// the app maps it to the OS cursor. Nothing else changes the cursor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Cursor {
    /// Nothing interactive under the pointer.
    #[default]
    Arrow,
    /// Anything clickable: buttons, tabs, toggles, dropdowns, chips.
    Pointer,
    /// Text fields, where the pointer places a caret.
    Text,
    /// A draggable handle: slider thumbs, range handles, scrollbar thumbs.
    Grab,
    /// A handle that is being dragged right now.
    Grabbing,
    /// A disabled control. It does nothing when clicked.
    NotAllowed,
    /// A splitter between two panes.
    ResizeHorizontal,
}

/// Layout tier, from the logical window width (ADL section 6.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Breakpoint {
    /// Under 640 logical px: one column, bottom navigation, full-width actions.
    Compact,
    /// 640 to 1024 logical px: two columns, a collapsible drawer.
    Medium,
    /// Over 1024 logical px: split panes and a persistent sidebar.
    Expanded,
}

impl Breakpoint {
    pub fn for_width(logical_width: f32) -> Self {
        if logical_width < 640.0 {
            Breakpoint::Compact
        } else if logical_width <= 1024.0 {
            Breakpoint::Medium
        } else {
            Breakpoint::Expanded
        }
    }
}

/// Safe area insets in logical pixels (ADL section 7.1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Destructive,
    Ghost,
    Success,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabState {
    Active,
    Done,
    Upcoming,
}

/// Font size is in design pixels and scales with the window.
#[derive(Clone, Copy, Debug)]
pub struct TextStyle {
    pub size: f32,
    /// CSS-style weight: 400 Regular, 500 Medium, 600 SemiBold (display sizes only).
    pub weight: u16,
    pub color: Color,
    pub role: FontRole,
    pub italic: bool,
}

impl TextStyle {
    /// UI-role text (Inter, Regular).
    pub const fn new(size: f32, color: Color) -> Self {
        Self {
            size,
            weight: WEIGHT_REGULAR,
            color,
            role: FontRole::Ui,
            italic: false,
        }
    }

    /// Medium weight, for headings, buttons, and labels.
    pub const fn medium(mut self) -> Self {
        self.weight = WEIGHT_MEDIUM;
        self
    }

    /// Body-role text (Noto Sans), for paragraphs.
    pub const fn body(mut self) -> Self {
        self.role = FontRole::Body;
        self
    }

    /// SemiBold, for display sizes (22 px and up): screen titles and headline numbers.
    pub const fn semibold(mut self) -> Self {
        self.weight = crate::text::WEIGHT_SEMIBOLD;
        self
    }

    /// Mono-role text (Fira Mono), for code and figures that must line up.
    pub const fn mono(mut self) -> Self {
        self.role = FontRole::Mono;
        self
    }

    /// Italic. Drawn with Inter Italic in the UI and body roles.
    pub const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }
}

/// A dropdown menu recorded during the frame and drawn in the global overlay pass.
struct MenuOverlay {
    id: ElementId,
    rect: Rect,
    items: Vec<String>,
    selected: usize,
    hovered: Option<usize>,
    fade: f32,
    row_h: f32,
    row_inset: f32,
    text_inset: f32,
    radius: f32,
    icon_px: f32,
}

pub struct Ui {
    pub text: TextSystem,
    pub draw: DrawList,
    /// Window size in physical pixels.
    pub size: Vec2,
    /// Physical pixels per logical pixel, from the OS scale factor. Every layout length is in
    /// logical pixels and goes through [`Ui::s`]. It never depends on the window's resolution.
    pub scale: f32,
    /// Safe area insets in logical pixels (status bar, notch, gesture bar). Zero on desktop.
    insets: Insets,
    /// Seconds since startup.
    pub time: f32,
    dt: f32,
    input: Input,
    active: Option<ElementId>,
    focus: Option<ElementId>,
    anim: HashMap<(ElementId, u8), f32>,
    /// Damped springs: position and velocity, by (id, channel).
    springs: HashMap<(ElementId, u8), (f32, f32)>,
    /// When set, motion jumps to its end state (ADL-MOBILE section 10.6).
    reduced_motion: bool,
    /// Floating surfaces blur what is beneath them (ADL-V3 section 3.5). On by default.
    backdrop_blur: bool,
    /// The design tokens every widget draws with (`Ui::theme`, `Ui::set_theme`).
    pub theme: Theme,
    /// A theme switch in progress: where it started, where it ends, and seconds elapsed.
    theme_anim: Option<(Theme, Theme, f32)>,
    /// Draw clips saved by `push_clip` and scroll areas, to restore in order.
    clip_stack: Vec<[f32; 4]>,
    /// Taffy's node store for `Ui::element`, kept between frames so layout allocates little.
    pub(crate) layout_tree: taffy::TaffyTree<usize>,
    /// Where a press started inside the left edge, while it may become a back swipe.
    edge_press: Option<Vec2>,
    icon_uvs: Vec<[f32; 4]>,
    screen: Rect,
    /// Dropdown menus recorded this frame, drawn by [`Ui::end`].
    menus: Vec<MenuOverlay>,
    /// Popup area registered this frame, and the one from the previous frame. The previous
    /// frame's area blocks pointer hover for widgets outside it.
    popup_next: Option<Rect>,
    popup_prev: Option<Rect>,
    /// Keyboard tab order as `(id, is_text_field)`, for this frame and the previous one.
    order_now: Vec<(ElementId, bool)>,
    order_prev: Vec<(ElementId, bool)>,
    /// Widget with keyboard focus, and whether the focus ring should show. The ring shows only
    /// after keyboard use, and a pointer press hides it.
    kb_focus: Option<ElementId>,
    focus_visible: bool,
    /// Caret and selection for each text field, by id.
    editors: HashMap<ElementId, Editor>,
    /// The text field that had focus at the end of the last frame. Used to put the caret at
    /// the end when a field gains focus.
    last_text_focus: Option<ElementId>,
    /// Text the app should copy to the clipboard after this frame (Ctrl+C or Ctrl+X).
    clipboard_out: Option<String>,
    /// A widget asked for the clipboard's text (the touch Paste button). The app answers by
    /// setting `Input::paste` on the next frame.
    paste_request: bool,
    /// Modal area registered this frame, and the one from the previous frame. While a modal is
    /// open, widgets outside it get no pointer input.
    modal_next: Option<Rect>,
    modal_prev: Option<Rect>,
    pub(crate) in_modal: bool,
    pub(crate) modal_scoped_next: bool,
    pub(crate) modal_scoped_prev: bool,
    pub(crate) modal_prev_layer: usize,
    /// Pointer position at the end of the last frame, for drag deltas.
    mouse_last: Vec2,
    /// Cursor requested by widgets during this frame, and the cursor chosen at the end of the
    /// last frame.
    cursor_want: Cursor,
    cursor: Cursor,
    /// Pictures that replace the OS pointer, one per shape. A shape without a picture keeps the
    /// OS pointer.
    custom_cursors: HashMap<Cursor, CursorImage>,
    /// Scroll areas by id, kept across frames.
    scrolls: HashMap<ElementId, ScrollState>,
    /// While a scroll area is open, the pointer reaches only widgets inside this rectangle.
    clip: Option<Rect>,
    /// Where the caret of the focused text field is this frame and last, for the input method's
    /// candidate window (`Ui::ime_area`).
    ime_next: Option<Rect>,
    ime_prev: Option<Rect>,
    /// A multi-line editor has focus this frame, and in the previous frame. Tab then types
    /// spaces instead of moving focus.
    capture_tab_now: bool,
    capture_tab_prev: bool,
    /// Seconds until the next frame is needed even without input: a caret blink or an animation
    /// that has not settled. `None` means the app may sleep until the next event.
    wake: Option<f32>,
    /// The pointer gesture recognizer, run in `begin`, and what it saw this frame.
    gesture: crate::gesture::Recognizer,
    gestures: Vec<crate::gesture::Gesture>,
    /// Actions the element trees emitted this frame (`Ui::take_actions`).
    actions: Vec<Box<dyn std::any::Any>>,
    /// Element trees built this frame, so a tree without an id still gets a stable one.
    pub(crate) element_calls: u64,
    /// Escape went to a handler this frame, so later trees do not see it.
    escape_taken: bool,
    /// The element press that is down: the ids it went through, target first.
    el_press: Vec<ElementId>,
    /// A long press, drag or swipe handler took that press, so its release does not click.
    el_claimed: bool,
    pub(crate) el_pulls: std::collections::HashMap<ElementId, Vec2>,
    /// Elements with hover handlers that were hovered in the last frame and so far in this one.
    el_hover_prev: std::collections::HashSet<ElementId>,
    el_hover_now: std::collections::HashSet<ElementId>,
    /// Roots of element trees above layer 0 (layer and box), from this frame and the last: the
    /// pointer does not reach a lower tree under them (`Element::layer`).
    el_covers: Vec<(u8, Rect)>,
    el_covers_prev: Vec<(u8, Rect)>,
    /// The tree being processed sits under a higher one at the pointer.
    el_covered: bool,
    /// A touch press began on an element that takes drags, so scroll containers leave it alone.
    scroll_blocked: bool,
}

/// State of one scroll area (ADL section 5.2).
#[derive(Clone, Copy, Debug, Default)]
struct ScrollState {
    /// How far the content has moved up, in physical pixels.
    offset: f32,
    /// The largest useful offset, from the last frame's content height.
    max: f32,
    viewport: Rect,
    /// Where the current press started, while it may become a drag.
    press_y: Option<f32>,
    last_y: f32,
    dragging: bool,
    /// The press that is down, or just came up, turned into a drag. Taps on rows check this so a
    /// scroll never also opens the row it started on.
    moved: bool,
    /// Fling velocity in physical px per second, positive moving content up. It is measured while
    /// a finger drags and decays after release (ADL-V3 section 4.9).
    velocity: f32,
}

/// How fast a fling slows down, per second. Higher stops sooner.
pub const FLING_FRICTION: f32 = 3.2;
/// Below this speed (logical px per second) a fling stops.
const FLING_STOP: f32 = 20.0;
/// How much of a drag past either end moves the content (ADL-MOBILE section 10.4).
pub const RUBBER_BAND: f32 = 0.55;
/// Rate at which overscroll returns to the edge after release.
const RUBBER_RETURN: f32 = 14.0;

/// One frame of momentum scrolling, shared by scroll areas and the text editor. `offset` may sit
/// outside `0..=max` while a finger pulls past an end; once released it springs back. Returns
/// true while it still needs frames.
pub(crate) fn coast(offset: &mut f32, velocity: &mut f32, max: f32, dt: f32, stop: f32) -> bool {
    if *offset < 0.0 || *offset > max {
        // Past an end: no momentum, ease back to the edge.
        *velocity = 0.0;
        let edge = offset.clamp(0.0, max);
        *offset = edge + (*offset - edge) * (-RUBBER_RETURN * dt).exp();
        if (*offset - edge).abs() < 0.5 {
            *offset = edge;
            return false;
        }
        return true;
    }
    if velocity.abs() < stop {
        *velocity = 0.0;
        return false;
    }
    *offset += *velocity * dt;
    *velocity *= (-FLING_FRICTION * dt).exp();
    if *offset < 0.0 || *offset > max {
        // A fling that reaches an end stops there instead of bouncing.
        *offset = offset.clamp(0.0, max);
        *velocity = 0.0;
    }
    true
}

/// A finger drag of `dy` while the offset is past an end moves the content less, so the edge
/// feels like it stretches.
pub(crate) fn drag_with_resistance(offset: f32, dy: f32, max: f32) -> f32 {
    let next = offset - dy;
    if next < 0.0 || next > max {
        offset - dy * RUBBER_BAND
    } else {
        next
    }
}

impl Ui {
    pub fn new(icons: &IconAtlas) -> Self {
        Self {
            text: TextSystem::new(),
            draw: DrawList::default(),
            size: Vec2::ONE,
            scale: 1.0,
            insets: Insets::default(),
            time: 0.0,
            dt: 0.0,
            input: Input::default(),
            active: None,
            focus: None,
            anim: HashMap::new(),
            springs: HashMap::new(),
            reduced_motion: false,
            backdrop_blur: true,
            theme: Theme::adl_dark(),
            theme_anim: None,
            clip_stack: Vec::new(),
            layout_tree: taffy::TaffyTree::new(),
            edge_press: None,
            icon_uvs: Icon::ALL.iter().map(|icon| icons.uv(*icon)).collect(),
            screen: Rect::default(),
            menus: Vec::new(),
            popup_next: None,
            popup_prev: None,
            order_now: Vec::new(),
            order_prev: Vec::new(),
            kb_focus: None,
            focus_visible: false,
            editors: HashMap::new(),
            last_text_focus: None,
            clipboard_out: None,
            paste_request: false,
            modal_next: None,
            modal_prev: None,
            in_modal: false,
            modal_scoped_next: false,
            modal_scoped_prev: false,
            modal_prev_layer: 0,
            mouse_last: Vec2::ZERO,
            cursor_want: Cursor::Arrow,
            cursor: Cursor::Arrow,
            custom_cursors: HashMap::new(),
            scrolls: HashMap::new(),
            clip: None,
            ime_next: None,
            ime_prev: None,
            capture_tab_now: false,
            capture_tab_prev: false,
            wake: None,
            gesture: crate::gesture::Recognizer::default(),
            gestures: Vec::new(),
            actions: Vec::new(),
            element_calls: 0,
            escape_taken: false,
            el_press: Vec::new(),
            el_claimed: false,
            el_pulls: std::collections::HashMap::new(),
            el_hover_prev: Default::default(),
            el_hover_now: Default::default(),
            el_covers: Vec::new(),
            el_covers_prev: Vec::new(),
            el_covered: false,
            scroll_blocked: false,
        }
    }

    /// Asks for another frame in `seconds` even if no input arrives. Several requests keep the
    /// soonest.
    pub fn request_wake(&mut self, seconds: f32) {
        let seconds = seconds.max(0.0);
        self.wake = Some(self.wake.map_or(seconds, |w| w.min(seconds)));
    }

    /// Asks for the next frame as soon as the display can take it: animations call this. It is
    /// not a timer at 60 Hz: vsync paces the frames, at whatever rate the display runs.
    pub fn request_frame(&mut self) {
        self.request_wake(0.0);
    }

    /// Seconds until the app must draw again without input, or `None` if it may sleep until the
    /// next event. `Some(0.0)` means draw the next frame right away. Read it after [`Ui::end`]. It is what keeps an idle app at zero CPU.
    pub fn next_wake(&self) -> Option<f32> {
        self.wake
    }

    /// Starts a frame. Call before building any widgets.
    /// `size` is the window in physical pixels. `scale_factor` is the OS scale factor, so
    /// one logical pixel is `scale_factor` physical pixels.
    pub fn begin(&mut self, input: Input, size: Vec2, scale_factor: f32, time: f32, dt: f32) {
        self.draw.clear();
        self.text.begin_frame();
        self.menus.clear();
        self.wake = None;
        self.input = input;
        self.size = size;
        self.time = time;
        self.dt = dt;
        self.scale = scale_factor.max(f32::EPSILON);
        self.step_theme_switch();
        self.screen = Rect::new(0.0, 0.0, size.x, size.y);
        self.clip = None;
        self.clip_stack.clear();
        self.in_modal = false;
        self.modal_prev_layer = 0;
        self.traverse_focus();
        self.gestures = self.gesture.update(&self.input, time, dt, self.scale);
        self.actions.clear();
        self.element_calls = 0;
        self.escape_taken = false;
        if self.input.pressed {
            self.el_press.clear();
            self.el_claimed = false;
        }
    }

    /// The pointer gestures recognized this frame: taps, long presses, drags, swipes.
    pub fn gestures(&self) -> &[crate::gesture::Gesture] {
        &self.gestures
    }

    /// Where the caret of the focused single-line text field was drawn last frame, in window
    /// px, or `None` when no text field has focus. Pass it to the window's IME cursor area so
    /// the input method's candidate list opens next to the text being typed.
    pub fn ime_area(&self) -> Option<Rect> {
        self.ime_prev
    }

    /// Emits `action` as if an element had, for immediate-mode code inside a `canvas` that
    /// should speak the same language as the rest of the tree.
    pub fn emit<A: 'static>(&mut self, action: A) {
        self.actions.push(Box::new(action));
    }

    /// [`Ui::emit`] for an action that is already boxed.
    pub(crate) fn emit_boxed(&mut self, action: Box<dyn std::any::Any>) {
        self.actions.push(action);
    }

    /// Takes the actions of type `A` that element trees emitted this frame, in order. Call it
    /// after building the frame. Actions of other types stay for their own `take_actions`.
    pub fn take_actions<A: 'static>(&mut self) -> Vec<A> {
        let mut taken = Vec::new();
        let mut rest = Vec::new();
        for a in self.actions.drain(..) {
            match a.downcast::<A>() {
                Ok(a) => taken.push(*a),
                Err(a) => rest.push(a),
            }
        }
        self.actions = rest;
        taken
    }

    /// Ends a frame. Draws the global overlay pass, releases the active widget, and trims
    /// unused text.
    pub fn end(&mut self) {
        self.flush_overlays();
        self.cursor = std::mem::take(&mut self.cursor_want);
        self.draw_cursor_image();
        self.set_layer(0);
        if self.input.released {
            self.active = None;
            self.el_press.clear();
        }
        self.popup_prev = self.popup_next.take();
        self.order_prev = std::mem::take(&mut self.order_now);
        self.last_text_focus = self.focus;
        self.modal_prev = self.modal_next.take();
        self.modal_scoped_prev = std::mem::take(&mut self.modal_scoped_next);
        self.in_modal = false;
        self.ime_prev = self.ime_next.take();
        self.el_hover_prev = std::mem::take(&mut self.el_hover_now);
        self.el_covers_prev = std::mem::take(&mut self.el_covers);
        self.capture_tab_prev = std::mem::take(&mut self.capture_tab_now);
        self.mouse_last = self.input.mouse;
        self.text.end_frame();
    }

    /// The hit box for a control: at least 44 × 44 logical px, centered on its visual (ADL
    /// section 7.2). Small visuals keep their look but stay easy to touch.
    fn touch_hit(&self, r: Rect) -> Rect {
        let min = self.s(44.0);
        let w = r.w().max(min);
        let h = r.h().max(min);
        let c = r.center();
        Rect::new(c.x - w * 0.5, c.y - h * 0.5, w, h)
    }

    /// Converts a logical-pixel length (ADL section 6.1) to physical pixels.
    pub fn s(&self, logical: f32) -> f32 {
        logical * self.scale
    }

    /// The window size in logical pixels. Breakpoints and layouts read this.
    pub fn logical_size(&self) -> Vec2 {
        self.size / self.scale
    }

    /// The layout tier for the current window width (ADL section 6.2).
    pub fn breakpoint(&self) -> Breakpoint {
        Breakpoint::for_width(self.logical_size().x)
    }

    /// Sets the safe area insets, in logical pixels. The app sets them from the platform.
    pub fn set_insets(&mut self, insets: Insets) {
        self.insets = insets;
    }

    /// The area that content may use: the window minus the safe area insets, in physical
    /// pixels (ADL section 7.1).
    pub fn safe_area(&self) -> Rect {
        let i = self.insets;
        let left = self.s(i.left);
        let top = self.s(i.top);
        let right = self.s(i.right);
        let bottom = self.s(i.bottom);
        Rect::new(
            left,
            top,
            (self.size.x - left - right).max(0.0),
            (self.size.y - top - bottom).max(0.0),
        )
    }

    /// Whole-window rectangle.
    pub fn screen_rect(&self) -> Rect {
        self.screen
    }

    /// Oscillates between 0 and 1 at `hz` cycles per second.
    pub fn pulse(&self, hz: f32) -> f32 {
        (self.time * hz * std::f32::consts::TAU).sin() * 0.5 + 0.5
    }

    /// Routes subsequent commands to a layer. Layer 1 draws above layer 0.
    pub fn set_layer(&mut self, layer: usize) {
        self.draw.current = layer.min(LAYER_COUNT - 1);
    }

    /// Declares a popup area. Widgets outside it stop receiving hover while the pointer is
    /// inside it. Takes effect from the next frame.
    pub fn pointer_overlay(&mut self, area: Rect) {
        self.popup_next = Some(area);
    }

    /// Declares a modal area. While it is open, only widgets inside it receive the pointer.
    /// Takes effect from the next frame.
    pub fn pointer_modal(&mut self, area: Rect) {
        self.modal_next = Some(area);
    }

    /// Moves keyboard focus for Tab and Shift+Tab, through last frame's tab order. Wraps at
    /// both ends. Focusing a text field also starts text entry in it.
    fn traverse_focus(&mut self) {
        self.order_now.clear();
        if self.input.pressed {
            // A press hides the keyboard ring and ends text entry. A text field pressed this
            // frame takes focus back when it is built.
            self.focus_visible = false;
            self.focus = None;
        }
        let forward = self.input.keys.contains(&UiKey::Tab) && !self.capture_tab_prev;
        let backward = self.input.keys.contains(&UiKey::BackTab) && !self.capture_tab_prev;
        if !(forward || backward) {
            return;
        }
        self.focus_visible = true;
        let n = self.order_prev.len();
        if n == 0 {
            return;
        }
        let current = self
            .order_prev
            .iter()
            .position(|(id, _)| Some(*id) == self.kb_focus);
        let next = match (current, forward) {
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
            (None, true) => 0,
            (None, false) => n - 1,
        };
        let (id, is_field) = self.order_prev[next];
        self.kb_focus = Some(id);
        self.focus = if is_field { Some(id) } else { None };
    }

    /// Adds a widget to this frame's keyboard tab order.
    pub fn register_focus(&mut self, id: impl Into<ElementId>, is_field: bool) {
        if self.modal_prev.is_some() && self.modal_scoped_prev && !self.in_modal {
            return;
        }
        let id: ElementId = id.into();
        if !self.order_now.iter().any(|(existing, _)| *existing == id) {
            self.order_now.push((id, is_field));
        }
    }

    /// True when `id` has keyboard focus and the focus ring should show.
    pub fn kb_focused(&self, id: impl Into<ElementId>) -> bool {
        let id: ElementId = id.into();
        self.focus_visible && self.kb_focus == Some(id)
    }

    /// True while `id` is held down: pressed on it and not yet released.
    pub fn is_active(&self, id: impl Into<ElementId>) -> bool {
        self.active == Some(id.into())
    }

    /// Keyboard press and release for a focused widget. Enter or Space pressed makes the widget
    /// active, so it sinks. Released while active, it clicks. Returns `(held, clicked)`.
    fn key_activation(&mut self, id: impl Into<ElementId>, kb: bool) -> (bool, bool) {
        let id: ElementId = id.into();
        if !kb {
            return (false, false);
        }
        let is_activate = |k: &UiKey| matches!(k, UiKey::Enter | UiKey::Space);
        if self.input.keys.iter().any(is_activate) {
            self.active = Some(id);
        }
        let released = self.input.key_releases.iter().any(is_activate);
        if released && self.active == Some(id) {
            self.active = None;
            return (false, true);
        }
        (self.active == Some(id), false)
    }

    /// Focus ring: a 1 px `RING` outline, drawn 1 px outside the widget.
    pub fn focus_ring(&mut self, r: Rect, radius: f32) {
        let gap = self.s(3.0);
        let ring = Rect::new(
            r.x() - gap,
            r.y() - gap,
            r.w() + gap * 2.0,
            r.h() + gap * 2.0,
        );
        let width = self.s(2.0).max(1.0);
        self.rect(
            ring,
            RectStyle::fill(with_alpha(self.theme.ring, 0.0))
                .radius(radius + gap)
                .border(self.theme.ring, width),
        );
    }

    /// Text the app should put on the clipboard, set by Ctrl+C or Ctrl+X. Returns it once.
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clipboard_out.take()
    }

    /// True once after a widget asked to paste (the touch menu's Paste). Read the clipboard and
    /// put its text in the next frame's `Input::paste`.
    pub fn take_paste_request(&mut self) -> bool {
        std::mem::take(&mut self.paste_request)
    }

    /// Starts a vertical scroll area in `viewport`. Returns the offset to apply to the content: a
    /// content point at local `y` is drawn at `viewport.y() - offset + y`. The wheel and a touch
    /// drag move the offset. While the area is open, the pointer reaches only widgets inside
    /// `viewport`, so content under a bar cannot be clicked. Close it with [`Ui::scroll_end`].
    pub fn scroll_begin(&mut self, id: impl Into<ElementId>, viewport: Rect) -> f32 {
        let modal_blocked = self.modal_prev.is_some() && self.modal_scoped_prev && !self.in_modal;
        let inside = viewport.contains(self.input.mouse) && !modal_blocked;
        let offset = self.scroll_physics(id.into(), viewport, inside);
        self.clip = Some(viewport);
        // What the area draws is clipped to it too, so a half-scrolled row never spills out.
        let prev = self.draw.push_clip(viewport);
        self.clip_stack.push(prev);
        offset
    }

    /// One frame of scrolling for `id`: wheel, drag, fling and rubber band. `owns` says the
    /// pointer is over this area and no inner area took it. Returns the new offset.
    pub(crate) fn scroll_physics(&mut self, id: ElementId, viewport: Rect, owns: bool) -> f32 {
        let mouse = self.input.mouse;
        let inside = owns;
        let slop = self.s(8.0);
        let (pressed, down, released, wheel) = (
            self.input.pressed,
            self.input.down,
            self.input.released,
            self.input.scroll,
        );
        let (dt, touch, reduced) = (self.dt, self.input.touch, self.reduced_motion);
        let stop = self.s(FLING_STOP);
        let blocked = self.scroll_blocked;
        let state = self.scrolls.entry(id).or_default();
        state.viewport = viewport;
        if touch && pressed && inside {
            state.press_y = Some(mouse.y);
            state.last_y = mouse.y;
            state.dragging = false;
            state.moved = false;
            // Touching a moving list catches it.
            state.velocity = 0.0;
        }
        if touch && down && !blocked {
            if let Some(press_y) = state.press_y {
                // A press becomes a drag once it moves past the slop, so taps still click.
                if !state.dragging && (mouse.y - press_y).abs() > slop {
                    state.dragging = true;
                    state.moved = true;
                }
                if state.dragging {
                    let dy = mouse.y - state.last_y;
                    state.offset = drag_with_resistance(state.offset, dy, state.max);
                    // Smoothed finger speed, for the fling on release.
                    if dt > 0.0 {
                        let v = -dy / dt;
                        state.velocity += (v - state.velocity) * 0.5;
                    }
                }
                state.last_y = mouse.y;
            }
        }
        if released {
            // Only a finger flings. A mouse drag stops where it is let go.
            if !(state.dragging && touch) || reduced {
                state.velocity = 0.0;
            }
            state.press_y = None;
            state.dragging = false;
        }
        if inside && wheel != 0.0 {
            state.offset = (state.offset + wheel).clamp(0.0, state.max);
            state.velocity = 0.0;
        }
        let mut moving = false;
        if !touch {
            state.offset = state.offset.clamp(0.0, state.max);
            state.velocity = 0.0;
        } else if state.dragging {
            // Held: no momentum yet, and overscroll stays where the finger has it.
        } else if reduced {
            state.offset = state.offset.clamp(0.0, state.max);
        } else {
            moving = coast(&mut state.offset, &mut state.velocity, state.max, dt, stop);
        }
        if !state.dragging && state.press_y.is_none() && !moving {
            state.offset = state.offset.clamp(0.0, state.max);
        }
        let offset = state.offset;
        if moving {
            self.request_frame();
        }
        offset
    }

    /// The scroll offset `id` had at the end of its last frame, and whether the press that is down
    /// (or just came up) dragged it.
    pub(crate) fn scroll_state(&self, id: ElementId) -> (f32, bool) {
        self.scrolls.get(&id).map_or((0.0, false), |s| (s.offset, s.moved))
    }

    /// Sets how far `id` can scroll, from this frame's content.
    pub(crate) fn set_scroll_max(&mut self, id: ElementId, max: f32) {
        self.scrolls.entry(id).or_default().max = max.max(0.0);
    }

    /// True if the press that is down, or came up this frame, was a drag that scrolled area `id`.
    /// A row inside the area ignores its click then, so scrolling never opens what the finger
    /// started on.
    pub fn scroll_moved(&self, id: impl Into<ElementId>) -> bool {
        let id: ElementId = id.into();
        self.scrolls.get(&id).is_some_and(|s| s.moved)
    }

    /// Closes the scroll area `id`. `content_bottom` is the bottom edge of the content in the
    /// window coordinates it was drawn with. It sets how far the area can scroll next frame.
    pub fn scroll_end(&mut self, id: impl Into<ElementId>, content_bottom: f32) {
        let id: ElementId = id.into();
        let mut thumb_rect = None;
        let (track_pad, bar_w, min_thumb) = (self.s(4.0), self.s(4.0), self.s(24.0));
        if let Some(state) = self.scrolls.get_mut(&id) {
            let extent = content_bottom - state.viewport.y() + state.offset;
            state.max = (extent - state.viewport.h()).max(0.0);
            if !self.input.touch {
                state.offset = state.offset.clamp(0.0, state.max);
            } else if !state.dragging && state.velocity == 0.0 && state.offset > state.max + 0.5 && self.reduced_motion {
                // Overscroll past the end is allowed while a finger pulls and while it springs back.
                state.offset = state.max;
            }
            if state.max > 1.0 && state.viewport.h() > 24.0 && extent > 0.0 {
                let vp = state.viewport;
                let track_h = (vp.h() - track_pad * 2.0).max(16.0);
                let thumb_h = ((vp.h() / extent) * track_h).clamp(min_thumb, track_h);
                let progress = (state.offset / state.max).clamp(0.0, 1.0);
                let thumb_y = vp.y() + track_pad + progress * (track_h - thumb_h);
                thumb_rect = Some(Rect::new(
                    vp.right() - bar_w - track_pad,
                    thumb_y,
                    bar_w,
                    thumb_h,
                ));
            }
        }
        if let Some(tr) = thumb_rect {
            self.rect(
                tr,
                RectStyle::fill(with_alpha(self.theme.muted, 0.45)).radius(tr.w() * 0.5),
            );
        }
        self.clip = None;
        if let Some(prev) = self.clip_stack.pop() {
            self.draw.pop_clip(prev);
        }
    }

    /// Clips what is drawn until [`Ui::pop_clip`] to `r`, inside whatever clip is in effect.
    pub fn push_clip(&mut self, r: Rect) {
        let prev = self.draw.push_clip(r);
        self.clip_stack.push(prev);
    }

    /// Ends the clip of the matching [`Ui::push_clip`].
    pub fn pop_clip(&mut self) {
        if let Some(prev) = self.clip_stack.pop() {
            self.draw.pop_clip(prev);
        }
    }

    /// True on the frame a back swipe completes. A back swipe starts within 20 logical px of the
    /// left edge and moves 80 logical px to the right, mostly along the horizontal (ADL-MOBILE 11).
    pub fn back_swipe(&mut self) -> bool {
        if self.input.pressed && self.input.mouse.x <= self.s(20.0) {
            self.edge_press = Some(self.input.mouse);
        }
        if !self.input.down {
            self.edge_press = None;
            return false;
        }
        let Some(start) = self.edge_press else {
            return false;
        };
        let moved = self.input.mouse - start;
        if moved.x >= self.s(80.0) && moved.y.abs() < moved.x * 0.5 {
            self.edge_press = None;
            return true;
        }
        false
    }

    /// True while a text field has focus. The app shows the soft keyboard then, on platforms that
    /// have one.
    pub fn text_input_active(&self) -> bool {
        self.focus.is_some()
    }

    /// The cursor the widgets chose during the last frame. The app applies it to the window.
    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    /// Asks for `cursor` this frame. Widgets call it only while the pointer is over them, and
    /// the last request in a frame wins.
    fn want_cursor(&mut self, cursor: Cursor) {
        self.cursor_want = cursor;
    }

    /// Width of the first `n` characters of `display`.
    fn prefix_width(&mut self, display: &str, n: usize, style: TextStyle) -> f32 {
        let prefix: String = display.chars().take(n).collect();
        self.text_width(&prefix, style)
    }

    /// The character index whose position is nearest to `mouse_x`, when `display` starts at
    /// `origin`.
    fn index_at(&mut self, display: &str, origin: f32, mouse_x: f32, style: TextStyle) -> usize {
        let count = display.chars().count();
        let mut best = 0;
        let mut best_dist = f32::MAX;
        for i in 0..=count {
            let x = origin + self.prefix_width(display, i, style);
            let dist = (x - mouse_x).abs();
            if dist < best_dist {
                best_dist = dist;
                best = i;
            }
        }
        best
    }

    fn line(&self) -> f32 {
        self.s(1.0).max(1.0)
    }

    fn hovered(&self, r: Rect) -> bool {
        let m = self.input.mouse;
        if !r.contains(m) {
            return false;
        }
        if let Some(clip) = self.clip {
            if !clip.contains(m) {
                return false;
            }
        }
        if let Some(modal) = self.modal_prev {
            if (self.modal_scoped_prev && !self.in_modal) || !modal.contains(r.center()) {
                return false;
            }
        }
        !matches!(
            self.popup_prev,
            Some(popup) if popup.contains(m) && !popup.contains(r.center())
        )
    }

    /// True while the primary button is held after a press that `id` took.
    fn dragging(&self, id: impl Into<ElementId>) -> bool {
        let id: ElementId = id.into();
        self.active == Some(id) && self.input.down
    }

    /// Moves the animated value for (`id`, `channel`) toward `target`. The first call starts
    /// at `target`, so nothing animates in from zero.
    pub fn animate(&mut self, id: impl Into<ElementId>, channel: u8, target: f32, rate: f32) -> f32 {
        let id: ElementId = id.into();
        if self.reduced_motion {
            self.anim.insert((id, channel), target);
            return target;
        }
        let k = 1.0 - (-rate * self.dt).exp();
        let value = self.anim.entry((id, channel)).or_insert(target);
        *value += (target - *value) * k;
        let value = *value;
        // An animation that has not arrived needs another frame, with no input to prompt it.
        if (target - value).abs() > 0.002 {
            self.request_frame();
        }
        value
    }

    /// The current value of an animated channel, if it has been set or animated.
    pub fn anim_value(&self, id: impl Into<ElementId>, channel: u8) -> Option<f32> {
        let id: ElementId = id.into();
        self.anim.get(&(id, channel)).copied()
    }

    /// Moves a damped spring toward `target` and returns its position. Overshoot and settle come
    /// from `stiffness` and `damping`. ADL-MOBILE uses about 260 and 23 for a 4% overshoot. The
    /// first call starts at `target`, so nothing moves in from zero.
    pub fn spring(
        &mut self,
        id: impl Into<ElementId>,
        channel: u8,
        target: f32,
        stiffness: f32,
        damping: f32,
    ) -> f32 {
        let id: ElementId = id.into();
        let reduced = self.reduced_motion;
        let dt = self.dt;
        let (mut pos, mut vel) = *self.springs.entry((id, channel)).or_insert((target, 0.0));
        if reduced {
            self.springs.insert((id, channel), (target, 0.0));
            return target;
        }
        // Semi-implicit Euler in steps of at most 1/240 s, so a 100 ms frame stays stable.
        let steps = (dt * 240.0).ceil().max(1.0) as u32;
        let h = dt / steps as f32;
        for _ in 0..steps {
            let accel = -stiffness * (pos - target) - damping * vel;
            vel += accel * h;
            pos += vel * h;
        }
        let moving = (pos - target).abs() > 0.0005 || vel.abs() > 0.0005;
        if moving {
            self.springs.insert((id, channel), (pos, vel));
            self.request_frame();
        } else {
            self.springs.insert((id, channel), (target, 0.0));
            pos = target;
        }
        pos
    }

    /// Puts a spring at `value` with no velocity. Use it to start a spring from somewhere other
    /// than its target.
    pub fn set_spring(&mut self, id: impl Into<ElementId>, channel: u8, value: f32) {
        let id: ElementId = id.into();
        self.springs.insert((id, channel), (value, 0.0));
    }

    /// Apple-style spring with `response` (oscillation period in seconds, e.g. 0.28s)
    /// and `damping_ratio` (1.0 = smooth critically damped, 0.76 = organic micro-overshoot).
    pub fn spring_apple(
        &mut self,
        id: impl Into<ElementId>,
        channel: u8,
        target: f32,
        response: f32,
        damping_ratio: f32,
    ) -> f32 {
        let sp = crate::theme::motion::Spring::apple(response, damping_ratio);
        self.spring(id, channel, target, sp.stiffness, sp.damping)
    }

    /// True when motion should jump to its end state.
    pub fn reduced_motion(&self) -> bool {
        self.reduced_motion
    }

    /// Turns reduced motion on or off. The app sets it from the system setting.
    pub fn set_reduced_motion(&mut self, on: bool) {
        self.reduced_motion = on;
    }

    /// Press depth for buttons: 0.035 on desktop, 0.03 on phones (ADL-MOBILE section 10.3).
    fn press_depth(&self) -> f32 {
        if self.logical_size().x < 640.0 {
            0.03
        } else {
            0.035
        }
    }

    /// Sets an animated value directly. Use it to restart a transition.
    pub fn set_anim(&mut self, id: impl Into<ElementId>, channel: u8, value: f32) {
        let id: ElementId = id.into();
        self.anim.insert((id, channel), value);
    }

    /// Hover amount in 0..1 for a widget, eased.
    pub fn hover_amount(&mut self, id: impl Into<ElementId>, r: Rect) -> f32 {
        let id: ElementId = id.into();
        let target = if self.hovered(r) { 1.0 } else { 0.0 };
        self.animate(id, CH_HOVER, target, 14.0)
    }

    // ------------------------------------------------------------------ primitives

    pub fn rect(&mut self, r: Rect, style: RectStyle) {
        self.draw.push_rect(RectInstance::new(r, style));
    }

    /// Card: `CARD` surface, 12 px radius, 1 px border. No shadow.
    pub fn card(&mut self, r: Rect) {
        let style = RectStyle::fill(self.theme.card)
            .radius(self.s(self.theme.radius.lg))
            .border(self.theme.border, self.line());
        self.rect(r, style);
    }

    pub fn divider(&mut self, x: f32, y: f32, w: f32) {
        let line = self.line();
        self.rect(Rect::new(x, y, w, line), RectStyle::fill(self.theme.border));
    }

    pub fn icon(&mut self, r: Rect, icon: Icon, tint: Color) {
        self.icon_rotated(r, icon, tint, 0.0);
    }

    /// An icon turned by `angle` radians around its center.
    pub fn icon_rotated(&mut self, r: Rect, icon: Icon, tint: Color, angle: f32) {
        let uv = self.icon_uvs[icon as usize];
        self.draw.push_icon(IconInstance {
            rect: [r.x(), r.y(), r.w(), r.h()],
            uv,
            tint,
            params: [angle, 0.0, 0.0, 0.0],
            clip: crate::draw::NO_CLIP,
        });
    }

    /// Draws a picture inside `r`. `fit` decides how its shape meets the box, and `radius`
    /// rounds the corners of the drawn area.
    pub fn image(&mut self, r: Rect, texture: TextureId, fit: Fit, radius: f32) {
        let (area, uv) = fit_image(r, texture, fit);
        let radius = self.s(radius);
        self.draw.push_image(
            texture.index,
            ImageInstance {
                rect: [area.x(), area.y(), area.w(), area.h()],
                uv,
                tint: [1.0; 4],
                params: [radius, 0.0, 0.0, 0.0],
                clip: crate::draw::NO_CLIP,
            },
        );
    }

    /// Sets (or with `None`, clears) the picture shown in place of the OS pointer while the
    /// pointer is in `shape`. For example, a hand over buttons and a grab over a dragged handle.
    pub fn set_cursor_image(&mut self, shape: Cursor, image: Option<CursorImage>) {
        match image {
            Some(image) => self.custom_cursors.insert(shape, image),
            None => self.custom_cursors.remove(&shape),
        };
    }

    /// True while the current shape has a custom picture. The app hides the OS pointer then.
    pub fn has_cursor_image(&self) -> bool {
        self.custom_cursors.contains_key(&self.cursor)
    }

    /// Draws the custom picture for the current shape at the pointer, with its top-left corner
    /// on the tip.
    fn draw_cursor_image(&mut self) {
        let Some(cursor) = self.custom_cursors.get(&self.cursor).copied() else {
            return;
        };
        let m = self.input.mouse;
        if m.x < 0.0 || m.y < 0.0 || m.x > self.size.x || m.y > self.size.y {
            return;
        }
        let h = self.s(cursor.height);
        let aspect = cursor.texture.width as f32 / cursor.texture.height.max(1) as f32;
        self.set_layer(1);
        self.image(
            Rect::new(m.x, m.y, h * aspect, h),
            cursor.texture,
            Fit::Stretch,
            0.0,
        );
        self.set_layer(0);
    }

    /// Shapes single-line `content`. Returns its key and size in pixels.
    pub fn measure(&mut self, content: &str, style: TextStyle) -> (TextKey, Vec2) {
        let px = self.s(style.size);
        let key = self
            .text
            .layout_rich(content, px, style.weight, style.role, style.italic, None, &[]);
        let width = self.text.width(&key);
        (key, Vec2::new(width, px * style.role.line_height()))
    }

    /// Shapes `content` wrapped to `max_w`. Returns its key and size in pixels.
    pub fn measure_wrapped(&mut self, content: &str, style: TextStyle, max_w: f32) -> (TextKey, Vec2) {
        let px = self.s(style.size);
        let key = self
            .text
            .layout_rich(content, px, style.weight, style.role, style.italic, Some(max_w), &[]);
        let lines = self.text.lines(&key) as f32;
        let width = self.text.width(&key).min(max_w);
        (key, Vec2::new(width, lines * px * style.role.line_height()))
    }

    /// Width of `content` in pixels, unwrapped.
    pub fn text_width(&mut self, content: &str, style: TextStyle) -> f32 {
        self.measure(content, style).1.x
    }

    /// The longest prefix of `content` that fits in `max_w` with an ellipsis, or all of
    /// `content` if it already fits. Returns an empty string if even the ellipsis does not fit.
    pub fn fit_text(&mut self, content: &str, style: TextStyle, max_w: f32) -> String {
        if max_w <= 0.0 {
            return String::new();
        }
        if self.text_width(content, style) <= max_w {
            return content.to_string();
        }
        if self.text_width(ELLIPSIS, style) > max_w {
            return String::new();
        }
        let chars: Vec<char> = content.chars().collect();
        // Binary search for the longest prefix whose ellipsized form still fits.
        let (mut lo, mut hi) = (0usize, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let prefix: String = chars[..mid].iter().collect();
            let candidate = format!("{}{}", prefix.trim_end(), ELLIPSIS);
            if self.text_width(&candidate, style) <= max_w {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        let prefix: String = chars[..lo].iter().collect();
        format!("{}{}", prefix.trim_end(), ELLIPSIS)
    }

    /// Draws text at (x, y), clipped to `clip`. Returns its width.
    pub fn text_clipped(
        &mut self,
        x: f32,
        y: f32,
        content: &str,
        style: TextStyle,
        clip: Rect,
    ) -> f32 {
        let (key, size) = self.measure(content, style);
        self.draw.push_text(TextCmd {
            key,
            x,
            y,
            color: style.color,
            clip,
        });
        size.x
    }

    /// Draws text at (x, y), clipped to the window. Returns its width.
    pub fn text(&mut self, x: f32, y: f32, content: &str, style: TextStyle) -> f32 {
        let clip = self.screen;
        self.text_clipped(x, y, content, style, clip)
    }

    /// Draws `content` at (x, y), ellipsized to fit in `max_w`. Returns the drawn width.
    pub fn text_fit(&mut self, x: f32, y: f32, content: &str, style: TextStyle, max_w: f32) -> f32 {
        let shown = self.fit_text(content, style, max_w);
        let (key, size) = self.measure(&shown, style);
        let clip = Rect::new(x, y, max_w.max(0.0), size.y.max(1.0));
        self.draw.push_text(TextCmd {
            key,
            x,
            y,
            color: style.color,
            clip,
        });
        size.x
    }

    /// Draws a paragraph wrapped to `max_w`, starting at (x, y). Returns its height.
    pub fn text_wrapped(&mut self, x: f32, y: f32, max_w: f32, content: &str, style: TextStyle) -> f32 {
        let (key, size) = self.measure_wrapped(content, style, max_w);
        let clip = Rect::new(x, y, max_w.max(0.0), size.y.max(1.0));
        self.draw.push_text(TextCmd {
            key,
            x,
            y,
            color: style.color,
            clip,
        });
        size.y
    }

    /// Paragraph wrapped to the width of `r` and clipped to `r`, so it never leaves its
    /// container. Draws from the top of `r`. Returns the height the text needs.
    pub fn text_wrapped_in(&mut self, r: Rect, content: &str, style: TextStyle) -> f32 {
        let (key, size) = self.measure_wrapped(content, style, r.w());
        self.draw.push_text(TextCmd {
            key,
            x: r.x(),
            y: r.y(),
            color: style.color,
            clip: r,
        });
        size.y
    }

    /// Reports a click on any region, with the same press-and-release rule as buttons.
    pub fn click_region(&mut self, id: impl Into<ElementId>, r: Rect) -> bool {
        let id: ElementId = id.into();
        let inside = self.hovered(r);
        if inside {
            self.want_cursor(Cursor::Pointer);
        }
        if inside && self.input.pressed {
            self.active = Some(id);
        }
        inside && self.input.released && self.active == Some(id)
    }

    /// Centers ellipsized `content` inside `r`.
    pub fn text_center(&mut self, r: Rect, content: &str, style: TextStyle) {
        let shown = self.fit_text(content, style, r.w());
        let (key, size) = self.measure(&shown, style);
        let x = r.x() + (r.w() - size.x) * 0.5;
        let y = r.y() + (r.h() - size.y) * 0.5;
        self.draw.push_text(TextCmd {
            key,
            x,
            y,
            color: style.color,
            clip: r,
        });
    }

    // ------------------------------------------------------------------ widgets

    /// Button. Returns true on the frame it is clicked (pressed and released over it).
    /// Pass an empty `label` for an icon-only button.
    pub fn button(
        &mut self,
        id: impl Into<ElementId>,
        r: Rect,
        label: &str,
        icon: Option<Icon>,
        variant: ButtonVariant,
        enabled: bool,
    ) -> bool {
        let id: ElementId = id.into();
        if enabled {
            self.register_focus(id, false);
        }
        let kb = enabled && self.kb_focused(id);
        let (kheld, kclicked) = self.key_activation(id, kb);
        if self.hovered(r) {
            self.want_cursor(if enabled {
                Cursor::Pointer
            } else {
                Cursor::NotAllowed
            });
        }
        let inside = enabled && self.hovered(r);
        let hover = self.animate(id, CH_HOVER, if inside { 1.0 } else { 0.0 }, 14.0);
        if inside && self.input.pressed {
            self.active = Some(id);
        }
        let held = (inside && self.active == Some(id)) || kheld;
        let clicked = (inside && self.input.released && self.active == Some(id)) || kclicked;

        let press = self.animate(id, CH_PRESS, if held { 1.0 } else { 0.0 }, 30.0);
        let br = r.scaled(1.0 + 0.01 * hover - self.press_depth() * press);
        let radius = self.s(self.theme.radius.md).min(br.h() * 0.5);
        let line = self.line();

        // Flat fills. Hover darkens a solid fill slightly, or tints a transparent one.
        let (fill, fg, border) = match (variant, enabled) {
            (_, false) => (self.theme.disabled_bg, self.theme.disabled, None),
            (ButtonVariant::Primary, true) => (
                lerp(self.theme.primary, self.theme.text_dim, 0.12 * hover),
                self.theme.primary_fg,
                None,
            ),
            (ButtonVariant::Secondary, true) => (
                lerp(self.theme.card_hi, self.theme.border, 0.5 * hover),
                self.theme.text,
                None,
            ),
            (ButtonVariant::Destructive, true) => (
                lerp(self.theme.danger, self.theme.bg, 0.12 * hover),
                self.theme.on_color,
                None,
            ),
            (ButtonVariant::Success, true) => (
                lerp(self.theme.success, self.theme.bg, 0.12 * hover),
                self.theme.on_color,
                None,
            ),
            (ButtonVariant::Ghost, true) => (
                with_alpha(self.theme.card_hi, hover),
                self.theme.text,
                None,
            ),
        };

        let mut style = RectStyle::fill(fill).radius(radius);
        if let Some(color) = border {
            style = style.border(color, line);
        }
        self.rect(br, style);
        if kb {
            self.focus_ring(br, radius);
        }

        // Label and icon, ellipsized so they stay inside the button. Desktop buttons (< 42pt)
        // use 13.5pt text and tighter padding so labels fit cleanly without premature ellipsis.
        let compact_btn = r.h() < self.s(42.0);
        let pad = self.s(if compact_btn { 10.0 } else { 16.0 }).min(br.w() * 0.2);
        let isz = self.s(if compact_btn { 15.0 } else { 18.0 });
        let gap = self.s(if compact_btn { 6.0 } else { 8.0 });
        let icon_w = if icon.is_some() { isz } else { 0.0 };
        let gap_w = if icon.is_some() && !label.is_empty() {
            gap
        } else {
            0.0
        };
        let label_style = TextStyle::new(if compact_btn { 13.5 } else { 16.0 }, fg).medium();
        let max_label = (br.w() - pad * 2.0 - icon_w - gap_w).max(0.0);
        let shown = self.fit_text(label, label_style, max_label);
        let (key, tsize) = self.measure(&shown, label_style);
        let total = icon_w + gap_w + tsize.x;
        let mut x = br.center().x - total * 0.5;
        let cy = br.center().y;
        if let Some(icon) = icon {
            self.icon(Rect::new(x, cy - isz * 0.5, isz, isz), icon, fg);
            x += isz + gap_w;
        }
        if !shown.is_empty() {
            self.draw.push_text(TextCmd {
                key,
                x,
                y: cy - tsize.y * 0.5,
                color: fg,
                clip: br,
            });
        }
        clicked
    }

    /// Single-line text field. Click to focus. Typed characters that pass `accept` are
    /// appended, up to `max_len`. Enter or Escape leaves the field. Unfocused values are
    /// ellipsized. Focused values scroll so the caret stays visible. The caller draws the
    /// field's label above `r`.
    /// Text input. Click to place the caret, and drag to select. Typing replaces the selection.
    /// Arrows move the caret (Shift extends the selection, Ctrl jumps by word), and Home and
    /// End go to the ends. Backspace and Delete remove text. Ctrl+A, Ctrl+C, Ctrl+X, and Ctrl+V
    /// select all, copy, cut, and paste. `kind` decides what may be typed. Unfocused values are
    /// ellipsized, and number fields are brought back into range when they lose focus. The
    /// caller draws the field's label above `r`.
    #[allow(clippy::too_many_arguments)]
    pub fn text_field(
        &mut self,
        id: impl Into<ElementId>,
        r: Rect,
        value: &mut String,
        max_len: usize,
        kind: InputKind,
        placeholder: &str,
        icon: Option<Icon>,
    ) {
        let id: ElementId = id.into();
        self.register_focus(id, true);
        let pressed_here = self.input.pressed && self.hovered(r);
        if self.hovered(r) {
            self.want_cursor(Cursor::Text);
        }
        if pressed_here {
            self.focus = Some(id);
            self.active = Some(id);
        }
        let focused = self.focus == Some(id);

        // Frame: the same border, focus ring, and text box as the other inputs.
        let f = self.animate(id, CH_FOCUS, if focused { 1.0 } else { 0.0 }, 14.0);
        let radius = self.s(self.theme.radius.md).min(r.h() * 0.5);
        let line = self.line();
        let border = lerp(self.theme.border, self.theme.ring, f);
        self.rect(
            r,
            RectStyle::fill(self.theme.sunken)
                .radius(radius)
                .border(border, line),
        );
        let pad = self.s(14.0);
        let isz = self.s(18.0);
        let mut x = r.x() + pad;
        if let Some(icon) = icon {
            let tint = lerp(self.theme.muted, self.theme.text, f);
            self.icon(Rect::new(x, r.y() + (r.h() - isz) * 0.5, isz, isz), icon, tint);
            x += isz + self.s(10.0);
        }
        let style = TextStyle::new(16.0, self.theme.text);
        let line_h = self.s(16.0) * FontRole::Ui.line_height();
        let y = r.y() + (r.h() - line_h) * 0.5;
        let clip = Rect::new(x, r.y(), (r.right() - pad - x).max(0.0), r.h());

        // A number that has just lost focus is brought into range and formatted.
        if !focused {
            if let Some(tidy) = kind.normalize(value) {
                *value = tidy;
            }
        }

        // Focus arriving from elsewhere puts the caret at the end.
        let mut ed = self.editors.remove(&id).unwrap_or_default();
        if focused && self.last_text_focus != Some(id) {
            ed = Editor::at_end(value.chars().count());
        }
        ed.clamp(value.chars().count());

        // Pointer: a press places the caret, and dragging extends the selection.
        let dragging = focused && self.active == Some(id) && self.input.down;
        let display = if kind.is_password() {
            "\u{2022}".repeat(value.chars().count())
        } else {
            value.clone()
        };
        if pressed_here || dragging {
            let idx = self.index_at(&display, x, self.input.mouse.x, style);
            let extend = if pressed_here { self.input.shift } else { true };
            ed.move_to(idx, extend);
        }

        // Keyboard, paste, and typing, only while focused.
        if focused {
            if let Some(paste) = self.input.paste.clone() {
                let clean: String = paste.chars().filter(|c| !c.is_control()).collect();
                ed.insert(value, &clean, max_len, &kind);
            }
            for c in self.input.typed.clone().chars() {
                let mut buf = [0u8; 4];
                ed.insert(value, c.encode_utf8(&mut buf), max_len, &kind);
            }
            let word = self.input.ctrl;
            let extend = self.input.shift;
            let mut copied: Option<String> = None;
            for key in self.input.keys.clone() {
                match key {
                    UiKey::Backspace => ed.backspace(value, word),
                    UiKey::Delete => ed.delete_forward(value, word),
                    UiKey::ArrowLeft => ed.left(value, extend, word),
                    UiKey::ArrowRight => ed.right(value, extend, word),
                    UiKey::Home => ed.home(extend),
                    UiKey::End => ed.end(value, extend),
                    UiKey::SelectAll => ed.select_all(value.chars().count()),
                    UiKey::Copy => {
                        if ed.has_selection() {
                            copied = Some(ed.selected_text(value));
                        }
                    }
                    UiKey::Cut => {
                        if ed.has_selection() {
                            copied = Some(ed.selected_text(value));
                            ed.delete_selection(value);
                        }
                    }
                    UiKey::ArrowUp | UiKey::ArrowDown => {
                        let delta = if key == UiKey::ArrowUp { 1.0 } else { -1.0 };
                        if let Some(next) = kind.step(value, delta) {
                            *value = next;
                            ed.end(value, false);
                        }
                    }
                    UiKey::Enter | UiKey::Escape => self.focus = None,
                    _ => {}
                }
            }
            if copied.is_some() {
                self.clipboard_out = copied;
            }
        }

        ed.clamp(value.chars().count());
        self.editors.insert(id, ed);

        let display = if kind.is_password() {
            "\u{2022}".repeat(value.chars().count())
        } else {
            value.clone()
        };
        if value.is_empty() && !focused {
            let hint_style = TextStyle::new(16.0, self.theme.muted);
            let hint = self.fit_text(placeholder, hint_style, clip.w());
            self.text_clipped(x, y, &hint, hint_style, clip);
            return;
        }
        if !focused {
            self.text_fit(x, y, &display, style, clip.w());
            return;
        }

        // Focused: draw the selection, then the text scrolled so the caret stays in view. Text an
        // input method is composing sits at the caret, underlined, and the caret follows it.
        let (sel_lo, sel_hi) = ed.selection();
        let composing = self.input.preedit.as_ref().map(|p| p.text.clone()).filter(|t| !t.is_empty());
        let (display, caret_i, compose_span) = match &composing {
            Some(p) => {
                let at = display.char_indices().nth(ed.caret).map_or(display.len(), |(b, _)| b);
                let mut shown = display.clone();
                shown.insert_str(at, p);
                let n = p.chars().count();
                (shown, ed.caret + n, Some((ed.caret, ed.caret + n)))
            }
            None => (display, ed.caret, None),
        };
        let caret_x = self.prefix_width(&display, caret_i, style);
        let caret_w = self.line().max(self.s(2.0));
        let shift = (clip.w() - caret_x - caret_w - self.s(2.0)).min(0.0);
        self.ime_next = Some(Rect::new(x + shift + caret_x, y, caret_w, line_h));
        if sel_hi > sel_lo && compose_span.is_none() {
            let x0 = (x + shift + self.prefix_width(&display, sel_lo, style)).max(clip.x());
            let x1 = (x + shift + self.prefix_width(&display, sel_hi, style)).min(clip.right());
            if x1 > x0 {
                self.rect(
                    Rect::new(x0, y, x1 - x0, line_h),
                    RectStyle::fill(self.theme.selection).radius(self.s(2.0)),
                );
            }
        }
        let (key, _) = self.measure(&display, style);
        self.draw.push_text(TextCmd {
            key,
            x: x + shift,
            y,
            color: self.theme.text,
            clip,
        });
        if let Some((from, to)) = compose_span {
            let x0 = x + shift + self.prefix_width(&display, from, style);
            let x1 = x + shift + self.prefix_width(&display, to, style);
            let thick = self.line();
            self.rect(Rect::new(x0, y + line_h - thick, x1 - x0, thick), RectStyle::fill(self.theme.text));
        }
        if (self.time * 1.2).fract() < 0.6 {
            let cx = x + shift + caret_x;
            self.rect(
                Rect::new(cx, y, caret_w, line_h),
                RectStyle::fill(self.theme.text).radius(caret_w * 0.5),
            );
        }
    }

    /// Horizontal slider for an integer `0..=max`. Press on the track to jump to that value,
    /// then hold and drag to follow the pointer. Arrow keys step it when it has keyboard focus.
    /// Returns true when the value changed this frame.
    pub fn slider(
        &mut self,
        id: impl Into<ElementId>,
        r: Rect,
        value: &mut u8,
        max: u8,
        color: Color,
    ) -> bool {
        let id: ElementId = id.into();
        let hit = Rect::new(r.x(), r.y() - self.s(12.0), r.w(), r.h() + self.s(24.0));
        self.register_focus(id, false);
        let kb = self.kb_focused(id);
        let over = self.hovered(hit);
        if self.input.pressed && over {
            self.active = Some(id);
        }
        // Held after a press on this slider: the value follows the pointer's x.
        let dragging = self.dragging(id);
        if dragging {
            self.want_cursor(Cursor::Grabbing);
        } else if over {
            self.want_cursor(Cursor::Pointer);
        }

        let mut changed = false;
        if dragging {
            let t = ((self.input.mouse.x - r.x()) / r.w().max(1.0)).clamp(0.0, 1.0);
            let v = (t * max as f32).round() as u8;
            if v != *value {
                *value = v;
                changed = true;
            }
        }
        if kb {
            if self.input.keys.contains(&UiKey::ArrowLeft) && *value > 0 {
                *value -= 1;
                changed = true;
            }
            if self.input.keys.contains(&UiKey::ArrowRight) && *value < max {
                *value += 1;
                changed = true;
            }
        }

        let hot = self.animate(id, CH_HOT, if over || dragging { 1.0 } else { 0.0 }, 18.0);
        let t = *value as f32 / max.max(1) as f32;
        let track_h = self.s(6.0);
        let track = Rect::new(r.x(), r.y() + (r.h() - track_h) * 0.5, r.w(), track_h);
        self.rect(
            track,
            RectStyle::fill(self.theme.border).radius(track_h * 0.5),
        );

        let fill_w = track.w() * t;
        if fill_w > 0.5 {
            self.rect(
                Rect::new(track.x(), track.y(), fill_w, track_h),
                RectStyle::fill(color).radius(track_h * 0.5),
            );
        }

        let d = self.s(16.0) + self.s(2.0) * hot;
        let cx = track.x() + fill_w;
        let cy = track.y() + track_h * 0.5;
        let line = self.line();
        self.rect(
            Rect::new(cx - d * 0.5, cy - d * 0.5, d, d),
            RectStyle::fill(color).radius(d * 0.5).border(self.theme.bg, line),
        );
        if kb {
            self.focus_ring(track, track.h() * 0.5);
        }
        changed
    }

    /// On/off switch. Returns true on the frame it is clicked. The caller flips its value.
    pub fn toggle(&mut self, id: impl Into<ElementId>, r: Rect, on: bool) -> bool {
        let id: ElementId = id.into();
        self.register_focus(id, false);
        let kb = self.kb_focused(id);
        let (_, kclicked) = self.key_activation(id, kb);
        let over = self.hovered(self.touch_hit(r));
        if over && self.input.pressed {
            self.active = Some(id);
        }
        let clicked = (over && self.input.released && self.active == Some(id)) || kclicked;
        if over {
            self.want_cursor(Cursor::Pointer);
        }

        let t = self.animate(id, CH_TOGGLE, if on { 1.0 } else { 0.0 }, 20.0);
        let track = lerp(self.theme.card_hi, self.theme.primary, t);
        self.rect(r, RectStyle::fill(track).radius(r.h() * 0.5));

        let pad = self.s(3.0);
        let d = r.h() - pad * 2.0;
        let kx = r.x() + pad + t * (r.w() - d - pad * 2.0);
        let knob = lerp(self.theme.text, self.theme.primary_fg, t);
        self.rect(
            Rect::new(kx, r.y() + pad, d, d),
            RectStyle::fill(knob).radius(d * 0.5),
        );
        if kb {
            self.focus_ring(r, r.h() * 0.5);
        }
        clicked
    }

    /// Kiosk step tab. Only `Done` tabs can be clicked, to go back.
    pub fn tab(&mut self, id: impl Into<ElementId>, r: Rect, icon: Icon, label: &str, state: TabState) -> bool {
        let id: ElementId = id.into();
        let enabled = state == TabState::Done;
        if enabled {
            self.register_focus(id, false);
        }
        let kb = enabled && self.kb_focused(id);
        let (kheld, kclicked) = self.key_activation(id, kb);
        let over = enabled && self.hovered(self.touch_hit(r));
        if over {
            self.want_cursor(Cursor::Pointer);
        }
        let hover = self.animate(id, CH_HOVER, if over { 1.0 } else { 0.0 }, 14.0);
        if over && self.input.pressed {
            self.active = Some(id);
        }
        let clicked = (over && self.input.released && self.active == Some(id)) || kclicked;

        // Pressing sinks the pill, the same depth as a button.
        let held = (over && self.active == Some(id)) || kheld;
        let press = self.animate(id, CH_PRESS, if held { 1.0 } else { 0.0 }, 30.0);
        let r = r.scaled(1.0 - 0.035 * press);

        let radius = r.h() * 0.5;
        let (fill, fg, icon_tint) = match state {
            TabState::Active => (self.theme.primary, self.theme.primary_fg, self.theme.primary_fg),
            TabState::Done => (
                lerp(self.theme.card_hi, self.theme.border, 0.5 * hover),
                self.theme.text,
                self.theme.success,
            ),
            TabState::Upcoming => (
                with_alpha(self.theme.card, 0.0),
                self.theme.muted,
                self.theme.muted,
            ),
        };
        self.rect(r, RectStyle::fill(fill).radius(radius));

        let shown_icon = if state == TabState::Done {
            Icon::Check
        } else {
            icon
        };
        let isz = self.s(18.0);
        let gap = self.s(8.0);
        let label_style = TextStyle::new(15.0, fg).medium();
        let max_label = (r.w() - isz - gap - self.s(16.0)).max(0.0);
        let shown = self.fit_text(label, label_style, max_label);
        let (key, tsize) = self.measure(&shown, label_style);
        let mut x = r.center().x - (isz + gap + tsize.x) * 0.5;
        let cy = r.center().y;
        self.icon(Rect::new(x, cy - isz * 0.5, isz, isz), shown_icon, icon_tint);
        x += isz + gap;
        self.draw.push_text(TextCmd {
            key,
            x,
            y: cy - tsize.y * 0.5,
            color: fg,
            clip: r,
        });
        if kb {
            self.focus_ring(r, r.h() * 0.5);
        }
        clicked
    }

    /// Segment of a segmented control. Draw the control's [`Ui::indicator`] first, then the
    /// segments. Returns true when clicked.
    pub fn segment(&mut self, id: impl Into<ElementId>, r: Rect, label: &str, active: bool) -> bool {
        let id: ElementId = id.into();
        self.register_focus(id, false);
        let kb = self.kb_focused(id);
        let (kheld, kclicked) = self.key_activation(id, kb);
        let over = self.hovered(self.touch_hit(r));
        if over {
            self.want_cursor(Cursor::Pointer);
        }
        let hover = self.animate(id, CH_HOVER, if over { 1.0 } else { 0.0 }, 14.0);
        if over && self.input.pressed {
            self.active = Some(id);
        }
        let clicked = (over && self.input.released && self.active == Some(id)) || kclicked;

        // Pressing sinks the segment: the pressed fill shrinks and deepens.
        let held = (over && self.active == Some(id)) || kheld;
        let press = self.animate(id, CH_PRESS, if held { 1.0 } else { 0.0 }, 30.0);
        if press > 0.01 {
            let sunk = r.scaled(1.0 - 0.04 * press);
            self.rect(
                sunk,
                RectStyle::fill(with_alpha(self.theme.card_hi, 0.8 * press))
                    .radius(self.s(self.theme.radius.md)),
            );
        }

        let fg = if active {
            self.theme.text
        } else {
            lerp(self.theme.muted, self.theme.text, hover)
        };
        let label_style = TextStyle::new(15.0, fg).medium();
        let shown = self.fit_text(label, label_style, (r.w() - self.s(16.0)).max(0.0));
        let (key, tsize) = self.measure(&shown, label_style);
        let x = r.center().x - tsize.x * 0.5;
        let y = r.center().y - tsize.y * 0.5;
        self.draw.push_text(TextCmd {
            key,
            x,
            y,
            color: fg,
            clip: r,
        });
        if kb {
            self.focus_ring(r, self.s(self.theme.radius.md));
        }
        clicked
    }

    /// Highlight behind a segmented control, drawn at `rect`. The caller eases the position
    /// in index space and converts it to `rect`, so a window resize re-lays it out with no lag.
    /// Draw it before the segments.
    pub fn indicator(&mut self, rect: Rect) {
        let line = self.line();
        let style = RectStyle::fill(self.theme.card_hi)
            .radius(self.s(self.theme.radius.md))
            .border(self.theme.border, line);
        self.rect(rect, style);
    }

    /// Dropdown (select). Returns true when the selection changed. Enter or Space toggles it
    /// from the keyboard, and the arrow keys move the choice while it is open. The menu is
    /// recorded for the overlay pass, so it sits above all other content.
    pub fn select(
        &mut self,
        id: impl Into<ElementId>,
        r: Rect,
        items: &[&str],
        selected: &mut usize,
        open: &mut bool,
    ) -> bool {
        let id: ElementId = id.into();
        if items.is_empty() {
            *open = false;
            return false;
        }
        *selected = (*selected).min(items.len() - 1);

        self.register_focus(id, false);
        let kb = self.kb_focused(id);
        let mut changed = false;
        if kb {
            let keys = &self.input.keys;
            if keys.contains(&UiKey::Enter) || keys.contains(&UiKey::Space) {
                *open = !*open;
            }
            if *open {
                if keys.contains(&UiKey::ArrowDown) && *selected + 1 < items.len() {
                    *selected += 1;
                    changed = true;
                }
                if keys.contains(&UiKey::ArrowUp) && *selected > 0 {
                    *selected -= 1;
                    changed = true;
                }
            }
        }

        let over_trigger = self.hovered(self.touch_hit(r));
        if over_trigger {
            self.want_cursor(Cursor::Pointer);
        }
        if over_trigger && self.input.pressed {
            *open = !*open;
        }
        let hover = self.animate(id, CH_HOVER, if over_trigger { 1.0 } else { 0.0 }, 14.0);
        let open_f = self.animate(id, CH_FOCUS, if *open { 1.0 } else { 0.0 }, 16.0);

        let radius_md = self.s(self.theme.radius.md).min(r.h() * 0.5);
        let line = self.line();
        let border = lerp(self.theme.border, self.theme.ring, open_f.max(hover * 0.5));
        self.rect(
            r,
            RectStyle::fill(self.theme.sunken)
                .radius(radius_md)
                .border(border, line),
        );
        let text_inset = self.s(14.0);
        let chevron = self.s(18.0);
        let label_style = TextStyle::new(16.0, self.theme.text);
        let label = items[*selected];
        let max_label = (r.w() - text_inset * 2.0 - chevron - self.s(8.0)).max(0.0);
        let shown = self.fit_text(label, label_style, max_label);
        let (key, tsize) = self.measure(&shown, label_style);
        self.draw.push_text(TextCmd {
            key,
            x: r.x() + text_inset,
            y: r.center().y - tsize.y * 0.5,
            color: self.theme.text,
            clip: Rect::new(r.x(), r.y(), max_label + text_inset, r.h()),
        });
        let chev = Rect::new(
            r.right() - text_inset - chevron,
            r.center().y - chevron * 0.5,
            chevron,
            chevron,
        );
        self.icon(chev, Icon::ChevronDown, lerp(self.theme.muted, self.theme.text, open_f));
        if kb {
            self.focus_ring(r, radius_md);
        }

        if self.popup_list(id, r, r.w(), items, selected, open) {
            changed = true;
        }
        changed
    }

    /// A popup list of `items` below `anchor`, `width` wide. It flips above when there is no
    /// room below, and it stays inside the window. Clicking a row picks it, which sets
    /// `selected` and closes the list. A click outside the anchor and the list, or Escape,
    /// closes it. Dropdowns, combo boxes, popovers, and context menus all use it. Returns true
    /// when a row was picked this frame.
    pub fn popup_list(
        &mut self,
        id: impl Into<ElementId>,
        anchor: Rect,
        width: f32,
        items: &[&str],
        selected: &mut usize,
        open: &mut bool,
    ) -> bool {
        let id: ElementId = id.into();
        if items.is_empty() {
            *open = false;
            return false;
        }
        let row_h = self.s(36.0);
        let row_inset = self.s(6.0);
        let text_inset = self.s(14.0);
        let icon_px = self.s(16.0);
        let gap = self.s(6.0);
        let radius_md = self.s(self.theme.radius.md);

        let menu_h = row_inset * 2.0 + row_h * items.len() as f32;
        let menu_w = width.max(0.0);
        let mut my = anchor.bottom() + gap;
        if my + menu_h > self.size.y {
            let above = anchor.y() - gap - menu_h;
            my = if above >= 0.0 {
                above
            } else {
                (self.size.y - menu_h).max(0.0)
            };
        }
        let mx = anchor.x().clamp(0.0, (self.size.x - menu_w).max(0.0));
        let menu = Rect::new(mx, my, menu_w, menu_h);

        let outside_click = self.input.pressed
            && !anchor.contains(self.input.mouse)
            && !menu.contains(self.input.mouse);
        if *open && (outside_click || self.input.keys.contains(&UiKey::Escape)) {
            *open = false;
        }
        let mut hovered_row = None;
        if *open {
            for i in 0..items.len() {
                let row = Rect::new(
                    menu.x() + row_inset,
                    menu.y() + row_inset + i as f32 * row_h,
                    menu.w() - row_inset * 2.0,
                    row_h,
                );
                if row.contains(self.input.mouse) {
                    hovered_row = Some(i);
                }
            }
        }
        if hovered_row.is_some() {
            self.want_cursor(Cursor::Pointer);
        }
        let mut changed = false;
        if *open && self.input.released {
            if let Some(i) = hovered_row {
                *selected = i;
                changed = true;
                *open = false;
            }
        }

        let open_f = self.animate(id, CH_POPUP, if *open { 1.0 } else { 0.0 }, 16.0);
        if *open {
            self.pointer_overlay(menu);
        }
        if open_f > 0.01 {
            self.menus.push(MenuOverlay {
                id,
                rect: menu,
                items: items.iter().map(|s| s.to_string()).collect(),
                selected: *selected,
                hovered: if *open { hovered_row } else { None },
                fade: open_f,
                row_h,
                row_inset,
                text_inset,
                radius: radius_md,
                icon_px,
            });
        }
        changed
    }

    /// Draws every dropdown menu recorded this frame, on layer 1, above all other content.
    fn flush_overlays(&mut self) {
        let menus = std::mem::take(&mut self.menus);
        for m in menus {
            self.set_layer(1);
            let fade = m.fade;
            let prev = self.set_opacity(fade);
            self.glass(m.rect, m.radius, self.theme.card);
            self.set_opacity(prev);

            for (i, item) in m.items.iter().enumerate() {
                let row = Rect::new(
                    m.rect.x() + m.row_inset,
                    m.rect.y() + m.row_inset + i as f32 * m.row_h,
                    m.rect.w() - m.row_inset * 2.0,
                    m.row_h,
                );
                let target = if m.hovered == Some(i) { 1.0 } else { 0.0 };
                let hot = self.animate(m.id, CH_ITEM.wrapping_add(i as u8), target, 18.0);
                if hot > 0.01 {
                    self.rect(
                        row,
                        RectStyle::fill(with_alpha(self.theme.card_hi, hot * fade))
                            .radius(self.s(self.theme.radius.sm)),
                    );
                }

                let is_selected = i == m.selected;
                let base = if is_selected {
                    self.theme.text
                } else {
                    self.theme.text_dim
                };
                let fg = with_alpha(lerp(base, self.theme.text, hot), fade);
                let style = TextStyle::new(16.0, fg);
                let icon_reserve = if is_selected { m.icon_px + self.s(8.0) } else { 0.0 };
                let max_text = (row.w() - m.text_inset * 2.0 - icon_reserve).max(0.0);
                let text = self.fit_text(item, style, max_text);
                let (key, tsize) = self.measure(&text, style);
                self.draw.push_text(TextCmd {
                    key,
                    x: row.x() + m.text_inset,
                    y: row.center().y - tsize.y * 0.5,
                    color: fg,
                    clip: row,
                });
                if is_selected {
                    let isz = m.icon_px;
                    self.icon(
                        Rect::new(
                            row.right() - m.text_inset - isz,
                            row.center().y - isz * 0.5,
                            isz,
                            isz,
                        ),
                        Icon::Check,
                        with_alpha(self.theme.text, fade),
                    );
                }
            }
        }
        self.set_layer(0);
    }

    /// Width a [`Ui::badge`] needs for this content, unclamped.
    pub fn badge_width(&mut self, has_icon: bool, label: &str) -> f32 {
        let pad = self.s(12.0);
        let icon_w = if has_icon {
            self.s(14.0) + self.s(6.0)
        } else {
            0.0
        };
        let text_w = self.text_width(label, TextStyle::new(13.0, self.theme.text).medium());
        pad * 2.0 + icon_w + text_w
    }

    /// Pill with an optional icon and label, tinted to its status color. The label
    /// ellipsizes to the pill's width.
    pub fn badge(&mut self, r: Rect, icon: Option<Icon>, label: &str, color: Color) {
        let line = self.line();
        self.rect(
            r,
            RectStyle::fill(with_alpha(color, 0.12))
                .radius(r.h() * 0.5)
                .border(with_alpha(color, 0.35), line),
        );

        let pad = self.s(12.0);
        let isz = self.s(14.0);
        let gap = self.s(6.0);
        let mut x = r.x() + pad;
        let icon_w = if icon.is_some() { isz + gap } else { 0.0 };
        let label_style = TextStyle::new(13.0, color).medium();
        let max_label = (r.w() - pad * 2.0 - icon_w).max(0.0);
        let shown = self.fit_text(label, label_style, max_label);
        let (key, tsize) = self.measure(&shown, label_style);
        let cy = r.center().y;
        if let Some(icon) = icon {
            self.icon(Rect::new(x, cy - isz * 0.5, isz, isz), icon, color);
            x += isz + gap;
        }
        self.draw.push_text(TextCmd {
            key,
            x,
            y: cy - tsize.y * 0.5,
            color,
            clip: r,
        });
    }

    /// Small solid dot, used for connection and sync state.
    pub fn status_dot(&mut self, center: Vec2, radius: f32, color: Color) {
        let r = Rect::new(
            center.x - radius,
            center.y - radius,
            radius * 2.0,
            radius * 2.0,
        );
        self.rect(r, RectStyle::fill(color).radius(radius));
    }
}

/// Where a picture lands inside `r`, and which part of the picture to show there. Returns the
/// drawn area and the `[u0, v0, u1, v1]` texture coordinates for it.
pub fn fit_image(r: Rect, texture: TextureId, fit: Fit) -> (Rect, [f32; 4]) {
    let full = [0.0, 0.0, 1.0, 1.0];
    let picture = texture.width as f32 / texture.height.max(1) as f32;
    let space = r.w() / r.h().max(1.0);
    match fit {
        Fit::Stretch => (r, full),
        Fit::Contain => {
            let (w, h) = if picture > space {
                (r.w(), r.w() / picture)
            } else {
                (r.h() * picture, r.h())
            };
            let area = Rect::new(r.x() + (r.w() - w) * 0.5, r.y() + (r.h() - h) * 0.5, w, h);
            (area, full)
        }
        Fit::Cover => {
            if picture > space {
                // Wider than the box: show the middle, and crop the sides.
                let keep = space / picture;
                let u0 = (1.0 - keep) * 0.5;
                (r, [u0, 0.0, u0 + keep, 1.0])
            } else {
                // Taller than the box: show the middle, and crop the top and bottom.
                let keep = picture / space;
                let v0 = (1.0 - keep) * 0.5;
                (r, [0.0, v0, 1.0, v0 + keep])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> Ui {
        Ui::new(&IconAtlas::build())
    }

    /// Starts a frame 50 ms long, so eased values actually move between frames.
    fn frame(ui: &mut Ui, input: Input) {
        ui.begin(input, Vec2::new(1280.0, 800.0), 1.0, 0.0, 0.05);
    }

    fn click_at(p: Vec2) -> (Input, Input) {
        let press = Input {
            mouse: p,
            pressed: true,
            down: true,
            ..Default::default()
        };
        let release = Input {
            mouse: p,
            released: true,
            ..Default::default()
        };
        (press, release)
    }

    #[test]
    fn fit_text_ellipsizes_long_values_inside_the_box() {
        let mut ui = ui();
        frame(&mut ui, Input::default());
        let style = TextStyle::new(16.0, Theme::adl_dark().text);
        let long = "THISISREALLYLONGTEXTTHATSHOULDBETRUNCATEDSAFELYWITHOUTOVERFLOWING";
        let max = 180.0;
        let shown = ui.fit_text(long, style, max);
        assert!(shown.ends_with(ELLIPSIS), "got {shown:?}");
        assert!(shown.chars().count() < long.chars().count());
        assert!(ui.text_width(&shown, style) <= max + 0.5);
        ui.end();
    }

    #[test]
    fn fit_text_leaves_short_values_alone() {
        let mut ui = ui();
        frame(&mut ui, Input::default());
        let style = TextStyle::new(16.0, Theme::adl_dark().text);
        assert_eq!(ui.fit_text("Short", style, 400.0), "Short");
        assert_eq!(ui.fit_text("Anything", style, 0.0), "");
        ui.end();
    }

    /// Each widget asks for the cursor that matches what it does, and only while the pointer is
    /// over it. Empty canvas keeps the arrow.
    #[test]
    fn cursor_follows_the_widget_under_the_pointer() {
        let mut ui = ui();
        let button = Rect::new(100.0, 100.0, 160.0, 44.0);
        let field = Rect::new(100.0, 200.0, 240.0, 44.0);

        frame(&mut ui, Input { mouse: Vec2::new(120.0, 120.0), ..Default::default() });
        ui.button("cursor-go", button, "Go", None, ButtonVariant::Primary, true);
        ui.end();
        assert_eq!(ui.cursor(), Cursor::Pointer);

        frame(&mut ui, Input { mouse: Vec2::new(120.0, 120.0), ..Default::default() });
        ui.button("cursor-go", button, "Go", None, ButtonVariant::Primary, false);
        ui.end();
        assert_eq!(ui.cursor(), Cursor::NotAllowed);

        let mut value = String::new();
        frame(&mut ui, Input { mouse: Vec2::new(120.0, 220.0), ..Default::default() });
        ui.text_field("cursor-name", field, &mut value, 20, InputKind::Text, "Name", None);
        ui.end();
        assert_eq!(ui.cursor(), Cursor::Text);

        frame(&mut ui, Input { mouse: Vec2::new(900.0, 700.0), ..Default::default() });
        ui.end();
        assert_eq!(ui.cursor(), Cursor::Arrow);
    }

    /// Pressing a slider switches to the grabbing cursor until the button comes up. Hovering
    /// after that shows the pointer.
    #[test]
    fn dragging_a_slider_shows_the_grabbing_cursor() {
        let mut ui = ui();
        let track = Rect::new(100.0, 300.0, 300.0, 24.0);
        let mut value = 0u8;
        let (press, release) = click_at(Vec2::new(200.0, 312.0));

        frame(&mut ui, press);
        ui.slider("cursor-volume", track, &mut value, 10, Theme::adl_dark().primary);
        ui.end();
        assert_eq!(ui.cursor(), Cursor::Grabbing);

        frame(&mut ui, release);
        ui.slider("cursor-volume", track, &mut value, 10, Theme::adl_dark().primary);
        ui.end();
        assert_eq!(ui.cursor(), Cursor::Pointer);
    }

    #[test]
    fn contain_centers_the_whole_picture_and_cover_crops_it() {
        let wide = TextureId {
            index: 0,
            width: 200,
            height: 100,
        };
        let square = Rect::new(10.0, 20.0, 100.0, 100.0);

        let (area, uv) = fit_image(square, wide, Fit::Contain);
        assert_eq!((area.w(), area.h()), (100.0, 50.0));
        assert_eq!((area.x(), area.y()), (10.0, 45.0), "centered vertically");
        assert_eq!(uv, [0.0, 0.0, 1.0, 1.0]);

        let (area, uv) = fit_image(square, wide, Fit::Cover);
        assert_eq!(area, square);
        assert!((uv[0] - 0.25).abs() < 1e-6 && (uv[2] - 0.75).abs() < 1e-6, "{uv:?}");
        assert_eq!((uv[1], uv[3]), (0.0, 1.0));

        let (area, uv) = fit_image(square, wide, Fit::Stretch);
        assert_eq!(area, square);
        assert_eq!(uv, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn cover_crops_the_top_and_bottom_of_a_tall_picture() {
        let tall = TextureId {
            index: 0,
            width: 100,
            height: 300,
        };
        let square = Rect::new(0.0, 0.0, 100.0, 100.0);
        let (_, uv) = fit_image(square, tall, Fit::Cover);
        assert!((uv[1] - 1.0 / 3.0).abs() < 1e-6 && (uv[3] - 2.0 / 3.0).abs() < 1e-6, "{uv:?}");
    }

    /// A custom picture replaces the OS pointer for its shape only: a hand over a button draws
    /// the hand picture, and an arrow over empty canvas draws nothing. Nothing is drawn outside
    /// the window.
    #[test]
    fn custom_cursor_picture_follows_the_shape_under_the_pointer() {
        let mut ui = ui();
        let hand = CursorImage {
            texture: TextureId {
                index: 3,
                width: 16,
                height: 16,
            },
            height: 24.0,
        };
        ui.set_cursor_image(Cursor::Pointer, Some(hand));

        let button = Rect::new(100.0, 100.0, 160.0, 44.0);
        frame(&mut ui, Input { mouse: Vec2::new(120.0, 120.0), ..Default::default() });
        ui.button("cursor-hand", button, "Go", None, ButtonVariant::Primary, true);
        ui.end();
        assert!(ui.has_cursor_image());
        assert_eq!(ui.draw.layers[1].images.len(), 1);
        let drawn = ui.draw.layers[1].images[0];
        assert_eq!(drawn.texture, 3);
        assert_eq!(drawn.instance.rect[..2], [120.0, 120.0]);

        frame(&mut ui, Input { mouse: Vec2::new(900.0, 700.0), ..Default::default() });
        ui.end();
        assert!(!ui.has_cursor_image(), "arrow has no picture");
        assert!(ui.draw.layers[1].images.is_empty());

        frame(&mut ui, Input { mouse: Vec2::new(-1.0, -1.0), ..Default::default() });
        ui.button("cursor-hand", button, "Go", None, ButtonVariant::Primary, true);
        ui.end();
        assert!(ui.draw.layers[1].images.is_empty(), "outside the window");

        ui.set_cursor_image(Cursor::Pointer, None);
        frame(&mut ui, Input { mouse: Vec2::new(120.0, 120.0), ..Default::default() });
        ui.button("cursor-hand", button, "Go", None, ButtonVariant::Primary, true);
        ui.end();
        assert!(!ui.has_cursor_image());
    }

    /// Layout lengths follow the OS scale factor. The window's resolution does not change them.
    #[test]
    fn scale_follows_the_os_scale_factor_only() {
        let mut ui = ui();
        ui.begin(Input::default(), Vec2::new(2400.0, 1600.0), 2.0, 0.0, 0.05);
        assert_eq!(ui.s(10.0), 20.0, "2x display");
        ui.end();
        ui.begin(Input::default(), Vec2::new(1280.0, 800.0), 1.0, 0.0, 0.05);
        assert_eq!(ui.s(10.0), 10.0, "same size on a 1x display");
        ui.end();
    }

    /// Breakpoints use the logical width: a 1080 px phone at 2.75x is 393 logical px wide.
    #[test]
    fn breakpoints_use_the_logical_width() {
        let mut ui = ui();
        ui.begin(Input::default(), Vec2::new(1080.0, 2400.0), 2.75, 0.0, 0.05);
        assert_eq!(ui.breakpoint(), Breakpoint::Compact);
        ui.end();
        ui.begin(Input::default(), Vec2::new(800.0, 600.0), 1.0, 0.0, 0.05);
        assert_eq!(ui.breakpoint(), Breakpoint::Medium);
        ui.end();
        ui.begin(Input::default(), Vec2::new(1920.0, 1080.0), 1.0, 0.0, 0.05);
        assert_eq!(ui.breakpoint(), Breakpoint::Expanded);
        ui.end();
    }

    /// A toggle drawn 40 × 20 still reacts to a pointer within 44 × 44 logical px of its center.
    #[test]
    fn small_controls_have_a_44_point_hit_box() {
        let mut ui = ui();
        let toggle = Rect::new(100.0, 100.0, 40.0, 20.0);
        frame(&mut ui, Input { mouse: Vec2::new(141.0, 110.0), ..Default::default() });
        ui.toggle("touch-toggle", toggle, false);
        ui.end();
        assert_eq!(ui.cursor(), Cursor::Pointer, "just outside the drawn toggle");

        frame(&mut ui, Input { mouse: Vec2::new(150.0, 110.0), ..Default::default() });
        ui.toggle("touch-toggle", toggle, false);
        ui.end();
        assert_eq!(ui.cursor(), Cursor::Arrow, "outside the 44 px box");
    }

    /// The wheel moves a scroll area, and it stops at the end of the content.
    #[test]
    fn scroll_area_moves_with_the_wheel_and_stops_at_the_content_end() {
        let mut ui = ui();
        let view = Rect::new(0.0, 100.0, 300.0, 200.0);
        let over = Vec2::new(50.0, 150.0);
        // Content 600 px tall: the area can scroll 400 px.
        frame(&mut ui, Input::default());
        let off = ui.scroll_begin("list", view);
        ui.scroll_end("list", view.y() - off + 600.0);

        frame(&mut ui, Input { mouse: over, scroll: 80.0, ..Default::default() });
        let off = ui.scroll_begin("list", view);
        assert_eq!(off, 80.0);
        ui.scroll_end("list", view.y() - off + 600.0);

        frame(&mut ui, Input { mouse: over, scroll: 1000.0, ..Default::default() });
        let off = ui.scroll_begin("list", view);
        assert_eq!(off, 400.0, "stops at the end of the content");
        ui.scroll_end("list", view.y() - off + 600.0);
    }

    /// An underdamped spring overshoots its target by about 4%, then settles on it.
    #[test]
    fn spring_overshoots_and_settles() {
        let mut ui = ui();
        frame(&mut ui, Input::default());
        ui.spring("spring-test", 0, 0.0, 260.0, 23.0);
        let mut peak = 0.0f32;
        let mut last = 0.0;
        for _ in 0..120 {
            ui.begin(Input::default(), Vec2::new(1280.0, 800.0), 1.0, 0.0, 0.05);
            last = ui.spring("spring-test", 0, 100.0, 260.0, 23.0);
            peak = peak.max(last);
            ui.end();
        }
        assert!(peak > 102.0 && peak < 106.0, "overshoot {peak}");
        assert!((last - 100.0).abs() < 0.5, "settled at {last}");
    }

    /// With reduced motion on, animations and springs jump straight to their targets.
    #[test]
    fn reduced_motion_jumps_to_the_end_state() {
        let mut ui = ui();
        ui.set_reduced_motion(true);
        frame(&mut ui, Input::default());
        assert_eq!(ui.animate("reduced-test", 0, 0.0, 10.0), 0.0);
        assert_eq!(ui.animate("reduced-test", 0, 1.0, 10.0), 1.0);
        ui.set_spring("reduced-spring", 0, 0.0);
        assert_eq!(ui.spring("reduced-spring", 0, 5.0, 260.0, 23.0), 5.0);
        ui.end();
    }

    /// A press at the left edge that moves right completes a back swipe, once.
    #[test]
    fn a_left_edge_swipe_completes_once() {
        let mut ui = ui();
        let at = |x: f32, y: f32, pressed: bool, down: bool| Input {
            mouse: Vec2::new(x, y),
            pressed,
            down,
            ..Default::default()
        };
        frame(&mut ui, at(5.0, 300.0, true, true));
        assert!(!ui.back_swipe());
        frame(&mut ui, at(60.0, 302.0, false, true));
        assert!(!ui.back_swipe(), "not far enough yet");
        frame(&mut ui, at(120.0, 304.0, false, true));
        assert!(ui.back_swipe());
        frame(&mut ui, at(160.0, 304.0, false, true));
        assert!(!ui.back_swipe(), "fires once per swipe");
    }

    /// A press that does not start at the left edge is never a back swipe.
    #[test]
    fn a_swipe_from_the_middle_is_not_a_back_swipe() {
        let mut ui = ui();
        frame(&mut ui, Input { mouse: Vec2::new(300.0, 300.0), pressed: true, down: true, ..Default::default() });
        assert!(!ui.back_swipe());
        frame(&mut ui, Input { mouse: Vec2::new(500.0, 300.0), down: true, ..Default::default() });
        assert!(!ui.back_swipe());
    }

    #[test]
    fn wrapped_paragraph_stays_within_its_width() {
        let mut ui = ui();
        frame(&mut ui, Input::default());
        let style = TextStyle::new(14.0, Theme::adl_dark().text_dim).body();
        let text = "Long words break by glyph, so this paragraph keeps every line inside the box. THISISREALLYLONGTEXTTHATSHOULDBETRUNCATEDSAFELY";
        let (_, size) = ui.measure_wrapped(text, style, 160.0);
        assert!(size.x <= 160.5, "width {}", size.x);
        let (_, one_line) = ui.measure_wrapped("Hi", style, 160.0);
        assert!(size.y > one_line.y, "wrapping should add lines");
        ui.end();
    }

    /// Drives the dropdown through every path that used to be risky: opening, selecting,
    /// clicking outside, Escape, an empty list, and an out-of-range index. Nothing may panic,
    /// and the menu must stay inside the window.
    #[test]
    fn dropdown_state_machine_is_safe() {
        let mut ui = ui();
        let trigger = Rect::new(100.0, 100.0, 220.0, 44.0);
        let items = ["Standard", "Elevated", "Urgent"];
        let mut selected = 9usize; // out of range on purpose
        let mut open = false;

        // Frame 1: out-of-range index is clamped; nothing opens yet.
        frame(&mut ui, Input::default());
        ui.select("dd", trigger, &items, &mut selected, &mut open);
        assert_eq!(selected, 2);
        assert!(!open);
        ui.end();

        // Frame 2: click the trigger to open.
        let (press, _) = click_at(trigger.center());
        frame(&mut ui, press);
        ui.select("dd", trigger, &items, &mut selected, &mut open);
        ui.end();
        assert!(open);

        // Frame 3: the overlay pass has recorded the menu above the base layer.
        frame(&mut ui, Input::default());
        ui.select("dd", trigger, &items, &mut selected, &mut open);
        ui.end();
        assert!(!ui.draw.layers[1].rects.is_empty(), "menu drawn on layer 1");

        // Frame 4: pick row 1 ("Elevated").
        let row1 = Vec2::new(trigger.x() + 40.0, trigger.bottom() + 6.0 + 6.0 + 36.0 * 1.5);
        let (press, _) = click_at(row1);
        frame(&mut ui, press);
        ui.select("dd", trigger, &items, &mut selected, &mut open);
        ui.end();
        let (_, release) = click_at(row1);
        frame(&mut ui, release);
        let changed = ui.select("dd", trigger, &items, &mut selected, &mut open);
        ui.end();
        assert!(changed);
        assert_eq!(selected, 1);
        assert!(!open);

        // Reopen, then click far outside the trigger and menu to close.
        let (press, release) = click_at(trigger.center());
        frame(&mut ui, press);
        ui.select("dd", trigger, &items, &mut selected, &mut open);
        ui.end();
        frame(&mut ui, release);
        ui.select("dd", trigger, &items, &mut selected, &mut open);
        ui.end();
        assert!(open);
        let (outside, _) = click_at(Vec2::new(900.0, 700.0));
        frame(&mut ui, outside);
        ui.select("dd", trigger, &items, &mut selected, &mut open);
        ui.end();
        assert!(!open, "outside click closes");

        // Reopen, then Escape.
        let (press, release) = click_at(trigger.center());
        frame(&mut ui, press);
        ui.select("dd", trigger, &items, &mut selected, &mut open);
        ui.end();
        frame(&mut ui, release);
        ui.select("dd", trigger, &items, &mut selected, &mut open);
        ui.end();
        assert!(open);
        frame(
            &mut ui,
            Input {
                keys: vec![UiKey::Escape],
                ..Default::default()
            },
        );
        ui.select("dd", trigger, &items, &mut selected, &mut open);
        ui.end();
        assert!(!open, "escape closes");

        // An empty list never opens and never indexes.
        let mut empty_open = true;
        let mut empty_sel = 4usize;
        frame(&mut ui, Input::default());
        ui.select("empty", trigger, &[], &mut empty_sel, &mut empty_open);
        ui.end();
        assert!(!empty_open);
    }

    #[test]
    fn keyboard_tab_and_enter_activate_buttons() {
        let mut ui = ui();
        let a = Rect::new(10.0, 10.0, 100.0, 40.0);
        let b = Rect::new(10.0, 60.0, 100.0, 40.0);
        let build = |ui: &mut Ui| {
            ui.button("kb-a", a, "A", None, ButtonVariant::Primary, true);
            ui.button("kb-b", b, "B", None, ButtonVariant::Primary, true);
        };
        let keys = |keys: Vec<UiKey>, releases: Vec<UiKey>| Input {
            keys,
            key_releases: releases,
            ..Default::default()
        };

        // Frame 1 records the tab order. Frame 2: Tab focuses the first button.
        frame(&mut ui, Input::default());
        build(&mut ui);
        ui.end();
        frame(&mut ui, keys(vec![UiKey::Tab], vec![]));
        build(&mut ui);
        ui.end();
        assert_eq!(ui.kb_focus, Some(ElementId::from("kb-a")));

        // Tab again moves to the second, and Shift+Tab wraps back to the first.
        frame(&mut ui, keys(vec![UiKey::Tab], vec![]));
        build(&mut ui);
        ui.end();
        assert_eq!(ui.kb_focus, Some(ElementId::from("kb-b")));
        frame(&mut ui, keys(vec![UiKey::BackTab], vec![]));
        build(&mut ui);
        ui.end();
        assert_eq!(ui.kb_focus, Some(ElementId::from("kb-a")));

        // Enter pressed on the focused button sinks it, but does not click yet.
        frame(&mut ui, keys(vec![UiKey::Enter], vec![]));
        let press = ui.button("kb-a", a, "A", None, ButtonVariant::Primary, true);
        ui.end();
        assert!(!press, "no click on key press");
        assert_eq!(ui.active, Some(ElementId::from("kb-a")), "held while the key is down");

        // Releasing Enter completes the click.
        frame(&mut ui, keys(vec![], vec![UiKey::Enter]));
        let clicked = ui.button("kb-a", a, "A", None, ButtonVariant::Primary, true);
        ui.end();
        assert!(clicked, "click on key release");
    }

    #[test]
    fn text_field_types_selects_copies_and_pastes() {
        let mut ui = ui();
        let r = Rect::new(10.0, 10.0, 300.0, 44.0);
        let mut value = String::new();
        let field = |ui: &mut Ui, value: &mut String| {
            ui.text_field("tf", r, value, 40, InputKind::Text, "", None);
        };

        // Click to focus the field, then type.
        let (press, _) = click_at(Vec2::new(50.0, 30.0));
        frame(&mut ui, press);
        field(&mut ui, &mut value);
        ui.end();
        frame(
            &mut ui,
            Input {
                typed: "hello world".into(),
                ..Default::default()
            },
        );
        field(&mut ui, &mut value);
        ui.end();
        assert_eq!(value, "hello world");

        // Ctrl+A selects all, and Ctrl+C copies it.
        frame(
            &mut ui,
            Input {
                keys: vec![UiKey::SelectAll, UiKey::Copy],
                ctrl: true,
                ..Default::default()
            },
        );
        field(&mut ui, &mut value);
        ui.end();
        assert_eq!(ui.take_clipboard().as_deref(), Some("hello world"));

        // Pasting replaces the selection.
        frame(
            &mut ui,
            Input {
                paste: Some("bye".into()),
                ..Default::default()
            },
        );
        field(&mut ui, &mut value);
        ui.end();
        assert_eq!(value, "bye");

        // Shift+Left twice selects the last two characters, and Backspace removes them.
        frame(
            &mut ui,
            Input {
                keys: vec![UiKey::ArrowLeft, UiKey::ArrowLeft],
                shift: true,
                ..Default::default()
            },
        );
        field(&mut ui, &mut value);
        ui.end();
        frame(
            &mut ui,
            Input {
                keys: vec![UiKey::Backspace],
                ..Default::default()
            },
        );
        field(&mut ui, &mut value);
        ui.end();
        assert_eq!(value, "b");
    }

    #[test]
    fn number_field_rejects_letters_and_clamps_on_blur() {
        let mut ui = ui();
        let r = Rect::new(10.0, 10.0, 200.0, 44.0);
        let kind = InputKind::Number {
            min: Some(0.0),
            max: Some(99.0),
            decimals: 0,
        };
        let mut value = String::new();
        let (press, _) = click_at(Vec2::new(40.0, 30.0));
        frame(&mut ui, press);
        ui.text_field("num", r, &mut value, 6, kind, "", None);
        ui.end();
        frame(
            &mut ui,
            Input {
                typed: "4a2".into(),
                ..Default::default()
            },
        );
        ui.text_field("num", r, &mut value, 6, kind, "", None);
        ui.end();
        assert_eq!(value, "42", "letters are refused");

        // Losing focus clamps an out-of-range value.
        let mut value = String::from("250");
        let other = Rect::new(10.0, 200.0, 200.0, 44.0);
        let (press_elsewhere, _) = click_at(Vec2::new(40.0, 220.0));
        frame(&mut ui, press_elsewhere);
        ui.text_field("num", r, &mut value, 6, kind, "", None);
        ui.end();
        assert_eq!(value, "99");
        let _ = other;
    }

    #[test]
    fn slider_click_and_drag_set_the_value() {
        let mut ui = ui();
        let r = Rect::new(100.0, 100.0, 200.0, 24.0);
        let mut v: u8 = 0;
        // Press at 75% of the track: the value jumps there.
        frame(
            &mut ui,
            Input {
                mouse: Vec2::new(250.0, 112.0),
                pressed: true,
                down: true,
                ..Default::default()
            },
        );
        let changed = ui.slider("sl", r, &mut v, 10, Theme::adl_dark().primary);
        ui.end();
        assert!(changed);
        assert_eq!(v, 8);
        // Hold and drag to 20%: the value follows.
        frame(
            &mut ui,
            Input {
                mouse: Vec2::new(140.0, 112.0),
                down: true,
                ..Default::default()
            },
        );
        ui.slider("sl", r, &mut v, 10, Theme::adl_dark().primary);
        ui.end();
        assert_eq!(v, 2);
        // Release ends the drag, so moving the pointer no longer changes the value.
        frame(
            &mut ui,
            Input {
                mouse: Vec2::new(290.0, 112.0),
                released: true,
                ..Default::default()
            },
        );
        ui.slider("sl", r, &mut v, 10, Theme::adl_dark().primary);
        ui.end();
        assert_eq!(v, 2);
    }

    #[test]
    fn dropdown_menu_flips_upward_near_the_bottom_edge() {
        let mut ui = ui();
        let low_trigger = Rect::new(100.0, 760.0, 220.0, 44.0);
        let items = ["A", "B", "C", "D", "E"];
        let mut selected = 0usize;
        let mut open = true;
        frame(&mut ui, Input::default());
        ui.select("flip", low_trigger, &items, &mut selected, &mut open);
        ui.end();
        let menu = ui.draw.layers[1].rects.first().expect("menu recorded");
        let top = menu.rect[1];
        let bottom = top + menu.rect[3];
        assert!(bottom <= 800.0 + 0.5, "menu stays in the window: bottom {bottom}");
        assert!(top < low_trigger.y(), "menu opens above the trigger");
    }
}
