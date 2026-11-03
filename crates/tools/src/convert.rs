//! `convert`: between JSON, YAML 1.2, TOML, CSV and XML, read and written by
//! jaq's formats (`toolwing-jq`), so JSON comes out as `tw jq` writes it.
//!
//! What each format holds is kept: YAML's documents are values of their own
//! (`---` between them in YAML out); TOML and XML hold one. CSV's first row
//! is its header: rows become objects with those keys, and objects become
//! rows under the keys they have, in order of first appearance; strings are
//! quoted, as jq's `@csv` quotes them. XML maps to
//! JSON as xmltodict maps it: an element is an object, its attributes keys
//! with `@`, its text `#text` (or the element is that text when it has
//! nothing else), and repeated elements an array; comments and declarations
//! are left out.

use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, InputKind, Meta, Output, Param, ParamKind, Reference, Span,
    Tool, Value, Values, en,
};
use toolwing_jq::doc::{self, Style};
use toolwing_jq::json::Map;
use toolwing_jq::{Format, Val};

pub struct Convert;

static META: Meta = Meta {
    id: "convert",
    group: "convert",
    action: "",
    action_flag: None,
    title: en!("Convert"),
    summary: en!("Between JSON, YAML 1.2, TOML, CSV and XML"),
    category: Category::Data,
    keywords: &[
        "convert",
        "json",
        "yaml",
        "yml",
        "toml",
        "csv",
        "xml",
        "transform",
    ],
    input: InputKind::Text {
        trim_newline: false,
    },
    streams: false,
    reference: Some(Reference {
        // What reads and writes them, to these specifications.
        name: "jaq-fmts (YAML 1.2, TOML 1.0, RFC 4180, XML 1.0)",
        version: "0.1.1",
        compat: Compat::Standard,
    }),
};

const FORMATS: &[(&str, &str)] = &[
    ("json", "JSON"),
    ("yaml", "YAML"),
    ("toml", "TOML"),
    ("csv", "CSV"),
    ("xml", "XML"),
];

static PARAMS: &[Param] = &[
    Param {
        key: "from",
        label: en!("From"),
        help: en!("The input's format; found from the input when left out"),
        kind: ParamKind::Choice {
            options: FORMATS,
            flags: false,
            unset: Some(en!("Detect")),
        },
        default: Value::Text(String::new()),
        cli: Cli {
            long: "from",
            short: Some('f'),
        },
    },
    Param {
        key: "to",
        label: en!("To"),
        help: en!("The output's format"),
        kind: ParamKind::Choice {
            options: FORMATS,
            flags: false,
            unset: None,
        },
        default: Value::Text(String::new()),
        cli: Cli {
            long: "to",
            short: Some('t'),
        },
    },
    Param {
        key: "indent",
        label: en!("Indent"),
        help: en!("Indent JSON and YAML by this many spaces; 0 writes JSON on one line"),
        kind: ParamKind::Int { min: 0, max: 8 },
        default: Value::Int(2),
        cli: Cli {
            long: "indent",
            short: None,
        },
    },
];

fn format(name: &str) -> Option<Format> {
    Some(match name {
        "json" => Format::Json,
        "yaml" => Format::Yaml,
        "toml" => Format::Toml,
        "csv" => Format::Csv,
        "xml" => Format::Xml,
        _ => return None,
    })
}

fn name(f: Format) -> &'static str {
    match f {
        Format::Json => "JSON",
        Format::Yaml => "YAML",
        Format::Toml => "TOML",
        Format::Csv => "CSV",
        Format::Xml => "XML",
        _ => "that format",
    }
}

/// The format `input` is most likely in: JSON when it parses as JSON, XML
/// when it starts with `<`, TOML when it parses as TOML and starts with a
/// `key =` or `[table]`, CSV when its first lines have the same number of
/// commas and no `key: value`, else YAML (of which JSON is a part).
pub fn detect(input: &[u8]) -> Format {
    let text = String::from_utf8_lossy(input);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    if text.starts_with('<') {
        return Format::Xml;
    }
    if !text.is_empty() && doc::parse(Format::Json, text.as_bytes()).is_ok() {
        return Format::Json;
    }
    let first = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("");
    let tomlish =
        first.starts_with('[') || first.split_once('=').is_some_and(|(k, _)| !k.contains(':'));
    if tomlish && doc::parse(Format::Toml, text.as_bytes()).is_ok() {
        return Format::Toml;
    }
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(5)
        .collect();
    let commas = |l: &str| l.matches(',').count();
    if let Some(head) = lines.first() {
        let csvish = commas(head) > 0
            && !head.contains(": ")
            && lines.iter().all(|l| commas(l) == commas(head));
        if csvish {
            return Format::Csv;
        }
    }
    Format::Yaml
}

impl Tool for Convert {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let from = format(&values.text(PARAMS, "from")).unwrap_or_else(|| detect(input));
        let to = format(&values.text(PARAMS, "to")).unwrap_or(Format::Json);
        let indent = usize::try_from(values.int(PARAMS, "indent")).unwrap_or(2);
        let parsed = doc::parse(from, input).map_err(|e| {
            let error = Error::input(format!("not {}: {}", name(from), e.message));
            match e.offset {
                Some(at) => error.at(Span::at(input, at.min(input.len()))),
                None => error,
            }
        })?;
        let parsed = match from {
            Format::Csv => vec![from_rows(parsed)],
            Format::Xml => parsed.iter().filter_map(from_xml).collect(),
            _ => parsed,
        };
        let style = Style {
            indent: (indent > 0 || matches!(to, Format::Yaml)).then(|| " ".repeat(indent.max(1))),
            ..Style::default()
        };
        let one = |what: &str| -> Result<&Val, Error> {
            match parsed.as_slice() {
                [v] => Ok(v),
                vs => Err(Error::input(format!(
                    "{} documents: {} holds one{what}",
                    vs.len(),
                    name(to)
                ))),
            }
        };
        let fail = |e: String| Error::input(format!("not written as {}: {e}", name(to)));
        let text = match to {
            Format::Csv => to_rows(one("")?)?
                .iter()
                .map(|row| doc::write_one(Format::Csv, row, &style).map_err(fail))
                .collect::<Result<String, _>>()?,
            Format::Xml => {
                doc::write_one(Format::Xml, &to_xml(one(", one root element")?)?, &style)
                    .map_err(fail)?
            }
            Format::Toml => doc::write_one(Format::Toml, one("")?, &style).map_err(fail)?,
            Format::Yaml => parsed
                .iter()
                .map(|v| doc::write_one(Format::Yaml, v, &style).map_err(fail))
                .collect::<Result<Vec<_>, _>>()?
                .join("---\n"),
            _ => parsed
                .iter()
                .map(|v| doc::write_one(Format::Json, v, &style).map_err(fail))
                .collect::<Result<String, _>>()?,
        };
        Ok(Output::Text(
            text.strip_suffix('\n').unwrap_or(&text).to_owned(),
        ))
    }
}

fn key(k: &str) -> Val {
    Val::from(k.to_owned())
}

fn text(v: &Val) -> Option<String> {
    match v {
        Val::TStr(s) | Val::BStr(s) => Some(String::from_utf8_lossy(s).into_owned()),
        _ => None,
    }
}

/// CSV's rows as objects keyed by its first row.
fn from_rows(rows: Vec<Val>) -> Val {
    let mut rows = rows.into_iter();
    let header: Vec<Val> = match rows.next() {
        Some(Val::Arr(h)) => h
            .iter()
            .map(|k| text(k).map_or_else(|| k.clone(), Val::from))
            .collect(),
        _ => return Val::Arr(Default::default()),
    };
    rows.map(|row| {
        let cells: Vec<Val> = match row {
            Val::Arr(a) => a.iter().cloned().collect(),
            v => vec![v],
        };
        let map: Map = header
            .iter()
            .cloned()
            .zip(cells.into_iter().chain(std::iter::repeat(Val::Null)))
            .collect();
        Val::obj(map)
    })
    .collect()
}

/// Rows for CSV: an array of arrays as it is, an array of objects under a
/// header of their keys. A nested value is written as its JSON.
fn to_rows(v: &Val) -> Result<Vec<Val>, Error> {
    let Val::Arr(items) = v else {
        return Err(Error::input("CSV needs an array of rows or of objects"));
    };
    let cell = |v: &Val| match v {
        Val::Arr(_) | Val::Obj(_) => Val::from(v.to_string()),
        v => v.clone(),
    };
    if items.iter().all(|i| matches!(i, Val::Arr(_))) {
        return Ok(items
            .iter()
            .map(|row| match row {
                Val::Arr(a) => a.iter().map(cell).collect(),
                v => v.clone(),
            })
            .collect());
    }
    let mut keys: Vec<Val> = Vec::new();
    for item in items.iter() {
        let Val::Obj(o) = item else {
            return Err(Error::input(
                "CSV needs an array of rows or of objects, not a mix",
            ));
        };
        for k in o.keys() {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
    }
    let mut rows = vec![keys.iter().cloned().collect::<Val>()];
    for item in items.iter() {
        if let Val::Obj(o) = item {
            rows.push(
                keys.iter()
                    .map(|k| o.get(k).map_or(Val::Null, cell))
                    .collect(),
            );
        }
    }
    Ok(rows)
}

/// jaq's XML value (`{"t": tag, "a": attributes, "c": content}`) as
/// xmltodict maps it; `None` for what is not an element.
fn from_xml(v: &Val) -> Option<Val> {
    let Val::Obj(o) = v else { return None };
    let tag = o.get(&key("t")).and_then(text)?;
    Some(Val::obj([(key(&tag), element(o))].into_iter().collect()))
}

fn element(o: &Map) -> Val {
    let mut out: Vec<(String, Vec<Val>)> = Vec::new();
    if let Some(Val::Obj(attrs)) = o.get(&key("a")) {
        for (k, v) in attrs.iter() {
            out.push((format!("@{}", text(k).unwrap_or_default()), vec![v.clone()]));
        }
    }
    let content: Vec<Val> = match o.get(&key("c")) {
        Some(Val::Arr(c)) => c.iter().cloned().collect(),
        Some(v) => vec![v.clone()],
        None => Vec::new(),
    };
    let mut body = String::new();
    for c in &content {
        match c {
            Val::Obj(child) => {
                if let Some(tag) = child.get(&key("t")).and_then(text) {
                    let v = element(child);
                    match out.iter_mut().find(|(k, _)| *k == tag) {
                        Some((_, vs)) => vs.push(v),
                        None => out.push((tag, vec![v])),
                    }
                } else if let Some(t) = child.get(&key("cdata")).and_then(text) {
                    body.push_str(&t);
                }
            }
            v => body.push_str(&text(v).unwrap_or_else(|| v.to_string())),
        }
    }
    let body = body.trim();
    if out.is_empty() {
        return if body.is_empty() {
            Val::Null
        } else {
            Val::from(body.to_owned())
        };
    }
    if !body.is_empty() {
        out.push(("#text".to_owned(), vec![Val::from(body.to_owned())]));
    }
    let map: Map = out
        .into_iter()
        .map(|(k, mut vs)| {
            let v = if vs.len() == 1 {
                vs.remove(0)
            } else {
                vs.into_iter().collect()
            };
            (key(&k), v)
        })
        .collect();
    Val::obj(map)
}

/// The other way: an object of one key, the root element, as jaq's XML value.
fn to_xml(v: &Val) -> Result<Val, Error> {
    let one_root = || Error::input("XML needs one root element: an object with one key");
    let Val::Obj(o) = v else {
        return Err(one_root());
    };
    let mut entries = o.iter();
    let (Some((tag, v)), None) = (entries.next(), entries.next()) else {
        return Err(one_root());
    };
    let tag = text(tag).unwrap_or_default();
    match to_elements(&tag, v)?.as_slice() {
        [root] => Ok(root.clone()),
        _ => Err(one_root()),
    }
}

/// `v` as elements named `tag`: several for an array.
fn to_elements(tag: &str, v: &Val) -> Result<Vec<Val>, Error> {
    if let Val::Arr(items) = v {
        let mut out = Vec::new();
        for item in items.iter() {
            out.extend(to_elements(tag, item)?);
        }
        return Ok(out);
    }
    let mut attrs = Map::default();
    let mut content: Vec<Val> = Vec::new();
    match v {
        Val::Obj(o) => {
            for (k, v) in o.iter() {
                let k = text(k).unwrap_or_default();
                let scalar = || text(v).unwrap_or_else(|| v.to_string());
                if let Some(attr) = k.strip_prefix('@') {
                    attrs.insert(key(attr), Val::from(scalar()));
                } else if k == "#text" {
                    content.push(Val::from(scalar()));
                } else {
                    content.extend(to_elements(&k, v)?);
                }
            }
        }
        Val::Null => {}
        v => content.push(Val::from(text(v).unwrap_or_else(|| v.to_string()))),
    }
    let mut element: Map = [(key("t"), key(tag))].into_iter().collect();
    if !attrs.is_empty() {
        element.insert(key("a"), Val::obj(attrs));
    }
    if !content.is_empty() {
        element.insert(key("c"), content.into_iter().collect());
    }
    Ok(vec![Val::obj(element)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(input: &str, from: &str, to: &str) -> Result<String, String> {
        let mut values = Values::new();
        values.set("from", Value::Text(from.into()));
        values.set("to", Value::Text(to.into()));
        match Convert.run(&Ctx::new(), input.as_bytes(), &values) {
            Ok(Output::Text(s)) => Ok(s),
            Ok(_) => Err("not text".into()),
            Err(e) => Err(e.message),
        }
    }

    #[test]
    fn detects_the_format() {
        assert!(matches!(detect(br#"{"a": 1}"#), Format::Json));
        assert!(matches!(detect(b"<a>1</a>"), Format::Xml));
        assert!(matches!(detect(b"[server]\nport = 80\n"), Format::Toml));
        assert!(matches!(detect(b"name = \"x\"\n"), Format::Toml));
        assert!(matches!(detect(b"a,b\n1,2\n"), Format::Csv));
        assert!(matches!(
            detect(b"apiVersion: v1\nkind: ConfigMap\n"),
            Format::Yaml
        ));
    }

    /// The sketch's ConfigMap: YAML 1.2 keeps `no` and `"1.10"` as strings.
    #[test]
    fn yaml_to_json() {
        let yaml = "apiVersion: v1\nkind: ConfigMap\ndata:\n  ENABLE_V2: \"no\"\n  VERSION: \"1.10\"\n  RAW: no\n";
        assert_eq!(
            convert(yaml, "", "json"),
            Ok("{\n  \"apiVersion\": \"v1\",\n  \"kind\": \"ConfigMap\",\n  \"data\": {\n    \"ENABLE_V2\": \"no\",\n    \"VERSION\": \"1.10\",\n    \"RAW\": \"no\"\n  }\n}".into())
        );
        assert_eq!(
            convert("a: 1\n---\nb: 2\n", "yaml", "json"),
            Ok("{\n  \"a\": 1\n}\n{\n  \"b\": 2\n}".into())
        );
    }

    #[test]
    fn json_to_yaml_and_toml() {
        assert_eq!(
            convert(r#"{"a":1,"b":"no"}"#, "json", "yaml"),
            Ok("a: 1\nb: \"no\"".into())
        );
        assert_eq!(
            convert(r#"{"a":1,"b":{"c":"x"}}"#, "json", "toml"),
            Ok("a = 1\n\n[b]\nc = \"x\"".into())
        );
        assert!(convert("1 2", "json", "toml").is_err_and(|e| e.contains("2 documents")));
    }

    #[test]
    fn csv_has_a_header() {
        let csv = "name,port\nweb,80\n\"a,b\",8080\n";
        assert_eq!(
            convert(csv, "csv", "json"),
            Ok("[\n  {\n    \"name\": \"web\",\n    \"port\": 80\n  },\n  {\n    \"name\": \"a,b\",\n    \"port\": 8080\n  }\n]".into())
        );
        let json = r#"[{"name":"web","port":80},{"name":"a,b","tags":["x"]}]"#;
        assert_eq!(
            convert(json, "json", "csv"),
            Ok("\"name\",\"port\",\"tags\"\n\"web\",80,\n\"a,b\",,\"[\"\"x\"\"]\"".into())
        );
    }

    #[test]
    fn xml_as_xmltodict() {
        let xml = r#"<?xml version="1.0"?><cfg env="prod"><name>billing</name><port>80</port><port>90</port><note lang="en">hi</note></cfg>"#;
        let json = convert(xml, "", "json").map(|s| s.replace([' ', '\n'], ""));
        assert_eq!(
            json,
            Ok(r##"{"cfg":{"@env":"prod","name":"billing","port":["80","90"],"note":{"@lang":"en","#text":"hi"}}}"##.into())
        );
        let back = convert(&json.unwrap_or_default(), "json", "xml");
        assert_eq!(
            back,
            Ok(r#"<cfg env="prod"><name>billing</name><port>80</port><port>90</port><note lang="en">hi</note></cfg>"#.into())
        );
        assert!(convert(r#"{"a":1,"b":2}"#, "json", "xml").is_err());
    }

    #[test]
    fn errors_say_where() {
        let mut values = Values::new();
        values.set("from", Value::Text("json".into()));
        let e = Convert.run(&Ctx::new(), b"{\"a\": [1,\n}", &values).err();
        assert_eq!(e.as_ref().and_then(|e| e.span).map(|s| s.line), Some(2));
    }
}
