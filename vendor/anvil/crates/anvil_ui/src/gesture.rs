//! One recognizer for the pointer gestures every widget used to work out on its own: tap, double
//! tap, long press, drag (axis-locked) and swipe (ANVIL-UI-2.md section 3.5).
//!
//! `Ui` runs it once per frame in `Ui::begin` and keeps what it saw in `Ui::gestures`. The
//! element tree dispatches from that list, and immediate-mode widgets may read it too.
//! Positions are physical px and times are seconds, like the rest of `Input`.

use glam::Vec2;

use crate::ui::Input;

/// How far a press may wander (logical px) and still be a tap or a long press. Past it, the
/// press is a drag.
pub const SLOP: f32 = 8.0;
/// How long a press must be held, without becoming a drag, to be a long press.
pub const LONG_PRESS: f32 = 0.5;
/// The longest gap between two taps that makes them a double tap.
pub const DOUBLE_TAP: f32 = 0.35;
/// The slowest release (logical px per second, along the drag's axis) that counts as a swipe.
pub const SWIPE_SPEED: f32 = 500.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Start,
    Move,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// One step of a drag. The axis is locked from the first move past the slop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drag {
    pub phase: Phase,
    /// Where the press started.
    pub start: Vec2,
    pub pos: Vec2,
    /// Movement since the last step.
    pub delta: Vec2,
    /// Movement since the press.
    pub total: Vec2,
    pub axis: Axis,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    /// The pointer went down.
    Press(Vec2),
    /// A press came up near where it started, before a long press fired.
    Tap(Vec2),
    /// The second of two quick taps in the same place. Its `Tap` comes first.
    DoubleTap(Vec2),
    /// A press held still for [`LONG_PRESS`]. No tap follows it.
    LongPress(Vec2),
    Drag(Drag),
    /// A drag released fast along its axis. Its `Drag` end comes first.
    Swipe { dir: Dir, velocity: Vec2 },
}

struct Press {
    start: Vec2,
    last: Vec2,
    t0: f32,
    axis: Option<Axis>,
    long_fired: bool,
    /// Smoothed pointer speed, physical px per second.
    velocity: Vec2,
}

#[derive(Default)]
pub struct Recognizer {
    press: Option<Press>,
    last_tap: Option<(f32, Vec2)>,
}

impl Recognizer {
    /// Feeds one frame of input. `scale` is physical px per logical px.
    pub fn update(&mut self, input: &Input, time: f32, dt: f32, scale: f32) -> Vec<Gesture> {
        let mut out = Vec::new();
        let pos = input.mouse;
        let slop = SLOP * scale;
        if input.pressed {
            self.press = Some(Press { start: pos, last: pos, t0: time, axis: None, long_fired: false, velocity: Vec2::ZERO });
            out.push(Gesture::Press(pos));
        }
        let Some(p) = &mut self.press else { return out };
        let total = pos - p.start;
        match p.axis {
            None if !p.long_fired && total.length() > slop => {
                let axis = if total.x.abs() > total.y.abs() { Axis::X } else { Axis::Y };
                p.axis = Some(axis);
                out.push(Gesture::Drag(Drag { phase: Phase::Start, start: p.start, pos, delta: total, total, axis }));
            }
            Some(axis) if pos != p.last => {
                out.push(Gesture::Drag(Drag { phase: Phase::Move, start: p.start, pos, delta: pos - p.last, total, axis }));
            }
            _ => {}
        }
        if input.down && !input.released && dt > 0.0 {
            // Still frames count too, so a finger that stops before lifting does not swipe.
            p.velocity += ((pos - p.last) / dt - p.velocity) * 0.5;
        }
        if p.axis.is_none() && !p.long_fired && input.down && time - p.t0 >= LONG_PRESS {
            p.long_fired = true;
            out.push(Gesture::LongPress(p.start));
        }
        p.last = pos;
        if input.released {
            if let Some(axis) = p.axis {
                out.push(Gesture::Drag(Drag { phase: Phase::End, start: p.start, pos, delta: Vec2::ZERO, total, axis }));
                let v = p.velocity;
                let speed = SWIPE_SPEED * scale;
                let dir = match axis {
                    Axis::X if v.x > speed => Some(Dir::Right),
                    Axis::X if v.x < -speed => Some(Dir::Left),
                    Axis::Y if v.y > speed => Some(Dir::Down),
                    Axis::Y if v.y < -speed => Some(Dir::Up),
                    _ => None,
                };
                if let Some(dir) = dir {
                    out.push(Gesture::Swipe { dir, velocity: v });
                }
            } else if !p.long_fired {
                out.push(Gesture::Tap(pos));
                let double = self.last_tap.is_some_and(|(t, at)| time - t < DOUBLE_TAP && (pos - at).length() < slop * 3.0);
                if double {
                    out.push(Gesture::DoubleTap(pos));
                    self.last_tap = None;
                } else {
                    self.last_tap = Some((time, pos));
                }
            }
            self.press = None;
        }
        out
    }

    /// Seconds until a held press becomes a long press, while it still may.
    pub fn long_press_due(&self, time: f32) -> Option<f32> {
        let p = self.press.as_ref()?;
        (p.axis.is_none() && !p.long_fired).then(|| (LONG_PRESS - (time - p.t0)).max(0.0))
    }

    /// True while a press is down and has turned into a drag.
    pub fn dragging(&self) -> bool {
        self.press.as_ref().is_some_and(|p| p.axis.is_some())
    }

    /// True while a press is down and has fired its long press.
    pub fn long_pressed(&self) -> bool {
        self.press.as_ref().is_some_and(|p| p.long_fired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives the recognizer at 60 fps.
    struct Rig {
        r: Recognizer,
        t: f32,
    }

    const DT: f32 = 1.0 / 60.0;

    impl Rig {
        fn new() -> Self {
            Self { r: Recognizer::default(), t: 0.0 }
        }
        fn frame(&mut self, x: f32, y: f32, pressed: bool, down: bool, released: bool) -> Vec<Gesture> {
            self.t += DT;
            let input = Input { mouse: Vec2::new(x, y), pressed, down, released, ..Default::default() };
            self.r.update(&input, self.t, DT, 1.0)
        }
        fn tap(&mut self, x: f32, y: f32) -> Vec<Gesture> {
            let mut g = self.frame(x, y, true, true, false);
            g.extend(self.frame(x, y, false, false, true));
            g
        }
        fn idle(&mut self, seconds: f32) {
            for _ in 0..(seconds / DT) as usize {
                self.frame(0.0, 0.0, false, false, false);
            }
        }
    }

    fn has(g: &[Gesture], f: impl Fn(&Gesture) -> bool) -> bool {
        g.iter().any(f)
    }

    #[test]
    fn a_short_press_is_a_tap() {
        let mut rig = Rig::new();
        let g = rig.tap(10.0, 10.0);
        assert_eq!(g, vec![Gesture::Press(Vec2::new(10.0, 10.0)), Gesture::Tap(Vec2::new(10.0, 10.0))]);
    }

    #[test]
    fn two_quick_taps_are_a_double_tap_and_slow_ones_are_not() {
        let mut rig = Rig::new();
        rig.tap(10.0, 10.0);
        rig.idle(0.1);
        assert!(has(&rig.tap(12.0, 11.0), |g| matches!(g, Gesture::DoubleTap(_))));
        rig.idle(0.5);
        rig.tap(10.0, 10.0);
        rig.idle(0.5);
        assert!(!has(&rig.tap(10.0, 10.0), |g| matches!(g, Gesture::DoubleTap(_))), "too slow");
        rig.idle(0.1);
        assert!(!has(&rig.tap(200.0, 10.0), |g| matches!(g, Gesture::DoubleTap(_))), "too far");
    }

    #[test]
    fn holding_still_is_a_long_press_and_no_tap_follows() {
        let mut rig = Rig::new();
        rig.frame(5.0, 5.0, true, true, false);
        assert!(rig.r.long_press_due(rig.t).unwrap() > 0.45);
        let mut fired = 0;
        for _ in 0..40 {
            fired += rig.frame(6.0, 5.0, false, true, false).iter().filter(|g| matches!(g, Gesture::LongPress(_))).count();
        }
        assert_eq!(fired, 1, "fires once");
        let up = rig.frame(6.0, 5.0, false, false, true);
        assert!(!has(&up, |g| matches!(g, Gesture::Tap(_))));
    }

    #[test]
    fn moving_past_the_slop_locks_an_axis_and_cancels_tap_and_long_press() {
        let mut rig = Rig::new();
        rig.frame(0.0, 0.0, true, true, false);
        assert!(rig.frame(5.0, 1.0, false, true, false).is_empty(), "inside the slop");
        let g = rig.frame(12.0, 3.0, false, true, false);
        assert!(matches!(g[..], [Gesture::Drag(Drag { phase: Phase::Start, axis: Axis::X, .. })]));
        // A later vertical move stays on the locked axis.
        let g = rig.frame(12.0, 40.0, false, true, false);
        assert!(matches!(g[..], [Gesture::Drag(Drag { phase: Phase::Move, axis: Axis::X, .. })]));
        for _ in 0..40 {
            assert!(!has(&rig.frame(12.0, 40.0, false, true, false), |g| matches!(g, Gesture::LongPress(_))));
        }
        let up = rig.frame(12.0, 40.0, false, false, true);
        assert!(has(&up, |g| matches!(g, Gesture::Drag(Drag { phase: Phase::End, .. }))));
        assert!(!has(&up, |g| matches!(g, Gesture::Tap(_) | Gesture::Swipe { .. })), "it stopped before lifting");
    }

    #[test]
    fn a_fast_release_is_a_swipe_in_its_direction() {
        let mut rig = Rig::new();
        rig.frame(300.0, 0.0, true, true, false);
        for i in 1..=6 {
            rig.frame(300.0 - i as f32 * 30.0, 2.0, false, true, false);
        }
        let up = rig.frame(120.0, 2.0, false, false, true);
        assert!(has(&up, |g| matches!(g, Gesture::Swipe { dir: Dir::Left, .. })), "{up:?}");
    }
}
