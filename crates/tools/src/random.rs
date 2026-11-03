//! `random password | token | bytes`: secrets from the operating system's
//! generator (`getrandom`), as `openssl rand` gives bytes.
//!
//! A password's characters are drawn uniformly from the classes asked for,
//! with no bias (rejection sampling), and a password without one of each
//! class is drawn again, which keeps every valid password as likely as any
//! other. Nothing is kept: each run draws anew.

use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, ErrorKind, InputKind, Meta, Output, Param, ParamKind,
    Reference, Tool, Value, Values, en,
};

const REFERENCE: Option<Reference> = Some(Reference {
    name: "openssl rand",
    version: "3",
    compat: Compat::Output,
});

const KEYWORDS: &[&str] = &[
    "random",
    "password",
    "passphrase",
    "token",
    "secret",
    "key",
    "bytes",
    "generate",
];

pub struct Password;
pub struct Token;
pub struct Bytes;

static PASSWORD_META: Meta = Meta {
    id: "random.password",
    group: "random",
    action: "password",
    action_flag: None,
    title: en!("Random password"),
    summary: en!("Passwords from the system's secure generator"),
    category: Category::Crypto,
    keywords: KEYWORDS,
    input: InputKind::None,
    streams: false,
    reference: REFERENCE,
};

static TOKEN_META: Meta = Meta {
    id: "random.token",
    group: "random",
    action: "token",
    action_flag: None,
    title: en!("Random token"),
    summary: en!("Random bytes written as hex or Base64, as openssl rand -hex does"),
    category: Category::Crypto,
    keywords: KEYWORDS,
    input: InputKind::None,
    streams: false,
    reference: REFERENCE,
};

static BYTES_META: Meta = Meta {
    id: "random.bytes",
    group: "random",
    action: "bytes",
    action_flag: None,
    title: en!("Random bytes"),
    summary: en!("Raw random bytes, as openssl rand does"),
    category: Category::Crypto,
    keywords: KEYWORDS,
    input: InputKind::None,
    streams: false,
    reference: REFERENCE,
};

const COUNT: Param = Param {
    key: "count",
    label: en!("How many"),
    help: en!("How many to generate, one a line"),
    kind: ParamKind::Int { min: 1, max: 1000 },
    default: Value::Int(1),
    cli: Cli {
        long: "count",
        short: Some('n'),
    },
};

const fn class(key: &'static str, label: &'static str, help: &'static str) -> Param {
    Param {
        key,
        label,
        help,
        kind: ParamKind::Flag,
        default: Value::Bool(true),
        cli: Cli {
            long: key,
            short: None,
        },
    }
}

static PASSWORD_PARAMS: &[Param] = &[
    Param {
        key: "length",
        label: en!("Length"),
        help: en!("Characters in each password"),
        kind: ParamKind::Int { min: 4, max: 1024 },
        default: Value::Int(20),
        cli: Cli {
            long: "length",
            short: Some('l'),
        },
    },
    class(
        "upper",
        en!("Uppercase A–Z"),
        en!("Uppercase letters, A to Z"),
    ),
    class(
        "lower",
        en!("Lowercase a–z"),
        en!("Lowercase letters, a to z"),
    ),
    class("digits", en!("Digits 0–9"), en!("Digits, 0 to 9")),
    class(
        "symbols",
        en!("Symbols !#$%&*+-=?@^_"),
        en!("The symbols !#$%&*+-=?@^_"),
    ),
    Param {
        key: "no_lookalike",
        label: en!("Leave out look-alikes I l 1 O 0"),
        help: en!("Leave out I, l, 1, O and 0, which are easily mistaken for one another"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "no-lookalike",
            short: None,
        },
    },
    COUNT,
];

const SIZE: Param = Param {
    key: "size",
    label: en!("Bytes"),
    help: en!("How many random bytes"),
    kind: ParamKind::Int {
        min: 1,
        max: 1 << 20,
    },
    default: Value::Int(32),
    cli: Cli {
        long: "size",
        short: Some('s'),
    },
};

static TOKEN_PARAMS: &[Param] = &[
    SIZE,
    Param {
        key: "encoding",
        label: en!("Written as"),
        help: en!("hex, Base64 or Base64 with the URL-safe alphabet and no padding"),
        kind: ParamKind::Choice {
            options: &[
                ("hex", en!("Hex")),
                ("base64", en!("Base64")),
                ("base64url", en!("Base64url")),
            ],
            flags: true,
            unset: None,
        },
        default: Value::Text(String::new()),
        cli: Cli {
            long: "encoding",
            short: None,
        },
    },
    COUNT,
];

static BYTES_PARAMS: &[Param] = &[SIZE];

/// The operating system's generator, a buffer at a time.
struct Rng {
    buf: [u8; 256],
    used: usize,
}

impl Rng {
    fn new() -> Rng {
        Rng {
            buf: [0; 256],
            used: 256,
        }
    }

    fn fill(buf: &mut [u8]) -> Result<(), Error> {
        getrandom::fill(buf)
            .map_err(|e| Error::new(ErrorKind::Io, format!("the system's generator failed: {e}")))
    }

    fn u32(&mut self) -> Result<u32, Error> {
        if self.used + 4 > self.buf.len() {
            Self::fill(&mut self.buf)?;
            self.used = 0;
        }
        let b = &self.buf[self.used..self.used + 4];
        self.used += 4;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A number below `n`, each as likely: draws past the last whole
    /// multiple of `n` are drawn again.
    fn below(&mut self, n: u32) -> Result<u32, Error> {
        let zone = u32::MAX - u32::MAX % n;
        loop {
            let x = self.u32()?;
            if x < zone {
                return Ok(x % n);
            }
        }
    }
}

const SYMBOLS: &str = "!#$%&*+-=?@^_";
const LOOKALIKES: &str = "Il1O0";

/// The classes asked for, each its characters.
fn classes(values: &Values) -> Vec<Vec<char>> {
    let lookalike = values.flag(PASSWORD_PARAMS, "no_lookalike");
    let keep = |c: &char| !(lookalike && LOOKALIKES.contains(*c));
    [
        ("upper", ('A'..='Z').collect::<String>()),
        ("lower", ('a'..='z').collect()),
        ("digits", ('0'..='9').collect()),
        ("symbols", SYMBOLS.to_owned()),
    ]
    .into_iter()
    .filter(|(key, _)| values.flag(PASSWORD_PARAMS, key))
    .map(|(_, chars)| chars.chars().filter(keep).collect())
    .collect()
}

/// About how many bits of entropy a password of `length` from `alphabet`
/// characters holds.
pub fn password_bits(length: usize, alphabet: usize) -> u32 {
    (length as f64 * (alphabet as f64).log2()).floor() as u32
}

impl Tool for Password {
    fn meta(&self) -> &'static Meta {
        &PASSWORD_META
    }

    fn params(&self) -> &'static [Param] {
        PASSWORD_PARAMS
    }

    fn run(&self, ctx: &Ctx, _input: &[u8], values: &Values) -> Result<Output, Error> {
        let length = usize::try_from(values.int(PASSWORD_PARAMS, "length")).unwrap_or(20);
        let count = values.int(PASSWORD_PARAMS, "count");
        let classes = classes(values);
        if classes.is_empty() {
            return Err(Error::usage("leave in one kind of character at least"));
        }
        let alphabet: Vec<char> = classes.iter().flatten().copied().collect();
        let mut rng = Rng::new();
        let mut lines = Vec::new();
        for _ in 0..count {
            ctx.check()?;
            let password = loop {
                let mut p = Vec::with_capacity(length);
                for _ in 0..length {
                    p.push(alphabet[rng.below(alphabet.len() as u32)? as usize]);
                }
                let has_each = classes.iter().all(|c| p.iter().any(|x| c.contains(x)));
                if has_each || length < classes.len() {
                    break p.into_iter().collect::<String>();
                }
            };
            lines.push(password);
        }
        Ok(Output::Lines(lines))
    }
}

impl Tool for Token {
    fn meta(&self) -> &'static Meta {
        &TOKEN_META
    }

    fn params(&self) -> &'static [Param] {
        TOKEN_PARAMS
    }

    fn run(&self, ctx: &Ctx, _input: &[u8], values: &Values) -> Result<Output, Error> {
        let size = usize::try_from(values.int(TOKEN_PARAMS, "size")).unwrap_or(32);
        let encoding = match values.text(TOKEN_PARAMS, "encoding").as_str() {
            "base64" => &data_encoding::BASE64,
            "base64url" => &data_encoding::BASE64URL_NOPAD,
            _ => &data_encoding::HEXLOWER,
        };
        let mut lines = Vec::new();
        let mut bytes = vec![0; size];
        for _ in 0..values.int(TOKEN_PARAMS, "count") {
            ctx.check()?;
            Rng::fill(&mut bytes)?;
            lines.push(encoding.encode(&bytes));
        }
        Ok(Output::Lines(lines))
    }
}

impl Tool for Bytes {
    fn meta(&self) -> &'static Meta {
        &BYTES_META
    }

    fn params(&self) -> &'static [Param] {
        BYTES_PARAMS
    }

    fn run(&self, _ctx: &Ctx, _input: &[u8], values: &Values) -> Result<Output, Error> {
        let mut bytes = vec![0; usize::try_from(values.int(BYTES_PARAMS, "size")).unwrap_or(32)];
        Rng::fill(&mut bytes)?;
        Ok(Output::Bytes(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passwords(set: &[(&str, Value)]) -> Result<Vec<String>, Error> {
        let mut v = Values::new();
        for (k, x) in set {
            v.set(k, x.clone());
        }
        match Password.run(&Ctx::new(), b"", &v)? {
            Output::Lines(l) => Ok(l),
            _ => Ok(Vec::new()),
        }
    }

    #[test]
    fn passwords_have_each_class_asked_for() {
        let ps =
            passwords(&[("count", Value::Int(200)), ("length", Value::Int(8))]).unwrap_or_default();
        assert_eq!(ps.len(), 200);
        for p in &ps {
            assert_eq!(p.chars().count(), 8);
            assert!(p.chars().any(|c| c.is_ascii_uppercase()), "{p}");
            assert!(p.chars().any(|c| c.is_ascii_lowercase()), "{p}");
            assert!(p.chars().any(|c| c.is_ascii_digit()), "{p}");
            assert!(p.chars().any(|c| SYMBOLS.contains(c)), "{p}");
        }
        let digits = [
            ("upper", Value::Bool(false)),
            ("lower", Value::Bool(false)),
            ("symbols", Value::Bool(false)),
            ("no_lookalike", Value::Bool(true)),
            ("count", Value::Int(50)),
        ];
        for p in passwords(&digits).unwrap_or_default() {
            assert!(
                p.chars()
                    .all(|c| c.is_ascii_digit() && c != '0' && c != '1'),
                "{p}"
            );
        }
        let none = [
            ("upper", Value::Bool(false)),
            ("lower", Value::Bool(false)),
            ("digits", Value::Bool(false)),
            ("symbols", Value::Bool(false)),
        ];
        assert!(passwords(&none).is_err());
    }

    /// Every character turns up about as often as any other: no bias toward
    /// the start of the alphabet.
    #[test]
    fn characters_are_uniform() {
        let ps = passwords(&[("count", Value::Int(1000)), ("length", Value::Int(100))])
            .unwrap_or_default();
        let mut seen = std::collections::HashMap::new();
        for c in ps.iter().flat_map(|p| p.chars()) {
            *seen.entry(c).or_insert(0usize) += 1;
        }
        // 100,000 draws from 75 characters: about 1,333 each.
        assert_eq!(seen.len(), 26 + 26 + 10 + SYMBOLS.len());
        assert!(
            seen.values().all(|n| (1_000..1_700).contains(n)),
            "{seen:?}"
        );
    }

    #[test]
    fn tokens_and_bytes_have_the_size_asked_for() {
        let mut v = Values::new();
        v.set("size", Value::Int(16));
        v.set("count", Value::Int(3));
        let Ok(Output::Lines(hex)) = Token.run(&Ctx::new(), b"", &v) else {
            panic!()
        };
        assert!(
            hex.len() == 3
                && hex
                    .iter()
                    .all(|t| t.len() == 32 && t.chars().all(|c| c.is_ascii_hexdigit()))
        );
        assert_ne!(hex[0], hex[1]);
        v.set("encoding", Value::Text("base64url".into()));
        let Ok(Output::Lines(b64)) = Token.run(&Ctx::new(), b"", &v) else {
            panic!()
        };
        assert!(b64.iter().all(|t| t.len() == 22 && !t.contains('=')));
        let Ok(Output::Bytes(b)) = Bytes.run(&Ctx::new(), b"", &v) else {
            panic!()
        };
        assert_eq!(b.len(), 16);
        assert_eq!(password_bits(20, 75), 124);
    }
}
