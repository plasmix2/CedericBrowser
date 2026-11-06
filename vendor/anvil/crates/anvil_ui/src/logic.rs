//! Pure logic behind the components: fuzzy search, tree visibility, calendar math, pagination,
//! colour conversion, syntax tokens, and a small Markdown block parser. Nothing here draws, so
//! every part is tested directly.

// ------------------------------------------------------------------ fuzzy search

/// How well `query` matches `candidate`, or `None` if the query's characters do not all
/// appear in order. Higher is better. Matches at word starts and runs of adjacent matches
/// score more. Gaps cost a little. Case is ignored.
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let cand: Vec<char> = candidate.to_lowercase().chars().collect();
    let mut score = 0;
    let mut qi = 0;
    let mut last_match: Option<usize> = None;
    for (ci, &c) in cand.iter().enumerate() {
        if qi < q.len() && c == q[qi] {
            score += 10;
            let word_start = ci == 0 || !cand[ci - 1].is_alphanumeric();
            if word_start {
                score += 8;
            }
            if let Some(prev) = last_match {
                if prev + 1 == ci {
                    score += 6;
                } else {
                    score -= (ci - prev - 1).min(5) as i32;
                }
            }
            last_match = Some(ci);
            qi += 1;
        }
    }
    (qi == q.len()).then_some(score)
}

/// Indices of `items` that match `query`, best match first. Ties keep their original order.
pub fn fuzzy_filter(query: &str, items: &[&str]) -> Vec<usize> {
    let mut scored: Vec<(usize, i32)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| fuzzy_score(query, item).map(|s| (i, s)))
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.into_iter().map(|(i, _)| i).collect()
}

// ------------------------------------------------------------------ tree visibility

/// Indices of the rows that are visible in a tree, in order. `depths` gives each row's
/// nesting depth in depth-first order. A row is hidden when any ancestor is collapsed.
/// `has_children` and `expanded` describe each row.
pub fn visible_rows(depths: &[usize], has_children: &[bool], expanded: &[bool]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut hidden_below: Option<usize> = None;
    for (i, &depth) in depths.iter().enumerate() {
        if let Some(parent_depth) = hidden_below {
            if depth > parent_depth {
                continue;
            }
            hidden_below = None;
        }
        out.push(i);
        if has_children.get(i).copied().unwrap_or(false) && !expanded.get(i).copied().unwrap_or(false) {
            hidden_below = Some(depth);
        }
    }
    out
}

// ------------------------------------------------------------------ calendar

pub fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days in month `month` (1 to 12) of `year`.
pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Weekday of the first day of the month, with Sunday as 0 (Sakamoto's method).
pub fn first_weekday(year: i32, month: u32) -> u32 {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let d = 1;
    ((y + y / 4 - y / 100 + y / 400 + T[(month - 1) as usize] + d) % 7) as u32
}

/// A six-week grid for the month: 42 cells, each a day number or `None` for padding.
pub fn month_grid(year: i32, month: u32) -> Vec<Option<u32>> {
    let lead = first_weekday(year, month) as usize;
    let days = days_in_month(year, month) as usize;
    (0..42)
        .map(|cell| {
            if cell >= lead && cell < lead + days {
                Some((cell - lead + 1) as u32)
            } else {
                None
            }
        })
        .collect()
}

// ------------------------------------------------------------------ pagination

/// Page buttons for `total` pages with `current` selected (1-based). `None` is an ellipsis.
/// Shows the first and last page, the current page with one neighbour each side, and fills
/// single gaps instead of collapsing them.
pub fn page_items(current: usize, total: usize) -> Vec<Option<usize>> {
    if total == 0 {
        return Vec::new();
    }
    let current = current.clamp(1, total);
    if total <= 7 {
        return (1..=total).map(Some).collect();
    }
    let mut keep: Vec<usize> = vec![1, total];
    for p in current.saturating_sub(1)..=current + 1 {
        if p >= 1 && p <= total {
            keep.push(p);
        }
    }
    keep.sort_unstable();
    keep.dedup();
    let mut out = Vec::new();
    let mut prev = 0;
    for p in keep {
        if prev != 0 {
            if p - prev == 2 {
                out.push(Some(prev + 1));
            } else if p - prev > 2 {
                out.push(None);
            }
        }
        out.push(Some(p));
        prev = p;
    }
    out
}

// ------------------------------------------------------------------ colour

/// HSV to RGB. `h` is in degrees (any value, wrapped), `s` and `v` in 0..1.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m]
}

/// `#rrggbb` for an RGB colour in 0..1.
pub fn rgb_to_hex(c: [f32; 3]) -> String {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c[0]), byte(c[1]), byte(c[2]))
}

// ------------------------------------------------------------------ syntax

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    Keyword,
    Type,
    Ident,
    Number,
    Str,
    Comment,
    Punct,
    Space,
}

const KEYWORDS: [&str; 16] = [
    "fn", "let", "mut", "pub", "struct", "enum", "impl", "use", "mod", "return", "if", "else", "match",
    "for", "while", "const",
];

/// Splits one line of Rust-like code into tokens. Covers keywords, types (capitalized names),
/// strings, numbers, `//` comments, and punctuation. Spaces are kept so the tokens rebuild
/// the line exactly.
pub fn tokenize_line(line: &str) -> Vec<(Tok, String)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let start = i;
        if c.is_whitespace() {
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            out.push((Tok::Space, chars[start..i].iter().collect()));
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            out.push((Tok::Comment, chars[i..].iter().collect()));
            i = chars.len();
        } else if c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(chars.len());
            out.push((Tok::Str, chars[start..i].iter().collect()));
        } else if c.is_ascii_digit() {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_') {
                i += 1;
            }
            out.push((Tok::Number, chars[start..i].iter().collect()));
        } else if c.is_alphabetic() || c == '_' {
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let kind = if KEYWORDS.contains(&word.as_str()) {
                Tok::Keyword
            } else if word.chars().next().is_some_and(|c| c.is_uppercase()) {
                Tok::Type
            } else {
                Tok::Ident
            };
            out.push((kind, word));
        } else {
            i += 1;
            out.push((Tok::Punct, c.to_string()));
        }
    }
    out
}

// ------------------------------------------------------------------ markdown

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// Heading with its level (1 to 3) and text.
    Heading(u8, String),
    Bullet(String),
    /// One line inside a fenced code block.
    Code(String),
    Para(String),
    Blank,
}

/// Parses the block structure of a small Markdown subset: `#` headings (up to three levels),
/// `-` and `*` bullets, fenced code blocks, and paragraphs. Inline emphasis is not parsed.
pub fn parse_markdown(src: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut in_code = false;
    for line in src.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            out.push(Block::Code(trimmed.to_string()));
            continue;
        }
        let t = trimmed.trim_start();
        if t.is_empty() {
            out.push(Block::Blank);
        } else if let Some(rest) = t.strip_prefix("### ") {
            out.push(Block::Heading(3, rest.to_string()));
        } else if let Some(rest) = t.strip_prefix("## ") {
            out.push(Block::Heading(2, rest.to_string()));
        } else if let Some(rest) = t.strip_prefix("# ") {
            out.push(Block::Heading(1, rest.to_string()));
        } else if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            out.push(Block::Bullet(rest.to_string()));
        } else {
            out.push(Block::Para(t.to_string()));
        }
    }
    out
}

// ------------------------------------------------------------------ motion helpers

/// Opacity for a toast with `remaining` seconds to live, fading over the last `fade` seconds.
pub fn toast_alpha(remaining: f32, fade: f32) -> f32 {
    if fade <= 0.0 {
        return if remaining > 0.0 { 1.0 } else { 0.0 };
    }
    (remaining / fade).clamp(0.0, 1.0)
}

/// A point on a cubic Bézier curve at parameter `t` in 0..1.
pub fn cubic_point(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], t: f32) -> [f32; 2] {
    let u = 1.0 - t;
    let a = u * u * u;
    let b = 3.0 * u * u * t;
    let c = 3.0 * u * t * t;
    let d = t * t * t;
    [
        a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0],
        a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_prefers_word_starts_and_runs() {
        assert!(fuzzy_score("zz", "open file").is_none());
        // A query matching word starts outranks one matching mid-word.
        let start = fuzzy_score("ope", "open file").unwrap();
        let middle = fuzzy_score("ope", "reopen").unwrap();
        assert!(start > middle, "word start {start} vs mid-word {middle}");
        let items = ["Save as", "Open file", "Close window"];
        let ranked = fuzzy_filter("open", &items);
        assert_eq!(ranked.first(), Some(&1));
        assert_eq!(fuzzy_filter("", &items), vec![0, 1, 2]);
    }

    #[test]
    fn collapsed_folders_hide_their_children() {
        // root(0) > a(1) > a1(2); root > b(1)
        let depths = [0, 1, 2, 1];
        let has_children = [true, true, false, false];
        assert_eq!(visible_rows(&depths, &has_children, &[true, false, false, false]), vec![0, 1, 3]);
        assert_eq!(visible_rows(&depths, &has_children, &[true, true, false, false]), vec![0, 1, 2, 3]);
        assert_eq!(visible_rows(&depths, &has_children, &[false, false, false, false]), vec![0]);
    }

    #[test]
    fn calendar_math_matches_the_real_calendar() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(1900, 2), 28, "century years are not leap years");
        // 1 January 2024 was a Monday.
        assert_eq!(first_weekday(2024, 1), 1);
        // 1 March 2025 was a Saturday.
        assert_eq!(first_weekday(2025, 3), 6);
        let grid = month_grid(2024, 1);
        assert_eq!(grid.len(), 42);
        assert_eq!(grid.iter().filter(|c| c.is_some()).count(), 31);
        assert_eq!(grid[1], Some(1), "Monday is the second cell");
    }

    #[test]
    fn pagination_keeps_the_ends_and_the_neighbours() {
        assert_eq!(page_items(1, 5), (1..=5).map(Some).collect::<Vec<_>>());
        assert_eq!(
            page_items(1, 20),
            vec![Some(1), Some(2), None, Some(20)]
        );
        assert_eq!(
            page_items(10, 20),
            vec![Some(1), None, Some(9), Some(10), Some(11), None, Some(20)]
        );
        // A single missing page between two shown pages is filled instead of an ellipsis.
        assert_eq!(
            page_items(4, 10),
            vec![Some(1), Some(2), Some(3), Some(4), Some(5), None, Some(10)]
        );
    }

    #[test]
    fn colour_conversions_are_exact_at_the_corners() {
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), [1.0, 0.0, 0.0]);
        assert_eq!(hsv_to_rgb(120.0, 1.0, 1.0), [0.0, 1.0, 0.0]);
        assert_eq!(hsv_to_rgb(240.0, 1.0, 1.0), [0.0, 0.0, 1.0]);
        assert_eq!(rgb_to_hex([1.0, 0.0, 0.5]), "#ff0080");
    }

    #[test]
    fn tokens_rebuild_the_line() {
        let line = "fn main() { let x = 42; // done \"hi\"";
        let tokens = tokenize_line(line);
        let rebuilt: String = tokens.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(rebuilt, line);
        assert_eq!(tokens[0], (Tok::Keyword, "fn".to_string()));
        assert!(tokens.iter().any(|(k, t)| *k == Tok::Number && t == "42"));
        assert!(tokens.iter().any(|(k, _)| *k == Tok::Comment));
        assert!(tokenize_line("Vec::new()").iter().any(|(k, t)| *k == Tok::Type && t == "Vec"));
    }

    #[test]
    fn markdown_parses_blocks() {
        let src = "# Title\nSome text\n- one\n* two\n```\ncode line\n```\n\n## Sub";
        let blocks = parse_markdown(src);
        assert_eq!(blocks[0], Block::Heading(1, "Title".into()));
        assert_eq!(blocks[1], Block::Para("Some text".into()));
        assert_eq!(blocks[2], Block::Bullet("one".into()));
        assert_eq!(blocks[3], Block::Bullet("two".into()));
        assert_eq!(blocks[4], Block::Code("code line".into()));
        // The fences themselves produce no block.
        assert_eq!(blocks[5], Block::Blank);
        assert_eq!(blocks[6], Block::Heading(2, "Sub".into()));
    }

    #[test]
    fn toast_fades_out() {
        assert_eq!(toast_alpha(2.0, 0.5), 1.0);
        assert!((toast_alpha(0.25, 0.5) - 0.5).abs() < 1e-6);
        assert_eq!(toast_alpha(-1.0, 0.5), 0.0);
    }
}
