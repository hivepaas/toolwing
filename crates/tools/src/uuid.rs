//! `uuid generate` and `uuid inspect` (RFC 9562).
//!
//! Generating: random (v4, the default), time-ordered (v7), from a name in a
//! namespace (v3 with MD5, v5 with SHA-1, as `uuidgen --md5`/`--sha1`), and
//! from the time and a node (v1, v6), the node random with its multicast bit
//! set, as RFC 9562 allows (section 6.10) instead of a network card's
//! address. Inspecting reads a UUID in any of its forms and says what it
//! holds.

use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, Field, InputKind, Meta, Output, Param, ParamKind, Reference,
    Score, Tool, Value, Values, en,
};
use uuid::{Uuid, Variant, Version};

pub struct Generate;
pub struct Inspect;

const REFERENCE: Option<Reference> = Some(Reference {
    name: "RFC 9562",
    version: "2024",
    compat: Compat::Standard,
});

static META: Meta = Meta {
    id: "uuid.generate",
    group: "uuid",
    action: "generate",
    action_flag: None,
    title: en!("UUID"),
    summary: en!("Generate UUIDs: random (v4), time-ordered (v7), from a name (v3, v5), or v1, v6"),
    category: Category::Data,
    keywords: &["uuid", "guid", "uuidgen", "v4", "v7", "v5", "id"],
    input: InputKind::None,
    streams: false,
    reference: REFERENCE,
};

static INSPECT_META: Meta = Meta {
    id: "uuid.inspect",
    group: "uuid",
    action: "inspect",
    action_flag: None,
    title: en!("UUID inspect"),
    summary: en!("What a UUID holds: its version, variant and time, and its other forms"),
    category: Category::Data,
    keywords: &["uuid", "guid", "inspect", "decode", "version", "timestamp"],
    input: InputKind::Text { trim_newline: true },
    streams: false,
    reference: REFERENCE,
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
                ("v5", en!("From --name in --namespace, with SHA-1")),
                ("v3", en!("From --name in --namespace, with MD5")),
                ("v1", en!("From the time and a random node")),
                ("v6", en!("From the time and a random node, sortable")),
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
        key: "namespace",
        label: en!("Namespace"),
        help: en!("For v3 and v5: dns, url, oid, x500, or a UUID; dns when left out"),
        kind: ParamKind::Text,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "namespace",
            short: None,
        },
    },
    Param {
        key: "name",
        label: en!("Name"),
        help: en!("For v3 and v5: the name the UUID is made from"),
        kind: ParamKind::Text,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "name",
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

/// `dns`, `url`, `oid`, `x500` (with or without uuidgen's `@`), or a UUID.
fn namespace(name: &str) -> Result<Uuid, Error> {
    Ok(match name.trim_start_matches('@') {
        "" | "dns" => Uuid::NAMESPACE_DNS,
        "url" => Uuid::NAMESPACE_URL,
        "oid" => Uuid::NAMESPACE_OID,
        "x500" => Uuid::NAMESPACE_X500,
        other => Uuid::try_parse(other).map_err(|_| {
            Error::usage(format!(
                "{other:?} is not a namespace: dns, url, oid, x500 or a UUID"
            ))
        })?,
    })
}

/// A random node, its multicast bit set so it is no network card's.
fn random_node() -> Result<[u8; 6], Error> {
    let mut node = [0; 6];
    getrandom::fill(&mut node)
        .map_err(|e| Error::usage(format!("the system's generator failed: {e}")))?;
    node[0] |= 1;
    Ok(node)
}

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
        let version = values.text(PARAMS, "version");
        let name = values.text(PARAMS, "name");
        if matches!(version.as_str(), "v3" | "v5") && name.is_empty() {
            return Err(Error::usage(
                "a v3 or v5 UUID is made from a name: give --name",
            ));
        }
        let ns = namespace(&values.text(PARAMS, "namespace"))?;
        let node = random_node()?;
        let upper = values.flag(PARAMS, "upper");
        let mut lines = Vec::new();
        for _ in 0..count {
            let id = match version.as_str() {
                "v7" => Uuid::now_v7(),
                "v5" => Uuid::new_v5(&ns, name.as_bytes()),
                "v3" => Uuid::new_v3(&ns, name.as_bytes()),
                "v1" => Uuid::now_v1(&node),
                "v6" => Uuid::now_v6(&node),
                _ => Uuid::new_v4(),
            };
            let s = id.hyphenated().to_string();
            lines.push(if upper { s.to_uppercase() } else { s });
        }
        Ok(Output::Lines(lines))
    }
}

impl Tool for Inspect {
    fn meta(&self) -> &'static Meta {
        &INSPECT_META
    }

    /// Reads it with dashes or without, in either case, in braces, or as
    /// `urn:uuid:…`.
    fn run(&self, _ctx: &Ctx, input: &[u8], _values: &Values) -> Result<Output, Error> {
        let text = String::from_utf8_lossy(input);
        let id =
            Uuid::try_parse(text.trim()).map_err(|e| Error::input(format!("not a UUID: {e}")))?;
        let version = if id.is_nil() {
            "nil: all zeros".to_owned()
        } else if id.is_max() {
            "max: all ones".to_owned()
        } else {
            let what = match id.get_version() {
                Some(Version::Mac) => "time and node",
                Some(Version::Dce) => "DCE security",
                Some(Version::Md5) => "from a name, MD5",
                Some(Version::Random) => "random",
                Some(Version::Sha1) => "from a name, SHA-1",
                Some(Version::SortMac) => "time and node, sortable",
                Some(Version::SortRand) => "time-ordered",
                Some(Version::Custom) => "custom",
                _ => "not one RFC 9562 defines",
            };
            format!("{} · {what}", id.get_version_num())
        };
        let variant = match id.get_variant() {
            Variant::RFC4122 => "RFC 9562",
            Variant::NCS => "NCS (reserved)",
            Variant::Microsoft => "Microsoft (reserved)",
            _ => "future (reserved)",
        };
        let mut fields = vec![
            Field::new("version", en!("Version"), version),
            Field::new("variant", en!("Variant"), variant),
        ];
        if let Some(ts) = id.get_timestamp() {
            let (secs, nanos) = ts.to_unix();
            let at = jiff::Timestamp::new(secs as i64, nanos as i32)
                .map(|t| format!("{t:.3}"))
                .unwrap_or_default();
            fields.push(Field::new("created", en!("Created"), at));
            let ms = u128::from(secs) * 1000 + u128::from(nanos / 1_000_000);
            fields.push(Field::new("unix_ms", en!("Unix ms"), ms.to_string()));
        }
        if let Some(node) = id.get_node_id() {
            let hex: Vec<String> = node.iter().map(|b| format!("{b:02x}")).collect();
            fields.push(Field::new("node", en!("Node"), hex.join(":")));
        }
        let random_bits = match id.get_version() {
            Some(Version::Random) => Some(122),
            Some(Version::SortRand) => Some(74),
            _ => None,
        };
        if let Some(bits) = random_bits {
            fields.push(Field::new(
                "random_bits",
                en!("Random bits"),
                bits.to_string(),
            ));
        }
        let same = "The same UUID as";
        let hyphenated = id.hyphenated().to_string();
        fields.extend([
            Field::new("uuid", en!("UUID"), hyphenated.clone()).in_section(same),
            Field::new("upper", en!("Uppercase"), hyphenated.to_uppercase()).in_section(same),
            Field::new("hex", en!("Hex"), id.simple().to_string()).in_section(same),
            Field::new("urn", en!("URN"), id.urn().to_string()).in_section(same),
            Field::new("braced", en!("Braced"), id.braced().to_string()).in_section(same),
        ]);
        Ok(Output::Fields(fields))
    }

    fn detect(&self, sample: &[u8]) -> Option<Score> {
        let s = std::str::from_utf8(sample).ok()?.trim();
        Uuid::try_parse(s).ok().map(|_| Score(90))
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

    fn generate(set: &[(&str, &str)]) -> Result<Vec<String>, Error> {
        let mut v = Values::new();
        for (k, x) in set {
            v.set(k, Value::Text((*x).into()));
        }
        match Generate.run(&Ctx::new(), b"", &v)? {
            Output::Lines(lines) => Ok(lines),
            _ => Ok(Vec::new()),
        }
    }

    fn inspect(id: &str) -> Vec<(String, String)> {
        match Inspect.run(&Ctx::new(), id.as_bytes(), &Values::new()) {
            Ok(Output::Fields(f)) => f.into_iter().map(|f| (f.key, f.value)).collect(),
            other => panic!("{other:?}"),
        }
    }

    /// RFC 9562's example of a v5 UUID (appendix A.4) and of a v3 (A.2), as
    /// `uuidgen --sha1 --namespace @dns --name www.example.com` gives them.
    #[test]
    fn names_give_the_rfcs_uuids() {
        let v5 = [("version", "v5"), ("name", "www.example.com")];
        assert_eq!(
            generate(&v5).ok(),
            Some(vec!["2ed6657d-e927-568b-95e1-2665a8aea6a2".into()])
        );
        let v3 = [
            ("version", "v3"),
            ("name", "www.example.com"),
            ("namespace", "@dns"),
        ];
        assert_eq!(
            generate(&v3).ok(),
            Some(vec!["5df41881-3aed-3515-88a7-2f4a814cf09e".into()])
        );
        assert!(generate(&[("version", "v5")]).is_err());
        assert!(generate(&[("version", "v5"), ("name", "x"), ("namespace", "nope")]).is_err());
        for (version, digit) in [("v1", '1'), ("v6", '6')] {
            let ids = generate(&[("version", version)]).unwrap_or_default();
            assert_eq!(ids[0].chars().nth(14), Some(digit), "{ids:?}");
        }
    }

    /// The sketch's v7 UUID, in braces and uppercase: its time and forms.
    #[test]
    fn inspects_what_a_uuid_holds() {
        let f = inspect("{01A19FDF-CA6B-7B3A-9F21-3C5D2E1A0B44}");
        let get = |k: &str| f.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
        assert_eq!(get("version"), Some("7 · time-ordered"));
        assert_eq!(get("variant"), Some("RFC 9562"));
        assert_eq!(get("unix_ms"), Some("1793683606123"));
        assert_eq!(get("created"), Some("2026-11-03T05:26:46.123Z"));
        assert_eq!(get("random_bits"), Some("74"));
        assert_eq!(
            get("urn"),
            Some("urn:uuid:01a19fdf-ca6b-7b3a-9f21-3c5d2e1a0b44")
        );
        assert_eq!(get("hex"), Some("01a19fdfca6b7b3a9f213c5d2e1a0b44"));
        let nil = inspect("00000000-0000-0000-0000-000000000000");
        assert_eq!(nil[0].1, "nil: all zeros");
        let v1 = inspect("c232ab00-9414-11ec-b3c8-9f6bdeced846");
        let get = |k: &str| v1.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
        assert_eq!(get("created"), Some("2022-02-22T19:22:22.000Z"));
        assert_eq!(get("node"), Some("9f:6b:de:ce:d8:46"));
        assert!(
            Inspect
                .run(&Ctx::new(), b"not-a-uuid", &Values::new())
                .is_err()
        );
    }

    #[test]
    fn refuses_counts_out_of_range() {
        let mut v = Values::new();
        v.set("count", Value::Int(0));
        assert!(Generate.run(&Ctx::new(), b"", &v).is_err());
    }
}
