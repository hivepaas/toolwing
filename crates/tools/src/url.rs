//! `url encode` and `url decode`: percent-encoding (RFC 3986); `url parse`:
//! a URL's parts, as WHATWG's URL Standard (and so a browser) reads it.
//!
//! Encoding keeps the unreserved characters (`A-Z a-z 0-9 - . _ ~`) as jq's
//! `@uri` does and escapes every other byte of the UTF-8. Decoding refuses a
//! `%` not followed by two hex digits and a result that is not UTF-8, as jq's
//! `@urid` and JavaScript's `decodeURIComponent` do.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, Field, InputKind, Meta, Output, Param, ParamKind, Reference,
    Span, Tool, Value, Values, en,
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
pub struct Parse;

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

static PARSE_META: Meta = Meta {
    id: "url.parse",
    group: "url",
    action: "parse",
    action_flag: None,
    title: en!("URL parse"),
    summary: en!("Take a URL apart: scheme, user, host, port, path, query parameters, fragment"),
    category: Category::Web,
    keywords: &["url", "uri", "parse", "query", "host", "port", "params"],
    input: InputKind::Text { trim_newline: true },
    streams: false,
    reference: Some(Reference {
        name: "WHATWG URL Standard",
        version: "2025",
        compat: Compat::Standard,
    }),
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

impl Tool for Parse {
    fn meta(&self) -> &'static Meta {
        &PARSE_META
    }

    /// The parts as fields, the query's parameters decoded in a section of
    /// their own; `--json` has them all, the port as a number (null when
    /// the scheme's default is meant) and the parameters as pairs in order.
    fn run(&self, _ctx: &Ctx, input: &[u8], _values: &Values) -> Result<Output, Error> {
        let text = std::str::from_utf8(input).map_err(|e| not_utf8(input, e.valid_up_to()))?;
        let url =
            url::Url::parse(text.trim()).map_err(|e| Error::input(format!("not a URL: {e}")))?;
        let mut fields = vec![Field::new("scheme", en!("Scheme"), url.scheme())];
        if !url.username().is_empty() {
            fields.push(Field::new("username", en!("User"), url.username()));
        }
        if let Some(p) = url.password() {
            fields.push(Field::new("password", en!("Password"), p));
        }
        if let Some(h) = url.host_str() {
            fields.push(Field::new("host", en!("Host"), h));
        }
        match (url.port(), url.port_or_known_default()) {
            (Some(p), _) => fields.push(Field::new("port", en!("Port"), p.to_string())),
            (None, Some(p)) => {
                let default = format!("{p} ({}'s default)", url.scheme());
                fields.push(Field::new("port", en!("Port"), default));
            }
            (None, None) => {}
        }
        fields.push(Field::new("path", en!("Path"), url.path()));
        if let Some(q) = url.query() {
            fields.push(Field::new("query", en!("Query"), q));
        }
        if let Some(f) = url.fragment() {
            fields.push(Field::new("fragment", en!("Fragment"), f));
        }
        let params: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        let section = match params.len() {
            1 => "Query · 1 parameter, decoded".to_owned(),
            n => format!("Query · {n} parameters, decoded"),
        };
        for (i, (k, v)) in params.iter().enumerate() {
            fields.push(
                Field::new(format!("param.{i}"), k.clone(), v.clone()).in_section(section.clone()),
            );
        }
        let json = serde_json::json!({
            "href": url.as_str(),
            "scheme": url.scheme(),
            "username": url.username(),
            "password": url.password(),
            "host": url.host_str(),
            "port": url.port(),
            "port_or_default": url.port_or_known_default(),
            "path": url.path(),
            "query": url.query(),
            "params": params.iter().map(|(k, v)| [k, v]).collect::<Vec<_>>(),
            "fragment": url.fragment(),
        });
        Ok(Output::Record {
            fields,
            json: json.to_string(),
        })
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

    /// The sketch's URL, and what WHATWG makes of an odd one.
    #[test]
    fn parses_as_a_browser_does() {
        let url = "https://admin@api.example.com:8443/v1/search?q=ti%E1%BA%BFng%20vi%E1%BB%87t&page=2&sort=-created&tag=a&tag=b#results";
        let Ok(Output::Record { fields, json }) =
            Parse.run(&Ctx::new(), url.as_bytes(), &Values::new())
        else {
            panic!("no record");
        };
        let get = |k: &str| fields.iter().find(|f| f.key == k).map(|f| f.value.as_str());
        assert_eq!(
            (get("host"), get("port"), get("username")),
            (Some("api.example.com"), Some("8443"), Some("admin"))
        );
        assert_eq!(
            (get("param.0"), get("fragment")),
            (Some("tiếng việt"), Some("results"))
        );
        assert_eq!(
            fields
                .iter()
                .filter(|f| f.section.as_deref() == Some("Query · 5 parameters, decoded"))
                .count(),
            5
        );
        let json: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
        assert_eq!(json["params"][3], serde_json::json!(["tag", "a"]));
        assert_eq!(
            (json["port"].as_u64(), json["port_or_default"].as_u64()),
            (Some(8443), Some(8443))
        );

        let odd = Parse.run(
            &Ctx::new(),
            b"HTTP://Ex%41mple.COM/a/../b c",
            &Values::new(),
        );
        let Ok(Output::Record { fields, .. }) = odd else {
            panic!("no record")
        };
        let get = |k: &str| fields.iter().find(|f| f.key == k).map(|f| f.value.clone());
        assert_eq!(get("host").as_deref(), Some("example.com"));
        assert_eq!(get("path").as_deref(), Some("/b%20c"));
        assert_eq!(get("port").as_deref(), Some("80 (http's default)"));
        assert!(
            Parse
                .run(&Ctx::new(), b"not a url", &Values::new())
                .is_err()
        );
    }
}
