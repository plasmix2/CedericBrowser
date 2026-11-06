//! ANVL Note widgets: the header with the logo, the multi-line text editor, the note list row,
//! and the status bar. Declared as a child of `ui`, so it can use the widget internals.
//!
//! Rectangles are in physical pixels, like every other widget. Lengths written as `self.s(n)` are
//! logical points. Colors come from `theme::anvl`.

use super::*;
use crate::editor::{line_col, word_at, EditKind, EditorCursor, SelEnd, TouchPress, VisualLines};
use crate::theme::motion;
use crate::markdown::{self, MdKind, MdSpan};
use crate::text::RichSpan;
use crate::layout::LayoutMode;
use crate::logo::{self, LogoPart};
use crate::render::Renderer;
use crate::text::TextKey;
use crate::theme::anvl;

/// Editor font size in logical points. Line height is 1.5 times this.
pub const EDITOR_FONT: f32 = 16.0;
/// Selection handle radius and how far from its center a finger still picks it up, in logical px.
const HANDLE_R: f32 = 9.0;
const HANDLE_GRAB: f32 = 26.0;
/// Seconds a still finger takes to become a long press (ADL-MOBILE section 11).
const LONG_PRESS: f32 = 0.5;
/// Most seconds between the taps of a double tap or double click.
const DOUBLE_TAP: f32 = 0.35;
/// Ids for the touch selection menu's buttons.
const MENU_IDS: [&str; 4] = ["anvl_ed_menu0", "anvl_ed_menu1", "anvl_ed_menu2", "anvl_ed_menu3"];

/// What a button in the touch selection menu does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuAct {
    Cut,
    Copy,
    Paste,
    SelectAll,
}

/// Caret blink half-period in seconds.
pub const BLINK: f32 = 0.53;
/// Height of the logo in the header, in logical points.
pub const LOGO_H: f32 = 26.0;

/// The logo pictures on the GPU, made for one scale factor.
#[derive(Clone, Copy, Debug)]
pub struct LogoTextures {
    /// Blade, divider, and ANVL wordmark, drawn from the polygons in `logo`.
    pub lockup: TextureId,
    /// The squircle app mark (`logo::mark_frame`).
    pub mark: TextureId,
    /// The scale factor the pictures were rasterized for.
    pub scale: f32,
}

impl LogoTextures {
    /// Rasterizes the logo at `scale` physical pixels per point and uploads it.
    pub fn build(renderer: &mut Renderer, scale: f32) -> Self {
        let px = (LOGO_H * scale).round().max(1.0) as u32;
        let mark_px = (48.0 * scale).round().max(1.0) as u32;
        let lockup = renderer.upload(&logo::polygon_frame(LogoPart::Lockup, px));
        let mark = renderer.upload(&logo::mark_frame(mark_px));
        Self { lockup, mark, scale }
    }
}

/// What happened in the editor this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EditorOutput {
    /// The text changed.
    pub changed: bool,
    /// Ctrl+S was pressed while the editor had focus.
    pub save: bool,
    /// Caret line and column, 1-based.
    pub line: usize,
    pub col: usize,
}

impl Ui {
    /// Gives keyboard focus to the widget `id`, as if it were clicked.
    pub fn set_focus(&mut self, id: impl Into<ElementId>) {
        let id: ElementId = id.into();
        self.focus = Some(id);
        self.kb_focus = Some(id);
    }

    /// The widget that has focus, if any.
    pub fn focused_id(&self) -> Option<ElementId> {
        self.focus
    }

    // ------------------------------------------------------------------ text editor

    /// Shapes `value` wrapped to `wrap_w`, with its Markdown styled (`md`, from
    /// [`markdown::highlight`]), and reads back its geometry.
    fn shape_editor(
        &mut self,
        value: &str,
        md: &[MdSpan],
        style: TextStyle,
        wrap_w: f32,
        line_h: f32,
    ) -> (TextKey, VisualLines) {
        let px = self.s(style.size);
        // Span colors are fixed when the text is shaped, so they carry the frame's opacity.
        // Quantized, so a fade reshapes the text a few dozen times, not every frame.
        let opacity = (self.draw.opacity * 32.0).round() / 32.0;
        let spans: Vec<RichSpan> = md.iter().map(|s| md_style(s, px, opacity, &self.theme)).collect();
        let key = self.text.layout_rich(value, px, style.weight, style.role, style.italic, Some(wrap_w), &spans);
        let rows = match self.text.buffer(&key) {
            Some(buf) => VisualLines::build(buf, value, line_h),
            None => VisualLines::build_empty(line_h),
        };
        (key, rows)
    }

    /// Multi-line text editor filling `r`. The text wraps to the width of the pane (to at most
    /// 820 pt, centered). Click to place the caret and drag to select. Typing, Enter, Tab (two
    /// spaces) and Shift+Tab, Backspace and Delete (Ctrl for words), the arrow keys with Home and
    /// End (Shift extends the selection, Ctrl jumps by word or to the ends of the text),
    /// Ctrl+A, Ctrl+C, Ctrl+X, Ctrl+V, and Ctrl+S all work while it has focus. The mouse wheel
    /// scrolls, and the view follows the caret. `cursor` holds the caret and scroll for this
    /// text, so keep one per note. Copied text comes out of [`Ui::take_clipboard`].
    pub fn text_editor(
        &mut self,
        id: impl Into<ElementId>,
        r: Rect,
        value: &mut String,
        cursor: &mut EditorCursor,
        placeholder: &str,
    ) -> EditorOutput {
        let id: ElementId = id.into();
        self.register_focus(id, true);
        // A press on the touch menu belongs to the menu, but the editor keeps its focus.
        let had_focus = self.focus == Some(id) || (self.input.pressed && self.last_text_focus == Some(id));
        if self.input.pressed && cursor.menu_rect.is_some_and(|m| m.contains(self.input.mouse)) {
            self.focus = Some(id);
        }
        let hovered = self.hovered(r);
        let pressed_here = self.input.pressed && hovered;
        if hovered {
            self.want_cursor(Cursor::Text);
        }
        if pressed_here {
            self.focus = Some(id);
            self.kb_focus = Some(id);
            self.active = Some(id);
        }
        let focused = self.focus == Some(id);
        if focused {
            self.capture_tab_now = true;
        }

        let style = TextStyle::new(EDITOR_FONT, self.theme.text).body();
        let px = self.s(EDITOR_FONT);
        let line_h = px * FontRole::Body.line_height();
        // A phone gets 16 pt margins (ADL-MOBILE section 4) and no focus border around the page.
        let compact = self.logical_size().x < LayoutMode::BREAKPOINT;
        let pad_y = self.s(if compact { 16.0 } else { 20.0 });
        let margins = self.s(if compact { 32.0 } else { 56.0 });
        let col_w = (r.w() - margins).min(self.s(820.0)).max(self.s(40.0));
        let x0 = r.x() + (r.w() - col_w) * 0.5;
        let top = r.y() + pad_y;
        let (shift, ctrl) = (self.input.shift, self.input.ctrl);

        cursor.ed.clamp(value.chars().count());
        let mut geo: Option<(TextKey, VisualLines)> = None;
        macro_rules! geo {
            () => {{
                if geo.is_none() {
                    let md = markdown::highlight(value.as_str());
                    geo = Some(self.shape_editor(value.as_str(), &md, style, col_w, line_h));
                }
                &geo.as_ref().expect("shaped above").1
            }};
        }
        let now = self.time;

        let (mut changed, mut moved, mut save) = (false, false, false);

        // Pointer: a press places the caret, and holding the button extends the selection.
        let touch = self.input.touch;
        let dragging = focused && !pressed_here && self.active == Some(id) && self.input.down;
        if touch {
            // ADL-MOBILE section 11.1. A finger drag scrolls (and flings on release). A tap
            // places the caret, a double tap or a long press selects a word, and the selection
            // handles drag either end. A scroll never moves the caret.
            let m = self.input.mouse;
            let slop = self.s(8.0);
            let origin = top - cursor.scroll;
            let (grab, drop) = (self.s(HANDLE_GRAB), self.s(HANDLE_R * 2.0 + 4.0));

            // A press on a selection handle picks it up instead of starting a scroll.
            if self.input.pressed && focused && cursor.touch_sel {
                let (lo, hi) = cursor.ed.selection();
                let g = geo!();
                for (end, idx) in [(SelEnd::Lo, lo), (SelEnd::Hi, hi)] {
                    let c = g.caret(value.as_str(), idx);
                    let p = Vec2::new(x0 + c.x, origin + c.top + c.height + self.s(HANDLE_R));
                    if (m - p).length() <= grab {
                        cursor.handle = Some(end);
                    }
                }
                if cursor.handle.is_some() {
                    self.active = Some(id);
                    cursor.touch_press = None;
                }
            }

            if let Some(end) = cursor.handle {
                if self.input.down && !self.input.released {
                    // The finger sits under the line, so aim above it.
                    let idx = geo!().hit(value.as_str(), m.x - x0, m.y - drop - origin);
                    let (lo, hi) = cursor.ed.selection();
                    let fixed = if end == SelEnd::Lo { hi } else { lo };
                    cursor.ed.anchor = fixed;
                    cursor.ed.caret = idx;
                    cursor.menu = false;
                    moved = true;
                } else {
                    cursor.handle = None;
                    // Ends that crossed swap roles.
                    cursor.menu = true;
                }
            } else {
                if pressed_here {
                    cursor.touch_press = Some(TouchPress {
                        start: (m.x, m.y),
                        last_y: m.y,
                        dragging: false,
                        t0: self.time,
                        long: false,
                        had_focus,
                    });
                    cursor.fling = 0.0;
                }
                let dt = self.dt;
                let mut long_hit = None;
                if let Some(tp) = cursor.touch_press.as_mut() {
                    if self.input.down && !self.input.released {
                        let (dx, dy) = (m.x - tp.start.0, m.y - tp.start.1);
                        if !tp.dragging && !tp.long && dy.abs() > slop {
                            tp.dragging = true;
                        }
                        if tp.dragging {
                            let step = m.y - tp.last_y;
                            cursor.scroll -= step;
                            if dt > 0.0 {
                                cursor.fling += (-step / dt - cursor.fling) * 0.5;
                            }
                        } else if !tp.long && dx.hypot(dy) <= slop {
                            let held = self.time - tp.t0;
                            if held >= LONG_PRESS {
                                tp.long = true;
                                long_hit = Some(tp.start);
                            } else {
                                self.request_wake(LONG_PRESS - held + 0.01);
                            }
                        }
                        tp.last_y = m.y;
                    }
                }
                // Long press: select the word under the finger and offer the menu.
                if let Some((sx, sy)) = long_hit {
                    let idx = geo!().hit(value.as_str(), sx - x0, sy - origin);
                    let (lo, hi) = word_at(value.as_str(), idx);
                    cursor.ed.anchor = lo;
                    cursor.ed.caret = hi;
                    cursor.touch_sel = hi > lo;
                    cursor.menu = true;
                    self.focus = Some(id);
                    moved = true;
                }
                if self.input.released {
                    if let Some(tp) = cursor.touch_press.take() {
                        if tp.dragging {
                            if self.reduced_motion {
                                cursor.fling = 0.0;
                            }
                        } else if !tp.long {
                            cursor.fling = 0.0;
                            let idx = geo!().hit(value.as_str(), tp.start.0 - x0, tp.start.1 - origin);
                            let double = cursor.last_tap.is_some_and(|(t, x, y)| {
                                self.time - t < DOUBLE_TAP && (x - tp.start.0).hypot(y - tp.start.1) < slop * 3.0
                            });
                            if double {
                                let (lo, hi) = word_at(value.as_str(), idx);
                                cursor.ed.anchor = lo;
                                cursor.ed.caret = hi;
                                cursor.touch_sel = hi > lo;
                                cursor.menu = true;
                                cursor.last_tap = None;
                            } else {
                                // A tap on the caret itself opens the menu (Paste, Select all).
                                let on_caret = !cursor.ed.has_selection() && idx == cursor.ed.caret && tp.had_focus;
                                cursor.ed.move_to(idx, false);
                                cursor.touch_sel = false;
                                cursor.menu = on_caret && !cursor.menu;
                                cursor.last_tap = Some((self.time, tp.start.0, tp.start.1));
                            }
                            cursor.goal_x = None;
                            moved = true;
                        }
                    }
                }
            }
        }
        if !touch && (pressed_here || dragging) {
            let m = self.input.mouse;
            let idx = geo!().hit(value.as_str(), m.x - x0, m.y - (top - cursor.scroll));
            // A double click selects a word, like a double tap.
            let double = pressed_here
                && !shift
                && cursor.last_tap.is_some_and(|(t, x, y)| self.time - t < DOUBLE_TAP && (x - m.x).hypot(y - m.y) < self.s(6.0));
            if double {
                let (lo, hi) = word_at(value.as_str(), idx);
                cursor.ed.anchor = lo;
                cursor.ed.caret = hi;
                cursor.last_tap = None;
                cursor.word_drag = Some((lo, hi));
            } else if let (true, Some((wlo, whi))) = (dragging, cursor.word_drag) {
                // After a double click, a drag extends the selection a whole word at a time.
                let (lo, hi) = word_at(value.as_str(), idx);
                if idx >= wlo {
                    cursor.ed.anchor = wlo;
                    cursor.ed.caret = hi.max(whi);
                } else {
                    cursor.ed.anchor = whi;
                    cursor.ed.caret = lo;
                }
            } else {
                cursor.ed.move_to(idx, if pressed_here { shift } else { true });
                if pressed_here {
                    cursor.last_tap = Some((self.time, m.x, m.y));
                    cursor.word_drag = None;
                }
            }
            cursor.touch_sel = false;
            cursor.menu = false;
            cursor.goal_x = None;
            moved = true;
        }

        if focused {
            if let Some(paste) = self.input.paste.clone() {
                let rec = cursor.history.record(value, cursor.ed, EditKind::Other, now);
                let ch = cursor.insert(value, &paste);
                cursor.history.finish(rec, ch, cursor.ed, now);
                changed |= ch;
                moved = true;
                geo = None;
            }
            let typed = self.input.typed.clone();
            if !typed.is_empty() {
                // A space ends the word before it, so undo takes back a word at a time.
                if typed.chars().all(char::is_whitespace) {
                    cursor.history.break_group();
                }
                let rec = cursor.history.record(value, cursor.ed, EditKind::Typing, now);
                let ch = cursor.insert(value, &typed);
                cursor.history.finish(rec, ch, cursor.ed, now);
                changed |= ch;
                moved = true;
                geo = None;
            }
            let mut copied: Option<String> = None;
            for key in self.input.keys.clone() {
                let mut edited = false;
                let kind = match key {
                    UiKey::Backspace | UiKey::Delete => Some(EditKind::Deleting),
                    UiKey::Enter | UiKey::Tab | UiKey::BackTab | UiKey::Cut => Some(EditKind::Other),
                    _ => None,
                };
                let rec = match kind {
                    Some(k) => cursor.history.record(value, cursor.ed, k, now),
                    None => false,
                };
                match key {
                    UiKey::Backspace => edited = cursor.backspace(value, ctrl),
                    UiKey::Delete => edited = cursor.delete_forward(value, ctrl),
                    UiKey::Enter => edited = cursor.newline(value),
                    UiKey::Tab => edited = cursor.tab(value),
                    UiKey::BackTab => edited = cursor.outdent(value),
                    UiKey::Undo => edited = cursor.history.undo(value, &mut cursor.ed),
                    UiKey::Redo => edited = cursor.history.redo(value, &mut cursor.ed),
                    UiKey::ArrowLeft => cursor.left(value, shift, ctrl),
                    UiKey::ArrowRight => cursor.right(value, shift, ctrl),
                    UiKey::Home => cursor.home(value, shift, ctrl),
                    UiKey::End => cursor.end(value, shift, ctrl),
                    UiKey::ArrowUp | UiKey::ArrowDown => {
                        let dir = if key == UiKey::ArrowUp { -1 } else { 1 };
                        let (idx, goal) = geo!().vertical(value.as_str(), cursor.caret(), cursor.goal_x, dir);
                        cursor.ed.move_to(idx, shift);
                        cursor.goal_x = Some(goal);
                    }
                    UiKey::SelectAll => cursor.select_all(value),
                    UiKey::Copy => {
                        if cursor.ed.has_selection() {
                            copied = Some(cursor.ed.selected_text(value));
                        }
                    }
                    UiKey::Cut => {
                        if cursor.ed.has_selection() {
                            copied = Some(cursor.ed.selected_text(value));
                            cursor.ed.delete_selection(value);
                            edited = true;
                        }
                    }
                    UiKey::Save => save = true,
                    _ => continue,
                }
                if kind.is_some() {
                    cursor.history.finish(rec, edited, cursor.ed, now);
                } else if !edited {
                    // A caret move ends the typing group.
                    cursor.history.break_group();
                }
                moved = true;
                if edited {
                    changed = true;
                    geo = None;
                }
            }
            if copied.is_some() {
                self.clipboard_out = copied;
            }
        }
        if changed {
            cursor.menu = false;
            cursor.touch_sel = false;
        }
        if !focused {
            cursor.menu = false;
            cursor.touch_sel = false;
            cursor.handle = None;
        }
        if changed || moved {
            cursor.touched = self.time;
        }
        cursor.ed.clamp(value.chars().count());

        // Shape the final text (reusing the shaping from above when nothing changed) and keep the
        // caret in view.
        let md = markdown::highlight(value.as_str());
        let (key, rows) = match geo.take() {
            Some(g) => g,
            None => self.shape_editor(value.as_str(), &md, style, col_w, line_h),
        };
        let content_h = rows.height() + pad_y * 2.0;
        let max_scroll = (content_h - r.h()).max(0.0);
        cursor.scroll += self.scroll_in(r);
        let caret = rows.caret(value.as_str(), cursor.caret());
        if moved {
            let caret_top = top + caret.top - cursor.scroll;
            if caret_top < r.y() {
                cursor.scroll -= r.y() - caret_top;
            } else if caret_top + caret.height > r.bottom() {
                cursor.scroll += caret_top + caret.height - r.bottom();
            }
        }
        cursor.max_scroll = max_scroll;
        let finger_down = cursor.touch_press.is_some_and(|t| t.dragging);
        if finger_down {
            // Past an end the page moves at half speed, like a stretched edge.
            let edge = cursor.scroll.clamp(0.0, max_scroll);
            let over = cursor.scroll - edge;
            cursor.scroll = edge + over.signum() * (over.abs() * crate::ui::RUBBER_BAND).min(r.h() * 0.3);
        } else if self.reduced_motion || !touch {
            cursor.fling = 0.0;
            cursor.scroll = cursor.scroll.clamp(0.0, max_scroll);
        } else if crate::ui::coast(&mut cursor.scroll, &mut cursor.fling, max_scroll, self.dt, self.s(20.0)) {
            self.request_frame();
        }
        let origin_y = top - cursor.scroll;

        // Draw: canvas, focus border, selection, text, caret, scroll thumb.
        self.rect(r, RectStyle::fill(self.theme.bg));
        let f = self.animate(id, CH_FOCUS, if focused { 1.0 } else { 0.0 }, anvl::EASE_RATE);
        if f > 0.01 && !compact {
            self.rect(
                r,
                RectStyle::fill(with_alpha(self.theme.ring, 0.0)).border(with_alpha(self.theme.ring, f), self.line()),
            );
        }
        self.markdown_decor(value.as_str(), &md, &rows, x0, origin_y, col_w, r);
        if value.is_empty() {
            let hint = TextStyle::new(EDITOR_FONT, self.theme.disabled).body();
            self.text_clipped(x0, top, placeholder, hint, r);
        } else {
            self.draw.push_text(TextCmd {
                key,
                x: x0,
                y: origin_y,
                color: self.theme.text,
                clip: r,
            });
        }
        if focused {
            let (lo, hi) = cursor.ed.selection();
            for [sx, sy, sw, sh] in rows.selection(value.as_str(), lo, hi) {
                let y0 = (origin_y + sy).max(r.y());
                let y1 = (origin_y + sy + sh).min(r.bottom());
                if y1 > y0 {
                    self.rect(
                        Rect::new(x0 + sx, y0, sw, y1 - y0),
                        RectStyle::fill(self.theme.selection).radius(self.s(2.0)),
                    );
                }
            }
            let since = (self.time - cursor.touched).max(0.0);
            let on = (since / BLINK) as u64 % 2 == 0;
            // The margin keeps an early OS timer from landing just before the toggle and asking again.
            self.request_wake(BLINK - since % BLINK + 0.02);
            let cy = origin_y + caret.top;
            if on && cy >= r.y() - 0.5 && cy + caret.height <= r.bottom() + 0.5 {
                let w = self.s(2.0);
                self.rect(
                    Rect::new(x0 + caret.x, cy, w, caret.height),
                    RectStyle::fill(self.theme.text).radius(w * 0.5),
                );
            }
        }
        // Touch selection (ADL-MOBILE section 9.1): a handle under each end, and the action menu
        // above the selection. A mouse never shows them.
        let show_touch = focused && self.input.touch;
        if show_touch && cursor.touch_sel && cursor.ed.has_selection() {
            let (lo, hi) = cursor.ed.selection();
            let layer = self.draw.current;
            self.set_layer(1);
            let (hr, stem) = (self.s(HANDLE_R), self.s(2.0));
            for idx in [lo, hi] {
                let c = rows.caret(value.as_str(), idx);
                let (x, y) = (x0 + c.x, origin_y + c.top + c.height);
                if y < r.y() || y > r.bottom() {
                    continue;
                }
                self.rect(Rect::new(x - stem * 0.5, origin_y + c.top, stem, c.height), RectStyle::fill(self.theme.text));
                self.rect(
                    Rect::new(x - hr, y, hr * 2.0, hr * 2.0),
                    RectStyle::fill(self.theme.text).radius(hr),
                );
            }
            self.set_layer(layer);
        }
        let menu_on = show_touch && cursor.menu && cursor.handle.is_none() && cursor.touch_press.is_none();
        let fade = self.presence("anvl_ed_menu", menu_on, motion::FAST, motion::FAST * 0.75);
        cursor.menu_rect = None;
        if fade > 0.001 && show_touch {
            let has_sel = cursor.ed.has_selection();
            let actions: &[(MenuAct, &str)] = if has_sel {
                &[(MenuAct::Cut, "Cut"), (MenuAct::Copy, "Copy"), (MenuAct::Paste, "Paste"), (MenuAct::SelectAll, "Select all")]
            } else {
                &[(MenuAct::Paste, "Paste"), (MenuAct::SelectAll, "Select all")]
            };
            let style = TextStyle::new(14.0, self.theme.text).medium();
            let (pad, h, gap) = (self.s(14.0), self.s(40.0), self.s(10.0));
            let widths: Vec<f32> = actions.iter().map(|(_, l)| self.text_width(l, style) + pad * 2.0).collect();
            let w: f32 = widths.iter().sum();
            let (lo, hi) = cursor.ed.selection();
            let a = rows.caret(value.as_str(), lo);
            let b = rows.caret(value.as_str(), hi);
            let cx = if a.row == b.row { x0 + (a.x + b.x) * 0.5 } else { x0 + col_w * 0.5 };
            let x = (cx - w * 0.5).clamp(r.x() + gap, (r.right() - w - gap).max(r.x() + gap));
            let above = origin_y + a.top - gap - h;
            let y = if above >= r.y() + gap {
                above
            } else {
                (origin_y + b.top + b.height + self.s(HANDLE_R * 2.0) + gap).min(r.bottom() - h - gap)
            };
            let rise = (1.0 - fade) * self.s(6.0);
            let menu = Rect::new(x, y + rise, w, h);
            cursor.menu_rect = Some(menu);
            self.pointer_overlay(menu);

            let layer = self.draw.current;
            self.set_layer(2);
            let prev = self.set_opacity(self.opacity() * fade);
            let radius = self.s(self.theme.radius.lg);
            self.glass(menu, radius, self.theme.card_hi);
            let mut sx = menu.x();
            let mut chosen = None;
            for (i, ((act, label), sw)) in actions.iter().zip(&widths).enumerate() {
                let seg = Rect::new(sx, menu.y(), *sw, h);
                let mid = MENU_IDS[i];
                if menu_on && self.click_region(mid, seg) {
                    chosen = Some(*act);
                }
                let hot = self.hover_amount(mid, seg).max(if self.active == Some(ElementId::from(mid)) { 1.0 } else { 0.0 });
                if hot > 0.01 {
                    self.rect(seg.inset(self.s(3.0)), RectStyle::fill(with_alpha(self.theme.selection, hot)).radius(self.s(self.theme.radius.md)));
                }
                if i > 0 {
                    self.rect(Rect::new(sx, menu.y() + self.s(10.0), self.line(), h - self.s(20.0)), RectStyle::fill(self.theme.border));
                }
                self.text_center(seg, label, style);
                sx += sw;
            }
            self.set_opacity(prev);
            self.set_layer(layer);

            match chosen {
                Some(MenuAct::Cut) => {
                    let rec = cursor.history.record(value, cursor.ed, EditKind::Other, now);
                    self.clipboard_out = Some(cursor.ed.selected_text(value));
                    cursor.ed.delete_selection(value);
                    cursor.history.finish(rec, true, cursor.ed, now);
                    changed = true;
                    cursor.menu = false;
                    cursor.touch_sel = false;
                }
                Some(MenuAct::Copy) => {
                    self.clipboard_out = Some(cursor.ed.selected_text(value));
                    cursor.ed.move_to(hi, false);
                    cursor.menu = false;
                    cursor.touch_sel = false;
                }
                Some(MenuAct::Paste) => {
                    self.paste_request = true;
                    cursor.menu = false;
                    cursor.touch_sel = false;
                }
                Some(MenuAct::SelectAll) => {
                    cursor.select_all(value);
                    cursor.touch_sel = true;
                }
                None => {}
            }
            if chosen.is_some() {
                self.request_wake(0.0);
            }
        }

        if max_scroll > 0.0 {
            let thumb_h = (r.h() * r.h() / content_h).max(self.s(24.0)).min(r.h());
            let ty = r.y() + (cursor.scroll / max_scroll) * (r.h() - thumb_h);
            let w = self.s(3.0);
            self.rect(
                Rect::new(r.right() - w - self.s(4.0), ty, w, thumb_h),
                RectStyle::fill(self.theme.card_hi).radius(w * 0.5),
            );
        }

        let (line, col) = line_col(value.as_str(), cursor.caret());
        EditorOutput {
            changed,
            save,
            line,
            col,
        }
    }
}

impl Ui {
    /// Shapes drawn under the editor's Markdown: a panel behind code blocks, a pill behind
    /// inline code, a bar beside quotes, and a line through rules. Text is drawn over them.
    #[allow(clippy::too_many_arguments)]
    fn markdown_decor(
        &mut self,
        value: &str,
        md: &[MdSpan],
        rows: &VisualLines,
        x0: f32,
        origin_y: f32,
        col_w: f32,
        clip: Rect,
    ) {
        if md.is_empty() {
            return;
        }
        let line = self.line();
        let pad = self.s(10.0);
        // Spans are in byte order, so the char index of each start is counted once.
        let (mut byte, mut ch) = (0usize, 0usize);
        let mut to_char = |b: usize| {
            ch += value[byte..b].chars().count();
            byte = b;
            ch
        };
        // Vertical extent of the code block being collected.
        let mut block: Option<(f32, f32)> = None;
        let flush = |ui: &mut Ui, block: &mut Option<(f32, f32)>| {
            if let Some((y0, y1)) = block.take() {
                let r = Rect::new(x0 - pad, origin_y + y0 - ui.s(4.0), col_w + pad * 2.0, y1 - y0 + ui.s(8.0));
                let r = intersect(r, clip);
                if r.h() > 0.0 {
                    ui.rect(
                        r,
                        RectStyle::fill(ui.theme.surface).radius(ui.s(ui.theme.radius.md)).border(ui.theme.border, line),
                    );
                }
            }
        };
        for s in md {
            let lo = to_char(s.range.start);
            let hi = lo + value[s.range.clone()].chars().count();
            // The rows a span covers. An empty code line still has its row.
            let extent = |rows: &VisualLines| {
                let rects = rows.selection(value, lo, hi.max(lo + 1).min(value.chars().count()));
                let caret = rows.caret(value, lo);
                rects.iter().fold((caret.top, caret.top + caret.height), |(a, b), r| (a.min(r[1]), b.max(r[1] + r[3])))
            };
            match s.kind {
                MdKind::Fence | MdKind::CodeBlock => {
                    let (a, b) = extent(rows);
                    block = Some(block.map_or((a, b), |(y0, y1)| (y0.min(a), y1.max(b))));
                    continue;
                }
                MdKind::InlineCode => {
                    for [sx, sy, sw, sh] in rows.selection(value, lo, hi) {
                        let r = Rect::new(x0 + sx - self.s(3.0), origin_y + sy + self.s(2.0), sw + self.s(6.0), sh - self.s(4.0));
                        if r.bottom() > clip.y() && r.y() < clip.bottom() {
                            self.rect(r, RectStyle::fill(self.theme.card_hi).radius(self.s(4.0)));
                        }
                    }
                }
                MdKind::QuoteMark => {
                    let (a, b) = extent(rows);
                    let bar = self.s(3.0);
                    let caret = rows.caret(value, lo);
                    let r = intersect(Rect::new(x0 + caret.x - pad, origin_y + a, bar, b - a), clip);
                    if r.h() > 0.0 {
                        self.rect(r, RectStyle::fill(self.theme.border).radius(bar * 0.5));
                    }
                }
                MdKind::Rule => {
                    let (a, b) = extent(rows);
                    let y = origin_y + (a + b) * 0.5;
                    let r = intersect(Rect::new(x0, y, col_w, line), clip);
                    if r.h() > 0.0 {
                        self.rect(r, RectStyle::fill(self.theme.border));
                    }
                }
                _ => {}
            }
            // Anything that is not code ends the block.
            flush(self, &mut block);
        }
        flush(self, &mut block);
    }
}

/// Heading sizes as multiples of the body size, by level.
fn heading_scale(level: u8) -> f32 {
    match level {
        1 => 1.6,
        2 => 1.35,
        3 => 1.15,
        _ => 1.0,
    }
}

/// How a Markdown part looks in the editor (ADL-V3 section 8.14). `px` is the body size in
/// physical pixels. Markup characters stay visible but recede, so the text reads as formatted
/// while every character is still there to edit.
fn md_style(s: &MdSpan, px: f32, opacity: f32, t: &Theme) -> RichSpan {
    let c = |color: Color| with_alpha(color, color[3] * opacity);
    let body = |kind| RichSpan::new(s.range.clone(), FontRole::Body, WEIGHT_REGULAR).color(c(kind));
    let ui = |weight, kind| RichSpan::new(s.range.clone(), FontRole::Ui, weight).color(c(kind));
    match s.kind {
        MdKind::HeadingMark(l) => ui(WEIGHT_REGULAR, t.disabled).size(px * heading_scale(l)),
        MdKind::Heading(l) => ui(WEIGHT_MEDIUM, t.text).size(px * heading_scale(l)),
        MdKind::ListMark => body(t.text_dim),
        MdKind::TaskDone => body(t.disabled),
        MdKind::QuoteMark => body(t.disabled),
        MdKind::Quote => body(t.text_dim).italic(),
        MdKind::Fence | MdKind::CodeMark | MdKind::EmphMark | MdKind::Rule => ui(WEIGHT_REGULAR, t.disabled),
        MdKind::CodeBlock | MdKind::InlineCode => {
            RichSpan::new(s.range.clone(), FontRole::Mono, WEIGHT_MEDIUM).color(c(t.code)).size(px * 0.9)
        }
        MdKind::Bold => RichSpan::new(s.range.clone(), FontRole::Body, WEIGHT_MEDIUM).color(c(t.text)),
        MdKind::Italic => body(t.text).italic(),
    }
}

/// The part of `a` that lies inside `b`. Empty if they do not overlap.
fn intersect(a: Rect, b: Rect) -> Rect {
    let (x0, y0) = (a.x().max(b.x()), a.y().max(b.y()));
    let (x1, y1) = (a.right().min(b.right()), a.bottom().min(b.bottom()));
    Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0.0, 0.0, 800.0, 600.0);

    struct Harness {
        ui: Ui,
        text: String,
        cur: EditorCursor,
        t: f32,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                ui: Ui::new(&crate::icons::IconAtlas::build()),
                text: String::new(),
                cur: EditorCursor::default(),
                t: 0.0,
            }
        }

        fn frame(&mut self, input: Input) -> EditorOutput {
            self.t += 0.016;
            self.ui.begin(input, Vec2::new(800.0, 600.0), 1.0, self.t, 0.016);
            let out = self.ui.text_editor("ed", AREA, &mut self.text, &mut self.cur, "Start typing");
            self.ui.end();
            out
        }

        fn click(&mut self, x: f32, y: f32) {
            self.frame(Input {
                mouse: Vec2::new(x, y),
                pressed: true,
                down: true,
                ..Input::default()
            });
            self.frame(Input {
                mouse: Vec2::new(x, y),
                released: true,
                ..Input::default()
            });
        }

        fn type_str(&mut self, s: &str) {
            self.frame(Input {
                typed: s.into(),
                ..Input::default()
            });
        }

        fn key(&mut self, k: UiKey) -> EditorOutput {
            self.frame(Input {
                keys: vec![k],
                ..Input::default()
            })
        }
    }

    fn finger(x: f32, y: f32, pressed: bool, down: bool, released: bool) -> Input {
        Input { mouse: Vec2::new(x, y), pressed, down, released, touch: true, ..Input::default() }
    }

    #[test]
    fn long_press_selects_a_word_and_opens_the_menu() {
        let mut h = Harness::new();
        h.text = "alpha beta gamma".into();
        // First tap focuses.
        h.frame(finger(80.0, 32.0, true, true, false));
        h.frame(finger(80.0, 32.0, false, false, true));
        h.frame(finger(80.0, 32.0, true, true, false));
        for _ in 0..40 {
            h.frame(finger(80.0, 32.0, false, true, false));
        }
        h.frame(finger(80.0, 32.0, false, false, true));
        let (lo, hi) = h.cur.ed.selection();
        assert!(hi > lo, "a word is selected");
        assert!(!h.cur.ed.selected_text(&h.text).contains(' '));
        assert!(h.cur.touch_sel && h.cur.menu);
        // The menu draws next frame. Its first button is Cut.
        h.frame(finger(80.0, 32.0, false, false, false));
        let menu = h.cur.menu_rect.expect("menu shown");
        let cut = Vec2::new(menu.x() + 10.0, menu.center().y);
        let before = h.text.clone();
        h.frame(finger(cut.x, cut.y, true, true, false));
        h.frame(finger(cut.x, cut.y, false, false, true));
        assert!(h.text.len() < before.len(), "cut removed the word");
        assert!(h.ui.take_clipboard().is_some());
    }

    #[test]
    fn double_tap_selects_a_word_and_a_drag_scrolls_without_moving_the_caret() {
        let mut h = Harness::new();
        h.text = "one two three".into();
        for _ in 0..2 {
            h.frame(finger(30.0, 32.0, true, true, false));
            h.frame(finger(30.0, 32.0, false, false, true));
        }
        assert!(h.cur.ed.has_selection());
        h.text = "line
".repeat(200);
        h.cur.ed.move_to(0, false);
        h.frame(finger(100.0, 400.0, true, true, false));
        for i in 1..10 {
            h.frame(finger(100.0, 400.0 - i as f32 * 30.0, false, true, false));
        }
        h.frame(finger(100.0, 130.0, false, false, true));
        assert_eq!(h.cur.caret(), 0, "a scroll never moves the caret");
        let after_release = h.cur.scroll;
        assert!(after_release > 100.0);
        h.frame(Input { touch: true, ..Input::default() });
        assert!(h.cur.scroll > after_release, "the page keeps moving after the finger lifts");
    }

    #[test]
    fn undo_takes_back_a_word_and_redo_puts_it_back() {
        let mut h = Harness::new();
        h.click(100.0, 100.0);
        for c in ["h", "i", " ", "y", "o", "u"] {
            h.type_str(c);
        }
        assert_eq!(h.text, "hi you");
        assert!(h.key(UiKey::Undo).changed);
        assert_eq!(h.text, "hi");
        h.key(UiKey::Undo);
        assert_eq!(h.text, "");
        assert!(h.key(UiKey::Redo).changed);
        assert_eq!(h.text, "hi");
        assert_eq!(h.cur.caret(), 2);
    }

    #[test]
    fn markdown_headings_make_taller_rows() {
        let mut h = Harness::new();
        h.text = "# Big
body".into();
        h.frame(Input::default());
        let mut ui = Ui::new(&crate::icons::IconAtlas::build());
        ui.begin(Input::default(), Vec2::new(800.0, 600.0), 1.0, 0.0, 0.016);
        let md = markdown::highlight(&h.text);
        let style = TextStyle::new(EDITOR_FONT, Theme::adl_dark().text).body();
        let (_, rows) = ui.shape_editor(&h.text, &md, style, 600.0, 24.0);
        let head = rows.caret(&h.text, 3);
        let body = rows.caret(&h.text, 8);
        assert!(head.height > body.height * 1.3, "{} vs {}", head.height, body.height);
    }

    #[test]
    fn click_focuses_and_typing_inserts() {
        let mut h = Harness::new();
        h.type_str("ignored");
        assert_eq!(h.text, "", "an unfocused editor takes no text");
        h.click(100.0, 100.0);
        h.type_str("hello");
        assert_eq!(h.text, "hello");
        let out = h.key(UiKey::Enter);
        assert!(out.changed);
        h.type_str("world");
        assert_eq!(h.text, "hello\nworld");
        let out = h.frame(Input::default());
        assert_eq!((out.line, out.col), (2, 6));
    }

    #[test]
    fn tab_inserts_spaces_instead_of_moving_focus() {
        let mut h = Harness::new();
        h.click(100.0, 100.0);
        h.frame(Input::default());
        h.key(UiKey::Tab);
        h.type_str("x");
        assert_eq!(h.text, "  x");
        assert_eq!(h.ui.focused_id(), Some(ElementId::from("ed")));
    }

    #[test]
    fn arrows_home_end_and_vertical_moves() {
        let mut h = Harness::new();
        h.click(100.0, 100.0);
        h.type_str("abcd");
        h.key(UiKey::Enter);
        h.type_str("ef");
        h.key(UiKey::ArrowUp);
        assert_eq!(h.cur.caret(), 2, "up keeps the column, clamped to the short line");
        h.key(UiKey::End);
        assert_eq!(h.cur.caret(), 4);
        h.key(UiKey::Home);
        assert_eq!(h.cur.caret(), 0);
        h.key(UiKey::ArrowDown);
        assert_eq!(h.cur.caret(), 5, "down from the first line lands on the second");
        h.key(UiKey::ArrowLeft);
        assert_eq!(h.cur.caret(), 4);
    }

    #[test]
    fn select_all_copy_cut_and_paste() {
        let mut h = Harness::new();
        h.click(100.0, 100.0);
        h.type_str("copy me");
        h.key(UiKey::SelectAll);
        h.key(UiKey::Copy);
        assert_eq!(h.ui.take_clipboard().as_deref(), Some("copy me"));
        h.key(UiKey::Cut);
        assert_eq!(h.text, "");
        assert_eq!(h.ui.take_clipboard().as_deref(), Some("copy me"));
        h.frame(Input {
            paste: Some("a\r\nb".into()),
            ..Input::default()
        });
        assert_eq!(h.text, "a\nb", "pasted CRLF becomes LF");
    }

    #[test]
    fn save_key_is_reported() {
        let mut h = Harness::new();
        h.click(100.0, 100.0);
        assert!(h.key(UiKey::Save).save);
    }

    #[test]
    fn clicking_text_moves_the_caret_and_drag_selects() {
        let mut h = Harness::new();
        h.click(100.0, 100.0);
        h.type_str("one two three");
        // The text column starts at x 28 and the first row at y 20.
        h.click(29.0, 32.0);
        assert!(h.cur.caret() <= 1, "a click at the text origin lands at the start");
        h.frame(Input {
            mouse: Vec2::new(29.0, 32.0),
            pressed: true,
            down: true,
            ..Input::default()
        });
        h.frame(Input {
            mouse: Vec2::new(700.0, 32.0),
            down: true,
            ..Input::default()
        });
        assert_eq!(h.cur.ed.selection(), (0, 13), "dragging to the right of the line selects it");
    }

    #[test]
    fn long_text_scrolls_to_keep_the_caret_visible() {
        let mut h = Harness::new();
        h.click(100.0, 100.0);
        let body: String = (0..80).map(|i| format!("line {i}\n")).collect();
        h.frame(Input {
            paste: Some(body),
            ..Input::default()
        });
        assert!(h.cur.scroll > 0.0, "the view followed the caret down");
        h.frame(Input {
            mouse: Vec2::new(300.0, 300.0),
            scroll: -10_000.0,
            ..Input::default()
        });
        assert_eq!(h.cur.scroll, 0.0, "scrolling clamps at the top");
    }

    #[test]
    fn idle_editor_asks_to_wake_for_the_blink_only() {
        let mut h = Harness::new();
        h.frame(Input::default());
        assert_eq!(h.ui.next_wake(), None, "an unfocused editor lets the app sleep");
        h.click(100.0, 100.0);
        for _ in 0..40 {
            h.frame(Input::default());
        }
        let wake = h.ui.next_wake().expect("a focused caret blinks");
        assert!(wake > 0.0 && wake <= BLINK + 0.03, "{wake}");
    }
}
