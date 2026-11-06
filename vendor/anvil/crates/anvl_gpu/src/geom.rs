//! Rectangles in window pixels (origin top-left), built on `glam::Vec2`.

use glam::Vec2;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub pos: Vec2,
    pub size: Vec2,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            pos: Vec2::new(x, y),
            size: Vec2::new(w, h),
        }
    }

    pub fn x(&self) -> f32 {
        self.pos.x
    }

    pub fn y(&self) -> f32 {
        self.pos.y
    }

    pub fn w(&self) -> f32 {
        self.size.x
    }

    pub fn h(&self) -> f32 {
        self.size.y
    }

    pub fn right(&self) -> f32 {
        self.pos.x + self.size.x
    }

    pub fn bottom(&self) -> f32 {
        self.pos.y + self.size.y
    }

    pub fn center(&self) -> Vec2 {
        self.pos + self.size * 0.5
    }

    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.pos.x && p.x < self.right() && p.y >= self.pos.y && p.y < self.bottom()
    }

    /// Converts a rectangle in logical points to physical pixels. Unlike [`Rect::scaled`], this
    /// scales the origin too, so the result lands where the OS scale factor puts it.
    pub fn to_physical(&self, scale: f32) -> Self {
        Self::new(self.x() * scale, self.y() * scale, self.w() * scale, self.h() * scale)
    }

    /// Shrinks or grows the rectangle around its center by `factor`.
    pub fn scaled(&self, factor: f32) -> Self {
        let c = self.center();
        let s = self.size * factor;
        Self {
            pos: c - s * 0.5,
            size: s,
        }
    }

    /// Insets every side by `d`.
    pub fn inset(&self, d: f32) -> Self {
        Self {
            pos: self.pos + Vec2::splat(d),
            size: (self.size - Vec2::splat(d * 2.0)).max(Vec2::ZERO),
        }
    }

    /// The smallest rectangle that holds both.
    pub fn union(&self, other: Rect) -> Self {
        let min = self.pos.min(other.pos);
        let max = (self.pos + self.size).max(other.pos + other.size);
        Self { pos: min, size: max - min }
    }

    /// The overlap of both, empty (zero size) when they do not meet.
    pub fn intersect(&self, other: Rect) -> Self {
        let min = self.pos.max(other.pos);
        let max = (self.pos + self.size).min(other.pos + other.size);
        Self { pos: min, size: (max - min).max(Vec2::ZERO) }
    }

    /// Moves by `dx`, `dy`.
    pub fn offset(&self, dx: f32, dy: f32) -> Self {
        Self {
            pos: self.pos + Vec2::new(dx, dy),
            size: self.size,
        }
    }
}
