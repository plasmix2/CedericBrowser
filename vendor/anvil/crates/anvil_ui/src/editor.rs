//! Multi-line text editing for `Ui::text_editor`.
//!
//! Two parts, both free of any GPU state so they can be tested alone:
//!
//!   * [`EditorCursor`] edits a `String` by character index: typing, deleting, line starts and
//!     ends, indent and outdent. It reuses the single-line [`Editor`] for the caret, the anchor,
//!     and the delete and insert rules.
//!   * [`VisualLines`] reads a shaped glyphon buffer and answers the geometry questions an editor
//!     needs: where the caret is drawn, which character a click hits, where Up and Down land, and
//!     which rectangles a selection covers.
//!
//! Text is always `\n`-separated. [`sanitize`] turns CRLF and lone CR into `\n` and drops other
//! control characters before anything reaches the buffer.

use crate::text::TextBuffer as Buffer;

use crate::input::Editor;

/// What Tab inserts.
pub const INDENT: &str = "  ";

/// Cleans text from the clipboard, a dropped file, or the keyboard: line endings become `\n`,
/// tabs become [`INDENT`], and other control characters are dropped.
pub fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\t' => out.push_str(INDENT),
            '\n' => out.push('\n'),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Characters and whitespace-separated words in `text`.
pub fn stats(text: &str) -> (usize, usize) {
    (text.chars().count(), text.split_whitespace().count())
}

/// The 1-based line and column of character index `idx`.
pub fn line_col(text: &str, idx: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for (i, c) in text.chars().enumerate() {
        if i >= idx {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn line_start(chars: &[char], idx: usize) -> usize {
    let mut i = idx.min(chars.len());
    while i > 0 && chars[i - 1] != '\n' {
        i -= 1;
    }
    i
}

fn line_end(chars: &[char], idx: usize) -> usize {
    let mut i = idx.min(chars.len());
    while i < chars.len() && chars[i] != '\n' {
        i += 1;
    }
    i
}

/// Caret, selection, scroll, and the bits of state a multi-line editor keeps between frames.
/// The app owns one per note.
#[derive(Clone, Debug, Default)]
pub struct EditorCursor {
    /// Caret and selection anchor, in character indices.
    pub ed: Editor,
    /// The x the caret keeps while moving through lines of different lengths, in pixels.
    pub goal_x: Option<f32>,
    /// How far the text has scrolled up, in physical pixels.
    pub scroll: f32,
    /// `Ui::time` of the last edit or caret move. The caret blink restarts from it.
    pub touched: f32,
    /// A finger that is down on the text and has not yet become a scroll or a tap.
    pub touch_press: Option<TouchPress>,
    /// Undo and redo for this text.
    pub history: History,
    /// Scroll velocity after a finger lets go, in physical px per second.
    pub fling: f32,
    /// The largest scroll from the last frame.
    pub max_scroll: f32,
    /// The selection came from touch, so it shows handles.
    pub touch_sel: bool,
    /// The touch selection menu (Cut, Copy, Paste, Select all) is open.
    pub menu: bool,
    /// Where the menu was drawn last frame, in physical px. A press there keeps focus.
    pub menu_rect: Option<crate::geom::Rect>,
    /// The selection handle a finger is dragging.
    pub handle: Option<SelEnd>,
    /// Time and place of the last tap or click, for double taps.
    pub last_tap: Option<(f32, f32, f32)>,
    /// The word a double click selected. A drag after it extends the selection by words.
    pub word_drag: Option<(usize, usize)>,
}

/// One end of a selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelEnd {
    Lo,
    Hi,
}

/// Seconds of pause that end a group of typing, so one undo never takes back too much.
pub const UNDO_GROUP_GAP: f32 = 1.0;
/// Undo steps kept per text. The oldest drop off.
pub const UNDO_LIMIT: usize = 200;

/// What kind of edit a step was. Runs of `Typing` or `Deleting` with no pause and no caret
/// jump between them become one step. Every other edit is a step of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Deleting,
    Other,
}

/// The text and caret before an edit.
#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    text: String,
    ed: Editor,
}

/// Undo and redo stacks of whole-text snapshots. Notes are small, so a snapshot per step costs
/// little and can never drift out of step with the text the way an edit log could.
#[derive(Clone, Debug, Default)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The open group: its kind, the caret it left, and when it last grew.
    group: Option<(EditKind, usize, f32)>,
}

impl History {
    /// Call before an edit of `kind` at time `now`. Records the text as it is now, unless the
    /// edit continues the open group. Returns true if a step was recorded, so a no-op edit can
    /// take it back with [`History::cancel`].
    pub fn record(&mut self, value: &str, ed: Editor, kind: EditKind, now: f32) -> bool {
        let continues = matches!(
            self.group,
            Some((k, caret, at)) if k == kind && kind != EditKind::Other && caret == ed.caret
                && !ed.has_selection() && now - at < UNDO_GROUP_GAP
        );
        if continues {
            return false;
        }
        self.undo.push(Snapshot { text: value.to_string(), ed });
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.group = Some((kind, ed.caret, now));
        true
    }

    /// Call after the edit that [`History::record`] prepared. A changed text clears redo and
    /// keeps the group open at the new caret. An unchanged one takes back the recorded step.
    pub fn finish(&mut self, recorded: bool, changed: bool, ed: Editor, now: f32) {
        if !changed {
            if recorded {
                self.undo.pop();
                self.group = None;
            }
            return;
        }
        self.redo.clear();
        if let Some(g) = self.group.as_mut() {
            g.1 = ed.caret;
            g.2 = now;
        }
    }

    /// Ends the open group, so the next edit is a step of its own. Caret moves call this.
    pub fn break_group(&mut self) {
        self.group = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Puts back the text before the last step. Returns true if there was one.
    pub fn undo(&mut self, value: &mut String, ed: &mut Editor) -> bool {
        Self::swap(&mut self.undo, &mut self.redo, value, ed, &mut self.group)
    }

    /// Applies the step that was last undone. Returns true if there was one.
    pub fn redo(&mut self, value: &mut String, ed: &mut Editor) -> bool {
        Self::swap(&mut self.redo, &mut self.undo, value, ed, &mut self.group)
    }

    fn swap(
        from: &mut Vec<Snapshot>,
        to: &mut Vec<Snapshot>,
        value: &mut String,
        ed: &mut Editor,
        group: &mut Option<(EditKind, usize, f32)>,
    ) -> bool {
        let Some(snap) = from.pop() else {
            return false;
        };
        to.push(Snapshot { text: std::mem::replace(value, snap.text), ed: *ed });
        *ed = snap.ed;
        *group = None;
        true
    }
}

/// A finger on the editor. A drag scrolls. A lift without a drag places the caret.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchPress {
    pub start: (f32, f32),
    pub last_y: f32,
    pub dragging: bool,
    /// `Ui::time` of the press, for the long press.
    pub t0: f32,
    /// It became a long press, so it no longer scrolls or taps.
    pub long: bool,
    /// The editor had focus before this press. Only then does a tap on the caret open the menu.
    pub had_focus: bool,
}

impl EditorCursor {
    /// Caret at the end of a value of `len` characters.
    pub fn at_end(len: usize) -> Self {
        Self {
            ed: Editor::at_end(len),
            ..Self::default()
        }
    }

    pub fn caret(&self) -> usize {
        self.ed.caret
    }

    /// Replaces the selection with `text` after cleaning it. Returns true if the value changed.
    pub fn insert(&mut self, value: &mut String, text: &str) -> bool {
        let clean = sanitize(text);
        if clean.is_empty() && !self.ed.has_selection() {
            return false;
        }
        // Not `Editor::insert`: its `Text` rule refuses newlines, which a note must accept.
        let chars: Vec<char> = value.chars().collect();
        let (lo, hi) = self.ed.selection();
        let mut out: String = chars[..lo].iter().collect();
        out.push_str(&clean);
        out.extend(chars[hi..].iter());
        *value = out;
        self.ed.move_to(lo + clean.chars().count(), false);
        self.goal_x = None;
        true
    }

    pub fn newline(&mut self, value: &mut String) -> bool {
        self.insert(value, "\n")
    }

    /// Backspace. With `word`, removes back to the start of the previous word.
    pub fn backspace(&mut self, value: &mut String, word: bool) -> bool {
        let before = value.len();
        let (c, a) = (self.ed.caret, self.ed.anchor);
        self.ed.backspace(value, word);
        self.goal_x = None;
        value.len() != before || (c, a) != (self.ed.caret, self.ed.anchor)
    }

    /// Delete. With `word`, removes forward to the end of the next word.
    pub fn delete_forward(&mut self, value: &mut String, word: bool) -> bool {
        let before = value.len();
        self.ed.delete_forward(value, word);
        self.goal_x = None;
        value.len() != before
    }

    pub fn left(&mut self, value: &str, extend: bool, word: bool) {
        self.goal_x = None;
        self.ed.left(value, extend, word);
    }

    pub fn right(&mut self, value: &str, extend: bool, word: bool) {
        self.goal_x = None;
        self.ed.right(value, extend, word);
    }

    /// Home: start of the caret's line. With `doc`, start of the text.
    pub fn home(&mut self, value: &str, extend: bool, doc: bool) {
        self.goal_x = None;
        let chars: Vec<char> = value.chars().collect();
        let to = if doc { 0 } else { line_start(&chars, self.ed.caret) };
        self.ed.move_to(to, extend);
    }

    /// End: end of the caret's line. With `doc`, end of the text.
    pub fn end(&mut self, value: &str, extend: bool, doc: bool) {
        self.goal_x = None;
        let chars: Vec<char> = value.chars().collect();
        let to = if doc { chars.len() } else { line_end(&chars, self.ed.caret) };
        self.ed.move_to(to, extend);
    }

    pub fn select_all(&mut self, value: &str) {
        self.goal_x = None;
        self.ed.select_all(value.chars().count());
    }

    /// Tab. A caret or a single-line selection gets [`INDENT`]. A selection over several lines
    /// indents each non-empty one and then selects them. Returns true if the value changed.
    pub fn tab(&mut self, value: &mut String) -> bool {
        let chars: Vec<char> = value.chars().collect();
        let (lo, hi) = self.ed.selection();
        if !chars[lo..hi].contains(&'\n') {
            return self.insert(value, INDENT);
        }
        let (first, last) = line_region(&chars, lo, hi);
        let region: String = chars[first..last].iter().collect();
        let indented: Vec<String> = region
            .split('\n')
            .map(|l| if l.is_empty() { String::new() } else { format!("{INDENT}{l}") })
            .collect();
        self.replace_region(value, &chars, first, last, &indented.join("\n"));
        true
    }

    /// Shift+Tab: removes up to one [`INDENT`] of leading spaces from each line the selection
    /// touches. Returns true if the value changed.
    pub fn outdent(&mut self, value: &mut String) -> bool {
        let chars: Vec<char> = value.chars().collect();
        let (lo, hi) = self.ed.selection();
        let (first, last) = line_region(&chars, lo, hi);
        let region: String = chars[first..last].iter().collect();
        let width = INDENT.chars().count();
        let mut removed_on_caret_line = 0;
        let lines: Vec<String> = region
            .split('\n')
            .enumerate()
            .map(|(i, l)| {
                let spaces = l.chars().take(width).take_while(|&c| c == ' ').count();
                if i == 0 {
                    removed_on_caret_line = spaces;
                }
                l.chars().skip(spaces).collect()
            })
            .collect();
        let out = lines.join("\n");
        if out == region {
            return false;
        }
        if self.ed.has_selection() {
            self.replace_region(value, &chars, first, last, &out);
        } else {
            // A bare caret keeps its place in the line.
            let caret = self.ed.caret;
            self.replace_region(value, &chars, first, last, &out);
            let col = (caret - first).saturating_sub(removed_on_caret_line);
            self.ed.move_to(first + col, false);
        }
        true
    }

    /// Swaps `chars[first..last]` for `with`, and selects the new text.
    fn replace_region(&mut self, value: &mut String, chars: &[char], first: usize, last: usize, with: &str) {
        let mut out: String = chars[..first].iter().collect();
        out.push_str(with);
        out.extend(chars[last..].iter());
        *value = out;
        self.ed.anchor = first;
        self.ed.caret = first + with.chars().count();
        self.goal_x = None;
    }
}

/// A Markdown formatting command, as the phone toolbar sends it (ADL-MOBILE section 8.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Cycles the lines through `# `, `## `, `### `, and back to plain.
    Heading,
    /// Toggles `- ` on the lines.
    Bullet,
    /// Puts `- [ ] ` on the lines, ticks an open task, and clears a ticked one.
    Task,
    Bold,
    Italic,
    Code,
}

/// Character range of the word around `idx`: letters, digits and `_`. On a space or a mark,
/// that one character. Empty only for empty text or a bare line break.
pub fn word_at(value: &str, idx: usize) -> (usize, usize) {
    let chars: Vec<char> = value.chars().collect();
    if chars.is_empty() {
        return (0, 0);
    }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    // A tap just past the end of a word belongs to that word.
    let mut i = idx.min(chars.len());
    if i == chars.len() || !word(chars[i]) {
        if i > 0 && word(chars[i - 1]) {
            i -= 1;
        } else {
            let i = i.min(chars.len() - 1);
            return if chars[i] == '\n' { (i, i) } else { (i, i + 1) };
        }
    }
    let mut lo = i;
    while lo > 0 && word(chars[lo - 1]) {
        lo -= 1;
    }
    let mut hi = i;
    while hi < chars.len() && word(chars[hi]) {
        hi += 1;
    }
    (lo, hi)
}

impl EditorCursor {
    /// Applies a formatting command to the selection or the caret's line, as one undo step.
    /// Returns true if the text changed.
    pub fn format(&mut self, value: &mut String, cmd: Format, now: f32) -> bool {
        self.history.break_group();
        let rec = self.history.record(value, self.ed, EditKind::Other, now);
        let changed = match cmd {
            Format::Bold => self.wrap(value, "**"),
            Format::Italic => self.wrap(value, "*"),
            Format::Code => self.wrap(value, "`"),
            Format::Heading | Format::Bullet | Format::Task => self.prefix_lines(value, cmd),
        };
        self.history.finish(rec, changed, self.ed, now);
        self.history.break_group();
        changed
    }

    /// Puts `mark` on both sides of the selection, or takes it off when it is already there.
    /// With no selection, inserts a pair and puts the caret between them.
    fn wrap(&mut self, value: &mut String, mark: &str) -> bool {
        let chars: Vec<char> = value.chars().collect();
        let (lo, hi) = self.ed.selection();
        let m: Vec<char> = mark.chars().collect();
        let n = m.len();
        let wrapped = lo >= n && hi + n <= chars.len() && chars[lo - n..lo] == m[..] && chars[hi..hi + n] == m[..];
        let inner: String = chars[lo..hi].iter().collect();
        if wrapped {
            self.replace_region(value, &chars, lo - n, hi + n, &inner);
            self.ed.anchor = lo - n;
            self.ed.caret = hi - n;
        } else {
            self.replace_region(value, &chars, lo, hi, &format!("{mark}{inner}{mark}"));
            self.ed.anchor = lo + n;
            self.ed.caret = hi + n;
        }
        true
    }

    /// Adds, cycles or removes a line prefix on every line the selection touches. A caret ends
    /// at the end of its line.
    fn prefix_lines(&mut self, value: &mut String, cmd: Format) -> bool {
        let chars: Vec<char> = value.chars().collect();
        let (lo, hi) = self.ed.selection();
        let (first, last) = line_region(&chars, lo, hi);
        let region: String = chars[first..last].iter().collect();
        let lines: Vec<String> = region.split('\n').map(|l| prefix_line(l, cmd)).collect();
        let out = lines.join("\n");
        if out == region {
            return false;
        }
        // Which line of the region the caret is on, so it can stay there.
        let caret_line = chars[first..self.ed.caret.clamp(first, last)].iter().filter(|&&c| c == '\n').count();
        self.replace_region(value, &chars, first, last, &out);
        if lo == hi {
            let new_chars: Vec<char> = value.chars().collect();
            let mut start = first;
            for _ in 0..caret_line {
                start = new_chars[start..].iter().position(|&c| c == '\n').map_or(new_chars.len(), |p| start + p + 1);
            }
            let end = line_end(&new_chars, start);
            self.ed.move_to(end, false);
        }
        true
    }
}

/// One line with a formatting command's prefix added, cycled or removed. Leading spaces stay.
fn prefix_line(line: &str, cmd: Format) -> String {
    let indent_len = line.len() - line.trim_start_matches(' ').len();
    let (indent, body) = line.split_at(indent_len);
    let next = match cmd {
        Format::Heading => {
            let level = body.chars().take_while(|&c| c == '#').count();
            let rest = body.trim_start_matches('#').trim_start();
            match level {
                0 => format!("# {body}"),
                1 | 2 => format!("{} {rest}", "#".repeat(level + 1)),
                _ => rest.to_string(),
            }
        }
        Format::Bullet => {
            if let Some(rest) = body.strip_prefix("- [ ] ").or_else(|| body.strip_prefix("- [x] ")) {
                format!("- {rest}")
            } else if let Some(rest) = body.strip_prefix("- ") {
                rest.to_string()
            } else {
                format!("- {body}")
            }
        }
        _ => {
            if let Some(rest) = body.strip_prefix("- [ ] ") {
                format!("- [x] {rest}")
            } else if let Some(rest) = body.strip_prefix("- [x] ") {
                rest.to_string()
            } else {
                format!("- [ ] {}", body.strip_prefix("- ").unwrap_or(body))
            }
        }
    };
    format!("{indent}{next}")
}

/// The whole lines a selection `lo..hi` touches, as a character range without the final newline.
/// A selection that ends at the very start of a line does not include that line.
fn line_region(chars: &[char], lo: usize, hi: usize) -> (usize, usize) {
    let first = line_start(chars, lo);
    let last = if hi > lo && hi == line_start(chars, hi) {
        hi - 1
    } else {
        line_end(chars, hi)
    };
    (first, last.max(first))
}

/// One character range on one logical line, from the first to the last glyph of a visual row.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Span {
    /// Character index of the first character on the logical line.
    char_start: usize,
    /// Byte index of the first byte on the logical line.
    byte_start: usize,
    /// Byte length of the logical line, without its `\n`.
    bytes: usize,
}

#[derive(Clone, Copy, Debug)]
struct G {
    start: usize,
    end: usize,
    x: f32,
    w: f32,
}

/// One visual row: a whole logical line, or one wrapped piece of one.
#[derive(Clone, Debug)]
struct Row {
    line: usize,
    top: f32,
    height: f32,
    glyphs: Vec<G>,
    /// First and one-past-last byte of the row inside its logical line.
    start: usize,
    end: usize,
    /// True for the last visual row of its logical line.
    last: bool,
}

/// Where the caret is drawn, relative to the top-left of the text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretPos {
    /// Index of the visual row.
    pub row: usize,
    pub x: f32,
    pub top: f32,
    pub height: f32,
}

/// Geometry of a shaped buffer: every visual row with its glyph positions.
#[derive(Clone, Debug)]
pub struct VisualLines {
    rows: Vec<Row>,
    spans: Vec<Span>,
    line_h: f32,
}

impl VisualLines {
    /// Reads the layout of `buf`, which must have been shaped from exactly `value`. `line_h` is
    /// the row height, used for rows that glyphon does not report (a trailing empty line).
    pub fn build(buf: &Buffer, value: &str, line_h: f32) -> Self {
        let mut spans = Vec::new();
        let (mut char_start, mut byte_start) = (0usize, 0usize);
        for piece in value.split('\n') {
            spans.push(Span {
                char_start,
                byte_start,
                bytes: piece.len(),
            });
            char_start += piece.chars().count() + 1;
            byte_start += piece.len() + 1;
        }

        let mut reported: Vec<Row> = buf
            .layout_runs()
            .map(|run| {
                let glyphs: Vec<G> = run
                    .glyphs
                    .iter()
                    .map(|g| G {
                        start: g.start,
                        end: g.end,
                        x: g.x,
                        w: g.w,
                    })
                    .collect();
                let (start, end) = match (glyphs.iter().map(|g| g.start).min(), glyphs.iter().map(|g| g.end).max()) {
                    (Some(s), Some(e)) => (s, e),
                    _ => (0, 0),
                };
                Row {
                    line: run.line_i,
                    top: run.line_top,
                    height: run.line_height,
                    glyphs,
                    start,
                    end,
                    last: false,
                }
            })
            .collect();

        // Glyphon reports no row for a logical line it considers empty at the end of the text.
        // Add one so the caret has somewhere to sit after a trailing newline.
        let mut rows = Vec::with_capacity(reported.len() + 1);
        let mut next_line = 0usize;
        let mut bottom = 0.0f32;
        for row in reported.drain(..) {
            while next_line < row.line {
                rows.push(empty_row(next_line, bottom, line_h));
                bottom += line_h;
                next_line += 1;
            }
            next_line = row.line + 1;
            bottom = row.top + row.height;
            rows.push(row);
        }
        while next_line < spans.len() {
            rows.push(empty_row(next_line, bottom, line_h));
            bottom += line_h;
            next_line += 1;
        }
        for i in 0..rows.len() {
            rows[i].last = rows.get(i + 1).is_none_or(|n| n.line != rows[i].line);
        }
        Self { rows, spans, line_h }
    }

    /// Geometry for text that has not been shaped: one empty row.
    pub fn build_empty(line_h: f32) -> Self {
        Self {
            rows: vec![empty_row(0, 0.0, line_h)],
            spans: vec![Span {
                char_start: 0,
                byte_start: 0,
                bytes: 0,
            }],
            line_h,
        }
    }

    /// Total height of the text in pixels.
    pub fn height(&self) -> f32 {
        self.rows.last().map_or(self.line_h, |r| r.top + r.height)
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// The logical line and the byte offset inside it for character index `idx`.
    fn locate(&self, value: &str, idx: usize) -> (usize, usize) {
        let line = self
            .spans
            .iter()
            .rposition(|s| s.char_start <= idx)
            .unwrap_or(0);
        let span = self.spans[line];
        let k = idx.saturating_sub(span.char_start);
        let text = &value[span.byte_start..span.byte_start + span.bytes];
        let byte = text.char_indices().nth(k).map_or(span.bytes, |(b, _)| b);
        (line, byte)
    }

    /// The character index for a byte offset on a logical line.
    fn char_index(&self, value: &str, line: usize, byte: usize) -> usize {
        let span = self.spans[line];
        let byte = byte.min(span.bytes);
        span.char_start + value[span.byte_start..span.byte_start + byte].chars().count()
    }

    fn x_at(&self, row: &Row, byte: usize) -> f32 {
        let Some(first) = row.glyphs.first() else {
            return 0.0;
        };
        if byte <= first.start {
            return first.x;
        }
        for g in &row.glyphs {
            if byte >= g.start && byte < g.end {
                return g.x;
            }
        }
        row.glyphs.last().map_or(0.0, |g| g.x + g.w)
    }

    /// Where the caret at character index `idx` is drawn.
    pub fn caret(&self, value: &str, idx: usize) -> CaretPos {
        let (line, byte) = self.locate(value, idx);
        let mut fallback = None;
        for (i, row) in self.rows.iter().enumerate().filter(|(_, r)| r.line == line) {
            fallback = Some(i);
            let inside = if row.glyphs.is_empty() {
                byte == 0
            } else {
                byte >= row.start && (byte < row.end || (byte == row.end && row.last))
            };
            if inside {
                return self.caret_in(i, byte);
            }
        }
        self.caret_in(fallback.unwrap_or(self.rows.len().saturating_sub(1)), byte)
    }

    fn caret_in(&self, row: usize, byte: usize) -> CaretPos {
        match self.rows.get(row) {
            Some(r) => CaretPos {
                row,
                x: self.x_at(r, byte),
                top: r.top,
                height: r.height,
            },
            None => CaretPos {
                row: 0,
                x: 0.0,
                top: 0.0,
                height: self.line_h,
            },
        }
    }

    /// The character index nearest to the point (`x`, `y`), relative to the top-left of the text.
    pub fn hit(&self, value: &str, x: f32, y: f32) -> usize {
        let Some(first) = self.rows.first() else {
            return 0;
        };
        let row = if y < first.top {
            0
        } else {
            self.rows
                .iter()
                .position(|r| y >= r.top && y < r.top + r.height)
                .unwrap_or(self.rows.len() - 1)
        };
        self.index_in_row(value, row, x)
    }

    fn index_in_row(&self, value: &str, row: usize, x: f32) -> usize {
        let r = &self.rows[row];
        let span = self.spans[r.line];
        let line_text = &value[span.byte_start..span.byte_start + span.bytes];
        let mut byte = None;
        for g in &r.glyphs {
            if x < g.x + g.w * 0.5 {
                byte = Some(g.start);
                break;
            }
        }
        let byte = byte.unwrap_or_else(|| match r.glyphs.last() {
            None => 0,
            Some(last) if r.last => last.end,
            // A wrapped row ends with the space it broke at. Land before it, so the caret stays
            // on this row instead of jumping to the start of the next one.
            Some(last) => {
                let tail = line_text.get(last.start..last.end).unwrap_or("");
                if tail.chars().all(char::is_whitespace) {
                    last.start
                } else {
                    last.end
                }
            }
        });
        self.char_index(value, r.line, byte)
    }

    /// Moves the caret one visual row up (`dir < 0`) or down (`dir > 0`), keeping `goal_x`.
    /// Returns the new character index and the x to remember. Up from the first row goes to the
    /// start of the text, and Down from the last row goes to its end.
    pub fn vertical(&self, value: &str, idx: usize, goal_x: Option<f32>, dir: i32) -> (usize, f32) {
        let here = self.caret(value, idx);
        let goal = goal_x.unwrap_or(here.x);
        let target = here.row as i64 + dir.signum() as i64;
        if target < 0 {
            return (0, goal);
        }
        if target as usize >= self.rows.len() {
            return (value.chars().count(), goal);
        }
        (self.index_in_row(value, target as usize, goal), goal)
    }

    /// Rectangles `[x, top, width, height]` covering the characters `lo..hi`, one per visual
    /// row. A selection that runs past the end of a line also covers a sliver for the newline.
    pub fn selection(&self, value: &str, lo: usize, hi: usize) -> Vec<[f32; 4]> {
        if lo >= hi {
            return Vec::new();
        }
        let (l1, b1) = self.locate(value, lo);
        let (l2, b2) = self.locate(value, hi);
        let mut out = Vec::new();
        for row in &self.rows {
            if row.line < l1 || row.line > l2 {
                continue;
            }
            if (row.line == l1 && b1 > row.end && !row.last) || (row.line == l2 && b2 < row.start) {
                continue;
            }
            let s = if row.line == l1 { b1.max(row.start) } else { row.start };
            let e = if row.line == l2 { b2.min(row.end) } else { row.end };
            let x0 = self.x_at(row, s);
            let mut x1 = if e >= s { self.x_at(row, e) } else { x0 };
            if row.last && row.line < l2 {
                x1 += self.line_h * 0.35;
            }
            if x1 > x0 {
                out.push([x0, row.top, x1 - x0, row.height]);
            }
        }
        out
    }
}

fn empty_row(line: usize, top: f32, height: f32) -> Row {
    Row {
        line,
        top,
        height,
        glyphs: Vec::new(),
        start: 0,
        end: 0,
        last: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{FontRole, TextSystem};

    #[test]
    fn sanitize_normalizes_line_endings_and_controls() {
        assert_eq!(sanitize("a\r\nb\rc\td\u{7}"), "a\nb\nc  d");
    }

    #[test]
    fn line_col_counts_from_one() {
        assert_eq!(line_col("ab\ncd", 0), (1, 1));
        assert_eq!(line_col("ab\ncd", 2), (1, 3));
        assert_eq!(line_col("ab\ncd", 4), (2, 2));
        assert_eq!(stats("one two\nthree"), (13, 3));
    }

    #[test]
    fn typing_enter_and_backspace() {
        let mut v = String::new();
        let mut c = EditorCursor::default();
        c.insert(&mut v, "ab");
        c.newline(&mut v);
        c.insert(&mut v, "cd");
        assert_eq!(v, "ab\ncd");
        c.backspace(&mut v, false);
        c.backspace(&mut v, false);
        c.backspace(&mut v, false);
        assert_eq!(v, "ab");
        assert_eq!(c.caret(), 2);
    }

    #[test]
    fn home_and_end_stay_on_the_line() {
        let v = "one\ntwo three\nfour".to_string();
        let mut c = EditorCursor::default();
        c.ed.move_to(6, false);
        c.home(&v, false, false);
        assert_eq!(c.caret(), 4);
        c.end(&v, false, false);
        assert_eq!(c.caret(), 13);
        c.home(&v, false, true);
        assert_eq!(c.caret(), 0);
        c.end(&v, true, true);
        assert_eq!(c.ed.selection(), (0, v.chars().count()));
    }

    #[test]
    fn tab_inserts_spaces_and_replaces_a_selection() {
        let mut v = "hello".to_string();
        let mut c = EditorCursor::at_end(5);
        assert!(c.tab(&mut v));
        assert_eq!(v, "hello  ");
        c.ed.anchor = 0;
        c.ed.caret = 5;
        c.tab(&mut v);
        assert_eq!(v, "    ", "the selected word is replaced by the indent, then the old trailing spaces");
    }

    #[test]
    fn tab_over_several_lines_indents_each() {
        let mut v = "a\nb\nc".to_string();
        let mut c = EditorCursor::default();
        c.ed.anchor = 0;
        c.ed.caret = 3;
        assert!(c.tab(&mut v));
        assert_eq!(v, "  a\n  b\nc");
    }

    #[test]
    fn outdent_removes_leading_spaces() {
        let mut v = "    a\n  b\nc".to_string();
        let mut c = EditorCursor::default();
        c.ed.anchor = 0;
        c.ed.caret = v.chars().count();
        assert!(c.outdent(&mut v));
        assert_eq!(v, "  a\nb\nc");
        assert!(!EditorCursor::default().outdent(&mut "x".to_string()));
    }

    fn shape(text: &str, wrap: Option<f32>) -> (TextSystem, VisualLines) {
        let mut ts = TextSystem::new();
        let key = ts.layout(text, 16.0, 400, FontRole::Body, wrap);
        let vl = VisualLines::build(ts.buffer(&key).unwrap(), text, 24.0);
        (ts, vl)
    }

    #[test]
    fn caret_and_hit_round_trip() {
        let text = "hello world\nsecond line\n\nlast";
        let (_ts, vl) = shape(text, None);
        for idx in 0..=text.chars().count() {
            let p = vl.caret(text, idx);
            let back = vl.hit(text, p.x + 0.5, p.top + p.height * 0.5);
            assert_eq!(back, idx, "index {idx} at {p:?}");
        }
    }

    #[test]
    fn trailing_newline_gets_its_own_row() {
        let text = "abc\n";
        let (_ts, vl) = shape(text, None);
        let end = vl.caret(text, 4);
        let start = vl.caret(text, 0);
        assert!(end.top > start.top, "caret after the newline is on the next row");
        assert_eq!(end.x, 0.0);
    }

    #[test]
    fn up_and_down_keep_the_column() {
        let text = "abcdef\nab\nabcdef";
        let (_ts, vl) = shape(text, None);
        let (down, goal) = vl.vertical(text, 5, None, 1);
        assert_eq!(down, 9, "short line clamps to its end");
        let (down2, _) = vl.vertical(text, down, Some(goal), 1);
        assert_eq!(down2, 15, "the goal column comes back on the long line");
        assert_eq!(vl.vertical(text, 2, None, -1).0, 0);
        assert_eq!(vl.vertical(text, 14, None, 1).0, text.chars().count());
    }

    #[test]
    fn wrapped_text_moves_through_visual_rows() {
        let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
        let (_ts, vl) = shape(text, Some(120.0));
        assert!(vl.row_count() > 2, "text wraps");
        let first = vl.caret(text, 0);
        let (idx, _) = vl.vertical(text, 0, None, 1);
        let second = vl.caret(text, idx);
        assert!(idx > 0 && second.top > first.top);
    }

    #[test]
    fn selection_covers_each_row_it_touches() {
        let text = "abc\ndef\nghi";
        let (_ts, vl) = shape(text, None);
        let rects = vl.selection(text, 1, 9);
        assert_eq!(rects.len(), 3);
        assert!(rects[0][0] > 0.0, "first row starts mid-line");
        assert!(vl.selection(text, 3, 3).is_empty());
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    /// Types `text` one character at a time, `dt` seconds apart, recording history like the
    /// editor does.
    fn type_chars(cur: &mut EditorCursor, value: &mut String, text: &str, t: &mut f32, dt: f32) {
        for c in text.chars() {
            *t += dt;
            let rec = cur.history.record(value, cur.ed, EditKind::Typing, *t);
            let changed = cur.insert(value, &c.to_string());
            cur.history.finish(rec, changed, cur.ed, *t);
        }
    }

    #[test]
    fn a_burst_of_typing_is_one_step() {
        let (mut cur, mut v, mut t) = (EditorCursor::default(), String::new(), 0.0);
        type_chars(&mut cur, &mut v, "hello", &mut t, 0.1);
        assert!(cur.history.undo(&mut v, &mut cur.ed));
        assert_eq!(v, "");
        assert_eq!(cur.caret(), 0);
        assert!(cur.history.redo(&mut v, &mut cur.ed));
        assert_eq!(v, "hello");
        assert_eq!(cur.caret(), 5);
    }

    #[test]
    fn a_pause_or_a_caret_jump_starts_a_new_step() {
        let (mut cur, mut v, mut t) = (EditorCursor::default(), String::new(), 0.0);
        type_chars(&mut cur, &mut v, "ab", &mut t, 0.1);
        t += UNDO_GROUP_GAP + 0.5;
        type_chars(&mut cur, &mut v, "cd", &mut t, 0.1);
        cur.ed.move_to(0, false);
        type_chars(&mut cur, &mut v, "X", &mut t, 0.1);
        assert_eq!(v, "Xabcd");
        cur.history.undo(&mut v, &mut cur.ed);
        assert_eq!(v, "abcd");
        cur.history.undo(&mut v, &mut cur.ed);
        assert_eq!(v, "ab");
        cur.history.undo(&mut v, &mut cur.ed);
        assert_eq!(v, "");
        assert!(!cur.history.can_undo());
    }

    #[test]
    fn a_new_edit_clears_redo_and_no_op_edits_leave_no_step() {
        let (mut cur, mut v, mut t) = (EditorCursor::default(), String::new(), 0.0);
        // Backspace at the start changes nothing, so it leaves nothing to undo.
        let rec = cur.history.record(&v, cur.ed, EditKind::Deleting, t);
        let changed = cur.backspace(&mut v, false);
        cur.history.finish(rec, changed, cur.ed, t);
        assert!(!cur.history.can_undo());

        type_chars(&mut cur, &mut v, "one", &mut t, 0.1);
        cur.history.undo(&mut v, &mut cur.ed);
        assert!(cur.history.can_redo());
        type_chars(&mut cur, &mut v, "two", &mut t, 0.1);
        assert!(!cur.history.can_redo());
        assert_eq!(v, "two");
    }

    #[test]
    fn history_is_capped() {
        let mut h = History::default();
        let ed = Editor::default();
        for i in 0..UNDO_LIMIT + 10 {
            h.record(&i.to_string(), ed, EditKind::Other, i as f32);
            h.finish(true, true, ed, i as f32);
        }
        assert_eq!(h.undo.len(), UNDO_LIMIT);
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;

    fn cur_at(lo: usize, hi: usize) -> EditorCursor {
        let mut c = EditorCursor::default();
        c.ed.anchor = lo;
        c.ed.caret = hi;
        c
    }

    #[test]
    fn word_at_finds_words_and_marks() {
        assert_eq!(word_at("hello world", 7), (6, 11));
        assert_eq!(word_at("hello world", 5), (0, 5));
        assert_eq!(word_at("a, b", 1), (0, 1));
        assert_eq!(word_at("a , b", 2), (2, 3));
        assert_eq!(word_at("", 0), (0, 0));
        assert_eq!(word_at("naïve café", 8), (6, 10));
    }

    #[test]
    fn wrap_adds_and_removes_marks() {
        let mut v = String::from("make bold now");
        let mut c = cur_at(5, 9);
        assert!(c.format(&mut v, Format::Bold, 0.0));
        assert_eq!(v, "make **bold** now");
        assert_eq!(c.ed.selection(), (7, 11));
        c.format(&mut v, Format::Bold, 1.0);
        assert_eq!(v, "make bold now");
        // Each command is one undo step.
        c.history.undo(&mut v, &mut c.ed);
        assert_eq!(v, "make **bold** now");
    }

    #[test]
    fn heading_cycles_and_lists_toggle() {
        let mut v = String::from("Title");
        let mut c = cur_at(2, 2);
        for want in ["# Title", "## Title", "### Title", "Title"] {
            c.format(&mut v, Format::Heading, 0.0);
            assert_eq!(v, want);
        }
        let mut v = String::from("one\ntwo");
        let mut c = cur_at(0, 7);
        c.format(&mut v, Format::Bullet, 0.0);
        assert_eq!(v, "- one\n- two");
        c.format(&mut v, Format::Task, 0.0);
        assert_eq!(v, "- [ ] one\n- [ ] two");
        c.format(&mut v, Format::Task, 0.0);
        assert_eq!(v, "- [x] one\n- [x] two");
        c.format(&mut v, Format::Bullet, 0.0);
        assert_eq!(v, "- one\n- two");
    }
}
