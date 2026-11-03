//! jq syntax jaq does not have, rewritten before a program is parsed, and
//! programs jaq cannot take, refused before.
//!
//! `$__loc__` is where it is written: `{"file":"<top-level>","line":N}`, as
//! jq 1.8.2 gives it (with `-f` too). It is a token of its own, so it is
//! replaced where it stands, outside strings and comments but inside a
//! string's `\(...)`; in an object, `{$__loc__}` is short for
//! `{"__loc__": $__loc__}`, as for any variable.
//!
//! jaq's parser and compiler recurse into brackets and along chains of
//! operators, and overflow even a large stack past some 100,000 levels; jq
//! refuses most such programs too ("memory exhausted"). A program is
//! refused when, at some point in it, the brackets open there and the
//! operators chained in each of them come to more than [`MAX_DEPTH`].

use std::borrow::Cow;

const LOC: &str = "$__loc__";

/// How deep a program may nest, its brackets and chained operators counted.
pub const MAX_DEPTH: usize = 10_000;

#[derive(Clone, Copy, PartialEq)]
enum Open {
    Paren,
    Bracket,
    Brace,
    /// `\(` in a string: its `)` goes back into the string.
    Interpolation,
}

/// Calls `f` with each byte of code (not of a string's text or a comment),
/// its offset and the brackets open around it; stops when `f` says false.
fn walk(code: &str, mut f: impl FnMut(usize, u8, &[Open]) -> bool) {
    let bytes = code.as_bytes();
    let mut open: Vec<Open> = Vec::new();
    let (mut in_string, mut i) = (false, 0);
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            match b {
                b'\\' if bytes.get(i + 1) == Some(&b'(') => {
                    open.push(Open::Interpolation);
                    in_string = false;
                    i += 2;
                    continue;
                }
                b'\\' => i += 1,
                b'"' => in_string = false,
                _ => {}
            }
            i += 1;
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'#' => {
                while i + 1 < bytes.len() && bytes[i + 1] != b'\n' {
                    i += 1;
                }
            }
            b'(' | b'[' | b'{' => {
                if !f(i, b, &open) {
                    return;
                }
                open.push(match b {
                    b'(' => Open::Paren,
                    b'[' => Open::Bracket,
                    _ => Open::Brace,
                });
            }
            b')' if open.last() == Some(&Open::Interpolation) => {
                open.pop();
                in_string = true;
            }
            b')' | b']' | b'}' => {
                open.pop();
                if !f(i, b, &open) {
                    return;
                }
            }
            _ => {
                if !f(i, b, &open) {
                    return;
                }
            }
        }
        i += 1;
    }
}

/// `code` with each `$__loc__` replaced by its location.
pub fn locations(code: &str) -> Cow<'_, str> {
    if !code.contains(LOC) {
        return Cow::Borrowed(code);
    }
    let bytes = code.as_bytes();
    let word = |c: Option<&u8>| c.is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_');
    let mut found = Vec::new();
    walk(code, |i, b, open| {
        if b == b'$' && code[i..].starts_with(LOC) && !word(bytes.get(i + LOC.len())) {
            let before = code[..i].trim_end().as_bytes().last().copied();
            let after = code[i + LOC.len()..]
                .trim_start()
                .as_bytes()
                .first()
                .copied();
            let shorthand = open.last() == Some(&Open::Brace)
                && matches!(before, Some(b'{' | b','))
                && matches!(after, Some(b',' | b'}'));
            found.push((i, shorthand));
        }
        true
    });
    let mut out = String::with_capacity(code.len());
    let mut copied = 0;
    for (i, shorthand) in found {
        out.push_str(&code[copied..i]);
        let line = 1 + code[..i].matches('\n').count();
        let loc = format!("{{\"file\":\"<top-level>\",\"line\":{line}}}");
        if shorthand {
            out.push_str(&format!("\"__loc__\": {loc}"));
        } else {
            out.push_str(&loc);
        }
        copied = i + LOC.len();
    }
    out.push_str(&code[copied..]);
    Cow::Owned(out)
}

/// Where `code` nests deeper than [`MAX_DEPTH`], as a line and column, if
/// it does: the brackets open at a point, and each one's operators so far.
pub fn too_deep(code: &str) -> Option<(usize, usize)> {
    // The operators in each level open, the outermost first.
    let mut chained: Vec<usize> = vec![0];
    let mut total = 0;
    let mut at = None;
    walk(code, |i, b, open| {
        match b {
            b'(' | b'[' | b'{' => {
                chained.push(0);
                total += 1;
            }
            b')' | b']' | b'}' => {
                if chained.len() > open.len() + 1 {
                    total -= 1 + chained.pop().unwrap_or(0);
                }
            }
            b'|' | b',' | b'+' | b'-' | b'*' | b'/' | b'%' | b'<' | b'>' | b'=' | b'!' => {
                if let Some(last) = chained.last_mut() {
                    *last += 1;
                    total += 1;
                }
            }
            _ => {}
        }
        if total > MAX_DEPTH {
            at = Some(i);
            return false;
        }
        true
    });
    at.map(|i| {
        let before = &code[..i];
        let line = 1 + before.matches('\n').count();
        let column = 1 + before
            .rfind('\n')
            .map_or(before.len(), |n| before.len() - n - 1);
        (line, column)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const L1: &str = r#"{"file":"<top-level>","line":1}"#;

    #[test]
    fn where_it_is_written() {
        assert_eq!(locations("$__loc__"), L1);
        assert_eq!(
            locations("1 |\n# $__loc__\n$__loc__"),
            "1 |\n# $__loc__\n{\"file\":\"<top-level>\",\"line\":3}"
        );
        assert_eq!(locations("[1, $__loc__]"), format!("[1, {L1}]"));
        assert_eq!(
            locations("{ a, $__loc__, c }"),
            format!("{{ a, \"__loc__\": {L1}, c }}")
        );
        assert_eq!(locations("{$__loc__}"), format!("{{\"__loc__\": {L1}}}"));
        assert_eq!(
            locations(r#""$__loc__ \($__loc__.line)""#),
            format!(r#""$__loc__ \({L1}.line)""#)
        );
        assert_eq!(locations("$__loc__x"), "$__loc__x");
        assert_eq!(locations(".a"), ".a");
    }

    #[test]
    fn too_deep_counts_brackets_and_chains() {
        assert_eq!(too_deep(&"[".repeat(MAX_DEPTH)), None);
        assert_eq!(
            too_deep(&"[".repeat(MAX_DEPTH + 1)),
            Some((1, MAX_DEPTH + 1))
        );
        assert_eq!(too_deep(&"1+".repeat(MAX_DEPTH)), None);
        assert!(too_deep(&"1+".repeat(MAX_DEPTH + 1)).is_some());
        let mixed = format!("{}{}", "(".repeat(5_000), ".|".repeat(5_001));
        assert!(too_deep(&mixed).is_some());
        // Closed brackets and their operators no longer count.
        let wide = format!("[{}]", "1,".repeat(9_000)).repeat(50);
        assert_eq!(too_deep(&wide), None);
        // Text in strings and comments is not code.
        assert_eq!(
            too_deep(&format!("\"{}\"", "[".repeat(MAX_DEPTH + 1))),
            None
        );
        assert_eq!(
            too_deep(&format!("# {}\n.", "[".repeat(MAX_DEPTH + 1))),
            None
        );
    }
}
