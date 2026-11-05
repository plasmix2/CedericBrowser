//! Drawing that is not a widget: lines, curves, arcs and rings, in physical pixels and widths.
//! They go to the current layer, faded by the frame opacity and clipped like everything else.
//! Declared as a child of `ui`, so it can use the widget internals.

use super::*;
use crate::path::Path;

impl Ui {
    /// A straight line from `a` to `b` with round ends.
    pub fn stroke_line(&mut self, a: Vec2, b: Vec2, width: f32, color: Color) {
        self.draw.stroke(&Path::polyline(&[[a.x, a.y], [b.x, b.y]]), width, color);
    }

    /// Lines through `points`, joined round.
    pub fn polyline(&mut self, points: &[Vec2], width: f32, color: Color) {
        let pts: Vec<[f32; 2]> = points.iter().map(|p| [p.x, p.y]).collect();
        self.draw.stroke(&Path::polyline(&pts), width, color);
    }

    /// Any path: lines and Bézier curves.
    pub fn stroke(&mut self, path: &Path, width: f32, color: Color) {
        self.draw.stroke(path, width, color);
    }

    /// An arc around `center` from `start` through `sweep` radians, clockwise on screen.
    pub fn arc(&mut self, center: Vec2, radius: f32, start: f32, sweep: f32, width: f32, color: Color) {
        self.draw.arc([center.x, center.y], radius, start, sweep, width, color);
    }

    /// A progress ring: a `BORDER` track and `fraction` of it in `color`, starting at the top
    /// and running clockwise (ADL-V3 section 8.16).
    pub fn progress_ring(&mut self, center: Vec2, radius: f32, width: f32, fraction: f32, color: Color) {
        let track = self.theme.border;
        let tau = std::f32::consts::TAU;
        self.draw.arc([center.x, center.y], radius, 0.0, tau, width, track);
        let f = fraction.clamp(0.0, 1.0);
        if f > 0.0 {
            self.draw.arc([center.x, center.y], radius, -tau / 4.0, tau * f, width, color);
        }
    }
}
