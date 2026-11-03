//! `json`: format, minify and validate JSON, written by `toolwing-jq`'s
//! writer, so the output is what `jq .`, `jq -c .` and `jq -S .` give, byte
//! for byte: numbers as jq writes them, keys in their order. Several values
//! one after another (JSON Lines) are each written.

use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, Field, InputKind, Meta, Output, Param, ParamKind, Reference,
    Span, Tool, Value, Values, en,
};
use toolwing_jq::Format;
use toolwing_jq::doc::{self, Style};

const REFERENCE: Option<Reference> = Some(Reference {
    name: "jq",
    version: "1.8.2",
    compat: Compat::Output,
});

const KEYWORDS: &[&str] = &[
    "json", "format", "pretty", "prettify", "beautify", "minify", "validate", "lint",
];

pub struct Pretty;
pub struct Minify;
pub struct Validate;

static FORMAT_META: Meta = Meta {
    id: "json.format",
    group: "json",
    action: "format",
    action_flag: None,
    title: en!("Format JSON"),
    summary: en!("Indent JSON, as jq . writes it"),
    category: Category::Data,
    keywords: KEYWORDS,
    input: InputKind::Text {
        trim_newline: false,
    },
    streams: false,
    reference: REFERENCE,
};

static MINIFY_META: Meta = Meta {
    id: "json.minify",
    group: "json",
    action: "minify",
    action_flag: None,
    title: en!("Minify JSON"),
    summary: en!("Each JSON value on one line, as jq -c . writes it"),
    category: Category::Data,
    keywords: KEYWORDS,
    input: InputKind::Text {
        trim_newline: false,
    },
    streams: false,
    reference: REFERENCE,
};

static VALIDATE_META: Meta = Meta {
    id: "json.validate",
    group: "json",
    action: "validate",
    action_flag: None,
    title: en!("Validate JSON"),
    summary: en!("Check that the input is JSON, and say where it is not"),
    category: Category::Data,
    keywords: KEYWORDS,
    input: InputKind::Text {
        trim_newline: false,
    },
    streams: false,
    reference: REFERENCE,
};

const SORT_KEYS: Param = Param {
    key: "sort_keys",
    label: en!("Sort keys"),
    help: en!("Write the keys of objects in order, as jq -S does"),
    kind: ParamKind::Flag,
    default: Value::Bool(false),
    cli: Cli {
        long: "sort-keys",
        short: Some('S'),
    },
};

const ASCII: Param = Param {
    key: "ascii",
    label: en!("Escape non-ASCII"),
    help: en!("Write characters outside ASCII as \\uXXXX, as jq -a does"),
    kind: ParamKind::Flag,
    default: Value::Bool(false),
    cli: Cli {
        long: "ascii",
        short: Some('a'),
    },
};

static FORMAT_PARAMS: &[Param] = &[
    Param {
        key: "indent",
        label: en!("Indent"),
        help: en!("Indent nested values by 2 or 4 spaces, or a tab"),
        kind: ParamKind::Choice {
            options: &[("2", "2"), ("4", "4"), ("tab", "Tab")],
            flags: false,
            unset: None,
        },
        default: Value::Text(String::new()),
        cli: Cli {
            long: "indent",
            short: None,
        },
    },
    SORT_KEYS,
    ASCII,
];

static MINIFY_PARAMS: &[Param] = &[SORT_KEYS, ASCII];

/// Every JSON value in `input`, or where it stops being JSON.
fn values(input: &[u8]) -> Result<Vec<toolwing_jq::Val>, Error> {
    doc::parse(Format::Json, input).map_err(|e| {
        let error = Error::input(e.message);
        match e.offset {
            Some(at) => error.at(Span::at(input, at.min(input.len()))),
            None => error,
        }
    })
}

/// Every value of `input` written in `style`, on a large stack: values may
/// nest 10,000 deep.
fn write(input: &[u8], style: &Style) -> Result<Output, Error> {
    crate::on_large_stack(|| write_values(input, style))
}

fn write_values(input: &[u8], style: &Style) -> Result<Output, Error> {
    let mut out = String::new();
    for v in values(input)? {
        out.push_str(&doc::write_one(Format::Json, &v, style).map_err(Error::input)?);
    }
    // The CLI ends text output with a newline itself.
    out.pop();
    Ok(Output::Text(out))
}

impl Tool for Pretty {
    fn meta(&self) -> &'static Meta {
        &FORMAT_META
    }

    fn params(&self) -> &'static [Param] {
        FORMAT_PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let indent = match values.text(FORMAT_PARAMS, "indent").as_str() {
            "4" => "    ",
            "tab" => "\t",
            _ => "  ",
        };
        let style = Style {
            indent: Some(indent.to_owned()),
            sort_keys: values.flag(FORMAT_PARAMS, "sort_keys"),
            ascii: values.flag(FORMAT_PARAMS, "ascii"),
        };
        write(input, &style)
    }
}

impl Tool for Minify {
    fn meta(&self) -> &'static Meta {
        &MINIFY_META
    }

    fn params(&self) -> &'static [Param] {
        MINIFY_PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let style = Style {
            indent: None,
            sort_keys: values.flag(MINIFY_PARAMS, "sort_keys"),
            ascii: values.flag(MINIFY_PARAMS, "ascii"),
        };
        write(input, &style)
    }
}

impl Tool for Validate {
    fn meta(&self) -> &'static Meta {
        &VALIDATE_META
    }

    /// Fails, with where, when the input is not JSON; RFC 8259 asks for one
    /// value at least, so empty input is not.
    fn run(&self, _ctx: &Ctx, input: &[u8], _values: &Values) -> Result<Output, Error> {
        crate::on_large_stack(|| validate(input))
    }
}

fn validate(input: &[u8]) -> Result<Output, Error> {
    let vals = values(input)?;
    if vals.is_empty() {
        return Err(Error::input("no JSON value: the input is empty"));
    }
    let mut kinds: Vec<&str> = Vec::new();
    for v in &vals {
        let kind = kind(v);
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    Ok(Output::Fields(vec![
        Field::new("valid", en!("Valid"), "yes"),
        Field::new("values", en!("Values"), vals.len().to_string()),
        Field::new("types", en!("Types"), kinds.join(", ")),
    ]))
}

/// A value's type, as jq's `type` names it.
fn kind(v: &toolwing_jq::Val) -> &'static str {
    use toolwing_jq::Val;
    match v {
        Val::Null => "null",
        Val::Bool(_) => "boolean",
        Val::Num(_) => "number",
        Val::TStr(_) | Val::BStr(_) => "string",
        Val::Arr(_) => "array",
        Val::Obj(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(tool: &dyn Tool, input: &str, set: &[(&str, Value)]) -> Result<String, Error> {
        let mut values = Values::new();
        for (k, v) in set {
            values.set(k, v.clone());
        }
        match tool.run(&Ctx::new(), input.as_bytes(), &values)? {
            Output::Text(s) => Ok(s),
            Output::Fields(f) => Ok(f
                .iter()
                .map(|f| format!("{}={}", f.key, f.value))
                .collect::<Vec<_>>()
                .join(" ")),
            _ => Ok(String::new()),
        }
    }

    /// Each as jq 1.8.2 writes it (`jq .`, `jq -c .`, `jq -S --tab .`).
    #[test]
    fn as_jq_writes_it() {
        let input = r#"{"b": 1.0, "a": [1e3, 10 / 1, "é"]}"#.replace("10 / 1", "10");
        assert_eq!(
            run(&Pretty, &input, &[]).ok().as_deref(),
            Some("{\n  \"b\": 1.0,\n  \"a\": [\n    1E+3,\n    10,\n    \"é\"\n  ]\n}")
        );
        assert_eq!(
            run(&Minify, &input, &[]).ok().as_deref(),
            Some(r#"{"b":1.0,"a":[1E+3,10,"é"]}"#)
        );
        let sorted_tab = [
            ("indent", Value::Text("tab".into())),
            ("sort_keys", Value::Bool(true)),
        ];
        assert_eq!(
            run(&Pretty, &input, &sorted_tab).ok().as_deref(),
            Some("{\n\t\"a\": [\n\t\t1E+3,\n\t\t10,\n\t\t\"é\"\n\t],\n\t\"b\": 1.0\n}")
        );
        assert_eq!(
            run(&Minify, "1 [2]\n{\"c\":3}", &[]).ok().as_deref(),
            Some("1\n[2]\n{\"c\":3}")
        );
    }

    #[test]
    fn errors_say_where() {
        let e = run(&Pretty, "{\"a\": 1,\n}", &[]).err();
        let span = e.as_ref().and_then(|e| e.span);
        assert_eq!(span.map(|s| (s.line, s.column)), Some((2, 1)));
        assert_eq!(e.map(|e| e.message), Some("value expected".to_owned()));
    }

    #[test]
    fn validate_counts_values() {
        assert_eq!(
            run(&Validate, "{} [] 1", &[]).ok().as_deref(),
            Some("valid=yes values=3 types=object, array, number")
        );
        assert!(run(&Validate, "  ", &[]).is_err());
        assert!(run(&Validate, "[1,", &[]).is_err());
    }
}
