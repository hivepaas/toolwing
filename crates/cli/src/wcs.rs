//! `wc`: GNU coreutils 9.7's command line, ported from its wc.c: options as
//! glibc's `getopt_long` reads them, `--files0-from`, `--total`, the width
//! of the columns (from the sizes of the regular files, 7 at least when an
//! input is not one), the names quoted as coreutils quote them, and every
//! message. Counts are toolwing_tools::wc's, as in the C.UTF-8 locale.
//!
//! Ported from GNU coreutils 9.7's wc.c: Copyright (C) 1985-2025 Free
//! Software Foundation, Inc. GPL-3.0-or-later.

use std::fs::{self, File};
use std::io::{self, Read, Write};

use toolwing_tools::wc::{Counts, count};

use crate::getopt::getopt_errors;
use crate::gnu::{self, error, strerror, try_help};
use crate::quotearg::{quoteaf, quotef};

/// tw's command for `wc`: everything after it is wc's.
pub fn command() -> clap::Command {
    clap::Command::new("wc")
        .about(toolwing_tools::group_info("wc").map_or("", |g| g.summary))
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

const PROG: &str = "wc";
const DEBUG: char = '\u{1}';
const FILES0_FROM: char = '\u{2}';
const TOTAL: char = '\u{3}';
const HELP: char = '\u{4}';
const VERSION: char = '\u{5}';

const LONG: &[(&str, bool, char)] = &[
    ("bytes", false, 'c'),
    ("chars", false, 'm'),
    ("lines", false, 'l'),
    ("words", false, 'w'),
    ("debug", false, DEBUG),
    ("files0-from", true, FILES0_FROM),
    ("max-line-length", false, 'L'),
    ("total", true, TOTAL),
    ("help", false, HELP),
    ("version", false, VERSION),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Total {
    Auto,
    Always,
    Only,
    Never,
}

const USAGE: &str = "\
Usage: wc [OPTION]... [FILE]...
  or:  wc [OPTION]... --files0-from=F
Print newline, word, and byte counts for each FILE, and a total line if
more than one FILE is specified.  A word is a nonempty sequence of non white
space delimited by white space characters or by start or end of input.

With no FILE, or when FILE is -, read standard input.

The options below may be used to select which counts are printed, always in
the following order: newline, word, character, byte, maximum line length.
  -c, --bytes            print the byte counts
  -m, --chars            print the character counts
  -l, --lines            print the newline counts
      --files0-from=F    read input from the files specified by
                           NUL-terminated names in file F;
                           If F is - then read names from standard input
  -L, --max-line-length  print the maximum display width
  -w, --words            print the word counts
      --total=WHEN       when to print a line with total counts;
                           WHEN can be: auto, always, only, never
";

/// Which counts are printed, and how wide.
struct Print {
    lines: bool,
    words: bool,
    chars: bool,
    bytes: bool,
    width_col: bool,
    number_width: usize,
}

/// Runs `tw wc`; the exit code. `args` are those after `wc`.
pub fn main(args: &[String]) -> u8 {
    let mut out = io::BufWriter::new(crate::stdout());
    let code = run(args, &mut out);
    if out.flush().is_err() {
        error(PROG, "write error");
        return 1;
    }
    code
}

fn run(args: &[String], out: &mut dyn Write) -> u8 {
    let (opts, operands) = getopt_errors(args, "clLmw", |c| c, LONG);
    let mut p = Print {
        lines: false,
        words: false,
        chars: false,
        bytes: false,
        width_col: false,
        number_width: 1,
    };
    let mut debug = false;
    let mut files_from: Option<String> = None;
    let mut total = Total::Auto;
    for o in opts {
        let o = match o {
            Ok(o) => o,
            Err(e) => {
                let _ = io::stderr().write_all(e.message(PROG).as_bytes());
                try_help(PROG);
                return 1;
            }
        };
        match o.opt {
            'c' => p.bytes = true,
            'm' => p.chars = true,
            'l' => p.lines = true,
            'w' => p.words = true,
            'L' => p.width_col = true,
            DEBUG => debug = true,
            FILES0_FROM => files_from = o.value,
            TOTAL => {
                let arg = o.value.unwrap_or_default();
                let choices = [
                    ("auto", Total::Auto),
                    ("always", Total::Always),
                    ("only", Total::Only),
                    ("never", Total::Never),
                ];
                match gnu::argmatch(PROG, "--total", &arg, &choices) {
                    Some(t) => total = t,
                    None => return 1,
                }
            }
            HELP => {
                let _ = write!(
                    out,
                    "{USAGE}{}{}",
                    gnu::HELP_VERSION,
                    gnu::ancillary_info(PROG)
                );
                return 0;
            }
            VERSION => {
                let _ = write!(
                    out,
                    "{}",
                    gnu::version(PROG, "Paul Rubin and David MacKenzie")
                );
                return 0;
            }
            _ => {
                try_help(PROG);
                return 1;
            }
        }
    }
    if !(p.lines || p.words || p.chars || p.bytes || p.width_col) {
        p.lines = true;
        p.words = true;
        p.bytes = true;
    }
    let posixly_correct = std::env::var_os("POSIXLY_CORRECT").is_some();

    // The names: the operands, or --files0-from's.
    let mut names: Vec<Option<String>> = Vec::new();
    let mut streamed = false;
    if let Some(from) = &files_from {
        if let Some(first) = operands.first() {
            error(PROG, &format!("extra operand {}", quoteaf(first)));
            let _ = writeln!(
                io::stderr(),
                "file operands cannot be combined with --files0-from"
            );
            try_help(PROG);
            return 1;
        }
        let (data, regular) = if from == "-" {
            let mut d = Vec::new();
            let regular = gnu::stdin_metadata().is_ok_and(|m| gnu::is_regular(&m));
            if io::stdin().lock().read_to_end(&mut d).is_err() {
                error(PROG, &format!("{}: read error", quotef(from)));
                return 1;
            }
            (d, regular)
        } else {
            match File::open(from) {
                Ok(mut f) => {
                    let regular = f.metadata().is_ok_and(|m| gnu::is_regular(&m));
                    let mut d = Vec::new();
                    if let Err(e) = f.read_to_end(&mut d) {
                        error(
                            PROG,
                            &format!("{}: read error: {}", quotef(from), strerror(&e)),
                        );
                        return 1;
                    }
                    (d, regular)
                }
                Err(e) => {
                    error(
                        PROG,
                        &format!(
                            "cannot open {} for reading: {}",
                            quoteaf(from),
                            strerror(&e)
                        ),
                    );
                    return 1;
                }
            }
        };
        streamed = !regular;
        let mut parts: Vec<&[u8]> = data.split(|&b| b == 0).collect();
        if data.last() == Some(&0) || data.is_empty() {
            parts.pop();
        }
        names = parts
            .into_iter()
            .map(|n| Some(String::from_utf8_lossy(n).into_owned()))
            .collect();
    } else if operands.is_empty() {
        names.push(None);
    } else {
        names = operands.iter().cloned().map(Some).collect();
    }

    // The width: from what stat says of each, unless only one number for one
    // input, or names that come as they are read.
    let nfiles = if streamed { 0 } else { names.len() };
    let counts_printed = [p.lines, p.words, p.chars, p.bytes, p.width_col]
        .iter()
        .filter(|b| **b)
        .count();
    p.number_width = if total == Total::Only || nfiles == 0 || (nfiles == 1 && counts_printed == 1)
    {
        1
    } else {
        let mut minimum = 1;
        let mut regular_total: u64 = 0;
        for n in &names {
            let m = match n.as_deref() {
                None | Some("-") => gnu::stdin_metadata(),
                Some(name) => fs::metadata(name),
            };
            if let Ok(m) = m {
                if gnu::is_regular(&m) {
                    regular_total = regular_total.saturating_add(m.len());
                } else {
                    minimum = 7;
                }
            }
        }
        regular_total.to_string().len().max(minimum)
    };

    let mut ok = true;
    let mut totals = Counts::default();
    let mut overflow = [false; 4];
    let mut debug_said = false;
    let lines_only = !p.chars && !p.words && !p.width_col && p.lines;
    for (i, name) in names.iter().enumerate() {
        if let (Some(from), Some(n)) = (&files_from, name) {
            if from == "-" && n == "-" {
                error(
                    PROG,
                    &format!(
                        "when reading file names from stdin, no file name of {} allowed",
                        quoteaf(n)
                    ),
                );
                ok = false;
                continue;
            }
            if n.is_empty() {
                error(
                    PROG,
                    &format!("{}:{}: invalid zero-length file name", quotef(from), i + 1),
                );
                ok = false;
                continue;
            }
        } else if name.as_deref() == Some("") {
            error(PROG, "invalid zero-length file name");
            ok = false;
            continue;
        }
        if debug && lines_only && !debug_said {
            debug_said = true;
            error(
                PROG,
                if avx2() {
                    "using avx2 hardware support"
                } else {
                    "avx2 support not detected"
                },
            );
        }
        let (c, err, shown) = match name.as_deref() {
            None | Some("-") => {
                let (c, e) = count(&mut io::stdin().lock(), posixly_correct);
                (c, e, "standard input".to_owned())
            }
            Some(n) => {
                if fs::metadata(n).is_ok_and(|m| m.is_dir()) {
                    // Opened, then not readable: "Is a directory".
                    let e = io::Error::from(io::ErrorKind::IsADirectory);
                    (Counts::default(), Some(e), n.to_owned())
                } else {
                    match File::open(n) {
                        Ok(mut f) => {
                            let (c, e) = count(&mut f, posixly_correct);
                            (c, e, n.to_owned())
                        }
                        Err(e) => {
                            error(PROG, &format!("{}: {}", quotef(n), strerror(&e)));
                            ok = false;
                            continue;
                        }
                    }
                }
            }
        };
        let c = Counts {
            chars: if p.chars { c.chars } else { 0 },
            ..c
        };
        if total != Total::Only {
            write_counts(out, &p, &c, name.as_deref());
        }
        let add = |t: &mut u64, v: u64, o: &mut bool| match t.checked_add(v) {
            Some(s) => *t = s,
            None => *o = true,
        };
        add(&mut totals.lines, c.lines, &mut overflow[0]);
        add(&mut totals.words, c.words, &mut overflow[1]);
        add(&mut totals.chars, c.chars, &mut overflow[2]);
        add(&mut totals.bytes, c.bytes, &mut overflow[3]);
        totals.width = totals.width.max(c.width);
        if let Some(e) = err {
            let _ = out.flush();
            error(PROG, &format!("{}: {}", quotef(&shown), strerror(&e)));
            ok = false;
        }
    }

    let n_args = names.len();
    if total != Total::Never && (total != Total::Auto || 1 < n_args) {
        for (i, what) in [
            "total lines",
            "total words",
            "total characters",
            "total bytes",
        ]
        .iter()
        .enumerate()
        {
            if overflow[i] {
                error(
                    PROG,
                    &format!("{what}: Value too large for defined data type"),
                );
                ok = false;
            }
        }
        if overflow[0] {
            totals.lines = u64::MAX;
        }
        if overflow[1] {
            totals.words = u64::MAX;
        }
        if overflow[2] {
            totals.chars = u64::MAX;
        }
        if overflow[3] {
            totals.bytes = u64::MAX;
        }
        write_counts(
            out,
            &p,
            &totals,
            if total != Total::Only {
                Some("total")
            } else {
                None
            },
        );
    }
    u8::from(!ok)
}

/// `write_counts`: the chosen counts, each right-aligned on the width, then
/// the name (quoted when it holds a newline).
fn write_counts(out: &mut dyn Write, p: &Print, c: &Counts, name: Option<&str>) {
    let w = p.number_width;
    let mut line = String::new();
    for (on, v) in [
        (p.lines, c.lines),
        (p.words, c.words),
        (p.chars, c.chars),
        (p.bytes, c.bytes),
        (p.width_col, c.width),
    ] {
        if on {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(&format!("{v:>w$}"));
        }
    }
    if let Some(n) = name {
        line.push(' ');
        if n.contains('\n') {
            line.push_str(&quotef(n));
        } else {
            line.push_str(n);
        }
    }
    line.push('\n');
    let _ = out.write_all(line.as_bytes());
}

/// Whether the processor has AVX2, as wc's `--debug` says.
fn avx2() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx2")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}
