//! Markdown highlighting for the note editor (ADL-V3 section 8.14).
//!
//! This is styling, not rendering: every character of the source stays on screen, so the caret,
//! selection and undo work on exactly what was typed. [`highlight`] finds the parts of the text
//! that look different (headings, list markers, quotes, code, emphasis, rules) and returns them
//! as sorted, non-overlapping byte ranges. Text outside every range is plain body text.
//!
//! The grammar is a small, line-based subset of CommonMark:
//!
//!   * `#` to `######` and a space start a heading.
//!   * `-`, `*` or `+` and a space, or digits, `.` or `)` and a space, start a list item.
//!     `[ ]` or `[x]` after the marker makes it a task.
//!   * `>` starts a quote.
//!   * A line of three or more `-`, `*` or `_` (spaces allowed) is a rule.
//!   * A line starting with three backticks opens or closes a code block.
//!   * Inside a line: `` `code` ``, `**bold**` and `*italic*`.
//!
//! Leading spaces before a marker are allowed, so nested lists highlight too.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MdKind {
    /// The `#`s and the space after them.
    HeadingMark(u8),
    /// The heading text, with its level 1 to 6.
    Heading(u8),
    /// `- `, `1. ` and the like, with a task box if there is one.
    ListMark,
    /// The text of a ticked task.
    TaskDone,
    /// `> `.
    QuoteMark,
    Quote,
    /// A ```` ``` ```` line.
    Fence,
    /// A line inside a code block.
    CodeBlock,
    /// The backticks around inline code.
    CodeMark,
    InlineCode,
    /// `**` or `*` around emphasis.
    EmphMark,
    Bold,
    Italic,
    /// A `---` line.
    Rule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MdSpan {
    /// Byte range in the source.
    pub range: Range<usize>,
    pub kind: MdKind,
}

/// The styled parts of `text`, sorted by start, never overlapping.
pub fn highlight(text: &str) -> Vec<MdSpan> {
    let mut out = Vec::new();
    let mut in_code = false;
    let mut start = 0;
    for line in text.split('\n') {
        let end = start + line.len();
        let trimmed = line.trim_start_matches(' ');
        let indent = line.len() - trimmed.len();
        let push = |out: &mut Vec<MdSpan>, a: usize, b: usize, kind| {
            if b > a {
                out.push(MdSpan { range: start + a..start + b, kind });
            }
        };

        if trimmed.starts_with("```") {
            push(&mut out, 0, line.len(), MdKind::Fence);
            in_code = !in_code;
        } else if in_code {
            push(&mut out, 0, line.len(), MdKind::CodeBlock);
        } else if let Some(level) = heading_level(trimmed) {
            let mark = indent + level as usize + 1;
            push(&mut out, 0, mark.min(line.len()), MdKind::HeadingMark(level));
            push(&mut out, mark.min(line.len()), line.len(), MdKind::Heading(level));
        } else if is_rule(trimmed) {
            push(&mut out, 0, line.len(), MdKind::Rule);
        } else if let Some(rest) = trimmed.strip_prefix('>') {
            let mark = indent + 1 + usize::from(rest.starts_with(' '));
            push(&mut out, 0, mark, MdKind::QuoteMark);
            push(&mut out, mark, line.len(), MdKind::Quote);
        } else if let Some(len) = list_marker(trimmed) {
            let mut mark = indent + len;
            let after = &line[mark..];
            let task = ["[ ] ", "[x] ", "[X] "].iter().find(|t| after.starts_with(*t));
            if let Some(t) = task {
                mark += t.len();
            }
            push(&mut out, 0, mark, MdKind::ListMark);
            if task.is_some_and(|t| *t != "[ ] ") {
                push(&mut out, mark, line.len(), MdKind::TaskDone);
            } else {
                inline(&line[mark..], start + mark, &mut out);
            }
        } else {
            inline(line, start, &mut out);
        }
        start = end + 1;
    }
    out
}

/// Heading level of a line that starts with `#`s and a space (or is only `#`s).
fn heading_level(line: &str) -> Option<u8> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    match line.as_bytes().get(hashes) {
        None | Some(b' ') => Some(hashes as u8),
        _ => None,
    }
}

fn is_rule(line: &str) -> bool {
    let line = line.trim_end();
    let Some(c) = line.chars().next() else {
        return false;
    };
    matches!(c, '-' | '*' | '_')
        && line.chars().filter(|&x| x == c).count() >= 3
        && line.chars().all(|x| x == c || x == ' ')
}

/// Byte length of a list marker and its space at the start of `line`.
fn list_marker(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    if b.len() >= 2 && matches!(b[0], b'-' | b'*' | b'+') && b[1] == b' ' {
        return Some(2);
    }
    let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if (1..=9).contains(&digits) && b.len() > digits + 1 && matches!(b[digits], b'.' | b')') && b[digits + 1] == b' ' {
        return Some(digits + 2);
    }
    None
}

/// Inline code, bold and italic inside one line. `base` is the line's byte offset.
fn inline(line: &str, base: usize, out: &mut Vec<MdSpan>) {
    let b = line.as_bytes();
    let mut push = |a: usize, z: usize, kind| {
        if z > a {
            out.push(MdSpan { range: base + a..base + z, kind });
        }
    };
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'`' {
            if let Some(close) = line[i + 1..].find('`').map(|j| i + 1 + j) {
                if close > i + 1 {
                    push(i, i + 1, MdKind::CodeMark);
                    push(i + 1, close, MdKind::InlineCode);
                    push(close, close + 1, MdKind::CodeMark);
                    i = close + 1;
                    continue;
                }
            }
        } else if line[i..].starts_with("**") {
            if let Some(close) = line[i + 2..].find("**").map(|j| i + 2 + j) {
                if close > i + 2 && b[i + 2] != b' ' && b[close - 1] != b' ' {
                    push(i, i + 2, MdKind::EmphMark);
                    push(i + 2, close, MdKind::Bold);
                    push(close, close + 2, MdKind::EmphMark);
                    i = close + 2;
                    continue;
                }
            }
        } else if b[i] == b'*' && b.get(i + 1).is_some_and(|&c| c != b' ' && c != b'*') {
            let close = line[i + 1..]
                .char_indices()
                .find(|&(j, c)| c == '*' && b[i + j] != b' ')
                .map(|(j, _)| i + 1 + j);
            if let Some(close) = close {
                push(i, i + 1, MdKind::EmphMark);
                push(i + 1, close, MdKind::Italic);
                push(close, close + 1, MdKind::EmphMark);
                i = close + 1;
                continue;
            }
        }
        // Step one whole character.
        i += line[i..].chars().next().map_or(1, char::len_utf8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(&str, MdKind)> {
        highlight(text).into_iter().map(|s| (&text[s.range], s.kind)).collect()
    }

    #[test]
    fn headings_split_mark_and_text() {
        assert_eq!(
            kinds("# Title\n### Sub"),
            vec![
                ("# ", MdKind::HeadingMark(1)),
                ("Title", MdKind::Heading(1)),
                ("### ", MdKind::HeadingMark(3)),
                ("Sub", MdKind::Heading(3)),
            ]
        );
        assert!(kinds("#hashtag").is_empty());
        assert!(kinds("####### seven").is_empty());
        assert_eq!(kinds("##"), vec![("##", MdKind::HeadingMark(2))]);
    }

    #[test]
    fn lists_tasks_and_quotes() {
        assert_eq!(
            kinds("- item\n  2. two\n- [x] done\n- [ ] todo\n> said"),
            vec![
                ("- ", MdKind::ListMark),
                ("  2. ", MdKind::ListMark),
                ("- [x] ", MdKind::ListMark),
                ("done", MdKind::TaskDone),
                ("- [ ] ", MdKind::ListMark),
                ("> ", MdKind::QuoteMark),
                ("said", MdKind::Quote),
            ]
        );
    }

    #[test]
    fn code_blocks_hide_markdown_inside() {
        assert_eq!(
            kinds("```rs\n# not a heading\n```\n# yes"),
            vec![
                ("```rs", MdKind::Fence),
                ("# not a heading", MdKind::CodeBlock),
                ("```", MdKind::Fence),
                ("# ", MdKind::HeadingMark(1)),
                ("yes", MdKind::Heading(1)),
            ]
        );
    }

    #[test]
    fn inline_code_and_emphasis() {
        assert_eq!(
            kinds("a `x*y` **b** *i* 2 * 3"),
            vec![
                ("`", MdKind::CodeMark),
                ("x*y", MdKind::InlineCode),
                ("`", MdKind::CodeMark),
                ("**", MdKind::EmphMark),
                ("b", MdKind::Bold),
                ("**", MdKind::EmphMark),
                ("*", MdKind::EmphMark),
                ("i", MdKind::Italic),
                ("*", MdKind::EmphMark),
            ]
        );
        assert!(kinds("** not bold **").is_empty());
        assert!(kinds("unclosed `tick").is_empty());
    }

    #[test]
    fn rules_and_non_rules() {
        assert_eq!(kinds("---"), vec![("---", MdKind::Rule)]);
        assert_eq!(kinds("* * *"), vec![("* * *", MdKind::Rule)]);
        assert!(kinds("--").is_empty());
    }

    #[test]
    fn spans_are_sorted_and_disjoint_on_unicode() {
        let text = "# Été\n- **naïve** `ç` *ü*\n> ✓ done";
        let spans = highlight(text);
        for w in spans.windows(2) {
            assert!(w[0].range.end <= w[1].range.start);
        }
        for s in &spans {
            assert!(text.is_char_boundary(s.range.start) && text.is_char_boundary(s.range.end));
        }
    }
}
