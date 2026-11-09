//! `uniq`: GNU coreutils 9.7's command line, ported from its uniq.c: its
//! arguments in order (`getopt_long` returning operands where they stand),
//! the obsolete `-N` and `+N`, `--all-repeated` and `--group` and their
//! METHODs, INPUT and OUTPUT, and every message. The filtering is
//! toolwing_tools::uniq's, as in the C.UTF-8 locale.
//!
//! Ported from GNU coreutils 9.7's uniq.c: Copyright (C) 1986-2025 Free
//! Software Foundation, Inc. GPL-3.0-or-later.

use std::fs::{self, File};
use std::io::{self, BufReader, Write};

use toolwing_tools::uniq::{Delimit, Failure, Grouping, Options, run};

use crate::getopt::{Event, Has, getopt_in_order};
use crate::gnu::{self, error, strerror, try_help};
use crate::quotearg::{quote, quoteaf, quotef};

/// tw's command for `uniq`: everything after it is uniq's.
pub fn command() -> clap::Command {
    clap::Command::new("uniq")
        .about(toolwing_tools::group_info("uniq").map_or("", |g| g.summary))
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

const PROG: &str = "uniq";
const GROUP: char = '\u{1}';
const HELP: char = '\u{2}';
const VERSION: char = '\u{3}';

const LONG: &[(&str, Has, char)] = &[
    ("count", Has::No, 'c'),
    ("repeated", Has::No, 'd'),
    ("all-repeated", Has::Optional, 'D'),
    ("group", Has::Optional, GROUP),
    ("ignore-case", Has::No, 'i'),
    ("unique", Has::No, 'u'),
    ("skip-fields", Has::Required, 'f'),
    ("skip-chars", Has::Required, 's'),
    ("check-chars", Has::Required, 'w'),
    ("zero-terminated", Has::No, 'z'),
    ("help", Has::No, HELP),
    ("version", Has::No, VERSION),
];

const USAGE: &str = "\
Usage: uniq [OPTION]... [INPUT [OUTPUT]]
Filter adjacent matching lines from INPUT (or standard input),
writing to OUTPUT (or standard output).

With no options, matching lines are merged to the first occurrence.

Mandatory arguments to long options are mandatory for short options too.
  -c, --count           prefix lines by the number of occurrences
  -d, --repeated        only print duplicate lines, one for each group
  -D                    print all duplicate lines
      --all-repeated[=METHOD]  like -D, but allow separating groups
                                 with an empty line;
                                 METHOD={none(default),prepend,separate}
  -f, --skip-fields=N   avoid comparing the first N fields
      --group[=METHOD]  show all items, separating groups with an empty line;
                          METHOD={separate(default),prepend,append,both}
  -i, --ignore-case     ignore differences in case when comparing
  -s, --skip-chars=N    avoid comparing the first N characters
  -u, --unique          only print unique lines
  -z, --zero-terminated     line delimiter is NUL, not newline
  -w, --check-chars=N   compare no more than N characters in lines
";

const NOTES: &str = "
A field is a run of blanks (usually spaces and/or TABs), then non-blank
characters.  Fields are skipped before chars.

'uniq' does not detect repeated lines unless they are adjacent.
You may want to sort the input first, or use 'sort -u' without 'uniq'.
";

/// How a number given to an option reads (`xstrtoimax` with no suffix):
/// a value (the largest when too large), or `None` when it is not one.
fn xstrtoimax(s: &str) -> Option<i64> {
    let t = s.trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r']);
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut v: i64 = 0;
    for b in digits.bytes() {
        let d = i64::from(b - b'0');
        v = match v.checked_mul(10).and_then(|v| v.checked_add(d)) {
            Some(v) => v,
            None => return Some(if neg { i64::MIN } else { i64::MAX }),
        };
    }
    Some(if neg { -v } else { v })
}

/// `strict_posix2`: _POSIX2_VERSION from 200112 to 200808.
fn strict_posix2() -> bool {
    std::env::var("_POSIX2_VERSION")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .is_some_and(|v| (200_112..200_809).contains(&v))
}

/// Runs `tw uniq`; the exit code. `args` are those after `uniq`.
pub fn main(args: &[String]) -> u8 {
    match parse(args) {
        Ok((o, files)) => check_file(&files[0], &files[1], &o),
        Err(code) => code,
    }
}

/// `size_opt`: a count for -f, -s or -w; the message and the end otherwise.
fn size_opt(arg: &str, what: &str) -> Result<usize, u8> {
    match xstrtoimax(arg) {
        Some(v) if v >= 0 => Ok(usize::try_from(v).unwrap_or(usize::MAX)),
        _ => {
            error(PROG, &format!("{arg}: {what}"));
            Err(1)
        }
    }
}

fn parse(args: &[String]) -> Result<(Options, [String; 2]), u8> {
    let mut o = Options::default();
    let mut files: Vec<String> = Vec::new();
    let mut obsolete_fields: Option<bool> = None;
    let mut output_option_used = false;
    let extra = |a: &str| {
        error(PROG, &format!("extra operand {}", quote(a)));
        try_help(PROG);
        1
    };
    let mut stdout = io::stdout().lock();
    for ev in getopt_in_order(args, "-0123456789Dcdf:is:uw:z", |c| c, LONG) {
        let g = match ev {
            Event::Operand(a) => {
                let obsolete = a.starts_with('+') && !strict_posix2();
                match xstrtoimax(&a).filter(|_| obsolete) {
                    Some(v) => o.skip_chars = usize::try_from(v).unwrap_or(usize::MAX),
                    None if files.len() == 2 => return Err(extra(&a)),
                    None => files.push(a),
                }
                continue;
            }
            Event::End(rest) => {
                for a in rest {
                    if files.len() == 2 {
                        return Err(extra(&a));
                    }
                    files.push(a);
                }
                break;
            }
            Event::Err(e) => {
                let _ = io::stderr().write_all(e.message(PROG).as_bytes());
                try_help(PROG);
                return Err(1);
            }
            Event::Opt(g) => g,
        };
        match g.opt {
            d @ '0'..='9' => {
                if obsolete_fields == Some(false) {
                    o.skip_fields = 0;
                }
                o.skip_fields = o
                    .skip_fields
                    .checked_mul(10)
                    .and_then(|v| v.checked_add(d as usize - '0' as usize))
                    .unwrap_or(usize::MAX);
                obsolete_fields = Some(true);
            }
            'c' => {
                o.count = true;
                output_option_used = true;
            }
            'd' => {
                o.output_unique = false;
                output_option_used = true;
            }
            'D' => {
                o.output_unique = false;
                o.output_later_repeated = true;
                o.delimit = match g.value {
                    None => Delimit::None,
                    Some(v) => {
                        let methods = [
                            ("none", Delimit::None),
                            ("prepend", Delimit::Prepend),
                            ("separate", Delimit::Separate),
                        ];
                        gnu::argmatch(PROG, "--all-repeated", &v, &methods).ok_or(1u8)?
                    }
                };
                output_option_used = true;
            }
            GROUP => {
                o.grouping = match g.value {
                    None => Grouping::Separate,
                    Some(v) => {
                        let methods = [
                            ("prepend", Grouping::Prepend),
                            ("append", Grouping::Append),
                            ("separate", Grouping::Separate),
                            ("both", Grouping::Both),
                        ];
                        gnu::argmatch(PROG, "--group", &v, &methods).ok_or(1u8)?
                    }
                };
            }
            'f' => {
                obsolete_fields = Some(false);
                o.skip_fields = size_opt(
                    &g.value.unwrap_or_default(),
                    "invalid number of fields to skip",
                )?;
            }
            'i' => o.ignore_case = true,
            's' => {
                o.skip_chars = size_opt(
                    &g.value.unwrap_or_default(),
                    "invalid number of bytes to skip",
                )?;
            }
            'u' => {
                o.output_first_repeated = false;
                output_option_used = true;
            }
            'w' => {
                o.check_chars = size_opt(
                    &g.value.unwrap_or_default(),
                    "invalid number of bytes to compare",
                )?;
            }
            'z' => o.delimiter = 0,
            HELP => {
                let _ = write!(
                    stdout,
                    "{USAGE}{}{NOTES}{}",
                    gnu::HELP_VERSION,
                    gnu::ancillary_info(PROG)
                );
                return Err(0);
            }
            VERSION => {
                let _ = write!(
                    stdout,
                    "{}",
                    gnu::version(PROG, "Richard M. Stallman and David MacKenzie")
                );
                return Err(0);
            }
            _ => {
                try_help(PROG);
                return Err(1);
            }
        }
    }
    let bad = |m: &str| {
        error(PROG, m);
        try_help(PROG);
        1
    };
    if o.grouping != Grouping::None && output_option_used {
        return Err(bad("--group is mutually exclusive with -c/-d/-D/-u"));
    }
    if o.grouping != Grouping::None && o.count {
        return Err(bad("grouping and printing repeat counts is meaningless"));
    }
    if o.count && o.output_later_repeated {
        return Err(bad(
            "printing all duplicated lines and repeat counts is meaningless",
        ));
    }
    let mut f = files.into_iter();
    let input = f.next().unwrap_or_else(|| "-".into());
    let output = f.next().unwrap_or_else(|| "-".into());
    Ok((o, [input, output]))
}

/// `check_file`: INPUT opened, then OUTPUT, then the lines.
fn check_file(infile: &str, outfile: &str, o: &Options) -> u8 {
    let stdin = io::stdin();
    let mut is_dir = false;
    let mut input: Box<dyn io::BufRead> = if infile == "-" {
        Box::new(stdin.lock())
    } else if fs::metadata(infile).is_ok_and(|m| m.is_dir()) {
        // Opened, then not readable (Windows cannot open it at all).
        is_dir = true;
        Box::new(io::empty())
    } else {
        match File::open(infile) {
            Ok(f) => Box::new(BufReader::with_capacity(1 << 16, f)),
            Err(e) => {
                error(PROG, &format!("{}: {}", quotef(infile), strerror(&e)));
                return 1;
            }
        }
    };
    let mut output: Box<dyn Write> = if outfile == "-" {
        Box::new(io::BufWriter::new(crate::stdout()))
    } else {
        match File::create(outfile) {
            Ok(f) => Box::new(io::BufWriter::new(f)),
            Err(e) => {
                error(PROG, &format!("{}: {}", quotef(outfile), strerror(&e)));
                return 1;
            }
        }
    };
    let r = if is_dir {
        Err(Failure::Read(io::Error::from(io::ErrorKind::IsADirectory)))
    } else {
        run(&mut *input, &mut *output, o)
    };
    let flushed = output.flush();
    match r {
        Err(Failure::Read(e)) => {
            error(
                PROG,
                &format!("error reading {}: {}", quoteaf(infile), strerror(&e)),
            );
            1
        }
        Err(Failure::Write(e)) => {
            error(PROG, &format!("write error: {}", strerror(&e)));
            1
        }
        Ok(()) => match flushed {
            Ok(()) => 0,
            Err(e) => {
                error(PROG, &format!("write error: {}", strerror(&e)));
                1
            }
        },
    }
}
