//! Text-entry rules: what an input accepts, and how a caret and selection edit a value.
//!
//! Everything here works on character indices, not bytes, so accented letters and emoji edit
//! correctly. Nothing here knows about the window or the GPU, so it is tested directly.

/// What a text input accepts. Each kind checks the whole candidate value, not one character,
/// so a rule like "one decimal point" works at any position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputKind {
    /// Any printable text.
    Text,
    /// Letters and digits only, for codes such as health card IDs.
    Alnum,
    /// Any printable text, shown as bullets.
    Password,
    /// Email-shaped text: no whitespace, at most one `@`.
    Email,
    /// A decimal number with at most `decimals` digits after the point, kept inside
    /// `min..=max` when it is normalized.
    Number {
        min: Option<f64>,
        max: Option<f64>,
        decimals: u8,
    },
}

impl InputKind {
    /// True if `candidate` is an acceptable value, or a prefix of one that could still become
    /// acceptable. A lone `-` or `.` is allowed so the user can start typing a number.
    pub fn accepts(&self, candidate: &str) -> bool {
        match self {
            InputKind::Text | InputKind::Password => !candidate.chars().any(|c| c.is_control()),
            InputKind::Alnum => candidate.chars().all(|c| c.is_ascii_alphanumeric()),
            InputKind::Email => {
                !candidate
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control())
                    && candidate.matches('@').count() <= 1
            }
            InputKind::Number { decimals, .. } => number_prefix_ok(candidate, *decimals),
        }
    }

    /// True for kinds whose text is hidden on screen.
    pub fn is_password(&self) -> bool {
        matches!(self, InputKind::Password)
    }

    /// Brings a finished number back into range and formats it. Returns `None` for kinds that
    /// are not numbers, for text that does not parse, and for values that are already correct.
    pub fn normalize(&self, value: &str) -> Option<String> {
        let InputKind::Number {
            min,
            max,
            decimals,
        } = *self
        else {
            return None;
        };
        let parsed: f64 = value.parse().ok()?;
        let clamped = clamp_number(parsed, min, max);
        let formatted = format_number(clamped, decimals);
        (formatted != value).then_some(formatted)
    }

    /// The value one step up or down, clamped. Returns `None` for non-number kinds.
    /// An empty number counts as zero.
    pub fn step(&self, value: &str, delta: f64) -> Option<String> {
        let InputKind::Number {
            min,
            max,
            decimals,
        } = *self
        else {
            return None;
        };
        let current = if value.is_empty() {
            0.0
        } else {
            value.parse::<f64>().ok()?
        };
        let next = clamp_number(current + delta, min, max);
        Some(format_number(next, decimals))
    }
}

fn number_prefix_ok(s: &str, decimals: u8) -> bool {
    let body = s.strip_prefix('-').unwrap_or(s);
    let (int_part, frac) = match body.split_once('.') {
        Some((int_part, frac)) => (int_part, Some(frac)),
        None => (body, None),
    };
    if !int_part.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    match frac {
        None => true,
        Some(frac) => {
            decimals > 0
                && frac.len() <= decimals as usize
                && frac.chars().all(|c| c.is_ascii_digit())
        }
    }
}

fn clamp_number(v: f64, min: Option<f64>, max: Option<f64>) -> f64 {
    let mut v = v;
    if let Some(min) = min {
        v = v.max(min);
    }
    if let Some(max) = max {
        v = v.min(max);
    }
    v
}

fn format_number(v: f64, decimals: u8) -> String {
    if decimals == 0 {
        format!("{}", v.round() as i64)
    } else {
        format!("{:.*}", decimals as usize, v)
    }
}

/// Caret and selection inside a value, in character indices. The caret is where typing
/// happens. The anchor is the other end of the selection. When they are equal, nothing is
/// selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Editor {
    pub caret: usize,
    pub anchor: usize,
}

impl Editor {
    /// Caret at the end of a value of `len` characters.
    pub fn at_end(len: usize) -> Self {
        Self {
            caret: len,
            anchor: len,
        }
    }

    /// Keeps both ends inside a value of `len` characters. Call it when the value changes
    /// from outside.
    pub fn clamp(&mut self, len: usize) {
        self.caret = self.caret.min(len);
        self.anchor = self.anchor.min(len);
    }

    /// The selected range as `(start, end)`, with `start <= end`.
    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    pub fn selected_text(&self, value: &str) -> String {
        let (lo, hi) = self.selection();
        value.chars().skip(lo).take(hi - lo).collect()
    }

    /// Moves the caret. With `extend`, the anchor stays put, so the selection grows.
    pub fn move_to(&mut self, index: usize, extend: bool) {
        self.caret = index;
        if !extend {
            self.anchor = index;
        }
    }

    pub fn select_all(&mut self, len: usize) {
        self.anchor = 0;
        self.caret = len;
    }

    /// Replaces the selection with `text`. Returns false, and changes nothing, if the result
    /// would break `kind` or be longer than `max_len` characters.
    pub fn insert(&mut self, value: &mut String, text: &str, max_len: usize, kind: &InputKind) -> bool {
        let chars: Vec<char> = value.chars().collect();
        let (lo, hi) = self.selection();
        let mut candidate: String = chars[..lo].iter().collect();
        candidate.push_str(text);
        candidate.extend(chars[hi..].iter());
        if candidate.chars().count() > max_len || !kind.accepts(&candidate) {
            return false;
        }
        *value = candidate;
        self.caret = lo + text.chars().count();
        self.anchor = self.caret;
        true
    }

    /// Removes the selected text. Returns true if anything was removed.
    pub fn delete_selection(&mut self, value: &mut String) -> bool {
        if !self.has_selection() {
            return false;
        }
        let chars: Vec<char> = value.chars().collect();
        let (lo, hi) = self.selection();
        let mut next: String = chars[..lo].iter().collect();
        next.extend(chars[hi..].iter());
        *value = next;
        self.caret = lo;
        self.anchor = lo;
        true
    }

    /// Backspace: removes the selection, or the character before the caret. With `word`, it
    /// removes back to the start of the previous word.
    pub fn backspace(&mut self, value: &mut String, word: bool) {
        if self.delete_selection(value) || self.caret == 0 {
            return;
        }
        let target = if word {
            word_start_before(value, self.caret)
        } else {
            self.caret - 1
        };
        self.remove_range(value, target, self.caret);
    }

    /// Delete: removes the selection, or the character after the caret. With `word`, it
    /// removes forward to the end of the next word.
    pub fn delete_forward(&mut self, value: &mut String, word: bool) {
        if self.delete_selection(value) {
            return;
        }
        let len = value.chars().count();
        if self.caret >= len {
            return;
        }
        let target = if word {
            word_end_after(value, self.caret)
        } else {
            self.caret + 1
        };
        self.remove_range(value, self.caret, target);
    }

    fn remove_range(&mut self, value: &mut String, from: usize, to: usize) {
        let chars: Vec<char> = value.chars().collect();
        let mut next: String = chars[..from].iter().collect();
        next.extend(chars[to..].iter());
        *value = next;
        self.caret = from;
        self.anchor = from;
    }

    /// Left arrow. With `word`, jumps a word. With `extend`, grows the selection.
    pub fn left(&mut self, value: &str, extend: bool, word: bool) {
        let target = if self.has_selection() && !extend {
            self.selection().0
        } else if word {
            word_start_before(value, self.caret)
        } else {
            self.caret.saturating_sub(1)
        };
        self.move_to(target, extend);
    }

    /// Right arrow. With `word`, jumps a word. With `extend`, grows the selection.
    pub fn right(&mut self, value: &str, extend: bool, word: bool) {
        let len = value.chars().count();
        let target = if self.has_selection() && !extend {
            self.selection().1
        } else if word {
            word_end_after(value, self.caret)
        } else {
            (self.caret + 1).min(len)
        };
        self.move_to(target, extend);
    }

    pub fn home(&mut self, extend: bool) {
        self.move_to(0, extend);
    }

    pub fn end(&mut self, value: &str, extend: bool) {
        self.move_to(value.chars().count(), extend);
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Index of the start of the word before `index`, skipping spaces first.
fn word_start_before(value: &str, index: usize) -> usize {
    let chars: Vec<char> = value.chars().collect();
    let mut i = index.min(chars.len());
    while i > 0 && !is_word_char(chars[i - 1]) {
        i -= 1;
    }
    while i > 0 && is_word_char(chars[i - 1]) {
        i -= 1;
    }
    i
}

/// Index of the end of the word after `index`, skipping spaces first.
fn word_end_after(value: &str, index: usize) -> usize {
    let chars: Vec<char> = value.chars().collect();
    let len = chars.len();
    let mut i = index.min(len);
    while i < len && !is_word_char(chars[i]) {
        i += 1;
    }
    while i < len && is_word_char(chars[i]) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: InputKind = InputKind::Text;

    #[test]
    fn number_kind_accepts_partial_and_complete_numbers() {
        let n = InputKind::Number {
            min: None,
            max: None,
            decimals: 2,
        };
        for ok in ["", "-", ".", "7", "-7", "12.5", "-0.25", "3."] {
            assert!(n.accepts(ok), "{ok:?} should be accepted");
        }
        for bad in ["1.2.3", "a", "--1", "1.234", "5-"] {
            assert!(!n.accepts(bad), "{bad:?} should be rejected");
        }
        let whole = InputKind::Number {
            min: None,
            max: None,
            decimals: 0,
        };
        assert!(!whole.accepts("1.5"), "whole numbers take no point");
    }

    #[test]
    fn number_normalize_clamps_and_formats() {
        let n = InputKind::Number {
            min: Some(0.0),
            max: Some(99.0),
            decimals: 0,
        };
        assert_eq!(n.normalize("250").as_deref(), Some("99"));
        assert_eq!(n.normalize("-4").as_deref(), Some("0"));
        assert_eq!(n.normalize("42"), None, "already correct");
        assert_eq!(n.normalize("-"), None, "unparsable text is left alone");
        let money = InputKind::Number {
            min: None,
            max: None,
            decimals: 2,
        };
        assert_eq!(money.normalize("3.5").as_deref(), Some("3.50"));
    }

    #[test]
    fn number_step_moves_and_clamps() {
        let n = InputKind::Number {
            min: Some(0.0),
            max: Some(10.0),
            decimals: 0,
        };
        assert_eq!(n.step("4", 1.0).as_deref(), Some("5"));
        assert_eq!(n.step("10", 1.0).as_deref(), Some("10"));
        assert_eq!(n.step("", 1.0).as_deref(), Some("1"));
        assert_eq!(TEXT.step("4", 1.0), None);
    }

    #[test]
    fn insert_replaces_the_selection_and_moves_the_caret() {
        let mut value = String::from("hello world");
        let mut ed = Editor { caret: 11, anchor: 6 };
        assert!(ed.insert(&mut value, "there", 40, &TEXT));
        assert_eq!(value, "hello there");
        assert_eq!(ed.caret, 11);
        assert!(!ed.has_selection());
    }

    #[test]
    fn insert_respects_max_len_and_kind() {
        let mut value = String::from("abc");
        let mut ed = Editor::at_end(3);
        assert!(!ed.insert(&mut value, "d", 3, &TEXT), "over max length");
        assert_eq!(value, "abc");
        let alnum = InputKind::Alnum;
        assert!(!ed.insert(&mut value, "!", 10, &alnum), "symbol rejected");
        assert_eq!(value, "abc");
    }

    #[test]
    fn character_by_character_selection_and_copy() {
        let value = String::from("abcdef");
        let mut ed = Editor::at_end(6);
        ed.left(&value, true, false);
        ed.left(&value, true, false);
        assert_eq!(ed.selected_text(&value), "ef");
        ed.left(&value, true, false);
        assert_eq!(ed.selected_text(&value), "def");
    }

    #[test]
    fn plain_arrow_collapses_the_selection_to_its_edge() {
        let value = String::from("abcdef");
        let mut ed = Editor { caret: 4, anchor: 1 };
        ed.left(&value, false, false);
        assert_eq!(ed.caret, 1);
        assert!(!ed.has_selection());
    }

    #[test]
    fn select_all_then_cut_and_paste() {
        let mut value = String::from("first line");
        let mut ed = Editor::at_end(value.chars().count());
        ed.select_all(value.chars().count());
        let copied = ed.selected_text(&value);
        assert_eq!(copied, "first line");
        assert!(ed.delete_selection(&mut value));
        assert_eq!(value, "");
        assert!(ed.insert(&mut value, "pasted", 40, &TEXT));
        assert_eq!(value, "pasted");
    }

    #[test]
    fn backspace_and_delete_work_by_character_and_by_word() {
        let mut value = String::from("hello big world");
        let mut ed = Editor::at_end(value.chars().count());
        ed.backspace(&mut value, false);
        assert_eq!(value, "hello big worl");
        ed.backspace(&mut value, true);
        assert_eq!(value, "hello big ");
        ed.home(false);
        ed.delete_forward(&mut value, true);
        assert_eq!(value, " big ");
    }

    #[test]
    fn word_jumps_stop_at_word_edges() {
        let value = String::from("one two  three");
        let mut ed = Editor::at_end(value.chars().count());
        ed.left(&value, false, true);
        assert_eq!(ed.caret, 9, "start of three");
        ed.left(&value, false, true);
        assert_eq!(ed.caret, 4, "start of two");
        ed.right(&value, false, true);
        assert_eq!(ed.caret, 7, "end of two");
    }

    #[test]
    fn multibyte_characters_index_by_character() {
        let mut value = String::from("café");
        let mut ed = Editor::at_end(4);
        ed.backspace(&mut value, false);
        assert_eq!(value, "caf");
    }

    #[test]
    fn password_and_email_rules() {
        assert!(InputKind::Password.is_password());
        assert!(InputKind::Email.accepts("a@b.co"));
        assert!(!InputKind::Email.accepts("a b@c"));
        assert!(!InputKind::Email.accepts("a@b@c"));
    }
}
