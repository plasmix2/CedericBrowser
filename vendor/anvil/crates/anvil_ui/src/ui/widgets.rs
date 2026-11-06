//! Extra widgets for the component library: drag and scroll primitives, tab bars, breadcrumbs,
//! chevrons, skeletons, toasts, dropzones, pagination, swatches, range sliders, calendars, tag
//! chips, and floating layers. Declared as a child of `ui`, so it can use the widget internals.

use super::*;
use crate::logic::page_items;

impl Ui {
    // ------------------------------------------------------------------ input queries

    /// Pointer position in window pixels.
    pub fn pointer(&self) -> Vec2 {
        self.input.mouse
    }

    /// Seconds since the previous frame.
    pub fn frame_dt(&self) -> f32 {
        self.dt
    }

    /// True on the frame the secondary (right) button went down inside `r`.
    pub fn secondary_in(&self, r: Rect) -> bool {
        self.input.secondary && self.hovered(r)
    }

    /// Scroll wheel movement this frame, if the pointer is inside `r`. Positive scrolls down.
    pub fn scroll_in(&self, r: Rect) -> f32 {
        if self.hovered(r) {
            self.input.scroll
        } else {
            0.0
        }
    }

    /// True on the frame `key` was pressed. Read keys before the widget that handles them is built,
    /// since a text field consumes Enter and Escape.
    pub fn key_pressed(&self, key: UiKey) -> bool {
        self.input.keys.contains(&key)
    }

    /// True on the frame the primary button went down outside every rectangle in `areas`.
    /// Use it to dismiss popovers and modals.
    pub fn pressed_outside(&self, areas: &[Rect]) -> bool {
        self.input.pressed && !areas.iter().any(|a| a.contains(self.input.mouse))
    }

    /// True on the frame Ctrl+K was pressed.
    pub fn command_pressed(&self) -> bool {
        self.input.keys.contains(&UiKey::Command)
    }

    // ------------------------------------------------------------------ pointer primitives

    /// Pointer movement since the last frame, while the button is held after a press inside
    /// `hit`. The press frame itself returns zero, so a long jump is not reported as a drag.
    pub fn drag_delta(&mut self, id: impl Into<ElementId>, hit: Rect) -> Option<Vec2> {
        let id: ElementId = id.into();
        if self.input.pressed && self.hovered(hit) {
            self.active = Some(id);
            return Some(Vec2::ZERO);
        }
        if self.active == Some(id) && self.input.down {
            Some(self.input.mouse - self.mouse_last)
        } else {
            None
        }
    }

    /// Fraction 0..1 along the horizontal `track` while the pointer is held after a press inside
    /// `hit`. Pressing jumps to the pointer, and dragging follows it.
    pub fn track_fraction(&mut self, id: impl Into<ElementId>, hit: Rect, track: Rect) -> Option<f32> {
        let id: ElementId = id.into();
        if self.input.pressed && self.hovered(hit) {
            self.active = Some(id);
        }
        if self.active == Some(id) && self.input.down {
            Some(((self.input.mouse.x - track.x()) / track.w().max(1.0)).clamp(0.0, 1.0))
        } else {
            None
        }
    }

    /// True while `id` holds text focus.
    pub fn is_focused(&self, id: impl Into<ElementId>) -> bool {
        let id: ElementId = id.into();
        self.focus == Some(id)
    }

    /// Vertical splitter between two panes. Drag the handle to move it. `ratio` is the share of
    /// `span` given to the first pane. Returns true when it changed.
    pub fn splitter(&mut self, id: impl Into<ElementId>, handle: Rect, span: f32, ratio: &mut f32, min: f32, max: f32) -> bool {
        let id: ElementId = id.into();
        let hit = Rect::new(handle.x() - self.s(4.0), handle.y(), handle.w() + self.s(8.0), handle.h());
        let hover = self.animate(id, CH_HOVER, if self.hovered(hit) { 1.0 } else { 0.0 }, 14.0);
        let mut changed = false;
        if let Some(d) = self.drag_delta(id, hit) {
            if span > 1.0 && d.x != 0.0 {
                let next = (*ratio + d.x / span).clamp(min, max);
                if (next - *ratio).abs() > f32::EPSILON {
                    *ratio = next;
                    changed = true;
                }
            }
        }
        if self.hovered(hit) || self.dragging(id) {
            self.want_cursor(Cursor::ResizeHorizontal);
        }
        let line = self.line();
        let grip = lerp(self.theme.border, self.theme.ring, hover.max(if changed { 1.0 } else { 0.0 }));
        self.rect(Rect::new(handle.x() + handle.w() * 0.5 - line * 0.5, handle.y(), line, handle.h()), RectStyle::fill(grip));
        changed
    }

    // ------------------------------------------------------------------ scrolling

    /// Vertical scrollbar for content of height `content` shown in `view` pixels. The thumb can be
    /// dragged, and a click on the track pages toward the pointer. Returns true when the offset
    /// changed. Wheel scrolling is handled by the caller from `Input::scroll`.
    pub fn scrollbar(&mut self, id: impl Into<ElementId>, track: Rect, content: f32, view: f32, offset: &mut f32) -> bool {
        let id: ElementId = id.into();
        let max_off = (content - view).max(0.0);
        if max_off <= 0.0 || track.h() <= 0.0 {
            *offset = 0.0;
            return false;
        }
        let thumb_h = (track.h() * view / content).clamp(self.s(28.0).min(track.h()), track.h());
        let travel = (track.h() - thumb_h).max(0.0);
        let thumb_y = track.y() + if max_off > 0.0 { travel * (*offset / max_off) } else { 0.0 };
        let thumb = Rect::new(track.x(), thumb_y, track.w(), thumb_h);
        let mut changed = false;

        // A press on the track, outside the thumb, pages one screen toward the pointer.
        if self.input.pressed && self.hovered(track) && !thumb.contains(self.input.mouse) {
            let dir = if self.input.mouse.y < thumb.y() { -1.0 } else { 1.0 };
            let next = (*offset + dir * view * 0.9).clamp(0.0, max_off);
            if (next - *offset).abs() > f32::EPSILON {
                *offset = next;
                changed = true;
            }
        }
        if let Some(d) = self.drag_delta(id, thumb) {
            if travel > 0.0 && d.y != 0.0 {
                let next = (*offset + d.y * max_off / travel).clamp(0.0, max_off);
                if (next - *offset).abs() > f32::EPSILON {
                    *offset = next;
                    changed = true;
                }
            }
        }

        if self.dragging(id) {
            self.want_cursor(Cursor::Grabbing);
        } else if self.hovered(thumb) {
            self.want_cursor(Cursor::Grab);
        }
        let hover = self.animate(id, CH_HOVER, if self.hovered(thumb) { 1.0 } else { 0.0 }, 14.0);
        self.rect(track, RectStyle::fill(self.theme.sunken).radius(track.w() * 0.5));
        self.rect(
            thumb,
            RectStyle::fill(lerp(self.theme.border, self.theme.muted, hover)).radius(track.w() * 0.5),
        );
        changed
    }

    // ------------------------------------------------------------------ navigation

    /// Document tab bar with close buttons and an active underline. `ids` select a tab and
    /// `close_ids` close it, one of each per tab. Returns the index of a tab closed this frame.
    pub fn tab_bar(
        &mut self,
        ids: &[&'static str],
        close_ids: &[&'static str],
        rect: Rect,
        labels: &[&str],
        active: &mut usize,
    ) -> Option<usize> {
        let tab_w = self.s(168.0);
        let mut closed = None;
        for (i, label) in labels.iter().enumerate() {
            let x = rect.x() + i as f32 * tab_w;
            if x >= rect.right() {
                break;
            }
            let r = Rect::new(x, rect.y(), tab_w.min(rect.right() - x), rect.h());
            let close = Rect::new(r.right() - self.s(30.0), r.center().y - self.s(9.0), self.s(18.0), self.s(18.0));
            let close_hit = Rect::new(close.x() - self.s(5.0), close.y() - self.s(5.0), close.w() + self.s(10.0), close.h() + self.s(10.0));
            if self.click_region(close_ids[i], close_hit) {
                closed = Some(i);
            }
            let body = Rect::new(r.x(), r.y(), (r.w() - self.s(30.0)).max(0.0), r.h());
            if self.click_region(ids[i], body) {
                *active = i;
            }

            let is_active = *active == i;
            let hover = self.animate(ids[i], CH_HOVER, if self.hovered(r) { 1.0 } else { 0.0 }, 14.0);
            let fill = if is_active {
                self.theme.card
            } else {
                with_alpha(self.theme.card_hi, 0.5 * hover)
            };
            self.rect(r, RectStyle::fill(fill));
            if is_active {
                let bar = self.s(2.0);
                self.rect(Rect::new(r.x(), r.bottom() - bar, r.w(), bar), RectStyle::fill(self.theme.primary));
            }
            let label_style = TextStyle::new(14.0, if is_active { self.theme.text } else { self.theme.muted }).medium();
            let shown = self.fit_text(label, label_style, body.w() - self.s(20.0));
            let (key, tsize) = self.measure(&shown, label_style);
            self.draw.push_text(TextCmd {
                key,
                x: r.x() + self.s(12.0),
                y: r.center().y - tsize.y * 0.5,
                color: label_style.color,
                clip: body,
            });
            let close_tint = if self.hovered(close_hit) { self.theme.text } else { self.theme.muted };
            self.icon(close, Icon::X, close_tint);
        }
        closed
    }

    /// Breadcrumb trail. `ids` belong to the parts. The last part is the current location and
    /// is not a link. Returns the index of a part clicked this frame.
    pub fn breadcrumb(&mut self, ids: &[&'static str], rect: Rect, parts: &[&str]) -> Option<usize> {
        let mut x = rect.x();
        let mut clicked = None;
        let last = parts.len().saturating_sub(1);
        for (i, part) in parts.iter().enumerate() {
            let is_last = i == last;
            let style = TextStyle::new(14.0, if is_last { self.theme.text } else { self.theme.muted });
            let w = self.text_width(part, style);
            let hit = Rect::new(x - self.s(4.0), rect.y(), w + self.s(8.0), rect.h());
            if !is_last && self.click_region(ids[i], hit) {
                clicked = Some(i);
            }
            let color = if !is_last && self.hovered(hit) {
                self.theme.text
            } else {
                style.color
            };
            let (key, tsize) = self.measure(part, style);
            self.draw.push_text(TextCmd {
                key,
                x,
                y: rect.center().y - tsize.y * 0.5,
                color,
                clip: rect,
            });
            x += w;
            if !is_last {
                let sep = self.s(14.0);
                self.icon(
                    Rect::new(x + self.s(4.0), rect.center().y - sep * 0.5, sep, sep),
                    Icon::ChevronRight,
                    self.theme.muted,
                );
                x += self.s(22.0);
            }
        }
        clicked
    }

    // ------------------------------------------------------------------ disclosure

    /// Chevron that turns from pointing right to pointing down as `expanded` goes true. The
    /// rotation eases over time. Returns true on click, and the caller flips its own state.
    pub fn chevron_toggle(&mut self, id: impl Into<ElementId>, r: Rect, expanded: bool) -> bool {
        let id: ElementId = id.into();
        let clicked = self.click_region(id, r);
        let hover = self.animate(id, CH_HOVER, if self.hovered(r) { 1.0 } else { 0.0 }, 14.0);
        let t = self.animate(id, CH_TOGGLE, if expanded { 1.0 } else { 0.0 }, 18.0);
        let px = self.s(16.0);
        let c = r.center();
        self.icon_rotated(
            Rect::new(c.x - px * 0.5, c.y - px * 0.5, px, px),
            Icon::ChevronRight,
            lerp(self.theme.muted, self.theme.text, hover),
            t * std::f32::consts::FRAC_PI_2,
        );
        clicked
    }

    // ------------------------------------------------------------------ feedback

    /// Thin progress bar. `fraction` is clamped to 0..1.
    pub fn progress(&mut self, r: Rect, fraction: f32, color: Color) {
        let f = fraction.clamp(0.0, 1.0);
        self.rect(r, RectStyle::fill(self.theme.border).radius(r.h() * 0.5));
        if f > 0.0 {
            self.rect(Rect::new(r.x(), r.y(), r.w() * f, r.h()), RectStyle::fill(color).radius(r.h() * 0.5));
        }
    }

    /// Loading placeholder with a light band sweeping across it. `phase` runs 0 to 1 and repeats.
    pub fn skeleton(&mut self, r: Rect, phase: f32) {
        self.rect(r, RectStyle::fill(self.theme.card_hi).radius(self.s(self.theme.radius.md)));
        let band_w = r.w() * 0.35;
        let x = r.x() + (r.w() + band_w) * phase.rem_euclid(1.0) - band_w;
        let x0 = x.max(r.x());
        let x1 = (x + band_w).min(r.right());
        if x1 > x0 {
            self.rect(
                Rect::new(x0, r.y(), x1 - x0, r.h()),
                RectStyle::fill(with_alpha(self.theme.on_color, 0.06)),
            );
        }
    }

    /// One toast notification. `alpha` fades the whole card in or out.
    pub fn toast_card(&mut self, r: Rect, title: &str, message: &str, color: Color, alpha: f32) {
        let a = alpha.clamp(0.0, 1.0);
        let line = self.line();
        self.rect(
            r,
            RectStyle::fill(with_alpha(self.theme.card, a))
                .radius(self.s(self.theme.radius.lg))
                .border(with_alpha(self.theme.border, a), line),
        );
        let bar = self.s(3.0);
        self.rect(
            Rect::new(r.x() + self.s(10.0), r.y() + self.s(12.0), bar, (r.h() - self.s(24.0)).max(0.0)),
            RectStyle::fill(with_alpha(color, a)).radius(bar * 0.5),
        );
        let x = r.x() + self.s(24.0);
        let inner_w = (r.right() - x - self.s(14.0)).max(0.0);
        let title_style = TextStyle::new(15.0, with_alpha(self.theme.text, a)).medium();
        self.text_fit(x, r.y() + self.s(12.0), title, title_style, inner_w);
        let body_style = TextStyle::new(14.0, with_alpha(self.theme.text_dim, a));
        self.text_fit(x, r.y() + self.s(40.0), message, body_style, inner_w);
    }

    // ------------------------------------------------------------------ upload

    /// Dashed drop area. Returns true when clicked. `active` shows that a file is hovering over it.
    pub fn dropzone(&mut self, id: impl Into<ElementId>, r: Rect, active: bool) -> bool {
        let id: ElementId = id.into();
        let clicked = self.click_region(id, r);
        let hover = self.animate(id, CH_HOVER, if self.hovered(r) || active { 1.0 } else { 0.0 }, 14.0);
        let color = lerp(self.theme.border, self.theme.ring, hover);
        let t = self.s(1.5).max(1.0);
        let dash = self.s(10.0);
        let gap = self.s(6.0);
        let mut x = r.x();
        while x < r.right() {
            let w = dash.min(r.right() - x);
            self.rect(Rect::new(x, r.y(), w, t), RectStyle::fill(color));
            self.rect(Rect::new(x, r.bottom() - t, w, t), RectStyle::fill(color));
            x += dash + gap;
        }
        let mut y = r.y();
        while y < r.bottom() {
            let h = dash.min(r.bottom() - y);
            self.rect(Rect::new(r.x(), y, t, h), RectStyle::fill(color));
            self.rect(Rect::new(r.right() - t, y, t, h), RectStyle::fill(color));
            y += dash + gap;
        }
        let icon = self.s(28.0);
        let content_h = icon + self.s(12.0) + self.text_height_ui(14.0);
        let top = r.center().y - content_h * 0.5;
        self.icon(
            Rect::new(r.center().x - icon * 0.5, top, icon, icon),
            Icon::Upload,
            lerp(self.theme.muted, self.theme.text, hover),
        );
        self.text_center(
            Rect::new(r.x(), top + icon + self.s(12.0), r.w(), self.text_height_ui(14.0)),
            "Drop files here or click to browse",
            TextStyle::new(14.0, self.theme.text_dim),
        );
        clicked
    }

    fn text_height_ui(&self, size: f32) -> f32 {
        self.s(size) * FontRole::Ui.line_height()
    }

    // ------------------------------------------------------------------ pagination

    /// Previous, page, and next buttons. `page_ids` needs one id per slot (at least nine).
    /// Returns true when the page changed.
    pub fn pagination(
        &mut self,
        prev_id: &'static str,
        next_id: &'static str,
        page_ids: &[&'static str],
        rect: Rect,
        current: &mut usize,
        total: usize,
    ) -> bool {
        let mut changed = false;
        let nav_w = self.s(124.0);
        let h = rect.h();
        let prev = Rect::new(rect.x(), rect.y(), nav_w, h);
        let next = Rect::new(rect.right() - nav_w, rect.y(), nav_w, h);
        if self.button(prev_id, prev, "Prev", Some(Icon::ArrowLeft), ButtonVariant::Secondary, *current > 1) {
            *current -= 1;
            changed = true;
        }
        if self.button(next_id, next, "Next", Some(Icon::ArrowRight), ButtonVariant::Secondary, *current < total) {
            *current += 1;
            changed = true;
        }

        let items = page_items(*current, total);
        let cell = h;
        let gap = self.s(6.0);
        let span = items.len() as f32 * cell + (items.len().saturating_sub(1)) as f32 * gap;
        let mut x = rect.center().x - span * 0.5;
        for (slot, item) in items.iter().enumerate() {
            let r = Rect::new(x, rect.y(), cell, h);
            match item {
                Some(page) => {
                    let id = page_ids.get(slot).copied().unwrap_or("pg-extra");
                    let variant = if *page == *current {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Ghost
                    };
                    if self.button(id, r, &page.to_string(), None, variant, true) {
                        *current = *page;
                        changed = true;
                    }
                }
                None => {
                    self.text_center(r, "\u{2026}", TextStyle::new(16.0, self.theme.muted));
                }
            }
            x += cell + gap;
        }
        changed
    }

    // ------------------------------------------------------------------ colour

    /// Grid of colour swatches. The selected one has a light outline. Returns true when the
    /// selection changed.
    pub fn swatch_grid(&mut self, ids: &[&'static str], rect: Rect, colors: &[Color], selected: &mut usize) -> bool {
        let cell = self.s(34.0);
        let gap = self.s(8.0);
        let cols = (((rect.w() + gap) / (cell + gap)).floor() as usize).max(1);
        let mut changed = false;
        for (i, color) in colors.iter().enumerate() {
            let col = i % cols;
            let row = i / cols;
            let r = Rect::new(
                rect.x() + col as f32 * (cell + gap),
                rect.y() + row as f32 * (cell + gap),
                cell,
                cell,
            );
            if r.bottom() > rect.bottom() + 0.5 {
                break;
            }
            let id = ids.get(i).copied().unwrap_or("sw-extra");
            if self.click_region(id, r) {
                *selected = i;
                changed = true;
            }
            let outline = if *selected == i {
                (self.theme.text, self.s(2.0))
            } else {
                (self.theme.border, self.line())
            };
            self.rect(r, RectStyle::fill(*color).radius(self.s(self.theme.radius.md)).border(outline.0, outline.1));
        }
        changed
    }

    // ------------------------------------------------------------------ ranges

    /// Two-handle range slider over `0..=max`. Press near a handle to pick it up, then drag.
    /// `lo` never passes `hi`. Returns true when either value changed.
    pub fn dual_range(&mut self, id: impl Into<ElementId>, rect: Rect, lo: &mut u8, hi: &mut u8, max: u8) -> bool {
        let id: ElementId = id.into();
        let track_h = self.s(6.0);
        let track = Rect::new(rect.x(), rect.y() + (rect.h() - track_h) * 0.5, rect.w(), track_h);
        let hit = Rect::new(rect.x(), rect.y() - self.s(12.0), rect.w(), rect.h() + self.s(24.0));
        let denom = max.max(1) as f32;
        let lo_x = track.x() + track.w() * (*lo as f32 / denom);
        let hi_x = track.x() + track.w() * (*hi as f32 / denom);

        if self.input.pressed && self.hovered(hit) {
            let mx = self.input.mouse.x;
            let knob = if (mx - lo_x).abs() <= (mx - hi_x).abs() { 0.0 } else { 1.0 };
            self.set_anim(id, CH_KNOB, knob);
        }
        let knob = self.anim.get(&(id, CH_KNOB)).copied().unwrap_or(0.0) as u8;

        let mut changed = false;
        if let Some(frac) = self.track_fraction(id, hit, track) {
            let v = (frac * denom).round() as u8;
            if knob == 0 {
                let next = v.min(*hi);
                if next != *lo {
                    *lo = next;
                    changed = true;
                }
            } else {
                let next = v.max(*lo);
                if next != *hi {
                    *hi = next;
                    changed = true;
                }
            }
        }

        if self.dragging(id) {
            self.want_cursor(Cursor::Grabbing);
        } else if self.hovered(hit) {
            self.want_cursor(Cursor::Pointer);
        }
        self.rect(track, RectStyle::fill(self.theme.border).radius(track_h * 0.5));
        let lo_px = track.x() + track.w() * (*lo as f32 / denom);
        let hi_px = track.x() + track.w() * (*hi as f32 / denom);
        self.rect(
            Rect::new(lo_px, track.y(), (hi_px - lo_px).max(0.0), track_h),
            RectStyle::fill(self.theme.primary).radius(track_h * 0.5),
        );
        let d = self.s(16.0);
        for x in [lo_px, hi_px] {
            self.rect(
                Rect::new(x - d * 0.5, track.center().y - d * 0.5, d, d),
                RectStyle::fill(self.theme.text).radius(d * 0.5).border(self.theme.bg, self.line()),
            );
        }
        changed
    }

    // ------------------------------------------------------------------ calendar

    /// Month grid of day buttons under a weekday header. `ids` needs 42 entries, one per cell.
    /// Clicking a day selects it. Returns true when the selection changed.
    pub fn date_grid(
        &mut self,
        ids: &[&'static str],
        rect: Rect,
        grid: &[Option<u32>],
        selected: &mut Option<u32>,
    ) -> bool {
        let names = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
        let gap = self.s(4.0);
        let cell_w = (rect.w() - gap * 6.0) / 7.0;
        let head_h = self.text_height_ui(12.0);
        for (c, name) in names.iter().enumerate() {
            let r = Rect::new(rect.x() + c as f32 * (cell_w + gap), rect.y(), cell_w, head_h);
            self.text_center(r, name, TextStyle::new(12.0, self.theme.muted).medium());
        }
        let body_top = rect.y() + head_h + self.s(6.0);
        let cell_h = ((rect.bottom() - body_top - gap * 5.0) / 6.0).max(0.0);
        let mut changed = false;
        for (i, day) in grid.iter().enumerate().take(42) {
            let col = i % 7;
            let row = i / 7;
            let r = Rect::new(
                rect.x() + col as f32 * (cell_w + gap),
                body_top + row as f32 * (cell_h + gap),
                cell_w,
                cell_h,
            );
            let Some(day) = *day else { continue };
            let id = ids.get(i).copied().unwrap_or("day-extra");
            let variant = if *selected == Some(day) {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Ghost
            };
            if self.button(id, r, &day.to_string(), None, variant, true) {
                *selected = Some(day);
                changed = true;
            }
        }
        changed
    }

    // ------------------------------------------------------------------ chips

    /// A row of removable tags. Clicking a tag's x removes it. Returns the index removed.
    pub fn tag_chips(&mut self, ids: &[&'static str], rect: Rect, tags: &[String]) -> Option<usize> {
        let h = rect.h();
        let gap = self.s(6.0);
        let pad = self.s(10.0);
        let mut x = rect.x();
        let mut removed = None;
        for (i, tag) in tags.iter().enumerate() {
            let label_style = TextStyle::new(13.0, self.theme.text).medium();
            let label_w = self.text_width(tag, label_style);
            let w = pad + label_w + self.s(8.0) + self.s(14.0) + pad;
            if x + w > rect.right() {
                break;
            }
            let r = Rect::new(x, rect.y(), w, h);
            self.rect(r, RectStyle::fill(self.theme.card_hi).radius(h * 0.5).border(self.theme.border, self.line()));
            let (key, tsize) = self.measure(tag, label_style);
            self.draw.push_text(TextCmd {
                key,
                x: x + pad,
                y: r.center().y - tsize.y * 0.5,
                color: self.theme.text,
                clip: r,
            });
            let x_size = self.s(14.0);
            let x_r = Rect::new(r.right() - pad - x_size, r.center().y - x_size * 0.5, x_size, x_size);
            let id = ids.get(i).copied().unwrap_or("tag-extra");
            let hit = Rect::new(x_r.x() - self.s(4.0), r.y(), x_size + self.s(8.0), h);
            if self.click_region(id, hit) {
                removed = Some(i);
            }
            self.icon(x_r, Icon::X, self.theme.muted);
            x += w + gap;
        }
        removed
    }

    // ------------------------------------------------------------------ floating layers

    /// Starts a floating layer at `rect`: a card drawn above the base UI, which takes the pointer
    /// while open. Draw its contents next, then call [`Ui::end_floating`].
    pub fn begin_floating(&mut self, rect: Rect) {
        self.set_layer(1);
        self.pointer_overlay(rect);
        self.surface(rect);
    }

    /// Ends a floating layer started with [`Ui::begin_floating`].
    pub fn end_floating(&mut self) {
        self.set_layer(0);
    }

    /// Pre-declares a modal panel at the start of a frame before background widgets are built,
    /// guaranteeing background widgets cannot be hovered, clicked, dragged, or scrolled even on
    /// the very first frame the modal opens.
    pub fn block_background_for_modal(&mut self, panel: Rect) {
        self.modal_prev = Some(panel);
        self.modal_next = Some(panel);
        self.modal_scoped_prev = true;
        self.modal_scoped_next = true;
    }

    /// Starts a modal: a dimming backdrop over the window, with `panel` as the only area that
    /// takes the pointer. Draw the modal's contents next, then call [`Ui::end_modal`].
    pub fn begin_modal(&mut self, panel: Rect) {
        self.modal_prev_layer = self.draw.current;
        self.in_modal = true;
        self.modal_scoped_next = true;
        self.modal_scoped_prev = true;
        self.modal_prev = Some(panel);
        self.set_layer((self.draw.current + 1).min(LAYER_COUNT - 1));
        let screen = self.screen_rect();
        self.rect(screen, RectStyle::fill(with_alpha(self.theme.bg, 0.72)));
        self.pointer_modal(panel);
        self.surface(panel);
    }

    /// Ends a modal started with [`Ui::begin_modal`].
    pub fn end_modal(&mut self) {
        self.in_modal = false;
        self.set_layer(self.modal_prev_layer);
    }
}

/// Animation channel for the second thumb of a range slider, which knob was picked up.
const CH_KNOB: u8 = 30;
