//! toolwing's tools and their registry (docs/design/architecture.md).

pub mod base;
pub mod base32;
pub mod base64;
mod basen;
pub mod case;
pub mod command;
pub mod convert;
pub mod cron;
pub mod diff;
pub mod hash;
pub mod hex;
pub mod jq;
pub mod json;
pub mod jwt;
pub mod regex;
pub mod time;
pub mod url;
pub mod uuid;

use toolwing_core::{Group, Tool, en};

/// The groups, in the order `tw --help` and the app's sidebar show them.
pub static GROUPS: &[Group] = &[
    Group {
        name: "jq",
        title: en!("jq"),
        summary: en!("Query and transform JSON, YAML, TOML, XML and CSV, as jq 1.8 does"),
    },
    Group {
        name: "json",
        title: en!("JSON"),
        summary: en!("Format, minify and validate JSON, written as jq . writes it"),
    },
    Group {
        name: "convert",
        title: en!("Convert"),
        summary: en!("Between JSON, YAML 1.2, TOML, CSV and XML"),
    },
    Group {
        name: "base64",
        title: en!("Base64"),
        summary: en!("Encode and decode Base64 (RFC 4648)"),
    },
    Group {
        name: "base32",
        title: en!("Base32"),
        summary: en!("Encode and decode Base32 (RFC 4648), as GNU base32 does"),
    },
    Group {
        name: "hex",
        title: en!("Hex"),
        summary: en!("Bytes as hexadecimal and back, as xxd -p and xxd -r -p"),
    },
    Group {
        name: "url",
        title: en!("URL"),
        summary: en!("Percent-encode, decode and take URLs apart (RFC 3986, WHATWG)"),
    },
    Group {
        name: "jwt",
        title: en!("JWT"),
        summary: en!("Decode, verify and create JSON Web Tokens (RFC 7519)"),
    },
    Group {
        name: "md5sum",
        title: en!("MD5"),
        summary: en!("MD5 checksums of files, or check them"),
    },
    Group {
        name: "sha1sum",
        title: en!("SHA-1"),
        summary: en!("SHA-1 checksums of files, or check them"),
    },
    Group {
        name: "sha256sum",
        title: en!("SHA-256"),
        summary: en!("SHA-256 checksums of files, or check them"),
    },
    Group {
        name: "sha512sum",
        title: en!("SHA-512"),
        summary: en!("SHA-512 checksums of files, or check them"),
    },
    Group {
        name: "b2sum",
        title: en!("BLAKE2b"),
        summary: en!("BLAKE2b checksums of files, or check them"),
    },
    Group {
        name: "uuid",
        title: en!("UUID"),
        summary: en!("Generate UUIDs: random (v4) or time-ordered (v7)"),
    },
    Group {
        name: "time",
        title: en!("Time"),
        summary: en!("Unix timestamps, ISO 8601 and RFC dates, in any time zone"),
    },
    Group {
        name: "cron",
        title: en!("Cron"),
        summary: en!("What a cron expression means, and when it runs next"),
    },
    Group {
        name: "regex",
        title: en!("Regex"),
        summary: en!("Test a pattern, see its matches and groups, replace with it"),
    },
    Group {
        name: "diff",
        title: en!("Diff"),
        summary: en!("Compare two texts or files line by line, as diff does"),
    },
    Group {
        name: "case",
        title: en!("Case"),
        summary: en!("One name in every case code uses: camelCase, snake_case, kebab-case…"),
    },
    Group {
        name: "base",
        title: en!("Number base"),
        summary: en!("A number in binary, octal, decimal, hex or any base from 2 to 36"),
    },
];

/// Every tool, in the order `tw --help` and the app's sidebar show them. A
/// group's first tool is its default action.
pub static TOOLS: &[&dyn Tool] = &[
    &jq::Jq,
    &json::Pretty,
    &json::Minify,
    &json::Validate,
    &convert::Convert,
    &base64::Encode,
    &base64::Decode,
    &base32::Encode,
    &base32::Decode,
    &hex::Encode,
    &hex::Decode,
    &url::Encode,
    &url::Decode,
    &url::Parse,
    &jwt::Decode,
    &jwt::Verify,
    &jwt::Encode,
    &hash::MD5SUM,
    &hash::SHA1SUM,
    &hash::SHA256SUM,
    &hash::SHA512SUM,
    &hash::B2SUM,
    &uuid::Generate,
    &time::Convert,
    &cron::Explain,
    &regex::Match,
    &regex::Replace,
    &diff::Compare,
    &case::Convert,
    &base::Convert,
];

/// The group names, in order.
pub fn groups() -> Vec<&'static str> {
    GROUPS.iter().map(|g| g.name).collect()
}

pub fn group_info(name: &str) -> Option<&'static Group> {
    GROUPS.iter().find(|g| g.name == name)
}

/// A group's tools, in registry order.
pub fn group(name: &str) -> Vec<&'static dyn Tool> {
    TOOLS
        .iter()
        .copied()
        .filter(|t| t.meta().group == name)
        .collect()
}

pub fn find(id: &str) -> Option<&'static dyn Tool> {
    TOOLS.iter().copied().find(|t| t.meta().id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use toolwing_core::{Compat, ParamKind, Value};

    #[test]
    fn ids_are_unique_and_spelled_from_group_and_action() {
        let mut seen = HashSet::new();
        for tool in TOOLS {
            let m = tool.meta();
            assert!(seen.insert(m.id), "{} twice", m.id);
            let want = if m.action.is_empty() {
                m.group.to_owned()
            } else {
                format!("{}.{}", m.group, m.action)
            };
            assert_eq!(m.id, want);
        }
    }

    #[test]
    fn groups_are_listed_in_registry_order() {
        let mut order: Vec<&str> = Vec::new();
        for tool in TOOLS {
            let g = tool.meta().group;
            if !order.contains(&g) {
                order.push(g);
            }
        }
        assert_eq!(groups(), order);
    }

    #[test]
    fn groups_are_contiguous_and_agree_on_their_shape() {
        let order: Vec<_> = TOOLS.iter().map(|t| t.meta().group).collect();
        for g in groups() {
            let first = order.iter().position(|x| *x == g).unwrap_or(0);
            let last = order.iter().rposition(|x| *x == g).unwrap_or(0);
            assert!(order[first..=last].iter().all(|x| *x == g), "{g} is split");
            let tools = group(g);
            let single = tools.len() == 1;
            for t in &tools {
                let m = t.meta();
                assert_eq!(m.action.is_empty(), single, "{}: action", m.id);
                assert_eq!(m.drop_in(), tools[0].meta().drop_in(), "{}: compat", m.id);
                if m.drop_in() && !single {
                    // The first action is the default; the others have a flag.
                    assert_eq!(
                        m.action_flag.is_none(),
                        m.id == tools[0].meta().id,
                        "{}",
                        m.id
                    );
                }
            }
        }
    }

    #[test]
    fn params_are_documented_and_defaults_fit() {
        for tool in TOOLS {
            for p in tool.params() {
                let id = tool.meta().id;
                assert!(!p.help.is_empty() && !p.label.is_empty(), "{id}.{}", p.key);
                let fits = match (p.kind, &p.default) {
                    (ParamKind::Flag, Value::Bool(_)) => true,
                    (ParamKind::Int { min, max }, Value::Int(n)) => (min..=max).contains(n),
                    (ParamKind::Choice { options, .. }, Value::Text(s)) => {
                        s.is_empty() || options.iter().any(|(o, _)| o == s)
                    }
                    (ParamKind::Text | ParamKind::Secret, Value::Text(_)) => true,
                    _ => false,
                };
                assert!(fits, "{id}.{}: default", p.key);
            }
        }
    }

    #[test]
    fn references_name_a_version() {
        for tool in TOOLS {
            let m = tool.meta();
            if let Some(r) = m.reference {
                assert!(!r.version.is_empty(), "{}", m.id);
                if r.compat == Compat::DropIn {
                    assert!(m.input != toolwing_core::InputKind::None, "{}", m.id);
                }
            }
        }
    }
}
