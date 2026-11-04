//! `html`: escape text for HTML, and unescape its character references as
//! a browser does (the WHATWG HTML standard's tokenizer: the 2,231 named
//! references, those of them that may go without a `;`, and numeric ones,
//! with its replacements for the C1 range and code points no text may
//! hold). Tested with html5lib-tests, the suite browsers' parsers share.

use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, InputKind, Meta, Output, Param, ParamKind, Reference, Score,
    Tool, Value, Values, en,
};

static REFERENCE: Option<Reference> = Some(Reference {
    name: "WHATWG HTML",
    version: "Living Standard",
    compat: Compat::Standard,
});

const fn meta(
    action: &'static str,
    id: &'static str,
    title: &'static str,
    summary: &'static str,
) -> Meta {
    Meta {
        id,
        group: "html",
        action,
        action_flag: None,
        title,
        summary,
        category: Category::Web,
        keywords: &[
            "html", "entity", "entities", "escape", "unescape", "xss", "amp", "nbsp",
        ],
        input: InputKind::Text { trim_newline: true },
        streams: false,
        reference: REFERENCE,
    }
}

static ESCAPE_META: Meta = meta(
    "escape",
    "html.escape",
    en!("HTML escape"),
    en!("Text made safe for HTML: & < > \" ' as character references"),
);
static UNESCAPE_META: Meta = meta(
    "unescape",
    "html.unescape",
    en!("HTML unescape"),
    en!("HTML's character references (&amp; &eacute; &#233;) as the text they stand for"),
);

static ESCAPE_PARAMS: &[Param] = &[
    Param {
        key: "no_quotes",
        label: en!("Leave quotes"),
        help: en!("Leave \" and ' as they are: for text outside attributes"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "no-quotes",
            short: None,
        },
    },
    Param {
        key: "ascii",
        label: en!("ASCII only"),
        help: en!(
            "Write characters past ASCII as numeric references too (é as &#xe9;), but for C1 controls, which none stands for"
        ),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "ascii",
            short: None,
        },
    },
];

static UNESCAPE_PARAMS: &[Param] = &[Param {
    key: "attribute",
    label: en!("In an attribute"),
    help: en!(
        "Read as an attribute's value is: a reference without ';' before a letter, digit or = stays"
    ),
    kind: ParamKind::Flag,
    default: Value::Bool(false),
    cli: Cli {
        long: "attribute",
        short: None,
    },
}];

pub struct Escape;
pub struct Unescape;

/// `&`, `<`, `>`, and unless `no_quotes`, `"` and `'` (as `&#39;`, which
/// HTML 4 readers know too, where `&apos;` is HTML 5's).
pub fn escape(text: &str, no_quotes: bool, ascii: bool) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if !no_quotes => out.push_str("&quot;"),
            '\'' if !no_quotes => out.push_str("&#39;"),
            // A reference to a C1 control (U+0080 to U+009F) reads as
            // windows-1252's character there (&#x85; is …): none stands for
            // them, so they stay as they are.
            c if ascii && !c.is_ascii() && !('\u{80}'..='\u{9f}').contains(&c) => {
                out.push_str(&format!("&#x{:x};", u32::from(c)))
            }
            c => out.push(c),
        }
    }
    out
}

/// The text a browser reads from `html`, in text or in an attribute value.
pub fn unescape(html: &str, attribute: bool) -> String {
    if attribute {
        htmlize::unescape_attribute(html).into_owned()
    } else {
        htmlize::unescape(html).into_owned()
    }
}

impl Tool for Escape {
    fn meta(&self) -> &'static Meta {
        &ESCAPE_META
    }

    fn params(&self) -> &'static [Param] {
        ESCAPE_PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let text =
            std::str::from_utf8(input).map_err(|e| Error::input(format!("not UTF-8: {e}")))?;
        Ok(Output::Text(escape(
            text,
            values.flag(ESCAPE_PARAMS, "no_quotes"),
            values.flag(ESCAPE_PARAMS, "ascii"),
        )))
    }
}

impl Tool for Unescape {
    fn meta(&self) -> &'static Meta {
        &UNESCAPE_META
    }

    fn params(&self) -> &'static [Param] {
        UNESCAPE_PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let text =
            std::str::from_utf8(input).map_err(|e| Error::input(format!("not UTF-8: {e}")))?;
        Ok(Output::Text(unescape(
            text,
            values.flag(UNESCAPE_PARAMS, "attribute"),
        )))
    }

    /// Text with references in it: `&amp;`, `&#39;`, `&eacute;`.
    fn detect(&self, sample: &[u8]) -> Option<Score> {
        let s = std::str::from_utf8(sample).ok()?;
        let refs = s
            .match_indices('&')
            .filter(|(i, _)| {
                let rest = &s[i + 1..];
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '#')
                    .collect();
                !name.is_empty() && rest[name.len()..].starts_with(';')
            })
            .count();
        (refs >= 2).then_some(Score(45))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(file: &str) -> Vec<(String, String)> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/html")
            .join(file);
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        text.lines()
            .map(|l| {
                serde_json::from_str::<(String, String)>(l).unwrap_or_else(|e| panic!("{l}: {e}"))
            })
            .collect()
    }

    /// html5lib-tests' tokenizer tests (testdata/html/SOURCE): every named
    /// and numeric reference in text, and the attribute cases.
    #[test]
    fn unescapes_as_html5lib_tests_expect() {
        let mut failures = Vec::new();
        for file in ["named.jsonl", "numeric.jsonl", "mixed.jsonl"] {
            for (input, want) in pairs(file) {
                let got = unescape(&input, false);
                if got != want {
                    failures.push(format!("{file}: {input:?}: {got:?}, want {want:?}"));
                }
            }
        }
        for (input, want) in pairs("attribute.jsonl") {
            let got = unescape(&input, true);
            if got != want {
                failures.push(format!("attribute: {input:?}: {got:?}, want {want:?}"));
            }
        }
        assert!(
            failures.is_empty(),
            "{} differ:\n{}",
            failures.len(),
            failures.join("\n")
        );
        assert_eq!(pairs("named.jsonl").len(), 4210);
    }

    #[test]
    fn escapes_what_html_reads_as_markup() {
        assert_eq!(
            escape("<a href=\"x\">Tom & Jerry's</a>", false, false),
            "&lt;a href=&quot;x&quot;&gt;Tom &amp; Jerry&#39;s&lt;/a&gt;"
        );
        assert_eq!(escape("\"it's\" <b>", true, false), "\"it's\" &lt;b&gt;");
        assert_eq!(
            escape("café ☕ 😀", false, true),
            "caf&#xe9; &#x2615; &#x1f600;"
        );
        // What is escaped reads back as itself, C1 controls too.
        for s in [
            "<x y=\"1\" z='2'>&amp;</x>",
            "é ☕ &copy",
            "a&b<c>d\"e'f",
            "\u{85}\u{80}\u{9f}",
        ] {
            assert_eq!(unescape(&escape(s, false, true), false), s);
        }
    }
}
