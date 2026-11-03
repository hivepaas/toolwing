//! `ulid generate` and `ulid inspect`: ULIDs (the ULID spec), 128 bits like
//! a UUID, 48 of them milliseconds since 1970 and 80 random, written in 26
//! characters of Crockford's Base32 so they sort by time. Generated ones are
//! monotonic: several in one millisecond still sort in the order made.

use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, Field, InputKind, Meta, Output, Param, ParamKind, Reference,
    Score, Tool, Value, Values, en,
};

pub struct Generate;
pub struct Inspect;

const REFERENCE: Option<Reference> = Some(Reference {
    name: "ULID spec",
    version: "2016",
    compat: Compat::Standard,
});

static GENERATE_META: Meta = Meta {
    id: "ulid.generate",
    group: "ulid",
    action: "generate",
    action_flag: None,
    title: en!("ULID"),
    summary: en!("Generate ULIDs: sortable by time, 26 characters"),
    category: Category::Data,
    keywords: &["ulid", "id", "sortable", "uuid", "generate"],
    input: InputKind::None,
    streams: false,
    reference: REFERENCE,
};

static INSPECT_META: Meta = Meta {
    id: "ulid.inspect",
    group: "ulid",
    action: "inspect",
    action_flag: None,
    title: en!("ULID inspect"),
    summary: en!(
        "A ULID's time and random part, and the same 128 bits as a UUID (or a UUID as a ULID)"
    ),
    category: Category::Data,
    keywords: &["ulid", "inspect", "decode", "timestamp", "uuid"],
    input: InputKind::Text { trim_newline: true },
    streams: false,
    reference: REFERENCE,
};

static GENERATE_PARAMS: &[Param] = &[
    Param {
        key: "count",
        label: en!("How many"),
        help: en!("How many ULIDs to generate"),
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
        key: "lower",
        label: en!("Lowercase"),
        help: en!("Write the letters in lowercase"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "lower",
            short: None,
        },
    },
];

impl Tool for Generate {
    fn meta(&self) -> &'static Meta {
        &GENERATE_META
    }

    fn params(&self) -> &'static [Param] {
        GENERATE_PARAMS
    }

    fn run(&self, _ctx: &Ctx, _input: &[u8], values: &Values) -> Result<Output, Error> {
        let lower = values.flag(GENERATE_PARAMS, "lower");
        let mut generator = ulid::Generator::new();
        let mut lines = Vec::new();
        for _ in 0..values.int(GENERATE_PARAMS, "count") {
            let id = generator
                .generate()
                .map_err(|e| Error::usage(format!("cannot make a ULID: {e}")))?
                .to_string();
            lines.push(if lower { id.to_lowercase() } else { id });
        }
        Ok(Output::Lines(lines))
    }
}

impl Tool for Inspect {
    fn meta(&self) -> &'static Meta {
        &INSPECT_META
    }

    /// A ULID, in either case, or a UUID in any of its forms.
    fn run(&self, _ctx: &Ctx, input: &[u8], _values: &Values) -> Result<Output, Error> {
        let text = String::from_utf8_lossy(input);
        let text = text.trim();
        let bits = match ulid::Ulid::from_string(text) {
            Ok(id) => id.0,
            Err(e) => match uuid::Uuid::try_parse(text) {
                Ok(id) => id.as_u128(),
                Err(_) => return Err(Error::input(format!("not a ULID ({e}) or a UUID"))),
            },
        };
        let id = ulid::Ulid(bits);
        let ms = id.timestamp_ms();
        let at = i64::try_from(ms)
            .ok()
            .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
            .map(|t| format!("{t:.3}"))
            .unwrap_or_default();
        Ok(Output::Fields(vec![
            Field::new("ulid", en!("ULID"), id.to_string()),
            Field::new(
                "uuid",
                en!("UUID"),
                uuid::Uuid::from_u128(bits).hyphenated().to_string(),
            ),
            Field::new("created", en!("Created"), at),
            Field::new("unix_ms", en!("Unix ms"), ms.to_string()),
            Field::new(
                "random",
                en!("Random part"),
                format!("{:020x}", id.random()),
            ),
        ]))
    }

    fn detect(&self, sample: &[u8]) -> Option<Score> {
        let s = std::str::from_utf8(sample).ok()?.trim();
        (s.len() == 26 && ulid::Ulid::from_string(s).is_ok()).then_some(Score(80))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate(count: i64) -> Vec<String> {
        let mut v = Values::new();
        v.set("count", Value::Int(count));
        match Generate.run(&Ctx::new(), b"", &v) {
            Ok(Output::Lines(l)) => l,
            other => panic!("{other:?}"),
        }
    }

    fn inspect(text: &str) -> Vec<(String, String)> {
        match Inspect.run(&Ctx::new(), text.as_bytes(), &Values::new()) {
            Ok(Output::Fields(f)) => f.into_iter().map(|f| (f.key, f.value)).collect(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn generated_ones_sort_in_the_order_made() {
        let ids = generate(500);
        assert!(ids.iter().all(|id| id.len() == 26));
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }

    /// The spec's example, 01ARZ3NDEKTSV4RRFFQ69G5FAV, and its UUID back.
    #[test]
    fn inspects_a_ulid_and_a_uuid() {
        let f = inspect("01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert_eq!(f[2], ("created".into(), "2016-07-30T23:54:10.259Z".into()));
        assert_eq!(f[3], ("unix_ms".into(), "1469922850259".into()));
        let uuid = f[1].1.clone();
        assert_eq!(uuid, "01563e3a-b5d3-d676-4c61-efb99302bd5b");
        assert_eq!(inspect(&uuid)[0].1, "01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert_eq!(
            inspect("01arz3ndektsv4rrffq69g5fav")[0].1,
            "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        );
        assert!(Inspect.run(&Ctx::new(), b"nope", &Values::new()).is_err());
    }
}
