//! `url encode` and `url decode`: percent-encoding (RFC 3986).
//!
//! Encoding keeps the unreserved characters (`A-Z a-z 0-9 - . _ ~`) as jq's
//! `@uri` does and escapes every other byte of the UTF-8. Decoding refuses a
//! `%` not followed by two hex digits and a result that is not UTF-8, as jq's
//! `@urid` and JavaScript's `decodeURIComponent` do.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, InputKind, Meta, Output, Param, ParamKind, Reference, Span,
    Tool, Value, Values, en,
};

const REFERENCE: Option<Reference> = Some(Reference {
    name: "RFC 3986",
    version: "2005",
    compat: Compat::Standard,
});

const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub struct Encode;
pub struct Decode;

static ENCODE_META: Meta = Meta {
    id: "url.encode",
    group: "url",
    action: "encode",
    action_flag: None,
    title: en!("URL encode"),
    summary: en!("Percent-encode text for a URL"),
    category: Category::Web,
    keywords: &["url", "uri", "percent", "escape", "encodeURIComponent"],
    input: InputKind::Text { trim_newline: true },
    streams: false,
    reference: REFERENCE,
};

static DECODE_META: Meta = Meta {
    id: "url.decode",
    group: "url",
    action: "decode",
    action_flag: None,
    title: en!("URL decode"),
    summary: en!("Decode percent-encoded text"),
    category: Category::Web,
    keywords: &["url", "uri", "percent", "unescape", "decodeURIComponent"],
    input: InputKind::Text { trim_newline: true },
    streams: false,
    reference: REFERENCE,
};

static PARAMS: &[Param] = &[Param {
    key: "form",
    label: en!("Form encoding"),
    help: en!("Spaces as + (application/x-www-form-urlencoded)"),
    kind: ParamKind::Flag,
    default: Value::Bool(false),
    cli: Cli {
        long: "form",
        short: None,
    },
}];

impl Tool for Encode {
    fn meta(&self) -> &'static Meta {
        &ENCODE_META
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let text = std::str::from_utf8(input).map_err(|e| not_utf8(input, e.valid_up_to()))?;
        let form = values.flag(PARAMS, "form");
        let encoded = if form {
            text.split(' ')
                .map(|part| utf8_percent_encode(part, UNRESERVED).to_string())
                .collect::<Vec<_>>()
                .join("+")
        } else {
            utf8_percent_encode(text, UNRESERVED).to_string()
        };
        Ok(Output::Text(encoded))
    }
}

impl Tool for Decode {
    fn meta(&self) -> &'static Meta {
        &DECODE_META
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let form = values.flag(PARAMS, "form");
        let mut out = Vec::with_capacity(input.len());
        let mut i = 0;
        while i < input.len() {
            match input[i] {
                b'%' => {
                    let hex = input.get(i + 1..i + 3).and_then(|h| {
                        std::str::from_utf8(h)
                            .ok()
                            .and_then(|h| u8::from_str_radix(h, 16).ok())
                    });
                    let Some(byte) = hex else {
                        return Err(
                            Error::input("% not followed by two hex digits").at(Span::at(input, i))
                        );
                    };
                    out.push(byte);
                    i += 3;
                }
                b'+' if form => {
                    out.push(b' ');
                    i += 1;
                }
                b => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        let text =
            String::from_utf8(out).map_err(|_| Error::input("the decoded bytes are not UTF-8"))?;
        Ok(Output::Text(text))
    }
}

fn not_utf8(input: &[u8], offset: usize) -> Error {
    Error::input("input is not UTF-8").at(Span::at(input, offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(tool: &dyn Tool, input: &str, form: bool) -> Result<String, Error> {
        let mut v = Values::new();
        v.set("form", Value::Bool(form));
        match tool.run(&Ctx::new(), input.as_bytes(), &v)? {
            Output::Text(s) => Ok(s),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn encodes_as_jq_uri() {
        // jq -rn '"a b&c=d/é~_.-" | @uri'
        assert_eq!(
            run(&Encode, "a b&c=d/é~_.-", false).as_deref(),
            Ok("a%20b%26c%3Dd%2F%C3%A9~_.-")
        );
        assert_eq!(run(&Encode, "a b+c", true).as_deref(), Ok("a+b%2Bc"));
    }

    #[test]
    fn decodes_and_refuses_bad_escapes() {
        assert_eq!(run(&Decode, "a%20b%C3%A9", false).as_deref(), Ok("a bé"));
        assert_eq!(run(&Decode, "a+b%2B", true).as_deref(), Ok("a b+"));
        assert_eq!(run(&Decode, "a+b", false).as_deref(), Ok("a+b"));
        let e = run(&Decode, "ab%g1", false).expect_err("a bad escape");
        assert_eq!(e.span.map(|s| s.column), Some(3));
        assert!(run(&Decode, "abc%", false).is_err());
        assert!(run(&Decode, "%C3", false).is_err());
    }
}
