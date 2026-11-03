//! `uuid`: generate UUIDs (RFC 9562).

use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, InputKind, Meta, Output, Param, ParamKind, Reference, Score,
    Tool, Value, Values, en,
};

pub struct Generate;

static META: Meta = Meta {
    id: "uuid",
    group: "uuid",
    action: "",
    action_flag: None,
    title: en!("UUID"),
    summary: en!("Generate UUIDs: random (v4) or time-ordered (v7)"),
    category: Category::Data,
    keywords: &["uuid", "guid", "uuidgen", "v4", "v7", "id"],
    input: InputKind::None,
    streams: false,
    reference: Some(Reference {
        name: "RFC 9562",
        version: "2024",
        compat: Compat::Standard,
    }),
};

static PARAMS: &[Param] = &[
    Param {
        key: "version",
        label: en!("Version"),
        help: en!("v4 is random; v7 starts with the time, so it sorts by creation"),
        kind: ParamKind::Choice {
            options: &[
                ("v4", en!("Random (the default)")),
                (
                    "v7",
                    en!("Time-ordered: starts with the time, so it sorts by creation"),
                ),
            ],
            flags: true,
            unset: None,
        },
        default: Value::Text(String::new()),
        cli: Cli {
            long: "version",
            short: None,
        },
    },
    Param {
        key: "count",
        label: en!("How many"),
        help: en!("How many UUIDs to generate"),
        kind: ParamKind::Int {
            min: 1,
            max: 10_000,
        },
        default: Value::Int(1),
        cli: Cli {
            long: "count",
            short: Some('n'),
        },
    },
    Param {
        key: "upper",
        label: en!("Uppercase"),
        help: en!("Write the hex digits in uppercase"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "upper",
            short: Some('u'),
        },
    },
];

impl Tool for Generate {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn run(&self, _ctx: &Ctx, _input: &[u8], values: &Values) -> Result<Output, Error> {
        let count = values.int(PARAMS, "count");
        if !(1..=10_000).contains(&count) {
            return Err(Error::usage("count must be from 1 to 10000"));
        }
        let v7 = values.text(PARAMS, "version") == "v7";
        let upper = values.flag(PARAMS, "upper");
        let lines = (0..count)
            .map(|_| {
                let id = if v7 {
                    uuid::Uuid::now_v7()
                } else {
                    uuid::Uuid::new_v4()
                };
                let s = id.hyphenated().to_string();
                if upper { s.to_uppercase() } else { s }
            })
            .collect();
        Ok(Output::Lines(lines))
    }

    fn detect(&self, sample: &[u8]) -> Option<Score> {
        let s = std::str::from_utf8(sample).ok()?.trim();
        uuid::Uuid::try_parse(s).ok().map(|_| Score(90))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(version: &str, count: i64) -> Vec<String> {
        let mut v = Values::new();
        v.set("version", Value::Text(version.into()));
        v.set("count", Value::Int(count));
        match Generate.run(&Ctx::new(), b"", &v) {
            Ok(Output::Lines(lines)) => lines,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn generates_the_asked_version() {
        for (version, digit) in [("v4", '4'), ("v7", '7'), ("", '4')] {
            for id in run(version, 3) {
                assert_eq!(id.len(), 36);
                assert_eq!(id.chars().nth(14), Some(digit), "{id}");
            }
        }
    }

    #[test]
    fn v7_sorts_by_creation() {
        let ids = run("v7", 50);
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn refuses_counts_out_of_range() {
        let mut v = Values::new();
        v.set("count", Value::Int(0));
        assert!(Generate.run(&Ctx::new(), b"", &v).is_err());
    }
}
