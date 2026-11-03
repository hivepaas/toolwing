//! jq syntax jaq does not have, rewritten before a program is parsed.
//!
//! `$__loc__` is where it is written: `{"file":"<top-level>","line":N}`, as
//! jq 1.8.2 gives it (with `-f` too). It is a token of its own, so it is
//! replaced where it stands, outside strings and comments but inside a
//! string's `\(...)`; in an object, `{$__loc__}` is short for
//! `{"__loc__": $__loc__}`, as for any variable.

use std::borrow::Cow;

const LOC: &str = "$__loc__";

/// `code` with each `$__loc__` replaced by its location.
pub fn locations(code: &str) -> Cow<'_, str> {
    if !code.contains(LOC) {
        return Cow::Borrowed(code);
    }
    #[derive(PartialEq)]
    enum Open {
        Paren,
        Bracket,
        Brace,
        /// `\(` in a string: its `)` goes back into the string.
        Interpolation,
    }
    let bytes = code.as_bytes();
    let mut out = String::with_capacity(code.len());
    let mut open: Vec<Open> = Vec::new();
    let (mut in_string, mut line, mut i) = (false, 1, 0);
    let mut copied = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\n' {
            line += 1;
        }
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
            b'(' => open.push(Open::Paren),
            b'[' => open.push(Open::Bracket),
            b'{' => open.push(Open::Brace),
            b')' => {
                if open.pop() == Some(Open::Interpolation) {
                    in_string = true;
                }
            }
            b']' | b'}' => {
                open.pop();
            }
            b'$' if code[i..].starts_with(LOC) => {
                let end = i + LOC.len();
                let word =
                    |c: Option<&u8>| c.is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_');
                if !word(bytes.get(end)) {
                    let before = code[..i].trim_end().as_bytes().last().copied();
                    let after = code[end..].trim_start().as_bytes().first().copied();
                    let shorthand = open.last() == Some(&Open::Brace)
                        && matches!(before, Some(b'{' | b','))
                        && matches!(after, Some(b',' | b'}'));
                    out.push_str(&code[copied..i]);
                    let loc = format!("{{\"file\":\"<top-level>\",\"line\":{line}}}");
                    if shorthand {
                        out.push_str(&format!("\"__loc__\": {loc}"));
                    } else {
                        out.push_str(&loc);
                    }
                    copied = end;
                    i = end;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    out.push_str(&code[copied..]);
    Cow::Owned(out)
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
}
