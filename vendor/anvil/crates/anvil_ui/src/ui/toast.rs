//! The toast stack (ADL-V3 section 8.10): short notices that rise in above the bottom edge,
//! stack newest-first from the bottom, and leave on their own. A toast can carry one action,
//! such as Undo. Hovering a toast holds it. A toast can be closed with its X or swiped aside.
//! Declared as a child of `ui`, so it can use the widget internals.

use super::*;
use crate::theme::motion;

/// Toasts on screen at once. An older one leaves when a newer one would make more.
const MAX_VISIBLE: usize = 3;
/// Widget ids by slot. A toast keeps its slot for its whole life.
const SLOTS: usize = 8;
const BODY_IDS: [&str; SLOTS] = ["toast0", "toast1", "toast2", "toast3", "toast4", "toast5", "toast6", "toast7"];
const ACTION_IDS: [&str; SLOTS] = ["toast_a0", "toast_a1", "toast_a2", "toast_a3", "toast_a4", "toast_a5", "toast_a6", "toast_a7"];
const CLOSE_IDS: [&str; SLOTS] = ["toast_x0", "toast_x1", "toast_x2", "toast_x3", "toast_x4", "toast_x5", "toast_x6", "toast_x7"];
/// Exit time in seconds (ADL-MOBILE section 10.2).
const EXIT: f32 = 0.2;

/// One notice in a [`Toasts`] stack.
#[derive(Clone, Debug)]
pub struct Toast {
    /// Handle returned by [`Toasts::push`]. An action reports it.
    pub id: u64,
    pub title: String,
    pub message: String,
    /// Status color of the accent bar.
    pub color: Color,
    /// Label of the one action button, if any.
    pub action: Option<String>,
    slot: usize,
    /// Seconds shown while not held, and how long it may stay.
    age: f32,
    ttl: f32,
    /// Entrance and exit progress, 0 to 1, linear in time.
    enter: f32,
    exit: Option<f32>,
    /// Top edge as a spring, so the stack glides when a toast above or below leaves.
    y: Option<(f32, f32)>,
    /// Sideways drag in physical px, while being swiped.
    drag_x: f32,
}

/// A stack of toasts. Keep one per window, push to it from anywhere, and draw it with
/// [`Ui::toast_stack`] once per frame, last.
#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next: u64,
}

impl Toasts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Shows a notice. Returns its handle.
    pub fn push(&mut self, title: &str, message: &str, color: Color) -> u64 {
        self.add(title, message, color, None)
    }

    /// Shows a notice with one action button, such as Undo. It stays longer than a plain one.
    /// [`Ui::toast_stack`] returns the handle on the frame the action is clicked.
    pub fn push_action(&mut self, title: &str, message: &str, color: Color, action: &str) -> u64 {
        self.add(title, message, color, Some(action.to_string()))
    }

    fn add(&mut self, title: &str, message: &str, color: Color, action: Option<String>) -> u64 {
        let id = self.next;
        self.next += 1;
        let ttl = if action.is_some() { motion::TOAST_ACTION_TTL } else { motion::TOAST_TTL };
        // A repeat of the toast on top restarts it instead of stacking a copy.
        if let Some(top) = self.items.iter_mut().rev().find(|t| t.exit.is_none()) {
            if top.title == title && top.message == message && top.action == action {
                top.age = 0.0;
                return top.id;
            }
        }
        self.items.push(Toast {
            id,
            title: title.to_string(),
            message: message.to_string(),
            color,
            action,
            slot: (id % SLOTS as u64) as usize,
            age: 0.0,
            ttl,
            enter: 0.0,
            exit: None,
            y: None,
            drag_x: 0.0,
        });
        id
    }

    /// Starts the exit of the toast `id`, if it is still shown.
    pub fn dismiss(&mut self, id: u64) {
        if let Some(t) = self.items.iter_mut().find(|t| t.id == id && t.exit.is_none()) {
            t.exit = Some(0.0);
        }
    }

    /// True when nothing is shown or leaving.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Toasts that are shown and not leaving, oldest first.
    pub fn live(&self) -> impl Iterator<Item = &Toast> {
        self.items.iter().filter(|t| t.exit.is_none())
    }
}

impl Ui {
    /// Draws `toasts` inside `area`, stacked up from its bottom edge: at the right on wide
    /// windows and full width on narrow ones. Toasts draw on the top layer and take the pointer
    /// over their cards. Returns the handle of a toast whose action was clicked this frame; that
    /// toast then leaves.
    pub fn toast_stack(&mut self, toasts: &mut Toasts, area: Rect) -> Option<u64> {
        let dt = self.dt;
        let reduced = self.reduced_motion;
        // Too many: the oldest live ones leave.
        let live = toasts.items.iter().filter(|t| t.exit.is_none()).count();
        for t in toasts.items.iter_mut().filter(|t| t.exit.is_none()).take(live.saturating_sub(MAX_VISIBLE)) {
            t.exit = Some(0.0);
        }
        toasts.items.retain(|t| t.exit.is_none_or(|e| e < 1.0));
        if toasts.items.is_empty() {
            return None;
        }

        let pad = self.s(16.0);
        let compact = self.logical_size().x < 640.0;
        let w = if compact { area.w() - pad * 2.0 } else { self.s(360.0).min(area.w() - pad * 2.0) };
        let h = self.s(68.0);
        let gap = self.s(8.0);
        let x = area.right() - pad - w;
        let line = self.line();
        let radius = self.s(self.theme.radius.lg);

        let layer = self.draw.current;
        self.set_layer(2);
        let mut action = None;
        let mut cover: Option<Rect> = None;
        let mut bottom = area.bottom() - pad;
        // Newest at the bottom: walk from the end.
        for t in toasts.items.iter_mut().rev() {
            let target = bottom - h;
            if t.exit.is_none() {
                bottom = target - gap;
            }
            // Position: a calm spring toward its slot in the stack.
            let (y, vy) = t.y.get_or_insert((target, 0.0));
            if reduced {
                *y = target;
                *vy = 0.0;
            } else {
                let steps = (dt * 240.0).ceil().max(1.0) as u32;
                let hstep = dt / steps as f32;
                let s = motion::SPRING_CALM;
                for _ in 0..steps {
                    let acc = -s.stiffness * (*y - target) - s.damping * *vy;
                    *vy += acc * hstep;
                    *y += *vy * hstep;
                }
            }
            let y = *y;

            // Enter: rise 16 pt and fade in. Exit: fall 8 pt and fade out.
            t.enter = if reduced { 1.0 } else { (t.enter + dt / motion::BASE).min(1.0) };
            let shown = motion::ease_out_cubic(t.enter);
            let gone = t.exit.map_or(0.0, motion::ease_in_cubic);
            let alpha = shown * (1.0 - gone);
            let dy = (1.0 - shown) * self.s(16.0) + gone * self.s(8.0);
            let card = Rect::new(x + t.drag_x, y + dy, w, h);
            cover = Some(cover.map_or(card, |c| c.union(card)));

            let body_id = BODY_IDS[t.slot];
            let leaving = t.exit.is_some();
            let held = !leaving && self.hovered(card);

            // Swipe aside to dismiss. A short drag springs back.
            if !leaving {
                if let Some(d) = self.drag_delta(body_id, card) {
                    t.drag_x = (t.drag_x + d.x).max(0.0);
                } else if t.drag_x > w * 0.35 {
                    t.exit = Some(0.0);
                } else {
                    t.drag_x *= (-dt * motion::rate(motion::FAST)).exp();
                    if t.drag_x < 0.5 {
                        t.drag_x = 0.0;
                    }
                }
            }
            let swipe_fade = 1.0 - (t.drag_x / w).clamp(0.0, 1.0);

            let prev = self.set_opacity(alpha * swipe_fade);
            self.glass(card, radius, self.theme.card);
            let bar = self.s(3.0);
            self.rect(
                Rect::new(card.x() + self.s(10.0), card.y() + self.s(12.0), bar, (h - self.s(24.0)).max(0.0)),
                RectStyle::fill(t.color).radius(bar * 0.5),
            );

            // Right side: the action, then the close button, which shows while held.
            let mut right = card.right() - self.s(10.0);
            let close = self.s(28.0);
            let close_r = Rect::new(right - close, card.y() + self.s(8.0), close, close);
            let close_a = self.animate(CLOSE_IDS[t.slot], CH_HOT, if held { 1.0 } else { 0.0 }, motion::rate(motion::FAST));
            if close_a > 0.01 {
                let hover = self.hover_amount(CLOSE_IDS[t.slot], close_r);
                let o = self.set_opacity(self.opacity() * close_a);
                if hover > 0.01 {
                    self.rect(close_r, RectStyle::fill(with_alpha(self.theme.card_hi, hover)).radius(self.s(self.theme.radius.md)));
                }
                let isz = self.s(14.0);
                let c = close_r.center();
                self.icon(Rect::new(c.x - isz * 0.5, c.y - isz * 0.5, isz, isz), Icon::X, self.theme.text_dim);
                self.set_opacity(o);
            }
            if !leaving && held && self.click_region(CLOSE_IDS[t.slot], close_r) {
                t.exit = Some(0.0);
            }
            right -= close + self.s(4.0);
            if let Some(label) = t.action.clone() {
                // Measured as the button draws it: 16 px Medium with 16 px of padding a side.
                let style = TextStyle::new(16.0, self.theme.text).medium();
                let bw = self.text_width(&label, style) + self.s(34.0);
                let bh = self.s(32.0);
                let br = Rect::new(right - bw, card.center().y - bh * 0.5, bw, bh);
                if !leaving
                    && self.button(ACTION_IDS[t.slot], br, &label, None, ButtonVariant::Secondary, true)
                {
                    action = Some(t.id);
                    t.exit = Some(0.0);
                }
                right = br.x() - self.s(8.0);
            }

            let tx = card.x() + self.s(24.0);
            let inner_w = (right - tx).max(0.0);
            self.text_fit(tx, card.y() + self.s(12.0), &t.title, TextStyle::new(15.0, self.theme.text).medium(), inner_w);
            self.text_fit(tx, card.y() + self.s(38.0), &t.message, TextStyle::new(14.0, self.theme.text_dim), inner_w);

            // Time left, as a thin line along the bottom edge. Held toasts do not age.
            if !leaving && !held && !self.dragging(body_id) {
                t.age += dt;
            }
            let left = (1.0 - t.age / t.ttl).clamp(0.0, 1.0);
            if !leaving {
                let lw = (w - radius * 2.0) * left;
                self.rect(
                    Rect::new(card.x() + radius, card.bottom() - line * 2.0, lw, line),
                    RectStyle::fill(with_alpha(t.color, 0.5)),
                );
            }
            self.set_opacity(prev);

            if !leaving && t.age >= t.ttl {
                t.exit = Some(0.0);
            }
            if let Some(e) = t.exit.as_mut() {
                *e = if reduced { 1.0 } else { (*e + dt / EXIT).min(1.0) };
            }
        }
        self.set_layer(layer);
        if let Some(c) = cover {
            if self.popup_next.is_none() {
                self.pointer_overlay(c);
            }
        }
        // Aging needs frames, but slowly: the countdown line moves a pixel at a time.
        self.request_wake(1.0 / 30.0);
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::IconAtlas;

    #[test]
    fn toasts_age_out_and_leave() {
        let mut ui = Ui::new(&IconAtlas::build());
        let mut toasts = Toasts::new();
        toasts.push("Saved", "ok", Theme::adl_dark().success);
        let area = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let mut t = 0.0;
        for _ in 0..((motion::TOAST_TTL + 1.0) * 30.0) as usize {
            t += 1.0 / 30.0;
            ui.begin(Input::default(), Vec2::new(1000.0, 800.0), 1.0, t, 1.0 / 30.0);
            ui.toast_stack(&mut toasts, area);
            ui.end();
        }
        assert!(toasts.is_empty());
    }

    #[test]
    fn stack_keeps_at_most_three_and_merges_repeats() {
        let mut ui = Ui::new(&IconAtlas::build());
        let mut toasts = Toasts::new();
        for i in 0..5 {
            toasts.push(&format!("T{i}"), "", Theme::adl_dark().success);
        }
        let again = toasts.push("T4", "", Theme::adl_dark().success);
        assert_eq!(again, 4);
        ui.begin(Input::default(), Vec2::new(1000.0, 800.0), 1.0, 0.0, 1.0 / 60.0);
        ui.toast_stack(&mut toasts, Rect::new(0.0, 0.0, 1000.0, 800.0));
        ui.end();
        assert_eq!(toasts.live().count(), 3);
        assert_eq!(toasts.live().next().map(|t| t.title.as_str()), Some("T2"));
    }
}
