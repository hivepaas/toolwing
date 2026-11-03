//! toolwing: jq's way of naming values in error messages, as jq 1.8.2's
//! `jv_kind_name` and `jv_dump_string_trunc` have it.

use crate::{Error, Val};
use alloc::format;
use alloc::string::{String, ToString};

/// A value's kind, as jq names it (and as `type` gives it).
pub fn kind(v: &Val) -> &'static str {
    match v {
        Val::Null => "null",
        Val::Bool(_) => "boolean",
        Val::Num(_) => "number",
        Val::TStr(_) | Val::BStr(_) => "string",
        Val::Arr(_) => "array",
        Val::Obj(_) => "object",
    }
}

/// A value as jq shows it in an error (30 bytes): its JSON, cut to 25 or 26
/// bytes and `...` (and the closing `"`, `]` or `}`) when longer than 29. A
/// string is cut to 30 characters first.
pub fn dump(v: &Val) -> String {
    const BUF: usize = 30;
    let json = match v {
        Val::TStr(s) | Val::BStr(s) if s.len() > BUF => {
            let s = String::from_utf8_lossy(s);
            Val::from(s.chars().take(BUF).collect::<String>()).to_string()
        }
        v => v.to_string(),
    };
    if json.len() < BUF {
        return json;
    }
    let delim = match json.as_bytes()[0] {
        b'"' => "\"",
        b'[' => "]",
        b'{' => "}",
        _ => "",
    };
    let mut end = BUF - if delim.is_empty() { 4 } else { 5 };
    while !json.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...{delim}", &json[..end])
}

/// A value with its kind: `string ("abc")`, as jq's `type_error` shows it.
pub fn typed(v: &Val) -> String {
    format!("{} ({})", kind(v), dump(v))
}

/// `Cannot index object with number (0)`.
pub fn index(v: &Val, i: &Val) -> Error {
    Error::str(format!("Cannot index {} with {}", kind(v), typed(i)))
}

/// `Cannot iterate over number (123)`.
pub fn iterate(v: &Val) -> Error {
    Error::str(format!("Cannot iterate over {}", typed(v)))
}

/// `string ("a") and number (1) cannot be added`.
pub fn binary(l: &Val, r: &Val, what: &str) -> Error {
    Error::str(format!("{} and {} {what}", typed(l), typed(r)))
}

/// `string ("foo") cannot be negated`.
pub fn unary(v: &Val, what: &str) -> Error {
    Error::str(format!("{} {what}", typed(v)))
}
