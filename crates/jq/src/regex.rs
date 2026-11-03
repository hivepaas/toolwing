//! jq's `_match_impl` on Oniguruma, the engine jq uses, ported from jq
//! 1.8.2's `f_match` (src/builtin.c, MIT): the same syntax (Perl_NG), the
//! same options for the same modifiers, and the same match objects, so
//! `match`, `test`, `capture`, `scan`, `splits` and `sub`, which the prelude
//! takes from jq's builtin.jq, give what jq gives.

use jaq_all::json::{Error, Map, Val};
use onig::{MatchParam, Regex, RegexOptions, Region, SearchOptions, Syntax};

use jaq_all::json::jq::typed;

/// `input | _match_impl(re; modifiers; test)`: with `test`, whether `re`
/// matches; else an array of its matches (all of them with `g`).
pub fn match_impl(input: &Val, re: &Val, modifiers: &Val, test: &Val) -> Result<Val, Error> {
    let test = matches!(test, Val::Bool(true));
    let input = match input {
        Val::TStr(s) | Val::BStr(s) => String::from_utf8_lossy(s),
        v => {
            return Err(fail(format_args!(
                "{} cannot be matched, as it is not a string",
                typed(v)
            )));
        }
    };
    let re = match re {
        Val::TStr(s) | Val::BStr(s) => String::from_utf8_lossy(s),
        v => return Err(fail(format_args!("{} is not a string", typed(v)))),
    };
    let (options, global) = options(modifiers)?;
    let regex = Regex::with_options(&re, options, Syntax::perl_ng())
        .map_err(|e| fail(format_args!("Regex failure: {}", e.description())))?;

    let bytes = input.as_bytes();
    let end = bytes.len();
    // Characters before byte `b`, counted as jq counts them: the bytes
    // that start one.
    let chars = |from: usize, to: usize| {
        bytes[from..to]
            .iter()
            .filter(|b| **b & 0xC0 != 0x80)
            .count()
    };
    let text =
        |from: usize, to: usize| Val::from(String::from_utf8_lossy(&bytes[from..to]).into_owned());
    let mut matches = Vec::new();
    let mut region = Region::new();
    let mut start = 0;
    loop {
        let found = regex
            .search_with_param(
                &*input,
                start,
                end,
                SearchOptions::SEARCH_OPTION_NONE,
                Some(&mut region),
                MatchParam::default(),
            )
            .map_err(|e| fail(format_args!("Regex failure: {}", e.description())))?;
        if found.is_none() {
            break;
        }
        if test {
            return Ok(Val::Bool(true));
        }
        let Some((beg, stop)) = region.pos(0) else {
            break;
        };
        let offset = chars(0, beg);
        let mut captures: Vec<Vec<(&str, Val)>> = Vec::new();
        for i in 1..region.len() {
            let cap = match region.pos(i) {
                // A group that took no part.
                None => vec![
                    ("offset", num(-1)),
                    ("string", Val::Null),
                    ("length", num(0)),
                ],
                // An empty group, in an empty match at the match's offset.
                Some((b, e)) if b == e || beg == stop => {
                    let at = if beg == stop { offset } else { chars(0, b) };
                    vec![
                        ("offset", num(at as isize)),
                        ("string", text(b, b)),
                        ("length", num(0)),
                    ]
                }
                Some((b, e)) => vec![
                    ("offset", num(chars(0, b) as isize)),
                    ("length", num(chars(b, e) as isize)),
                    ("string", text(b, e)),
                ],
            };
            captures.push(cap);
        }
        let mut names = vec![Val::Null; captures.len()];
        regex.foreach_name(|name, groups| {
            for g in groups {
                if let Some(n) = names.get_mut(*g as usize - 1) {
                    *n = Val::from(name.to_owned());
                }
            }
            true
        });
        let captures: Val = captures
            .into_iter()
            .zip(names)
            .map(|(mut cap, name)| {
                cap.push(("name", name));
                object(cap)
            })
            .collect();
        let length = chars(beg, stop);
        matches.push(object(vec![
            ("offset", num(offset as isize)),
            ("length", num(length as isize)),
            ("string", text(beg, stop)),
            ("captures", captures),
        ]));
        // After an empty match, one byte on, so `"qux" | match("(?=u)"; "g")`
        // matches once.
        start = if beg == stop { stop + 1 } else { stop };
        if !global || start > end {
            break;
        }
    }
    Ok(if test {
        Val::Bool(false)
    } else {
        matches.into_iter().collect()
    })
}

/// The Oniguruma options of jq's modifiers, and whether `g` is among them.
fn options(modifiers: &Val) -> Result<(RegexOptions, bool), Error> {
    let mut options = RegexOptions::REGEX_OPTION_CAPTURE_GROUP;
    let mut global = false;
    let modifiers = match modifiers {
        Val::Null => return Ok((options, global)),
        Val::TStr(s) | Val::BStr(s) => String::from_utf8_lossy(s),
        v => return Err(fail(format_args!("{} is not a string", typed(v)))),
    };
    for m in modifiers.chars() {
        options |= match m {
            'g' => {
                global = true;
                RegexOptions::REGEX_OPTION_NONE
            }
            'i' => RegexOptions::REGEX_OPTION_IGNORECASE,
            'x' => RegexOptions::REGEX_OPTION_EXTEND,
            'm' => RegexOptions::REGEX_OPTION_MULTILINE,
            's' => RegexOptions::REGEX_OPTION_SINGLELINE,
            'p' => RegexOptions::REGEX_OPTION_MULTILINE | RegexOptions::REGEX_OPTION_SINGLELINE,
            'l' => RegexOptions::REGEX_OPTION_FIND_LONGEST,
            'n' => RegexOptions::REGEX_OPTION_FIND_NOT_EMPTY,
            _ => {
                return Err(fail(format_args!(
                    "{modifiers} is not a valid modifier string"
                )));
            }
        };
    }
    Ok((options, global))
}

fn fail(msg: core::fmt::Arguments) -> Error {
    Error::str(msg)
}

fn num(n: isize) -> Val {
    Val::from(n)
}

/// An object with its keys in the order given, as jq builds it.
fn object(fields: Vec<(&str, Val)>) -> Val {
    let map: Map = fields
        .into_iter()
        .map(|(k, v)| (Val::from(k.to_owned()), v))
        .collect();
    Val::obj(map)
}
