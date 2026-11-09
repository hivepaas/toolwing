//! `sort`: GNU coreutils 9.7's command line, ported from its sort.c: its
//! arguments in order (`getopt_long` returning operands where they stand),
//! keys (`-k`, and the obsolescent `+POS1 -POS2`), global orderings and
//! how keys inherit them, `--check`, `--merge` (`--batch-size` at a time),
//! `-o`, `--files0-from`, `-R` and `--random-source`, `--debug` and its
//! warnings, and every message and exit status. The ordering is
//! toolwing_tools::sort's, as in the C.UTF-8 locale.
//!
//! Ported from GNU coreutils 9.7's sort.c: Copyright (C) 1988-2025 Free
//! Software Foundation, Inc. GPL-3.0-or-later.

use std::fs::{self, File};
use std::io::{self, Read, Write};

use toolwing_tools::sort::{Ignore, Key, NONE, Sorter, lines};

use crate::getopt::{Event, Has, InOrder};
use crate::gnu::{self, error, strerror, try_help};
use crate::quotearg::{quote, quoteaf, quotef};

/// tw's command for `sort`: everything after it is sort's.
pub fn command() -> clap::Command {
    clap::Command::new("sort")
        .about(toolwing_tools::group_info("sort").map_or("", |g| g.summary))
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(
            clap::Arg::new("args")
                .value_name("ARGS")
                .num_args(0..)
                .allow_hyphen_values(true)
                .trailing_var_arg(true),
        )
}

const PROG: &str = "sort";
/// `SORT_FAILURE`.
const FAILURE: u8 = 2;

const CHECK: char = '\u{1}';
const COMPRESS_PROGRAM: char = '\u{2}';
const DEBUG: char = '\u{3}';
const FILES0_FROM: char = '\u{4}';
const NMERGE: char = '\u{5}';
const RANDOM_SOURCE: char = '\u{6}';
const SORT: char = '\u{7}';
const PARALLEL: char = '\u{8}';
const HELP: char = '\u{9}';
const VERSION: char = '\u{a}';

const SHORT: &str = "-bcCdfghik:mMno:rRsS:t:T:uVy:z";

const LONG: &[(&str, Has, char)] = &[
    ("ignore-leading-blanks", Has::No, 'b'),
    ("check", Has::Optional, CHECK),
    ("compress-program", Has::Required, COMPRESS_PROGRAM),
    ("debug", Has::No, DEBUG),
    ("dictionary-order", Has::No, 'd'),
    ("ignore-case", Has::No, 'f'),
    ("files0-from", Has::Required, FILES0_FROM),
    ("general-numeric-sort", Has::No, 'g'),
    ("ignore-nonprinting", Has::No, 'i'),
    ("key", Has::Required, 'k'),
    ("merge", Has::No, 'm'),
    ("month-sort", Has::No, 'M'),
    ("numeric-sort", Has::No, 'n'),
    ("human-numeric-sort", Has::No, 'h'),
    ("version-sort", Has::No, 'V'),
    ("random-sort", Has::No, 'R'),
    ("random-source", Has::Required, RANDOM_SOURCE),
    ("sort", Has::Required, SORT),
    ("output", Has::Required, 'o'),
    ("reverse", Has::No, 'r'),
    ("stable", Has::No, 's'),
    ("batch-size", Has::Required, NMERGE),
    ("buffer-size", Has::Required, 'S'),
    ("field-separator", Has::Required, 't'),
    ("temporary-directory", Has::Required, 'T'),
    ("unique", Has::No, 'u'),
    ("zero-terminated", Has::No, 'z'),
    ("parallel", Has::Required, PARALLEL),
    ("help", Has::No, HELP),
    ("version", Has::No, VERSION),
];

/// The command ends: its exit status, the message said.
struct Exit(u8);

type R<T> = Result<T, Exit>;

fn die(msg: &str) -> Exit {
    error(PROG, msg);
    Exit(FAILURE)
}

/// What `xstrtoumax` makes of a number (`enum strtol_error`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Strtol {
    Ok,
    Invalid,
    SuffixChar,
    Overflow,
}

/// `xstrtoumax` in base 10 with `valid` suffixes (1024 each step): the
/// status, the value, and where the suffix starts.
fn xstrtoumax(s: &str, valid: &str) -> (Strtol, u64, usize) {
    let b = s.as_bytes();
    let space = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r');
    let mut i = 0;
    while i < b.len() && space(b[i]) {
        i += 1;
    }
    if b.get(i) == Some(&b'-') {
        return (Strtol::Invalid, 0, 0);
    }
    if b.get(i) == Some(&b'+') {
        i += 1;
    }
    let digits_start = i;
    let mut v: u64 = 0;
    let mut err = Strtol::Ok;
    while i < b.len() && b[i].is_ascii_digit() {
        match v
            .checked_mul(10)
            .and_then(|x| x.checked_add(u64::from(b[i] - b'0')))
        {
            Some(x) => v = x,
            None => {
                v = u64::MAX;
                err = Strtol::Overflow;
            }
        }
        i += 1;
    }
    if i == digits_start {
        i = 0;
        match b.first() {
            Some(c) if valid.as_bytes().contains(c) => v = 1,
            _ => return (Strtol::Invalid, 0, 0),
        }
    }
    if i < b.len() {
        if !valid.as_bytes().contains(&b[i]) {
            return (Strtol::SuffixChar, v, i);
        }
        let power: u32 = match b[i] {
            b'k' | b'K' => 1,
            b'm' | b'M' => 2,
            b'g' | b'G' => 3,
            b't' | b'T' => 4,
            b'P' => 5,
            b'E' => 6,
            b'Z' => 7,
            b'Y' => 8,
            b'R' => 9,
            b'Q' => 10,
            _ => 0,
        };
        for _ in 0..power {
            match v.checked_mul(1024) {
                Some(x) => v = x,
                None => {
                    v = u64::MAX;
                    err = Strtol::Overflow;
                }
            }
        }
        i += 1;
        if i < b.len() {
            return (Strtol::SuffixChar, v, i);
        }
    }
    (err, v, i)
}

/// `xstrtol_fatal`: the message for a bad number of an option.
fn xstrtol_fatal(e: Strtol, name: Option<&str>, c: char, arg: &str) -> Exit {
    let option = match name {
        Some(n) => format!("--{n}"),
        None => format!("-{c}"),
    };
    die(&match e {
        Strtol::Invalid | Strtol::Ok => format!("invalid {option} argument '{arg}'"),
        Strtol::SuffixChar => format!("invalid suffix in {option} argument '{arg}'"),
        Strtol::Overflow => format!("{option} argument '{arg}' too large"),
    })
}

/// `parse_field_count`: a count and what follows it, or the message.
fn parse_field_count<'a>(s: &'a str, msgid: Option<&str>) -> R<Option<(usize, &'a str)>> {
    let (e, v, end) = xstrtoumax(s, "");
    match e {
        Strtol::Ok | Strtol::SuffixChar => {
            Ok(Some((usize::try_from(v).unwrap_or(NONE), &s[end..])))
        }
        Strtol::Overflow => {
            // The digits run to the suffix; past them, any rest.
            let b = s.as_bytes();
            let mut i = 0;
            while i < b.len() && !b[i].is_ascii_digit() {
                i += 1;
            }
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            Ok(Some((NONE, &s[i..])))
        }
        Strtol::Invalid => match msgid {
            Some(m) => Err(die(&format!("{m}: invalid count at start of {}", quote(s)))),
            None => Ok(None),
        },
    }
}

fn badfieldspec(spec: &str, msgid: &str) -> Exit {
    die(&format!(
        "{msgid}: invalid field specification {}",
        quote(spec)
    ))
}

fn incompatible(opts: &str) -> Exit {
    die(&format!("options '-{opts}' are incompatible"))
}

/// The most files that may be open at once, less stdin, stdout, stderr.
fn max_nmerge() -> u64 {
    let limit = fs::read_to_string("/proc/self/limits").ok().and_then(|l| {
        l.lines()
            .find(|x| x.starts_with("Max open files"))
            .and_then(|x| {
                x.split_whitespace()
                    .nth(3)
                    .and_then(|v| v.parse::<u64>().ok())
            })
    });
    u64::from(u32::try_from(limit.unwrap_or(1024).saturating_sub(3)).unwrap_or(u32::MAX))
}

/// The options as parsed.
struct Parsed {
    keys: Vec<Key>,
    gkey: Key,
    tab: Option<u8>,
    eol: u8,
    stable: bool,
    unique: bool,
    checkonly: Option<char>,
    mergeonly: bool,
    debug: bool,
    outfile: Option<String>,
    files: Vec<String>,
    files_from: Option<String>,
    random_source: Option<String>,
    nmerge: usize,
    /// `--parallel`: the merge tree has two nodes a thread.
    nthreads: u64,
}

/// Runs `tw sort`; the exit code. `args` are those after `sort`.
pub fn main(args: &[String]) -> u8 {
    let mut out = io::BufWriter::with_capacity(1 << 16, crate::stdout());
    let code = match run(args, &mut out) {
        Ok(code) | Err(Exit(code)) => code,
    };
    if let Err(e) = out.flush() {
        error(PROG, &format!("write error: {}", strerror(&e)));
        return FAILURE;
    }
    code
}

fn parse(args: &[String], out: &mut dyn Write) -> R<Parsed> {
    let mut p = Parsed {
        keys: Vec::new(),
        gkey: Key {
            sword: NONE,
            ..Key::default()
        },
        tab: None,
        eol: b'\n',
        stable: false,
        unique: false,
        checkonly: None,
        mergeonly: false,
        debug: false,
        outfile: None,
        files: Vec::new(),
        files_from: None,
        random_source: None,
        nmerge: 16,
        nthreads: 0,
    };
    let posixly_correct = std::env::var_os("POSIXLY_CORRECT").is_some();
    let posix_ver = std::env::var("_POSIX2_VERSION")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(200_809);
    let mut traditional = !(200_112..200_809).contains(&posix_ver);
    let mut compress_program: Option<String> = None;
    let mut g = InOrder::new(args, SHORT, LONG);
    loop {
        // With POSIXLY_CORRECT, operands after the first file are files,
        // but for "-o FILE" in traditional usage.
        if posixly_correct
            && !p.files.is_empty()
            && let Some(next) = g.peek()
        {
            let o = next.as_bytes();
            let dash_o = traditional
                && p.checkonly.is_none()
                && o.len() >= 2
                && o[0] == b'-'
                && o[1] == b'o'
                && (o.len() > 2 || g.optind + 1 != args.len());
            if !dash_o {
                p.files.push(next.clone());
                g.optind += 1;
                continue;
            }
        }
        let Some(ev) = g.next(&|c| c) else {
            break;
        };
        let o = match ev {
            Event::End(rest) => {
                p.files.extend(rest);
                break;
            }
            Event::Err(e) => {
                let _ = io::stderr().write_all(e.message(PROG).as_bytes());
                try_help(PROG);
                return Err(Exit(FAILURE));
            }
            Event::Operand(a) => {
                let mut key = None;
                if let Some(pos) = a.strip_prefix('+') {
                    let minus_pos = g.peek().is_some_and(|n| {
                        n.starts_with('-') && n.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
                    });
                    traditional |= minus_pos && !posixly_correct;
                    if traditional {
                        key = traditional_key(pos, minus_pos, &mut g)?;
                    }
                }
                match key {
                    Some(k) => p.keys.push(k),
                    None => p.files.push(a),
                }
                continue;
            }
            Event::Opt(o) => o,
        };
        let value = o.value.clone().unwrap_or_default();
        match o.opt {
            SORT => {
                let words = [
                    ("general-numeric", 'g'),
                    ("human-numeric", 'h'),
                    ("month", 'M'),
                    ("numeric", 'n'),
                    ("random", 'R'),
                    ("version", 'V'),
                ];
                let c = gnu::argmatch(PROG, "--sort", &value, &words).ok_or(Exit(1))?;
                p.gkey.set_ordering(&c.to_string(), true, true);
            }
            c @ ('b' | 'd' | 'f' | 'g' | 'h' | 'i' | 'M' | 'n' | 'r' | 'R' | 'V') => {
                p.gkey.set_ordering(&c.to_string(), true, true);
            }
            c @ (CHECK | 'c' | 'C') => {
                let c = if c == CHECK {
                    match &o.value {
                        Some(v) => {
                            let words = [("quiet", 'C'), ("silent", 'C'), ("diagnose-first", 'c')];
                            gnu::argmatch(PROG, "--check", v, &words).ok_or(Exit(1))?
                        }
                        None => 'c',
                    }
                } else {
                    c
                };
                if p.checkonly.is_some_and(|x| x != c) {
                    return Err(incompatible("cC"));
                }
                p.checkonly = Some(c);
            }
            COMPRESS_PROGRAM => {
                if compress_program.as_ref().is_some_and(|x| *x != value) {
                    return Err(die("multiple compress programs specified"));
                }
                compress_program = Some(value);
            }
            DEBUG => p.debug = true,
            FILES0_FROM => p.files_from = Some(value),
            'k' => p.keys.push(parse_key(&value)?),
            'm' => p.mergeonly = true,
            NMERGE => {
                let name = o.name.unwrap_or("batch-size");
                let (e, n, _) = xstrtoumax(&value, "");
                let mut e = e;
                if e == Strtol::Ok {
                    if n > u64::from(u32::MAX) {
                        e = Strtol::Overflow;
                    } else if n < 2 {
                        error(
                            PROG,
                            &format!("invalid --{name} argument {}", quote(&value)),
                        );
                        return Err(die(&format!("minimum --{name} argument is {}", quote("2"))));
                    } else if max_nmerge() < n {
                        e = Strtol::Overflow;
                    } else {
                        p.nmerge = usize::try_from(n).unwrap_or(usize::MAX);
                    }
                }
                if e == Strtol::Overflow {
                    error(
                        PROG,
                        &format!("--{name} argument {} too large", quote(&value)),
                    );
                    return Err(die(&format!(
                        "maximum --{name} argument with current rlimit is {}",
                        max_nmerge()
                    )));
                } else if e != Strtol::Ok {
                    return Err(xstrtol_fatal(e, o.name, 'S', &value));
                }
            }
            'o' => {
                if p.outfile.as_ref().is_some_and(|x| *x != value) {
                    return Err(die("multiple output files specified"));
                }
                p.outfile = Some(value);
            }
            RANDOM_SOURCE => {
                if p.random_source.as_ref().is_some_and(|x| *x != value) {
                    return Err(die("multiple random sources specified"));
                }
                p.random_source = Some(value);
            }
            's' => p.stable = true,
            'S' => {
                let (mut e, n, suf) = xstrtoumax(&value, "EgGkKmMPQRtTYZ");
                let b = value.as_bytes();
                let before_digit = suf > 0 && b.get(suf - 1).is_some_and(u8::is_ascii_digit);
                // No suffix: KiB.
                if e == Strtol::Ok && before_digit && n > u64::MAX / 1024 {
                    e = Strtol::Overflow;
                }
                if e == Strtol::SuffixChar
                    && before_digit
                    && suf + 1 == b.len()
                    && matches!(b[suf], b'b' | b'%')
                {
                    e = Strtol::Ok;
                }
                if e != Strtol::Ok {
                    return Err(xstrtol_fatal(e, o.name, 'S', &value));
                }
            }
            't' => {
                let b = value.as_bytes();
                let Some(&first) = b.first() else {
                    return Err(die("empty tab"));
                };
                let newtab = if b.len() > 1 {
                    if value == "\\0" {
                        0
                    } else {
                        return Err(die(&format!("multi-character tab {}", quote(&value))));
                    }
                } else {
                    first
                };
                if p.tab.is_some_and(|t| t != newtab) {
                    return Err(die("incompatible tabs"));
                }
                p.tab = Some(newtab);
            }
            'T' => {}
            PARALLEL => {
                let (e, n, _) = xstrtoumax(&value, "");
                match e {
                    Strtol::Overflow => p.nthreads = u64::MAX,
                    Strtol::Ok if n == 0 => return Err(die("number in parallel must be nonzero")),
                    Strtol::Ok => p.nthreads = n,
                    _ => return Err(xstrtol_fatal(e, o.name, 'S', &value)),
                }
            }
            'u' => p.unique = true,
            'y' => {
                // Solaris' -y: a value of digits only is taken; another is
                // left for the next argument.
                if g.value_was_next && !value.bytes().all(|c| c.is_ascii_digit()) {
                    g.optind -= 1;
                }
            }
            'z' => p.eol = 0,
            HELP => {
                let _ = out.write_all(include_str!("sort_help.txt").as_bytes());
                return Err(Exit(0));
            }
            VERSION => {
                let _ =
                    out.write_all(gnu::version(PROG, "Mike Haertel and Paul Eggert").as_bytes());
                return Err(Exit(0));
            }
            _ => {
                try_help(PROG);
                return Err(Exit(FAILURE));
            }
        }
    }
    Ok(p)
}

/// `-k KEYDEF`.
fn parse_key(spec: &str) -> R<Key> {
    let mut key = Key::default();
    let Some((sword, mut s)) = parse_field_count(spec, Some("invalid number at field start"))?
    else {
        return Err(Exit(FAILURE));
    };
    if sword == 0 {
        return Err(badfieldspec(spec, "field number is zero"));
    }
    key.sword = sword - 1;
    if let Some(rest) = s.strip_prefix('.') {
        let Some((schar, r)) = parse_field_count(rest, Some("invalid number after '.'"))? else {
            return Err(Exit(FAILURE));
        };
        if schar == 0 {
            return Err(badfieldspec(spec, "character offset is zero"));
        }
        key.schar = schar - 1;
        s = r;
    }
    if key.sword == 0 && key.schar == 0 {
        key.sword = NONE;
    }
    s = key.set_ordering(s, true, false);
    if let Some(rest) = s.strip_prefix(',') {
        let Some((eword, r)) = parse_field_count(rest, Some("invalid number after ','"))? else {
            return Err(Exit(FAILURE));
        };
        if eword == 0 {
            return Err(badfieldspec(spec, "field number is zero"));
        }
        key.eword = eword - 1;
        s = r;
        if let Some(rest) = s.strip_prefix('.') {
            let Some((echar, r)) = parse_field_count(rest, Some("invalid number after '.'"))?
            else {
                return Err(Exit(FAILURE));
            };
            key.echar = echar;
            s = r;
        }
        s = key.set_ordering(s, false, true);
    } else {
        key.eword = NONE;
        key.echar = 0;
    }
    if !s.is_empty() {
        return Err(badfieldspec(spec, "stray character in field spec"));
    }
    Ok(key)
}

/// `+POS1 [-POS2]`: a key, or `None` when POS1 is not one (a file then).
fn traditional_key(pos: &str, minus_pos: bool, g: &mut InOrder<'_, char>) -> R<Option<Key>> {
    let mut key = Key::default();
    let Some((sword, mut s)) = parse_field_count(pos, None)? else {
        return Ok(None);
    };
    key.sword = sword;
    if let Some(rest) = s.strip_prefix('.') {
        let Some((schar, r)) = parse_field_count(rest, None)? else {
            return Ok(None);
        };
        key.schar = schar;
        s = r;
    }
    if key.sword == 0 && key.schar == 0 {
        key.sword = NONE;
    }
    if !key.set_ordering(s, true, false).is_empty() {
        return Ok(None);
    }
    if minus_pos {
        let Some(arg) = g.peek().cloned() else {
            return Ok(None);
        };
        g.optind += 1;
        let Some((eword, mut s)) = parse_field_count(&arg[1..], Some("invalid number after '-'"))?
        else {
            return Err(Exit(FAILURE));
        };
        key.eword = eword;
        if let Some(rest) = s.strip_prefix('.') {
            let Some((echar, r)) = parse_field_count(rest, Some("invalid number after '.'"))?
            else {
                return Err(Exit(FAILURE));
            };
            key.echar = echar;
            s = r;
        }
        if key.echar == 0 && key.eword != 0 {
            key.eword -= 1;
        }
        if !key.set_ordering(s, false, true).is_empty() {
            return Err(badfieldspec(&arg, "stray character in field spec"));
        }
    }
    key.traditional_used = true;
    Ok(Some(key))
}

/// What a file holds, read as sort reads it ("-" is stdin, once).
fn read_input(name: &str) -> R<Vec<u8>> {
    let mut data = Vec::new();
    if name == "-" {
        if let Err(e) = io::stdin().lock().read_to_end(&mut data) {
            return Err(die(&format!(
                "read failed: {}: {}",
                quotef(name),
                strerror(&e)
            )));
        }
        return Ok(data);
    }
    if fs::metadata(name).is_ok_and(|m| m.is_dir()) {
        let e = io::Error::from(io::ErrorKind::IsADirectory);
        return Err(die(&format!(
            "read failed: {}: {}",
            quotef(name),
            strerror(&e)
        )));
    }
    match File::open(name) {
        Ok(mut f) => match f.read_to_end(&mut data) {
            Ok(_) => Ok(data),
            Err(e) => Err(die(&format!(
                "read failed: {}: {}",
                quotef(name),
                strerror(&e)
            ))),
        },
        Err(e) => Err(die(&format!(
            "open failed: {}: {}",
            quotef(name),
            strerror(&e)
        ))),
    }
}

fn run(args: &[String], out: &mut dyn Write) -> R<u8> {
    let mut p = parse(args, out)?;

    if let Some(from) = p.files_from.clone() {
        if let Some(first) = p.files.first() {
            error(PROG, &format!("extra operand {}", quoteaf(first)));
            let _ = writeln!(
                io::stderr(),
                "file operands cannot be combined with --files0-from"
            );
            try_help(PROG);
            return Err(Exit(FAILURE));
        }
        let data = read_input(&from)?;
        let mut names: Vec<&[u8]> = data.split(|&c| c == 0).collect();
        if data.last() == Some(&0) || data.is_empty() {
            names.pop();
        }
        if names.is_empty() {
            return Err(die(&format!("no input from {}", quoteaf(&from))));
        }
        for (i, n) in names.iter().enumerate() {
            if *n == b"-" {
                return Err(die(&format!(
                    "when reading file names from stdin, no file name of {} allowed",
                    quoteaf("-")
                )));
            }
            if n.is_empty() {
                return Err(die(&format!(
                    "{}:{}: invalid zero-length file name",
                    quotef(&from),
                    i + 1
                )));
            }
        }
        p.files = names
            .iter()
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .collect();
    }

    // Keys with no ordering of their own take the global one.
    let gkey = p.gkey.clone();
    for key in &mut p.keys {
        if key.default_compare() && !key.reverse {
            key.ignore = gkey.ignore;
            key.translate = gkey.translate;
            key.skipsblanks = gkey.skipsblanks;
            key.skipeblanks = gkey.skipeblanks;
            key.month = gkey.month;
            key.numeric = gkey.numeric;
            key.general_numeric = gkey.general_numeric;
            key.human_numeric = gkey.human_numeric;
            key.version = gkey.version;
            key.random = gkey.random;
            key.reverse = gkey.reverse;
        }
    }
    let mut gkey_only = false;
    if p.keys.is_empty() && !gkey.default_compare() {
        gkey_only = true;
        p.keys.push(gkey.clone());
    }
    let need_random = p.keys.iter().any(|k| k.random);

    for key in &p.keys {
        let kinds = usize::from(key.numeric)
            + usize::from(key.general_numeric)
            + usize::from(key.human_numeric)
            + usize::from(key.month)
            + usize::from(key.version || key.random || key.ignore != Ignore::None);
        if kinds > 1 {
            let k = Key {
                skipsblanks: false,
                skipeblanks: false,
                reverse: false,
                ..key.clone()
            };
            return Err(incompatible(&k.to_opts()));
        }
    }

    if p.debug {
        if p.checkonly.is_some() || p.outfile.is_some() {
            let c = p.checkonly.unwrap_or('o');
            return Err(incompatible(&format!("{c} --debug")));
        }
        error(
            PROG,
            &format!(
                "text ordering performed using {} sorting rules",
                quote("C.UTF-8")
            ),
        );
        key_warnings(&p, &gkey, gkey_only);
    }

    let mut seed = [0u8; 16];
    if need_random {
        match &p.random_source {
            Some(src) => {
                let mut f = File::open(src)
                    .map_err(|e| die(&format!("open failed: {}: {}", quotef(src), strerror(&e))))?;
                let mut got = 0;
                while got < 16 {
                    match f.read(&mut seed[got..]) {
                        Ok(0) => return Err(die(&format!("{}: end of file", quote(src)))),
                        Ok(n) => got += n,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                        Err(e) => {
                            return Err(die(&format!(
                                "{}: read error: {}",
                                quote(src),
                                strerror(&e)
                            )));
                        }
                    }
                }
            }
            None => {
                if getrandom::fill(&mut seed).is_err() {
                    return Err(die("open failed: getrandom"));
                }
            }
        }
    }

    if p.files.is_empty() {
        p.files.push("-".into());
    }
    let sorter = Sorter {
        keys: p.keys.clone(),
        tab: p.tab,
        eol: p.eol,
        reverse: gkey.reverse,
        stable: p.stable,
        unique: p.unique,
        seed,
    };

    if let Some(c) = p.checkonly {
        if let Some(extra) = p.files.get(1) {
            return Err(die(&format!(
                "extra operand {} not allowed with -{c}",
                quoteaf(extra)
            )));
        }
        if p.outfile.is_some() {
            return Err(incompatible(&format!("{c}o")));
        }
        let name = &p.files[0];
        let data = read_input(name)?;
        let l = lines(&data, p.eol);
        if let Some(i) = sorter.check(&l) {
            if c == 'c' {
                let mut e = io::stderr().lock();
                let _ = write!(e, "{PROG}: {name}:{}: disorder: ", i + 1);
                let _ = e.write_all(l[i]);
                let _ = e.write_all(&[p.eol]);
            }
            return Ok(1);
        }
        return Ok(0);
    }

    // Every input readable, the output writable, before anything.
    for f in &p.files {
        if f == "-" {
            continue;
        }
        let ok = match fs::metadata(f) {
            Ok(m) if m.is_dir() => Ok(()),
            Ok(_) => File::open(f).map(|_| ()),
            Err(e) => Err(e),
        };
        if let Err(e) = ok {
            return Err(die(&format!(
                "cannot read: {}: {}",
                quotef(f),
                strerror(&e)
            )));
        }
    }
    let mut out_file = match &p.outfile {
        Some(o) => {
            let mut opts = fs::OpenOptions::new();
            opts.write(true).create(true);
            Some(
                opts.open(o)
                    .map_err(|e| die(&format!("open failed: {}: {}", quotef(o), strerror(&e))))?,
            )
        }
        None => None,
    };

    let mut inputs: Vec<Vec<u8>> = Vec::with_capacity(p.files.len());
    for f in &p.files {
        inputs.push(read_input(f)?);
    }
    let result = if p.mergeonly {
        let files: Vec<Vec<&[u8]>> = inputs.iter().map(|d| lines(d, p.eol)).collect();
        sorter.merge(files, p.nmerge)
    } else {
        let mut all: Vec<&[u8]> = inputs.iter().flat_map(|d| lines(d, p.eol)).collect();
        // The merge tree for --parallel's threads (two nodes of 128 bytes
        // each, at most SIZE_MAX / 256 threads): past any memory, sort
        // cannot allocate it once it has lines to sort.
        let tree = u128::from(p.nthreads.min(u64::MAX / 256)) * 256;
        if all.len() > 1 && tree > 1 << 47 {
            return Err(die("memory exhausted"));
        }
        sorter.sort(&mut all);
        sorter.unique_lines(all)
    };

    let mut buf: Vec<u8> = Vec::new();
    for l in &result {
        if p.debug {
            buf.extend(sorter.debug_line(l));
        } else {
            buf.extend_from_slice(l);
            buf.push(p.eol);
        }
    }
    match (&mut out_file, &p.outfile) {
        (Some(f), Some(name)) => {
            let written = f.set_len(0).and_then(|()| f.write_all(&buf));
            if let Err(e) = written {
                return Err(die(&format!(
                    "write failed: {}: {}",
                    quotef(name),
                    strerror(&e)
                )));
            }
        }
        _ => {
            if let Err(e) = out.write_all(&buf) {
                return Err(die(&format!(
                    "write failed: {}: {}",
                    quotef("standard output"),
                    strerror(&e)
                )));
            }
        }
    }
    Ok(0)
}

/// `key_warnings`: what `--debug` says of the keys and options.
fn key_warnings(p: &Parsed, gkey: &Key, gkey_only: bool) {
    let mut ugkey = gkey.clone();
    let mut basic_numeric_field = false;
    let mut general_numeric_field = false;
    let mut basic_span = false;
    let mut general_span = false;
    for (n, key) in p.keys.iter().enumerate() {
        let keynum = n + 1;
        if key.is_numeric() {
            if key.general_numeric {
                general_numeric_field = true;
            } else {
                basic_numeric_field = true;
            }
        }
        if key.traditional_used {
            let sword = if key.sword == NONE { 0 } else { key.sword };
            let mut obuf = format!("+{sword}");
            let mut nbuf = format!("-k {}", sword.wrapping_add(1));
            if key.eword != NONE {
                obuf.push_str(&format!(" -{}", key.eword.wrapping_add(1)));
                nbuf.push_str(&format!(
                    ",{}",
                    key.eword
                        .wrapping_add(1)
                        .wrapping_add(usize::from(key.echar == NONE))
                ));
            }
            error(
                PROG,
                &format!(
                    "obsolescent key {} used; consider {} instead",
                    quote(&obuf),
                    quote(&nbuf)
                ),
            );
        }
        let zero_width = key.sword != NONE && key.eword < key.sword;
        if zero_width {
            error(
                PROG,
                &format!("key {keynum} has zero width and will be ignored"),
            );
        }
        let implicit_skip = key.is_numeric() || key.month;
        let line_offset = key.eword == 0 && key.echar != 0;
        if !zero_width
            && !gkey_only
            && p.tab.is_none()
            && !line_offset
            && ((!key.skipsblanks && !implicit_skip)
                || (!key.skipsblanks && key.schar != 0)
                || (!key.skipeblanks && key.echar != 0))
        {
            error(
                PROG,
                &format!(
                    "leading blanks are significant in key {keynum}; consider also specifying 'b'"
                ),
            );
        }
        if !gkey_only && key.is_numeric() {
            let sword = key.sword.wrapping_add(1);
            let eword = key.eword.wrapping_add(1);
            let sword = if sword == 0 { 1 } else { sword };
            if eword == 0 || sword < eword {
                error(
                    PROG,
                    &format!("key {keynum} is numeric and spans multiple fields"),
                );
                if key.general_numeric {
                    general_span = true;
                } else {
                    basic_span = true;
                }
            }
        }
        if ugkey.ignore != Ignore::None && ugkey.ignore == key.ignore {
            ugkey.ignore = Ignore::None;
        }
        if ugkey.translate && key.translate {
            ugkey.translate = false;
        }
        ugkey.skipsblanks &= !key.skipsblanks;
        ugkey.skipeblanks &= !key.skipeblanks;
        ugkey.month &= !key.month;
        ugkey.numeric &= !key.numeric;
        ugkey.general_numeric &= !key.general_numeric;
        ugkey.human_numeric &= !key.human_numeric;
        ugkey.random &= !key.random;
        ugkey.version &= !key.version;
        ugkey.reverse &= !key.reverse;
    }
    // No thousands separator in C.UTF-8; the decimal point is '.'.
    let mut number_locale_warned = false;
    if basic_span || general_span {
        if p.tab == Some(b'.') {
            error(
                PROG,
                &format!(
                    "field separator {} is treated as a decimal point in numbers",
                    quote(".")
                ),
            );
            number_locale_warned = true;
        } else if p.tab == Some(b'-') {
            error(
                PROG,
                &format!(
                    "field separator {} is treated as a minus sign in numbers",
                    quote("-")
                ),
            );
        } else if general_span && p.tab == Some(b'+') {
            error(
                PROG,
                &format!(
                    "field separator {} is treated as a plus sign in numbers",
                    quote("+")
                ),
            );
        }
    }
    if (basic_numeric_field || general_numeric_field) && !number_locale_warned {
        error(
            PROG,
            &format!(
                "numbers use {} as a decimal point in this locale",
                quote(".")
            ),
        );
    }
    if !ugkey.default_compare() || (ugkey.reverse && (p.stable || p.unique) && !p.keys.is_empty()) {
        let ugkey_reverse = ugkey.reverse;
        if !(p.stable || p.unique) {
            ugkey.reverse = false;
        }
        let opts = ugkey.to_opts();
        let msg = if opts.len() == 1 {
            format!("option '-{opts}' is ignored")
        } else {
            format!("options '-{opts}' are ignored")
        };
        error(PROG, &msg);
        ugkey.reverse = ugkey_reverse;
    }
    if ugkey.reverse && !(p.stable || p.unique) && !p.keys.is_empty() {
        error(PROG, "option '-r' only applies to last-resort comparison");
    }
}
