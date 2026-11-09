//! The app's sort: lines sorted as `sort` sorts them, reversed, by number,
//! without case, or unique.

use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, InputKind, Meta, Output, Param, ParamKind, Reference, Tool,
    Value, Values, en,
};

use super::{Key, NONE, Sorter, lines};

static META: Meta = Meta {
    id: "sort",
    group: "sort",
    action: "",
    action_flag: None,
    title: en!("Sort lines"),
    summary: en!("Lines sorted as sort sorts them: by number, reversed, without case, unique"),
    category: Category::Text,
    keywords: &[
        "sort",
        "order",
        "lines",
        "alphabetical",
        "numeric",
        "unique",
    ],
    input: InputKind::Bytes,
    streams: false,
    reference: Some(Reference {
        name: "sort",
        version: "9.7",
        compat: Compat::DropIn,
    }),
};

const fn flag(
    key: &'static str,
    label: &'static str,
    help: &'static str,
    long: &'static str,
    short: char,
) -> Param {
    Param {
        key,
        label,
        help,
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long,
            short: Some(short),
        },
    }
}

static PARAMS: &[Param] = &[
    flag(
        "numeric",
        en!("Numbers"),
        en!("Compare the numbers the lines start with"),
        "numeric-sort",
        'n',
    ),
    flag(
        "reverse",
        en!("Reverse"),
        en!("Largest first"),
        "reverse",
        'r',
    ),
    flag(
        "ignore_case",
        en!("Ignore case"),
        en!("Compare ASCII letters without their case"),
        "ignore-case",
        'f',
    ),
    flag(
        "unique",
        en!("Unique"),
        en!("Each line once"),
        "unique",
        'u',
    ),
];

pub struct SortTool;

impl Tool for SortTool {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let f = |k| values.flag(PARAMS, k);
        // The global options as one key, as sort makes them.
        let key = Key {
            sword: NONE,
            numeric: f("numeric"),
            translate: f("ignore_case"),
            reverse: f("reverse"),
            ..Key::default()
        };
        let keys = if key.default_compare() {
            Vec::new()
        } else {
            vec![key]
        };
        let s = Sorter {
            keys,
            eol: b'\n',
            reverse: f("reverse"),
            unique: f("unique"),
            ..Sorter::default()
        };
        let mut l = lines(input, b'\n');
        s.sort(&mut l);
        let l = s.unique_lines(l);
        let mut out = Vec::new();
        for (i, line) in l.iter().enumerate() {
            if i > 0 {
                out.push(b'\n');
            }
            out.extend_from_slice(line);
        }
        Ok(match String::from_utf8(out) {
            Ok(s) => Output::Text(s),
            Err(e) => Output::Bytes(e.into_bytes()),
        })
    }

    fn command_args(&self, values: &Values) -> Option<Vec<String>> {
        let mut args = vec!["sort".to_owned()];
        for (k, flag) in [
            ("numeric", "-n"),
            ("reverse", "-r"),
            ("ignore_case", "-f"),
            ("unique", "-u"),
        ] {
            if values.flag(PARAMS, k) {
                args.push(flag.to_owned());
            }
        }
        Some(args)
    }
}
