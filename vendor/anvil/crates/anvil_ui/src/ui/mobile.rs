//! Phone widgets (ADL-MOBILE v2): the bottom navigation bar, toolbar icon buttons, and rows that
//! swipe aside to reveal an action. Declared as a child of `ui`, so it can use the widget
//! internals. Colors come from `theme::anvl`.

use super::*;
use crate::theme::motion;

/// Spring position of the nav bar indicator, in slot-index space.
const CH_NAV: u8 = 60;
/// Icon scale after a tap: it springs from 1.08 back to 1 (ADL-MOBILE section 10.3).
const CH_BOUNCE: u8 = 61;
/// Swipe state: press point, axis lock (0 undecided, 1 sideways), and the row's offset.
const CH_SW_X: u8 = 62;
const CH_SW_Y: u8 = 63;
const CH_SW_LOCK: u8 = 64;
const CH_SW_OFF: u8 = 65;

/// One slot in a [`Ui::bottom_nav`] bar.
#[derive(Clone, Copy, Debug)]
pub struct NavItem<'a> {
    pub icon: Icon,
    pub label: &'a str,
    /// The screen's one primary action, drawn as a raised round button in its slot. It is an
    /// action, not a destination, so the indicator never rests on it.
    pub primary: bool,
}

/// What a swipe on a row did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Swipe {
    /// Sideways offset to draw the row at, in physical px. Negative is to the left.
    pub offset: f32,
    /// How far toward the action the row has gone, 0 to 1 (1 at the threshold).
    pub progress: f32,
    /// The finger let go past the threshold: do the action.
    pub committed: bool,
}

impl Ui {
    /// True on the frame a press lands inside `r`.
    pub fn pressed_in(&self, r: Rect) -> bool {
        self.input.pressed && r.contains(self.input.mouse)
    }

    /// Takes keyboard focus away from every widget. On a phone this closes the soft keyboard.
    pub fn clear_focus(&mut self) {
        self.focus = None;
        self.kb_focus = None;
    }

    /// Bottom navigation bar (ADL-MOBILE section 8.1). Each item gets an equal slot with a 24 pt
    /// icon over a 12 pt label. The active item sits on a pill that glides between slots on a
    /// spring, and a tapped icon bounces. `ids` gives one id per item. Returns the tapped item.
    pub fn bottom_nav(&mut self, ids: &[&'static str], r: Rect, items: &[NavItem], active: usize) -> Option<usize> {
        let line = self.line();
        self.rect(r, RectStyle::fill(self.theme.surface));
        self.rect(Rect::new(r.x(), r.y(), r.w(), line), RectStyle::fill(self.theme.border));
        if items.is_empty() || ids.len() < items.len() {
            return None;
        }
        let slot_w = r.w() / items.len() as f32;
        let s = motion::SPRING;
        let pos = self.spring(ids[0], CH_NAV, active as f32, s.stiffness, s.damping);
        if (pos - active as f32).abs() > 0.002 {
            self.request_frame();
        }
        let (pill_w, pill_h) = (self.s(56.0), self.s(30.0));
        let icon_y = r.y() + self.s(10.0);
        // The indicator, drawn under the icons, at the spring's place.
        if !items[active.min(items.len() - 1)].primary {
            let cx = r.x() + slot_w * (pos + 0.5);
            self.rect(
                Rect::new(cx - pill_w * 0.5, icon_y - self.s(3.0), pill_w, pill_h),
                RectStyle::fill(self.theme.card_hi).radius(pill_h * 0.5),
            );
        }

        let mut tapped = None;
        for (i, item) in items.iter().enumerate() {
            let id = ids[i];
            let slot = Rect::new(r.x() + slot_w * i as f32, r.y(), slot_w, r.h());
            if self.click_region(id, slot) {
                tapped = Some(i);
                self.set_spring(id, CH_BOUNCE, 1.08);
            }
            let held = self.active == Some(ElementId::from(id)) && self.input.down;
            let press = self.animate(id, CH_PRESS, if held { 1.0 } else { 0.0 }, motion::rate(motion::INSTANT));
            let bounce = self.spring(id, CH_BOUNCE, 1.0, s.stiffness, s.damping);
            if (bounce - 1.0).abs() > 0.002 {
                self.request_frame();
            }
            let scale = bounce * (1.0 - 0.06 * press);
            let cx = slot.center().x;
            if item.primary {
                // A raised round button, centered in the slot.
                let d = self.s(48.0) * scale;
                let c = Rect::new(cx - d * 0.5, slot.center().y - d * 0.5, d, d);
                self.shadow(c, d * 0.5);
                self.rect(c, RectStyle::fill(lerp(self.theme.text, self.theme.text_dim, press * 0.3)).radius(d * 0.5));
                let isz = self.s(24.0) * scale;
                self.icon(Rect::new(cx - isz * 0.5, c.center().y - isz * 0.5, isz, isz), item.icon, self.theme.bg);
                continue;
            }
            let on = self.animate(id, CH_TOGGLE, if i == active { 1.0 } else { 0.0 }, motion::rate(motion::FAST));
            let tint = lerp(self.theme.text_dim, self.theme.text, on);
            let isz = self.s(22.0) * scale;
            self.icon(Rect::new(cx - isz * 0.5, icon_y + (self.s(24.0) - isz) * 0.5, isz, isz), item.icon, tint);
            let style = TextStyle::new(12.0, tint).medium();
            self.text_center(Rect::new(slot.x(), icon_y + self.s(30.0), slot.w(), self.s(16.0)), item.label, style);
        }
        tapped
    }

    /// A 40 pt icon button for a toolbar. `on` shows a toggled state; a disabled button keeps
    /// its icon in `DISABLED_TEXT` and ignores taps. Presses sink and the icon tints on hover.
    pub fn toolbar_button(&mut self, id: impl Into<ElementId>, r: Rect, icon: Icon, enabled: bool, on: bool) -> bool {
        let id: ElementId = id.into();
        let clicked = enabled && self.click_region(id, r);
        let hover = if enabled { self.hover_amount(id, r) } else { 0.0 };
        let held = enabled && self.active == Some(id) && self.input.down;
        let press = self.animate(id, CH_PRESS, if held { 1.0 } else { 0.0 }, motion::rate(motion::INSTANT));
        let fill = (hover * 0.6).max(press).max(if on { 1.0 } else { 0.0 });
        if fill > 0.01 {
            let inset = self.s(2.0);
            self.rect(
                r.inset(inset).scaled(1.0 - 0.06 * press),
                RectStyle::fill(with_alpha(self.theme.card_hi, fill)).radius(self.s(self.theme.radius.md)),
            );
        }
        let tint = if !enabled {
            self.theme.disabled
        } else {
            lerp(self.theme.text_dim, self.theme.text, hover.max(press).max(if on { 1.0 } else { 0.0 }))
        };
        let isz = self.s(20.0) * (1.0 - 0.06 * press);
        let c = r.center();
        self.icon(Rect::new(c.x - isz * 0.5, c.y - isz * 0.5, isz, isz), icon, tint);
        if !enabled && self.hovered(r) {
            self.want_cursor(Cursor::NotAllowed);
        }
        clicked
    }

    /// A row that a finger (or a mouse) drags to the left to reveal an action, such as Delete
    /// (ADL-MOBILE section 9.2). The drag locks to the axis it starts on, so a vertical drag
    /// still scrolls the list and a sideways one never opens the row. Past `threshold` px the
    /// release commits; short of it the row springs back. Call it before drawing the row, and
    /// draw the row at `offset`.
    pub fn swipe_row(&mut self, id: impl Into<ElementId>, r: Rect, threshold: f32) -> Swipe {
        let id: ElementId = id.into();
        let m = self.input.mouse;
        let slop = self.s(8.0);
        let mut out = Swipe::default();
        if self.input.pressed && self.hovered(r) {
            self.anim.insert((id, CH_SW_X), m.x);
            self.anim.insert((id, CH_SW_Y), m.y);
            self.anim.insert((id, CH_SW_LOCK), 0.0);
        }
        let lock = self.anim.get(&(id, CH_SW_LOCK)).copied();
        if let (Some(lock), true) = (lock, self.input.down && !self.input.released) {
            let dx = m.x - self.anim[&(id, CH_SW_X)];
            let dy = m.y - self.anim[&(id, CH_SW_Y)];
            let mut lock = lock;
            if lock == 0.0 {
                if dx.abs() > slop && dx.abs() > dy.abs() * 1.5 {
                    lock = 1.0;
                    self.anim.insert((id, CH_SW_LOCK), 1.0);
                } else if dy.abs() > slop {
                    self.anim.remove(&(id, CH_SW_LOCK));
                }
            }
            if lock == 1.0 {
                // Owning the press stops the row from clicking on release.
                self.active = Some(id);
                self.want_cursor(Cursor::Grabbing);
                // Left only. Past the threshold the row slows, like a stretched edge.
                let raw = dx.min(0.0);
                let off = if -raw > threshold { -threshold - (-raw - threshold) * RUBBER_BAND } else { raw };
                self.anim.insert((id, CH_SW_OFF), off);
            }
        } else if let Some(lock) = lock {
            // Released.
            self.anim.remove(&(id, CH_SW_LOCK));
            let off = self.anim.get(&(id, CH_SW_OFF)).copied().unwrap_or(0.0);
            if lock == 1.0 && -off >= threshold {
                out.committed = true;
                self.anim.remove(&(id, CH_SW_OFF));
                return out;
            }
        }
        let dragging = self.anim.get(&(id, CH_SW_LOCK)) == Some(&1.0);
        let mut off = self.anim.get(&(id, CH_SW_OFF)).copied().unwrap_or(0.0);
        if !dragging && off != 0.0 {
            // Spring back.
            off *= (-self.dt * motion::rate(motion::BASE)).exp();
            if off.abs() < 0.5 || self.reduced_motion {
                off = 0.0;
                self.anim.remove(&(id, CH_SW_OFF));
            } else {
                self.anim.insert((id, CH_SW_OFF), off);
                self.request_frame();
            }
        }
        out.offset = off;
        out.progress = (-off / threshold.max(1.0)).clamp(0.0, 1.0);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::IconAtlas;

    fn at(ui: &mut Ui, x: f32, y: f32, pressed: bool, down: bool, released: bool) {
        ui.end();
        let t = ui.time + 1.0 / 60.0;
        let input = Input { mouse: Vec2::new(x, y), pressed, down, released, touch: true, ..Input::default() };
        ui.begin(input, Vec2::new(400.0, 800.0), 1.0, t, 1.0 / 60.0);
    }

    #[test]
    fn sideways_swipe_past_the_threshold_commits_and_a_short_one_springs_back() {
        let mut ui = Ui::new(&IconAtlas::build());
        ui.begin(Input::default(), Vec2::new(400.0, 800.0), 1.0, 0.0, 1.0 / 60.0);
        let row = Rect::new(0.0, 100.0, 400.0, 72.0);
        at(&mut ui, 300.0, 130.0, true, true, false);
        ui.swipe_row("r", row, 120.0);
        for x in [280.0, 240.0, 200.0, 150.0] {
            at(&mut ui, x, 132.0, false, true, false);
            ui.swipe_row("r", row, 120.0);
        }
        at(&mut ui, 150.0, 132.0, false, false, true);
        assert!(ui.swipe_row("r", row, 120.0).committed);

        at(&mut ui, 300.0, 130.0, true, true, false);
        ui.swipe_row("r", row, 120.0);
        at(&mut ui, 250.0, 130.0, false, true, false);
        assert!(ui.swipe_row("r", row, 120.0).offset < -40.0);
        at(&mut ui, 250.0, 130.0, false, false, true);
        assert!(!ui.swipe_row("r", row, 120.0).committed);
        for _ in 0..40 {
            at(&mut ui, 250.0, 130.0, false, false, false);
            ui.swipe_row("r", row, 120.0);
        }
        assert_eq!(ui.swipe_row("r", row, 120.0).offset, 0.0);
    }

    #[test]
    fn vertical_drag_never_swipes() {
        let mut ui = Ui::new(&IconAtlas::build());
        ui.begin(Input::default(), Vec2::new(400.0, 800.0), 1.0, 0.0, 1.0 / 60.0);
        let row = Rect::new(0.0, 100.0, 400.0, 72.0);
        at(&mut ui, 300.0, 130.0, true, true, false);
        ui.swipe_row("r", row, 120.0);
        for y in [150.0, 180.0, 220.0] {
            at(&mut ui, 290.0, y, false, true, false);
            assert_eq!(ui.swipe_row("r", row, 120.0).offset, 0.0);
        }
    }

    #[test]
    fn nav_reports_the_tapped_slot() {
        let mut ui = Ui::new(&IconAtlas::build());
        ui.begin(Input::default(), Vec2::new(400.0, 800.0), 1.0, 0.0, 1.0 / 60.0);
        let bar = Rect::new(0.0, 700.0, 400.0, 64.0);
        let items = [
            NavItem { icon: Icon::FileText, label: "Notes", primary: false },
            NavItem { icon: Icon::Plus, label: "New", primary: true },
            NavItem { icon: Icon::Search, label: "Search", primary: false },
        ];
        let ids = ["n0", "n1", "n2"];
        at(&mut ui, 330.0, 730.0, true, true, false);
        ui.bottom_nav(&ids, bar, &items, 0);
        at(&mut ui, 330.0, 730.0, false, false, true);
        assert_eq!(ui.bottom_nav(&ids, bar, &items, 0), Some(2));
    }
}
