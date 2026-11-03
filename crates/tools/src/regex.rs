//! `regex match` and `regex replace`: a pattern's matches and groups in a
//! text, or the text with them replaced.
//!
//! Two engines: Rust's `regex` by default, which runs in time linear in the
//! text whatever the pattern; and with `--pcre`, `fancy-regex`, which adds
//! look-around and backreferences by backtracking, stopped after a million
//! steps so a pattern cannot hang the run.

use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, ErrorKind, Field, InputKind, Meta, Output, Param, ParamKind,
    Reference, Span, Tool, Value, Values, en,
};

pub struct Match;
pub struct Replace;

const REFERENCE: Option<Reference> = Some(Reference {
    name: "Rust regex syntax",
    version: "1.13",
    compat: Compat::Standard,
});

/// More matches than this are not listed.
const MATCHES_MAX: usize = 10_000;
/// Backtracking steps `--pcre` may take before it gives up.
const BACKTRACK_LIMIT: usize = 1_000_000;

const fn meta(
    action: &'static str,
    id: &'static str,
    title: &'static str,
    summary: &'static str,
) -> Meta {
    Meta {
        id,
        group: "regex",
        action,
        action_flag: None,
        title,
        summary,
        category: Category::Text,
        keywords: &[
            "regex", "regexp", "pattern", "grep", "match", "replace", "sed",
        ],
        // The text is matched as it is, its last newline included.
        input: InputKind::Text {
            trim_newline: false,
        },
        streams: false,
        reference: REFERENCE,
    }
}

static MATCH_META: Meta = meta(
    "match",
    "regex.match",
    en!("Regex match"),
    en!("A pattern's matches in a text, with their groups and where they are"),
);
static REPLACE_META: Meta = meta(
    "replace",
    "regex.replace",
    en!("Regex replace"),
    en!("Replace a pattern's matches: $1 or ${name} for a group"),
);

const PATTERN: Param = Param {
    key: "pattern",
    label: en!("Pattern"),
    help: en!("The regular expression"),
    kind: ParamKind::Text,
    default: Value::Text(String::new()),
    cli: Cli {
        long: "regexp",
        short: Some('e'),
    },
};

const fn flag(
    key: &'static str,
    long: &'static str,
    label: &'static str,
    help: &'static str,
) -> Param {
    Param {
        key,
        label,
        help,
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli { long, short: None },
    }
}

const IGNORE_CASE: Param = flag(
    "ignore_case",
    "ignore-case",
    en!("Ignore case"),
    en!("Match upper and lower case alike (i)"),
);
const MULTI_LINE: Param = flag(
    "multi_line",
    "multi-line",
    en!("Multi-line"),
    en!("^ and $ match at the start and end of each line (m)"),
);
const DOT_ALL: Param = flag(
    "dot_all",
    "dot-all",
    en!("Dot matches newline"),
    en!(". matches a newline too (s)"),
);
const EXTENDED: Param = flag(
    "extended",
    "extended",
    en!("Extended"),
    en!("White space in the pattern is ignored and # starts a comment (x)"),
);
const PCRE: Param = flag(
    "pcre",
    "pcre",
    en!("PCRE-style"),
    en!("Allow look-around and backreferences, by backtracking (fancy-regex)"),
);

static MATCH_PARAMS: &[Param] = &[
    PATTERN,
    IGNORE_CASE,
    MULTI_LINE,
    DOT_ALL,
    EXTENDED,
    PCRE,
    flag(
        "first",
        "first",
        en!("First only"),
        en!("Stop at the first match"),
    ),
];

static REPLACE_PARAMS: &[Param] = &[
    PATTERN,
    Param {
        key: "with",
        label: en!("Replace with"),
        help: en!("The replacement: $1 or ${name} for a group, $$ for a dollar sign"),
        kind: ParamKind::Text,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "with",
            short: Some('r'),
        },
    },
    IGNORE_CASE,
    MULTI_LINE,
    DOT_ALL,
    EXTENDED,
    PCRE,
    flag(
        "first",
        "first",
        en!("First only"),
        en!("Replace the first match only"),
    ),
];

enum Engine {
    Rust(regex::Regex),
    Fancy(fancy_regex::Regex),
}

/// One match: where it is, and where each group is (`None`: did not take part).
struct Found {
    start: usize,
    end: usize,
    groups: Vec<Option<(usize, usize)>>,
}

impl Engine {
    fn build(params: &[Param], values: &Values) -> Result<Engine, Error> {
        let pattern = values.text(params, "pattern");
        if pattern.is_empty() {
            return Err(Error::usage("give the pattern: -e PATTERN"));
        }
        let on = |k| values.flag(params, k);
        let bad = |e: String| Error::usage(format!("the pattern is not valid: {e}"));
        if on("pcre") {
            fancy_regex::RegexBuilder::new(&pattern)
                .case_insensitive(on("ignore_case"))
                .multi_line(on("multi_line"))
                .dot_matches_new_line(on("dot_all"))
                .ignore_whitespace(on("extended"))
                .backtrack_limit(BACKTRACK_LIMIT)
                .build()
                .map(Engine::Fancy)
                .map_err(|e| bad(e.to_string()))
        } else {
            regex::RegexBuilder::new(&pattern)
                .case_insensitive(on("ignore_case"))
                .multi_line(on("multi_line"))
                .dot_matches_new_line(on("dot_all"))
                .ignore_whitespace(on("extended"))
                .build()
                .map(Engine::Rust)
                .map_err(|e| {
                    let message = e.to_string();
                    // The last line names the problem; the others draw it.
                    bad(message.lines().last().unwrap_or(&message).trim().to_owned())
                })
        }
    }

    /// The groups' names, by index (0, the whole match, has none).
    fn names(&self) -> Vec<Option<String>> {
        match self {
            Engine::Rust(r) => r.capture_names().map(|n| n.map(str::to_owned)).collect(),
            Engine::Fancy(r) => r.capture_names().map(|n| n.map(str::to_owned)).collect(),
        }
    }

    fn find(&self, ctx: &Ctx, text: &str, first: bool) -> Result<Vec<Found>, Error> {
        let limit = if first { 1 } else { MATCHES_MAX };
        let mut found = Vec::new();
        match self {
            Engine::Rust(r) => {
                for caps in r.captures_iter(text).take(limit) {
                    ctx.check()?;
                    let whole = caps.get(0).map_or((0, 0), |m| (m.start(), m.end()));
                    found.push(Found {
                        start: whole.0,
                        end: whole.1,
                        groups: (1..caps.len())
                            .map(|i| caps.get(i).map(|m| (m.start(), m.end())))
                            .collect(),
                    });
                }
            }
            Engine::Fancy(r) => {
                for caps in r.captures_iter(text).take(limit) {
                    ctx.check()?;
                    let caps = caps.map_err(backtracked)?;
                    let whole = caps.get(0).map_or((0, 0), |m| (m.start(), m.end()));
                    found.push(Found {
                        start: whole.0,
                        end: whole.1,
                        groups: (1..caps.len())
                            .map(|i| caps.get(i).map(|m| (m.start(), m.end())))
                            .collect(),
                    });
                }
            }
        }
        Ok(found)
    }

    fn replace(&self, text: &str, with: &str, first: bool) -> Result<String, Error> {
        Ok(match (self, first) {
            (Engine::Rust(r), true) => r.replace(text, with).into_owned(),
            (Engine::Rust(r), false) => r.replace_all(text, with).into_owned(),
            (Engine::Fancy(r), true) => r
                .try_replacen(text, 1, with)
                .map_err(backtracked)?
                .into_owned(),
            (Engine::Fancy(r), false) => r
                .try_replacen(text, 0, with)
                .map_err(backtracked)?
                .into_owned(),
        })
    }
}

fn backtracked(e: fancy_regex::Error) -> Error {
    Error::input(format!(
        "the pattern took too long on this text ({e}); without --pcre it runs in linear time"
    ))
}

fn text_of(input: &[u8]) -> Result<&str, Error> {
    std::str::from_utf8(input)
        .map_err(|e| Error::input("input is not UTF-8").at(Span::at(input, e.valid_up_to())))
}

impl Tool for Match {
    fn meta(&self) -> &'static Meta {
        &MATCH_META
    }

    fn params(&self) -> &'static [Param] {
        MATCH_PARAMS
    }

    fn run(&self, ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let text = text_of(input)?;
        let engine = Engine::build(MATCH_PARAMS, values)?;
        let names = engine.names();
        let found = engine.find(ctx, text, values.flag(MATCH_PARAMS, "first"))?;
        if found.is_empty() {
            return Err(Error::new(ErrorKind::Check, "no match"));
        }
        let mut fields = Vec::new();
        let mut json = Vec::new();
        for (n, f) in found.iter().enumerate() {
            let at = Span::at(text.as_bytes(), f.start);
            let section = format!("Match {} · line {}, column {}", n + 1, at.line, at.column);
            let whole = &text[f.start..f.end];
            fields
                .push(Field::new(format!("{}.match", n + 1), "match", whole).in_section(&section));
            let mut groups = serde_json::Map::new();
            for (i, g) in f.groups.iter().enumerate() {
                let index = i + 1;
                let name = names.get(index).cloned().flatten();
                let label = name.clone().unwrap_or_else(|| format!("${index}"));
                let key = name.unwrap_or_else(|| index.to_string());
                let value = g.map(|(s, e)| &text[s..e]);
                fields.push(
                    Field::new(
                        format!("{}.{key}", n + 1),
                        label,
                        value.unwrap_or("(did not match)"),
                    )
                    .in_section(&section),
                );
                groups.insert(
                    key,
                    value.map_or(serde_json::Value::Null, serde_json::Value::from),
                );
            }
            json.push(serde_json::json!({
                "match": whole,
                "start": f.start,
                "end": f.end,
                "line": at.line,
                "column": at.column,
                "groups": groups,
            }));
        }
        if found.len() == MATCHES_MAX {
            fields.push(Field::new(
                "more",
                "…",
                format!("the first {MATCHES_MAX} matches only"),
            ));
        }
        Ok(Output::Record {
            fields,
            json: serde_json::Value::from(json).to_string(),
        })
    }
}

impl Tool for Replace {
    fn meta(&self) -> &'static Meta {
        &REPLACE_META
    }

    fn params(&self) -> &'static [Param] {
        REPLACE_PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let text = text_of(input)?;
        let engine = Engine::build(REPLACE_PARAMS, values)?;
        let with = values.text(REPLACE_PARAMS, "with");
        let replaced = engine.replace(text, &with, values.flag(REPLACE_PARAMS, "first"))?;
        Ok(Output::Text(replaced))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "INFO  2026-11-03 deploy started\nWARN  disk at 91%\nERROR 2026-11-04 rollback, see #4521\n";
    const DATE: &str = r"(?<year>\d{4})-(?<month>\d{2})-(\d{2})";

    fn values(set: &[(&str, Value)]) -> Values {
        let mut v = Values::new();
        for (k, x) in set {
            v.set(k, x.clone());
        }
        v
    }

    fn text(s: &str) -> Value {
        Value::Text(s.into())
    }

    fn matches(
        input: &str,
        set: &[(&str, Value)],
    ) -> Result<(Vec<Field>, serde_json::Value), Error> {
        match Match.run(&Ctx::new(), input.as_bytes(), &values(set))? {
            Output::Record { fields, json } => {
                Ok((fields, serde_json::from_str(&json).unwrap_or_default()))
            }
            other => panic!("{other:?}"),
        }
    }

    fn replace(input: &str, set: &[(&str, Value)]) -> Result<String, Error> {
        match Replace.run(&Ctx::new(), input.as_bytes(), &values(set))? {
            Output::Text(s) => Ok(s),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn every_match_with_its_groups_and_place() {
        let (fields, json) =
            matches(LOG, &[("pattern", text(DATE))]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(json.as_array().map(Vec::len), Some(2));
        assert_eq!(json[1]["match"], "2026-11-04");
        assert_eq!(json[1]["line"], 3);
        assert_eq!(json[1]["column"], 7);
        assert_eq!(json[1]["groups"]["year"], "2026");
        assert_eq!(json[1]["groups"]["3"], "04");
        assert_eq!(
            fields[0].section.as_deref(),
            Some("Match 1 · line 1, column 7")
        );
        let labels: Vec<&str> = fields.iter().take(4).map(|f| f.label.as_str()).collect();
        assert_eq!(labels, ["match", "year", "month", "$3"]);
    }

    #[test]
    fn flags_change_what_matches() {
        assert!(matches("Hello", &[("pattern", text("hello"))]).is_err());
        assert!(
            matches(
                "Hello",
                &[
                    ("pattern", text("hello")),
                    ("ignore_case", Value::Bool(true))
                ]
            )
            .is_ok()
        );
        let starts = |set: &[(&str, Value)]| {
            matches("a\nb", set).map(|(_, j)| j.as_array().map_or(0, Vec::len))
        };
        assert_eq!(starts(&[("pattern", text("^\\w"))]).ok(), Some(1));
        assert_eq!(
            starts(&[("pattern", text("^\\w")), ("multi_line", Value::Bool(true))]).ok(),
            Some(2)
        );
        assert!(matches("a\nb", &[("pattern", text("a.b"))]).is_err());
        assert!(
            matches(
                "a\nb",
                &[("pattern", text("a.b")), ("dot_all", Value::Bool(true))]
            )
            .is_ok()
        );
        let x = [
            ("pattern", text("a b # spaced out")),
            ("extended", Value::Bool(true)),
        ];
        assert!(matches("ab", &x).is_ok());
        let first = matches(
            LOG,
            &[("pattern", text(DATE)), ("first", Value::Bool(true))],
        );
        assert_eq!(
            first.map(|(_, j)| j.as_array().map_or(0, Vec::len)).ok(),
            Some(1)
        );
    }

    #[test]
    fn no_match_is_a_failed_check_and_a_bad_pattern_says_why() {
        let e = matches(LOG, &[("pattern", text("nope"))])
            .err()
            .map(|e| e.kind);
        assert_eq!(e, Some(ErrorKind::Check));
        let e = matches(LOG, &[("pattern", text("(unclosed"))])
            .err()
            .map(|e| (e.kind, e.message));
        assert_eq!(e.as_ref().map(|e| e.0), Some(ErrorKind::Usage));
        assert!(e.is_some_and(|e| e.1.starts_with("the pattern is not valid")));
        assert!(matches(LOG, &[]).is_err());
    }

    #[test]
    fn pcre_style_has_look_around_and_backreferences() {
        let look = [("pattern", text("foo(?=bar)"))];
        assert!(
            matches("foobar", &look).is_err(),
            "Rust regex has no look-around"
        );
        let mut pcre = look.to_vec();
        pcre.push(("pcre", Value::Bool(true)));
        assert!(matches("foobar", &pcre).is_ok());
        let twice = [("pattern", text(r"(\w+) \1")), ("pcre", Value::Bool(true))];
        assert_eq!(
            matches("say hello hello", &twice)
                .map(|(_, j)| j[0]["match"].clone())
                .ok(),
            Some("hello hello".into())
        );
        // A pattern with nothing fancy runs on the linear engine even with
        // --pcre: no match, at once.
        let nested = [("pattern", text("(a+)+$")), ("pcre", Value::Bool(true))];
        let input = format!("{}b", "a".repeat(40));
        assert_eq!(
            matches(&input, &nested).err().map(|e| e.kind),
            Some(ErrorKind::Check)
        );
        // A backreference needs backtracking: stopped rather than hung on.
        let bomb = [
            ("pattern", text(r"(a|aa)+\1$")),
            ("pcre", Value::Bool(true)),
        ];
        let e = matches(&input, &bomb).err().map(|e| e.message);
        assert!(
            e.as_deref().is_some_and(|m| m.contains("took too long")),
            "{e:?}"
        );
    }

    #[test]
    fn replaces_with_groups() {
        let set = [("pattern", text(DATE)), ("with", text("$3/${month}/$year"))];
        assert_eq!(
            replace("on 2026-11-03 and 2026-11-04", &set)
                .ok()
                .as_deref(),
            Some("on 03/11/2026 and 04/11/2026")
        );
        let mut first = set.to_vec();
        first.push(("first", Value::Bool(true)));
        assert_eq!(
            replace("2026-11-03 2026-11-04", &first).ok().as_deref(),
            Some("03/11/2026 2026-11-04")
        );
        let pcre = [
            ("pattern", text(r"(\w+) \1")),
            ("with", text("$1")),
            ("pcre", Value::Bool(true)),
        ];
        assert_eq!(
            replace("the the cat", &pcre).ok().as_deref(),
            Some("the cat")
        );
    }
}
