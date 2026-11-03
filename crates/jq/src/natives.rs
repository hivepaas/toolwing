//! Filters written in Rust where jaq's own differ from jq's
//! (docs/design/jq.md, "Native filters"). The prelude binds jq's names to
//! them, so jaq's stay as they are for jaq's own definitions.

use jaq_all::data::DataKind;
use jaq_all::jaq_core::RunPtr;
use jaq_all::jaq_core::native::{self, Filter, Fun, bome, v};
use jaq_all::json::jq::typed;
use jaq_all::json::{Error, Val};

/// jaq's natives, then ours.
pub fn funs() -> impl Iterator<Item = Fun<DataKind>> {
    let ours: Box<[Filter<RunPtr<DataKind>>]> = Box::new([
        ("_toolwing_base64d", v(0), |cv| bome(base64d(&cv.1))),
        ("_toolwing_urid", v(0), |cv| bome(urid(&cv.1))),
        ("_match_impl", v(3), |mut cv| {
            let test = cv.0.pop_var();
            let modifiers = cv.0.pop_var();
            let re = cv.0.pop_var();
            bome(crate::regex::match_impl(&cv.1, &re, &modifiers, &test))
        }),
    ]);
    jaq_all::data::funs().chain(ours.into_vec().into_iter().map(native::run::<DataKind>))
}

/// jq's `@base64d` on a string: padding is optional, and decoding stops at
/// the first `=`; the bytes are read as UTF-8, invalid ones replaced.
fn base64d(input: &Val) -> Result<Val, Error> {
    let bytes = match input {
        Val::TStr(b) | Val::BStr(b) => b.as_ref(),
        v => {
            return Err(Error::str(format_args!(
                "{} cannot be base64-decoded",
                typed(v)
            )));
        }
    };
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3 + 2);
    let (mut code, mut read) = (0u32, 0);
    for &b in bytes.iter().take_while(|b| **b != b'=') {
        let six = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => {
                let msg = format_args!("{} is not valid base64 data", typed(input));
                return Err(Error::str(msg));
            }
        };
        code = code << 6 | u32::from(six);
        read += 1;
        if read == 4 {
            out.extend_from_slice(&code.to_be_bytes()[1..]);
            (code, read) = (0, 0);
        }
    }
    match read {
        3 => out.extend_from_slice(&[(code >> 10) as u8, (code >> 2) as u8]),
        2 => out.push((code >> 4) as u8),
        1 => {
            let msg = format_args!("{} trailing base64 byte found", typed(input));
            return Err(Error::str(msg));
        }
        _ => {}
    }
    Ok(Val::from(String::from_utf8_lossy(&out).into_owned()))
}

/// jq's `@urid` on a string: each `%` and two hex digits a byte, and the
/// bytes valid UTF-8, or it fails.
fn urid(input: &Val) -> Result<Val, Error> {
    let fail = || Error::str(format_args!("{} is not a valid uri encoding", typed(input)));
    let bytes = match input {
        Val::TStr(b) | Val::BStr(b) => b.as_ref(),
        _ => return Err(fail()),
    };
    let hex = |b: u8| char::from(b).to_digit(16);
    let mut out = Vec::with_capacity(bytes.len());
    let mut it = bytes.iter().copied();
    while let Some(b) = it.next() {
        if b != b'%' {
            out.push(b);
            continue;
        }
        let (hi, lo) = (it.next().and_then(hex), it.next().and_then(hex));
        let (Some(hi), Some(lo)) = (hi, lo) else {
            return Err(fail());
        };
        out.push((hi << 4 | lo) as u8);
    }
    String::from_utf8(out).map(Val::from).map_err(|_| fail())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jaq_all::json::jq::dump;

    fn decode(s: &str) -> Result<String, String> {
        base64d(&Val::from(s.to_owned()))
            .map(|v| v.to_string())
            .map_err(|e| e.to_string())
    }

    /// jq 1.8.2's `tests/base64.test`, and a JWT's payload.
    #[test]
    fn base64d_as_jq() {
        assert_eq!(decode(""), Ok(r#""""#.into()));
        assert_eq!(decode("="), Ok(r#""""#.into()));
        assert_eq!(decode("Zm/Ds2Jhcgo="), Ok(r#""foóbar\n""#.into()));
        assert_eq!(decode("cWl4YmF6Cg"), Ok(r#""qixbaz\n""#.into()));
        assert_eq!(decode("eyJhIjoxfQ"), Ok(r#""{\"a\":1}""#.into()));
        assert_eq!(
            decode("Not base64 data"),
            Err(r#""string (\"Not base64 data\") is not valid base64 data""#.into())
        );
        assert_eq!(
            decode("QUJDa"),
            Err(r#""string (\"QUJDa\") trailing base64 byte found""#.into())
        );
    }

    /// jq 1.8.2's `tests/uri.test`.
    #[test]
    fn urid_as_jq() {
        let urid = |s: &str| {
            urid(&Val::from(s.to_owned()))
                .map(|v| v.to_string())
                .map_err(|e| e.to_string())
        };
        assert_eq!(urid("%E2%9C%93%20a%2B"), Ok(r#""✓ a+""#.into()));
        for bad in [
            "abc%",
            "abc%f",
            "abc%g",
            "%FX%9F%98%8E",
            "%F0%93%81",
            "%F0%C0%81%8E",
        ] {
            let shown = dump(&Val::from(bad.to_owned()));
            let want = Val::from(format!("string ({shown}) is not a valid uri encoding"));
            assert_eq!(urid(bad), Err(want.to_string()), "{bad}");
        }
    }

    #[test]
    fn values_in_errors_are_cut_as_jq_cuts_them() {
        let long = Val::from("abcdefghijklmnopqrstuvwxyz0123456789".to_owned());
        // As jq 1.8.2 shows it.
        assert_eq!(dump(&long), r#""abcdefghijklmnopqrstuvwx...""#);
        assert_eq!(dump(&Val::from(1usize)), "1");
    }
}
