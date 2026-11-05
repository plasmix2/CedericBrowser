//! ADL-V3 motion and elevation helpers: drop shadows for floating surfaces, a frame-wide
//! opacity, timed presence (enter and exit), staggered entrances, error shakes, and tooltips.
//! Declared as a child of `ui`, so it can use the widget internals.
//!
//! Durations and curves come from `theme::motion`. Every helper honors reduced motion: values
//! jump to their end state, and nothing that motion showed is lost.

use super::*;
use crate::theme::{motion, shadow};

/// Timed progress of a presence, 0 hidden to 1 shown.
const CH_PRESENCE: u8 = 6;
/// Seconds since a staggered entrance started (negative while it waits for its turn).
const CH_ENTER: u8 = 7;
/// Seconds the pointer has rested on a tooltip's anchor.
const CH_TIP: u8 = 8;
/// Seconds since an error shake started.
const CH_SHAKE: u8 = 9;

/// Amplitude of an error shake, in logical px (ADL-MOBILE section 10.4).
const SHAKE_PX: f32 = 4.0;

impl Ui {
    // ------------------------------------------------------------------ elevation

    /// `SHADOW_OVERLAY` under a floating surface at `r` with corner `radius` (physical px). Draw
    /// it before the surface, on the same layer. Layer 0 never has a shadow.
    pub fn shadow(&mut self, r: Rect, radius: f32) {
        for sh in shadow::OVERLAY {
            let (dy, blur) = (self.s(sh.offset_y), self.s(sh.blur));
            self.rect(
                Rect::new(r.x(), r.y() + dy, r.w(), r.h()),
                RectStyle::fill([0.0, 0.0, 0.0, sh.alpha]).radius(radius).soft(blur),
            );
        }
    }

    /// A floating surface (Layer 1): glass with a `CARD` tint, 1 px `BORDER`, `LG` radius.
    pub fn surface(&mut self, r: Rect) {
        let radius = self.s(self.theme.radius.lg);
        self.glass(r, radius, self.theme.card);
    }

    /// Shows the layers beneath `r`, blurred, through a rounded window (ADL-V3 section 3.5).
    /// Draw it on a floating layer, before the surface's own fill. Does nothing on layer 0, or
    /// while blur is off.
    pub fn backdrop_blur(&mut self, r: Rect, radius: f32) {
        if self.backdrop_blur {
            self.draw.push_blur(RectInstance::new(r, RectStyle::fill([0.0, 0.0, 0.0, 1.0]).radius(radius)));
        }
    }

    /// A glass surface: the overlay shadow, the blurred backdrop, then `tint` at the theme's
    /// `glass_alpha` (80%, or opaque in high contrast) with a 1 px `BORDER`. With blur off, the tint is opaque, so the surface reads the
    /// same without the effect. This is the one way floating surfaces are drawn.
    pub fn glass(&mut self, r: Rect, radius: f32, tint: Color) {
        self.shadow(r, radius);
        let fill = if self.backdrop_blur && self.draw.current > 0 {
            self.backdrop_blur(r, radius);
            with_alpha(tint, tint[3] * self.theme.glass_alpha)
        } else {
            tint
        };
        self.rect(r, RectStyle::fill(fill).radius(radius).border(self.theme.border, self.line()));
    }

    /// The design tokens widgets draw with.
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Swaps the theme. Every widget reads it as it draws, so the next frame shows the new one.
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme_anim = None;
        self.theme = theme;
    }

    /// Swaps the theme with a transition: every color eases from the current theme to `to` over
    /// `motion::FAST` (instantly with reduced motion). Switching again midway continues from
    /// the colors on screen.
    pub fn switch_theme(&mut self, to: Theme) {
        if self.reduced_motion || self.theme == to {
            self.set_theme(to);
        } else {
            self.theme_anim = Some((self.theme, to, 0.0));
            self.request_frame();
        }
    }

    /// The theme a switch is heading to, or the current one when none is running.
    pub fn target_theme(&self) -> Theme {
        self.theme_anim.map_or(self.theme, |(_, to, _)| to)
    }

    /// Advances a theme switch by this frame's `dt`. Runs in [`Ui::begin`].
    pub(crate) fn step_theme_switch(&mut self) {
        let Some((from, to, t)) = self.theme_anim else { return };
        let t = t + self.dt;
        if t >= motion::FAST {
            self.theme_anim = None;
            self.theme = to;
        } else {
            self.theme_anim = Some((from, to, t));
            self.theme = from.lerp(&to, motion::ease_out_cubic(t / motion::FAST));
            self.request_frame();
        }
    }

    /// Turns backdrop blur on or off. It is on by default. Turn it off for a reduce-transparency
    /// setting, or on a GPU too slow for the extra passes; glass surfaces then draw opaque.
    pub fn set_backdrop_blur(&mut self, on: bool) {
        self.backdrop_blur = on;
    }

    pub fn backdrop_blur_enabled(&self) -> bool {
        self.backdrop_blur
    }

    // ------------------------------------------------------------------ opacity

    /// Multiplies the alpha of everything drawn until it is set back. Returns the previous
    /// value, so nested fades can restore it. It resets to 1 every frame.
    pub fn set_opacity(&mut self, opacity: f32) -> f32 {
        std::mem::replace(&mut self.draw.opacity, opacity.clamp(0.0, 1.0))
    }

    /// The alpha multiplier in effect.
    pub fn opacity(&self) -> f32 {
        self.draw.opacity
    }

    // ------------------------------------------------------------------ timed motion

    /// Enter and exit progress for something that appears and disappears: 0 hidden, 1 shown.
    /// It takes `enter` seconds to show and `exit` seconds to hide, eased out on the way in and
    /// in on the way out. A presence starts hidden, so the first frame it is visible it
    /// animates in. Reversing midway continues from where it is.
    pub fn presence(&mut self, id: impl Into<ElementId>, visible: bool, enter: f32, exit: f32) -> f32 {
        let id: ElementId = id.into();
        let target = if visible { 1.0 } else { 0.0 };
        if self.reduced_motion {
            self.anim.insert((id, CH_PRESENCE), target);
            return target;
        }
        let step = if visible { self.dt / enter.max(1e-3) } else { -self.dt / exit.max(1e-3) };
        let t = self.anim.entry((id, CH_PRESENCE)).or_insert(0.0);
        *t = (*t + step).clamp(0.0, 1.0);
        let t = *t;
        if t != target {
            self.request_frame();
        }
        motion::ease_out_cubic(t)
    }

    /// The presence progress `id` has now (0 hidden to 1 shown, before easing), if it has one.
    pub(crate) fn presence_value(&self, id: ElementId) -> Option<f32> {
        self.anim.get(&(id, CH_PRESENCE)).copied()
    }

    /// Puts a presence at its end state with no animation.
    pub fn set_presence(&mut self, id: impl Into<ElementId>, visible: bool) {
        let id: ElementId = id.into();
        self.anim.insert((id, CH_PRESENCE), if visible { 1.0 } else { 0.0 });
    }

    /// Entrance progress, 0 to 1, for the `index`th item of a list that appears together
    /// (ADL-V3 section 4.3). Each item waits `STAGGER` seconds after the one before, for at most
    /// `STAGGER_MAX` items, then fades in and rises over `BASE`. It plays once per id. Call it
    /// for every item, including ones scrolled out of view, so they are done when they show.
    pub fn entrance(&mut self, id: impl Into<ElementId>, index: usize) -> f32 {
        let id: ElementId = id.into();
        if self.reduced_motion {
            return 1.0;
        }
        let delay = motion::STAGGER * index.min(motion::STAGGER_MAX) as f32;
        let dt = self.dt;
        let e = self.anim.entry((id, CH_ENTER)).or_insert(-delay);
        if *e < motion::BASE {
            *e += dt;
            let e = *e;
            self.request_frame();
            return motion::ease_out_cubic(e / motion::BASE);
        }
        1.0
    }

    /// Plays an item's entrance again the next time [`Ui::entrance`] is called for it.
    pub fn replay_entrance(&mut self, id: impl Into<ElementId>) {
        let id: ElementId = id.into();
        self.anim.remove(&(id, CH_ENTER));
    }

    /// Starts an error shake on `id`. Read the offset with [`Ui::shake_offset`].
    pub fn shake(&mut self, id: impl Into<ElementId>) {
        let id: ElementId = id.into();
        self.anim.insert((id, CH_SHAKE), 0.0);
    }

    /// Horizontal offset in physical px for a shaking widget: 4 px left and right, three times,
    /// over `SLOW`. Zero when it is not shaking or motion is reduced.
    pub fn shake_offset(&mut self, id: impl Into<ElementId>) -> f32 {
        let id: ElementId = id.into();
        let dt = self.dt;
        let Some(e) = self.anim.get_mut(&(id, CH_SHAKE)) else {
            return 0.0;
        };
        *e += dt;
        let t = *e / motion::SLOW;
        if t >= 1.0 || self.reduced_motion {
            self.anim.remove(&(id, CH_SHAKE));
            return 0.0;
        }
        self.request_frame();
        motion::shake(t) * self.s(SHAKE_PX)
    }

    // ------------------------------------------------------------------ tooltip

    /// A tooltip for the control at `anchor`. Call it after the control, with the control's id.
    /// It shows after the pointer rests on the anchor for `TOOLTIP_DELAY`, below the anchor (or
    /// above, near the bottom edge), and hides on press or when the pointer leaves. Touch has no
    /// hover, so touch input never shows one: a tooltip only repeats what an icon means.
    pub fn tooltip(&mut self, id: impl Into<ElementId>, anchor: Rect, text: &str) {
        let id: ElementId = id.into();
        if self.input.touch || text.is_empty() {
            return;
        }
        let over = self.hovered(anchor) && !self.input.down;
        let dt = self.dt;
        let rest = self.anim.entry((id, CH_TIP)).or_insert(0.0);
        *rest = if over { *rest + dt } else { 0.0 };
        let rest = *rest;
        let show = over && rest >= motion::TOOLTIP_DELAY;
        if over && !show {
            self.request_wake(motion::TOOLTIP_DELAY - rest);
        }
        let a = self.presence(id, show, motion::FAST, motion::FAST * 0.75);
        if a <= 0.001 {
            return;
        }

        let style = TextStyle::new(13.0, self.theme.text).medium();
        let (pad_x, pad_y, gap) = (self.s(8.0), self.s(5.0), self.s(6.0));
        let screen = self.screen_rect();
        let max_w = (screen.w() - self.s(16.0)).max(0.0);
        let shown = self.fit_text(text, style, (max_w - pad_x * 2.0).max(0.0));
        let (_, size) = self.measure(&shown, style);
        let (w, h) = (size.x + pad_x * 2.0, size.y + pad_y * 2.0);
        let edge = self.s(8.0);
        let x = (anchor.center().x - w * 0.5).clamp(screen.x() + edge, (screen.right() - w - edge).max(edge));
        let below = anchor.bottom() + gap;
        let (y, rise) = if below + h <= screen.bottom() - edge {
            (below, -self.s(4.0))
        } else {
            (anchor.y() - gap - h, self.s(4.0))
        };
        let r = Rect::new(x, y + rise * (1.0 - a), w, h);

        let layer = self.draw.current;
        self.set_layer(2);
        let prev = self.set_opacity(a * self.opacity());
        let radius = self.s(self.theme.radius.md);
        self.glass(r, radius, self.theme.card_hi);
        self.text(r.x() + pad_x, r.y() + pad_y, &shown, style);
        self.set_opacity(prev);
        self.set_layer(layer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::IconAtlas;

    fn ui() -> Ui {
        let mut ui = Ui::new(&IconAtlas::build());
        ui.begin(Input::default(), Vec2::new(800.0, 600.0), 1.0, 0.0, 1.0 / 60.0);
        ui
    }

    fn step(ui: &mut Ui, dt: f32) {
        ui.end();
        let t = ui.time + dt;
        ui.begin(Input::default(), Vec2::new(800.0, 600.0), 1.0, t, dt);
    }

    #[test]
    fn presence_animates_in_and_out_within_its_durations() {
        let mut ui = ui();
        let first = ui.presence("p", true, 0.2, 0.1);
        assert!(first > 0.0 && first < 1.0);
        for _ in 0..12 {
            step(&mut ui, 1.0 / 60.0);
            ui.presence("p", true, 0.2, 0.1);
        }
        assert_eq!(ui.presence("p", true, 0.2, 0.1), 1.0);
        for _ in 0..7 {
            step(&mut ui, 1.0 / 60.0);
            ui.presence("p", false, 0.2, 0.1);
        }
        assert_eq!(ui.presence("p", false, 0.2, 0.1), 0.0);
    }

    #[test]
    fn reduced_motion_jumps_to_the_end() {
        let mut ui = ui();
        ui.set_reduced_motion(true);
        assert_eq!(ui.presence("p", true, 0.2, 0.1), 1.0);
        assert_eq!(ui.entrance("e", 3), 1.0);
        ui.shake("s");
        assert_eq!(ui.shake_offset("s"), 0.0);
    }

    #[test]
    fn later_items_enter_later_and_all_finish() {
        let mut ui = ui();
        let a = ui.entrance("a", 0);
        let b = ui.entrance("b", 4);
        assert!(a > b);
        for _ in 0..40 {
            step(&mut ui, 1.0 / 60.0);
            ui.entrance("a", 0);
            ui.entrance("b", 4);
        }
        assert_eq!(ui.entrance("a", 0), 1.0);
        assert_eq!(ui.entrance("b", 4), 1.0);
    }

    #[test]
    fn shake_ends_at_rest() {
        let mut ui = ui();
        ui.shake("s");
        let mut moved = false;
        for _ in 0..30 {
            step(&mut ui, 1.0 / 60.0);
            moved |= ui.shake_offset("s").abs() > 1.0;
        }
        assert!(moved);
        assert_eq!(ui.shake_offset("s"), 0.0);
    }
}
