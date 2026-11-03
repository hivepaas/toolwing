//! `base`: a number in binary, octal, decimal, hex or any base from 2 to 36.
//!
//! The input may carry its base as a prefix (`0x`, `0o`, `0b`) and group its
//! digits with `_` or spaces. Numbers go up to 128 bits. With `--bits`, the
//! number is read as an integer of that width too: its pattern, signed and
//! unsigned (two's complement).

use toolwing_core::{
    Category, Cli, Ctx, Error, Field, InputKind, Meta, Output, Param, ParamKind, Tool, Value,
    Values, en,
};

pub struct Convert;

static META: Meta = Meta {
    id: "base",
    group: "base",
    action: "",
    action_flag: None,
    title: en!("Number base"),
    summary: en!("A number in binary, octal, decimal, hex or any base from 2 to 36"),
    category: Category::Text,
    keywords: &[
        "base", "radix", "binary", "octal", "hex", "decimal", "convert",
    ],
    input: InputKind::Text { trim_newline: true },
    streams: false,
    reference: None,
};

static PARAMS: &[Param] = &[
    Param {
        key: "from",
        label: en!("From base"),
        help: en!("The input's base, 2 to 36; 0 reads it from a 0x, 0o or 0b prefix, else decimal"),
        kind: ParamKind::Int { min: 0, max: 36 },
        default: Value::Int(0),
        cli: Cli {
            long: "from",
            short: None,
        },
    },
    Param {
        key: "to",
        label: en!("To base"),
        help: en!("Write the number in this base only, 2 to 36; 0 for the usual bases"),
        kind: ParamKind::Int { min: 0, max: 36 },
        default: Value::Int(0),
        cli: Cli {
            long: "to",
            short: None,
        },
    },
    Param {
        key: "group",
        label: en!("Group digits by"),
        help: en!("Put a space between groups of this many digits, from the right; 0 for none"),
        kind: ParamKind::Int { min: 0, max: 64 },
        default: Value::Int(0),
        cli: Cli {
            long: "group",
            short: Some('g'),
        },
    },
    Param {
        key: "upper",
        label: en!("Uppercase"),
        help: en!("Write digits above 9 in uppercase"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "upper",
            short: Some('u'),
        },
    },
    Param {
        key: "bits",
        label: en!("As an integer of"),
        help: en!("Also read the number as a signed and unsigned integer of this many bits"),
        kind: ParamKind::Choice {
            options: &[
                ("8", "8 bits"),
                ("16", "16 bits"),
                ("32", "32 bits"),
                ("64", "64 bits"),
                ("128", "128 bits"),
            ],
            flags: false,
            unset: Some("None"),
        },
        default: Value::Text(String::new()),
        cli: Cli {
            long: "bits",
            short: None,
        },
    },
];

/// A number read from text: its sign and size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Number {
    pub negative: bool,
    pub magnitude: u128,
}

/// Reads `text` in base `from`, or from its prefix when `from` is 0.
pub fn parse(text: &str, from: u32) -> Result<Number, Error> {
    let t = text.trim();
    let (negative, t) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let lower = t.to_ascii_lowercase();
    let prefixed = [("0x", 16), ("0o", 8), ("0b", 2)]
        .into_iter()
        .find(|(p, _)| lower.starts_with(p));
    let (base, digits) = match (from, prefixed) {
        (0, Some((p, b))) => (b, &t[p.len()..]),
        (0, None) => (10, t),
        (b, Some((p, pb))) if b == pb => (b, &t[p.len()..]),
        (b, _) => (b, t),
    };
    if !(2..=36).contains(&base) {
        return Err(Error::usage("a base is from 2 to 36"));
    }
    let mut magnitude: u128 = 0;
    let mut any = false;
    for (i, c) in digits.char_indices() {
        if c == '_' || c == ' ' {
            continue;
        }
        let d = c.to_digit(base).ok_or_else(|| {
            Error::input(format!("{c:?} is not a digit in base {base}")).at(
                toolwing_core::Span::at(text.as_bytes(), text.len() - digits.len() + i),
            )
        })?;
        magnitude = magnitude
            .checked_mul(u128::from(base))
            .and_then(|m| m.checked_add(u128::from(d)))
            .ok_or_else(|| Error::input("the number is larger than 128 bits"))?;
        any = true;
    }
    if !any {
        return Err(Error::input("no digits"));
    }
    Ok(Number {
        negative: negative && magnitude != 0,
        magnitude,
    })
}

/// `n` in `base` (2 to 36), digits grouped by `group` from the right.
pub fn format(n: u128, base: u32, group: usize, upper: bool) -> String {
    let mut digits = Vec::new();
    let mut n = n;
    loop {
        let d = (n % u128::from(base)) as u32;
        let c = char::from_digit(d, base).unwrap_or('?');
        digits.push(if upper { c.to_ascii_uppercase() } else { c });
        n /= u128::from(base);
        if n == 0 {
            break;
        }
    }
    let mut out = String::with_capacity(digits.len() * 2);
    for (i, c) in digits.iter().enumerate().rev() {
        out.push(*c);
        if group > 0 && i > 0 && i.is_multiple_of(group) {
            out.push(' ');
        }
    }
    out
}

fn signed(n: Number, s: String) -> String {
    if n.negative { format!("-{s}") } else { s }
}

/// The bit pattern of `n` as an integer of `bits`, or why it does not fit.
fn pattern(n: Number, bits: u32) -> Result<u128, Error> {
    let mask = if bits == 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    };
    if n.negative {
        let limit = 1u128 << (bits - 1);
        if n.magnitude > limit {
            return Err(Error::input(format!(
                "-{} does not fit in {bits} bits",
                n.magnitude
            )));
        }
        Ok(n.magnitude.wrapping_neg() & mask)
    } else {
        if n.magnitude > mask {
            return Err(Error::input(format!(
                "{} does not fit in {bits} bits",
                n.magnitude
            )));
        }
        Ok(n.magnitude)
    }
}

impl Tool for Convert {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let text = std::str::from_utf8(input).map_err(|_| Error::input("input is not UTF-8"))?;
        let base_of = |key| match u32::try_from(values.int(PARAMS, key)) {
            Ok(b) if b == 0 || (2..=36).contains(&b) => Ok(b),
            _ => Err(Error::usage("a base is from 2 to 36")),
        };
        let (from, to) = (base_of("from")?, base_of("to")?);
        let group = usize::try_from(values.int(PARAMS, "group")).unwrap_or(0);
        let upper = values.flag(PARAMS, "upper");
        let n = parse(text, from)?;
        if to != 0 {
            return Ok(Output::Text(signed(
                n,
                format(n.magnitude, to, group, upper),
            )));
        }
        let mut fields = vec![
            Field::new(
                "binary",
                "Binary",
                signed(n, format(n.magnitude, 2, group, upper)),
            ),
            Field::new(
                "octal",
                "Octal",
                signed(n, format(n.magnitude, 8, group, upper)),
            ),
            Field::new(
                "decimal",
                "Decimal",
                signed(n, format(n.magnitude, 10, group, upper)),
            ),
            Field::new(
                "hex",
                "Hex",
                signed(n, format(n.magnitude, 16, group, upper)),
            ),
            Field::new(
                "base36",
                "Base 36",
                signed(n, format(n.magnitude, 36, group, upper)),
            ),
        ];
        let bits = values.text(PARAMS, "bits");
        if !bits.is_empty() {
            let width: u32 = bits
                .parse()
                .map_err(|_| Error::usage("bits are 8, 16, 32, 64 or 128"))?;
            let p = pattern(n, width)?;
            let top = 1u128 << (width - 1);
            let as_signed = if p >= top && width < 128 {
                format!("-{}", (1u128 << width) - p)
            } else if p >= top {
                format!("-{}", p.wrapping_neg())
            } else {
                p.to_string()
            };
            let hex_digits = (width / 4) as usize;
            let mut hex = format(p, 16, group, upper);
            let bare = hex.replace(' ', "");
            if bare.len() < hex_digits {
                let padded = format!("{}{bare}", "0".repeat(hex_digits - bare.len()));
                hex = group_str(&padded, group);
            }
            let bin = format(p, 2, group, upper).replace(' ', "");
            let bin = format!("{}{bin}", "0".repeat(width as usize - bin.len()));
            fields.push(Field::new(
                "bits",
                format!("{width} bits"),
                group_str(&bin, group.max(4)),
            ));
            fields.push(Field::new("bits_hex", format!("{width} bits, hex"), hex));
            fields.push(Field::new(
                "unsigned",
                format!("Unsigned {width}-bit"),
                p.to_string(),
            ));
            fields.push(Field::new(
                "signed",
                format!("Signed {width}-bit"),
                as_signed,
            ));
        }
        Ok(Output::Fields(fields))
    }
}

fn group_str(digits: &str, group: usize) -> String {
    if group == 0 {
        return digits.to_owned();
    }
    let chars: Vec<char> = digits.chars().collect();
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        let left = chars.len() - i;
        if i > 0 && left.is_multiple_of(group) {
            out.push(' ');
        }
        out.push(*c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(input: &str, set: &[(&str, Value)]) -> Vec<(String, String)> {
        let mut v = Values::new();
        for (k, x) in set {
            v.set(k, x.clone());
        }
        match Convert.run(&Ctx::new(), input.as_bytes(), &v) {
            Ok(Output::Fields(f)) => f.into_iter().map(|f| (f.key, f.value)).collect(),
            other => panic!("{other:?}"),
        }
    }

    fn value<'a>(f: &'a [(String, String)], key: &str) -> &'a str {
        f.iter().find(|(k, _)| k == key).map_or("", |(_, v)| v)
    }

    #[test]
    fn reads_prefixes_and_groups() {
        for input in [
            "255",
            "0xff",
            "0XFF",
            "0o377",
            "0b1111_1111",
            "+255",
            "1111 1111",
        ] {
            let from = if input.contains(' ') { 2 } else { 0 };
            assert_eq!(parse(input, from).map(|n| n.magnitude), Ok(255), "{input}");
        }
        assert_eq!(parse("ff", 16).map(|n| n.magnitude), Ok(255));
        assert_eq!(parse("0xff", 16).map(|n| n.magnitude), Ok(255));
        assert_eq!(parse("-0", 0).map(|n| n.negative), Ok(false));
        assert!(parse("12g", 0).is_err());
        assert!(parse("", 0).is_err());
        assert!(parse("0x", 0).is_err());
        let big = "340282366920938463463374607431768211456"; // 2^128
        assert!(parse(big, 0).is_err());
        let e = parse("12z4", 10).expect_err("z is not decimal");
        assert_eq!(e.span.map(|s| s.column), Some(3));
    }

    #[test]
    fn every_usual_base() {
        let f = fields("255", &[]);
        assert_eq!(value(&f, "binary"), "11111111");
        assert_eq!(value(&f, "octal"), "377");
        assert_eq!(value(&f, "hex"), "ff");
        assert_eq!(value(&f, "base36"), "73");
        let f = fields(
            "255",
            &[("group", Value::Int(4)), ("upper", Value::Bool(true))],
        );
        assert_eq!(value(&f, "binary"), "1111 1111");
        assert_eq!(value(&f, "hex"), "FF");
        let f = fields("-42", &[]);
        assert_eq!(value(&f, "hex"), "-2a");
    }

    #[test]
    fn one_base() {
        let mut v = Values::new();
        v.set("to", Value::Int(7));
        assert_eq!(
            Convert.run(&Ctx::new(), b"255", &v),
            Ok(Output::Text("513".into()))
        );
        v.set("to", Value::Int(1));
        assert!(Convert.run(&Ctx::new(), b"255", &v).is_err());
    }

    #[test]
    fn integers_of_a_width() {
        let f = fields("255", &[("bits", Value::Text("8".into()))]);
        assert_eq!(value(&f, "signed"), "-1");
        assert_eq!(value(&f, "unsigned"), "255");
        assert_eq!(value(&f, "bits"), "1111 1111");
        let f = fields("-1", &[("bits", Value::Text("16".into()))]);
        assert_eq!(value(&f, "unsigned"), "65535");
        assert_eq!(value(&f, "bits_hex"), "ffff");
        let f = fields("5", &[("bits", Value::Text("16".into()))]);
        assert_eq!(value(&f, "bits_hex"), "0005");
        let f = fields("-128", &[("bits", Value::Text("8".into()))]);
        assert_eq!(value(&f, "bits"), "1000 0000");
        let mut v = Values::new();
        v.set("bits", Value::Text("8".into()));
        assert!(Convert.run(&Ctx::new(), b"256", &v).is_err());
        assert!(Convert.run(&Ctx::new(), b"-129", &v).is_err());
    }
}
