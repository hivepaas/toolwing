//! Documents in and out, for the tools that format and convert them (`json`,
//! `convert`): read every value of an input in one of jaq's formats, and write
//! values as `tw jq` writes them, so a tool formats as `tw jq .` does.

use jaq_all::fmts::{Format, read, write};
use jaq_all::json::Val;
use jaq_all::json::write::Pp;

/// How deeply values may nest, as jq limits it: deeper input is refused
/// before it is parsed, as the parser would overflow its stack.
pub const MAX_DEPTH: usize = 10_000;

/// The stack values are read and written on, as the app's jq worker and
/// `tw jq` have: reading and writing are recursive, and 10,000 levels take
/// more than a thread's 2 MB (the app's background threads have that).
/// Reserved, not used, until a deep value needs it.
pub const STACK: usize = 256 << 20;

/// Runs `f` on a thread with [`STACK`], for work that reads or writes
/// nested values; a panic in it goes on in the caller.
pub fn on_large_stack<T: Send>(f: impl FnOnce() -> T + Send) -> std::io::Result<T> {
    std::thread::scope(|scope| {
        let thread = std::thread::Builder::new()
            .stack_size(STACK)
            .spawn_scoped(scope, f)?;
        Ok(thread
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic)))
    })
}

/// Why an input does not parse, and where when the format says.
#[derive(Debug, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub offset: Option<usize>,
}

/// Every value in `bytes`, read as `format`: several JSON values one after
/// another, YAML's documents, one TOML table, CSV's rows (each an array).
pub fn parse(format: Format, bytes: &[u8]) -> Result<Vec<Val>, ParseError> {
    let deep = match format {
        Format::Xml => too_deep_xml(bytes),
        _ => too_deep(bytes),
    };
    if let Some(offset) = deep {
        let message = format!("nested more than {MAX_DEPTH} levels deep");
        return Err(ParseError {
            message,
            offset: Some(offset),
        });
    }
    if matches!(format, Format::Json) {
        return read::json::parse_many(bytes)
            .map(|r| {
                r.map_err(|e| ParseError {
                    message: e.message(),
                    offset: Some(e.offset()),
                })
            })
            .collect();
    }
    let fail = |e: std::io::Error| ParseError {
        message: e.to_string(),
        offset: None,
    };
    let bytes = bytes::Bytes::copy_from_slice(bytes);
    let text = read::bytes_str(format, &bytes).map_err(fail)?;
    read::parse(format, &bytes, text, false)
        .map(|r| r.map_err(fail))
        .collect()
}

/// How values are written.
#[derive(Clone, Debug, Default)]
pub struct Style {
    /// The indent of nested values; `None` writes each value on one line.
    pub indent: Option<String>,
    pub sort_keys: bool,
    /// Escape every character outside ASCII as `\uXXXX`, as `jq -a` does.
    pub ascii: bool,
}

/// `value` written as `format` (JSON, YAML, TOML, CSV, XML), ending with a
/// newline.
pub fn write_one(format: Format, value: &Val, style: &Style) -> Result<String, String> {
    let writer = write::Writer {
        format,
        pp: Pp {
            indent: style.indent.clone(),
            sort_keys: style.sort_keys,
            styles: Default::default(),
            sep_space: style.indent.is_some() || matches!(format, Format::Yaml),
        },
        // No YAML `---` and `...` around each document: `convert` writes the
        // separators between them.
        join: matches!(format, Format::Yaml),
    };
    let mut out = Vec::new();
    write::write(&mut out, &writer, value).map_err(|e| e.to_string())?;
    let mut s = String::from_utf8_lossy(&out).into_owned();
    if !s.ends_with('\n') {
        s.push('\n');
    }
    Ok(if style.ascii { escape_ascii(&s) } else { s })
}

/// Every character outside ASCII as JSON's `\uXXXX` (two for those past
/// U+FFFF). In JSON they can only be in strings, where this is what `jq -a`
/// writes.
pub fn escape_ascii(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut units = [0u16; 2];
            for u in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{u:04x}"));
            }
        }
    }
    out
}

/// Where `bytes` nests deeper than [`MAX_DEPTH`] in brackets or braces
/// outside JSON strings, if it does. Counted for every format: none nests
/// that deep in use.
fn too_deep(bytes: &[u8]) -> Option<usize> {
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Some(i);
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

/// Where `bytes` nests XML elements deeper than [`MAX_DEPTH`], if it does:
/// a start tag goes in, an end tag out; self-closing tags, comments, CDATA,
/// declarations and processing instructions neither.
fn too_deep_xml(bytes: &[u8]) -> Option<usize> {
    let find = |from: usize, what: &[u8]| {
        bytes[from..]
            .windows(what.len())
            .position(|w| w == what)
            .map(|p| from + p + what.len())
    };
    let (mut depth, mut i) = (0usize, 0);
    while let Some(lt) = bytes[i..].iter().position(|&b| b == b'<').map(|p| i + p) {
        let rest = &bytes[lt + 1..];
        let end = if rest.starts_with(b"!--") {
            find(lt, b"-->")
        } else if rest.starts_with(b"![CDATA[") {
            find(lt, b"]]>")
        } else if rest.starts_with(b"?") || rest.starts_with(b"!") {
            find(lt, b">")
        } else {
            // To the tag's `>`, past quoted attribute values.
            let mut quote = None;
            let close = bytes[lt..].iter().enumerate().find(|&(_, &b)| match quote {
                Some(q) if b == q => {
                    quote = None;
                    false
                }
                Some(_) => false,
                None if b == b'"' || b == b'\'' => {
                    quote = Some(b);
                    false
                }
                None => b == b'>',
            });
            let gt = close.map(|(p, _)| lt + p)?;
            if rest.starts_with(b"/") {
                depth = depth.saturating_sub(1);
            } else if bytes[gt - 1] != b'/' {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Some(lt);
                }
            }
            Some(gt + 1)
        };
        i = end?;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pretty() -> Style {
        Style {
            indent: Some("  ".into()),
            ..Style::default()
        }
    }

    #[test]
    fn json_written_as_jq_writes_it() {
        let vals = parse(Format::Json, br#"{"b":1.0,"a":[1e3,10]} 2"#).unwrap_or_default();
        assert_eq!(vals.len(), 2);
        let compact = Style::default();
        assert_eq!(
            write_one(Format::Json, &vals[0], &compact),
            Ok("{\"b\":1.0,\"a\":[1E+3,10]}\n".into())
        );
        let sorted = Style {
            sort_keys: true,
            ..pretty()
        };
        assert_eq!(
            write_one(Format::Json, &vals[0], &sorted),
            Ok("{\n  \"a\": [\n    1E+3,\n    10\n  ],\n  \"b\": 1.0\n}\n".into())
        );
    }

    #[test]
    fn errors_say_where() {
        let e = parse(Format::Json, b"{\"a\":1,}").err();
        assert_eq!(e.map(|e| e.offset), Some(Some(7)));
        let xml = format!(
            "<?xml version=\"1.0\"?><!-- <a> -->{}",
            "<a x='>'>".repeat(MAX_DEPTH + 1)
        );
        assert!(parse(Format::Xml, xml.as_bytes()).is_err_and(|e| e.message.contains("deep")));
        let fine = format!(
            "{}<b/>{}",
            "<a>".repeat(MAX_DEPTH),
            "</a>".repeat(MAX_DEPTH)
        );
        assert!(too_deep_xml(fine.as_bytes()).is_none());
        let deep = "[".repeat(MAX_DEPTH + 1);
        assert_eq!(
            parse(Format::Json, deep.as_bytes())
                .err()
                .and_then(|e| e.offset),
            Some(MAX_DEPTH)
        );
    }

    #[test]
    fn ascii_as_jq_a() {
        assert_eq!(
            escape_ascii("\"café ☕ 😀\""),
            r#""caf\u00e9 \u2615 \ud83d\ude00""#
        );
    }
}
