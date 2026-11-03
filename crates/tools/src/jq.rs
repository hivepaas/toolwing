//! `jq`: in the registry for its place in `tw list` and the app's sidebar.
//! Its command line is `tw jq`'s own (toolwing-cli's jq.rs), as jq reads
//! its arguments, and its page in the app runs it in a worker process
//! (architecture.md); neither goes through this tool's params.

use toolwing_core::{
    Category, Compat, Ctx, Error, InputKind, Meta, Output, Reference, Tool, Values, en,
};
use toolwing_jq::Format;
use toolwing_jq::doc::{self, Style};

pub struct Jq;

static META: Meta = Meta {
    id: "jq",
    group: "jq",
    action: "",
    action_flag: None,
    title: en!("jq"),
    summary: en!("Query and transform JSON, YAML, TOML, XML and CSV, as jq 1.8 does"),
    category: Category::Data,
    keywords: &[
        "jq",
        "json",
        "query",
        "filter",
        "select",
        "transform",
        "jaq",
    ],
    input: InputKind::Text {
        trim_newline: false,
    },
    streams: false,
    reference: Some(Reference {
        name: "jq",
        version: "1.8.2",
        compat: Compat::DropIn,
    }),
};

impl Tool for Jq {
    fn meta(&self) -> &'static Meta {
        &META
    }

    /// The filter `.`: the input as `jq .` writes it.
    fn run(&self, _ctx: &Ctx, input: &[u8], _values: &Values) -> Result<Output, Error> {
        let style = Style {
            indent: Some("  ".into()),
            ..Style::default()
        };
        let mut out = String::new();
        for v in doc::parse(Format::Json, input).map_err(|e| Error::input(e.message))? {
            out.push_str(&doc::write_one(Format::Json, &v, &style).map_err(Error::input)?);
        }
        out.pop();
        Ok(Output::Text(out))
    }
}
