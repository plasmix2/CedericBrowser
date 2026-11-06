//! Flex-style stacking. Every child is clamped to its parent's rectangle, so a child can never
//! be placed outside its container. A child with no room left gets zero size instead.

use crate::geom::Rect;

/// Stacks children top to bottom, full width.
#[derive(Clone, Copy, Debug)]
pub struct Column {
    rect: Rect,
    cursor: f32,
    gap: f32,
}

impl Column {
    pub fn new(rect: Rect, gap: f32) -> Self {
        Self {
            rect,
            cursor: rect.y(),
            gap,
        }
    }

    /// Height still free below the cursor.
    pub fn remaining(&self) -> f32 {
        (self.rect.bottom() - self.cursor).max(0.0)
    }

    /// Takes up to `height` pixels from the top. The result never extends past the parent.
    pub fn take(&mut self, height: f32) -> Rect {
        let h = height.clamp(0.0, self.remaining());
        let r = Rect::new(self.rect.x(), self.cursor, self.rect.w(), h);
        self.advance(h);
        r
    }

    /// Takes all remaining height.
    pub fn rest(&mut self) -> Rect {
        let h = self.remaining();
        self.take(h)
    }

    fn advance(&mut self, used: f32) {
        if used > 0.0 {
            self.cursor = (self.cursor + used + self.gap).min(self.rect.bottom());
        }
    }
}

/// Stacks children left to right, full height.
#[derive(Clone, Copy, Debug)]
pub struct Row {
    rect: Rect,
    cursor: f32,
    gap: f32,
}

impl Row {
    pub fn new(rect: Rect, gap: f32) -> Self {
        Self {
            rect,
            cursor: rect.x(),
            gap,
        }
    }

    /// Width still free to the right of the cursor.
    pub fn remaining(&self) -> f32 {
        (self.rect.right() - self.cursor).max(0.0)
    }

    /// Takes up to `width` pixels from the left. The result never extends past the parent.
    pub fn take(&mut self, width: f32) -> Rect {
        let w = width.clamp(0.0, self.remaining());
        let r = Rect::new(self.cursor, self.rect.y(), w, self.rect.h());
        if w > 0.0 {
            self.cursor = (self.cursor + w + self.gap).min(self.rect.right());
        }
        r
    }

    /// Takes all remaining width.
    pub fn rest(&mut self) -> Rect {
        let w = self.remaining();
        self.take(w)
    }
}

// ---------------------------------------------------------------------------------------------
// Responsive shell for ANVL Note. Everything below works in logical points (physical pixels
// divided by the scale factor). Callers turn the result into physical pixels with
// `Rect::to_physical`.

/// How an axis hands out space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    /// Exactly this many points.
    Fixed(f32),
    /// The child's own measured size.
    Hug,
    /// A share of whatever is left after Fixed and Hug children, by weight.
    Fill(f32),
}

/// Window posture, chosen from the logical width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutMode {
    /// 640 points or wider: sidebar and editor side by side.
    Expanded,
    /// Narrower than 640 points: one view at a time, with a back button in the header.
    Compact,
}

impl LayoutMode {
    pub const BREAKPOINT: f32 = 640.0;

    pub fn for_width(logical_width: f32) -> Self {
        if logical_width >= Self::BREAKPOINT {
            LayoutMode::Expanded
        } else {
            LayoutMode::Compact
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LayoutMode::Expanded => "EXPANDED",
            LayoutMode::Compact => "COMPACT",
        }
    }
}

/// Which view Compact mode shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompactView {
    List,
    Editor,
}

/// Splits `total` points along one axis. `hug` holds the measured size of each child by index
/// and is read only for Hug children. Fixed and Hug children keep their size unless the axis is
/// too short, in which case they shrink in proportion. Fill children share the rest by weight.
/// The sizes plus the gaps never exceed `total`.
pub fn solve_axis(total: f32, sizes: &[Size], hug: &[f32], gap: f32) -> Vec<f32> {
    let total = total.max(0.0);
    let gaps = gap * sizes.len().saturating_sub(1) as f32;
    let avail = (total - gaps).max(0.0);
    let mut out: Vec<f32> = sizes
        .iter()
        .enumerate()
        .map(|(i, s)| match *s {
            Size::Fixed(v) => v.max(0.0),
            Size::Hug => hug.get(i).copied().unwrap_or(0.0).max(0.0),
            Size::Fill(_) => 0.0,
        })
        .collect();
    let used: f32 = out.iter().sum();
    if used > avail && used > 0.0 {
        let k = avail / used;
        for v in &mut out {
            *v *= k;
        }
    }
    let left = (avail - out.iter().sum::<f32>()).max(0.0);
    let weight: f32 = sizes
        .iter()
        .map(|s| match s {
            Size::Fill(w) => w.max(0.0),
            _ => 0.0,
        })
        .sum();
    if weight > 0.0 {
        for (v, s) in out.iter_mut().zip(sizes) {
            if let Size::Fill(w) = s {
                *v = left * w.max(0.0) / weight;
            }
        }
    }
    out
}

/// Header height in points on desktop.
pub const HEADER_H: f32 = 52.0;
/// Header height in points on a phone (ADL-MOBILE section 2).
pub const HEADER_H_COMPACT: f32 = 60.0;
/// Status bar height in points on desktop. A phone has none.
pub const STATUS_H: f32 = 28.0;
/// The bottom action zone on a phone's list screen: a 56 pt button with 16 pt around it.
pub const BOTTOM_BAR_H: f32 = 88.0;
/// Sidebar width in Expanded mode, in points.
pub const SIDEBAR_W: f32 = 260.0;

/// Every region of the app, in logical points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellLayout {
    pub mode: LayoutMode,
    pub header: Rect,
    /// The note list. Absent in Compact mode while the editor is showing.
    pub sidebar: Option<Rect>,
    /// The editor. Absent in Compact mode while the list is showing.
    pub editor: Option<Rect>,
    pub status: Rect,
}

impl ShellLayout {
    /// Lays out a window of `w` by `h` points.
    pub fn solve(w: f32, h: f32, view: CompactView) -> Self {
        let mode = LayoutMode::for_width(w);
        let (head, foot) = match mode {
            LayoutMode::Expanded => (HEADER_H, STATUS_H),
            LayoutMode::Compact => (HEADER_H_COMPACT, 0.0),
        };
        let rows = solve_axis(h, &[Size::Fixed(head), Size::Fill(1.0), Size::Fixed(foot)], &[], 0.0);
        let (header_h, body_h, status_h) = (rows[0], rows[1], rows[2]);
        let header = Rect::new(0.0, 0.0, w, header_h);
        let status = Rect::new(0.0, header_h + body_h, w, status_h);
        let (sidebar, editor) = match mode {
            LayoutMode::Expanded => {
                let cols = solve_axis(w, &[Size::Fixed(SIDEBAR_W), Size::Fill(1.0)], &[], 0.0);
                (
                    Some(Rect::new(0.0, header_h, cols[0], body_h)),
                    Some(Rect::new(cols[0], header_h, cols[1], body_h)),
                )
            }
            LayoutMode::Compact => {
                let full = Rect::new(0.0, header_h, w, body_h);
                match view {
                    CompactView::List => (Some(full), None),
                    CompactView::Editor => (None, Some(full)),
                }
            }
        };
        Self {
            mode,
            header,
            sidebar,
            editor,
            status,
        }
    }
}

#[cfg(test)]
mod shell_tests {
    use super::*;

    #[test]
    fn breakpoint_is_640() {
        assert_eq!(LayoutMode::for_width(640.0), LayoutMode::Expanded);
        assert_eq!(LayoutMode::for_width(639.9), LayoutMode::Compact);
    }

    #[test]
    fn expanded_splits_260_and_fill() {
        let l = ShellLayout::solve(1000.0, 700.0, CompactView::List);
        let (s, e) = (l.sidebar.unwrap(), l.editor.unwrap());
        assert_eq!(s.w(), 260.0);
        assert_eq!(e.x(), 260.0);
        assert_eq!(e.w(), 740.0);
        assert_eq!(s.h(), 700.0 - HEADER_H - STATUS_H);
        assert_eq!(l.status.bottom(), 700.0);
    }

    #[test]
    fn a_phone_has_a_taller_header_and_no_status_bar() {
        let l = ShellLayout::solve(393.0, 800.0, CompactView::List);
        assert_eq!(l.header.h(), HEADER_H_COMPACT);
        assert_eq!(l.status.h(), 0.0);
        assert_eq!(l.sidebar.unwrap().bottom(), 800.0, "the body runs to the bottom edge");
    }

    #[test]
    fn compact_shows_one_view() {
        let l = ShellLayout::solve(400.0, 700.0, CompactView::List);
        assert!(l.sidebar.is_some() && l.editor.is_none());
        let l = ShellLayout::solve(400.0, 700.0, CompactView::Editor);
        assert!(l.sidebar.is_none() && l.editor.unwrap().w() == 400.0);
    }

    #[test]
    fn solver_never_overflows() {
        let v = solve_axis(100.0, &[Size::Fixed(80.0), Size::Hug, Size::Fill(1.0)], &[0.0, 60.0], 10.0);
        assert!(v.iter().sum::<f32>() + 20.0 <= 100.0 + 1e-3);
        let v = solve_axis(300.0, &[Size::Fixed(100.0), Size::Fill(1.0), Size::Fill(3.0)], &[], 0.0);
        assert_eq!(v, vec![100.0, 50.0, 150.0]);
    }
}
