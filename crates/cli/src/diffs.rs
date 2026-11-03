//! `diff`: GNU diffutils 3.10's command line, written by hand as the hash
//! commands are (architecture.md): two files, its normal and unified formats,
//! `-q`, `-s`, the ignore flags, `--label`, and its exit codes: 0 the same, 1
//! different, 2 trouble.

use std::io::{self, Read, Write};

use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};
use toolwing_tools::diff::{self, Options};

/// GNU diff's exit code for trouble, a usage error included.
pub const TROUBLE: u8 = 2;

pub fn command() -> Command {
    let flag = |id: &'static str, short: char, long: &'static str, help: &'static str| {
        Arg::new(id)
            .short(short)
            .long(long)
            .help(help)
            .action(ArgAction::SetTrue)
    };
    Command::new("diff")
        .about("Compare two files line by line, as GNU diff does")
        .arg(
            Arg::new("files")
                .value_name("FILE")
                .num_args(0..=2)
                .help("The two files; - for stdin"),
        )
        .arg(
            Arg::new("unified_default")
                .short('u')
                .help("Unified format, 3 lines of context")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("unified")
                .short('U')
                .long("unified")
                .value_name("NUM")
                .help("Unified format with NUM lines of context (3 when not given)")
                .num_args(0..=1)
                .require_equals(false)
                .default_missing_value("3")
                .value_parser(value_parser!(i64).range(0..=1_000)),
        )
        .arg(
            Arg::new("label")
                .long("label")
                .value_name("LABEL")
                .action(ArgAction::Append)
                .help("Name a file LABEL in the unified header; twice for both"),
        )
        .arg(flag(
            "brief",
            'q',
            "brief",
            "Say only whether the files differ",
        ))
        .arg(flag(
            "report_identical",
            's',
            "report-identical-files",
            "Say so when the files are the same",
        ))
        .arg(flag(
            "ignore_case",
            'i',
            "ignore-case",
            "Lines that differ in case only are the same",
        ))
        .arg(flag(
            "ignore_all_space",
            'w',
            "ignore-all-space",
            "White space is not compared at all",
        ))
        .arg(flag(
            "ignore_space_change",
            'b',
            "ignore-space-change",
            "Runs of white space are one space, and trailing white space is none",
        ))
}

fn read(name: &str) -> io::Result<Vec<u8>> {
    if name == "-" {
        let mut all = Vec::new();
        io::stdin().lock().read_to_end(&mut all)?;
        Ok(all)
    } else {
        std::fs::read(name)
    }
}

/// `--- NAME\tMTIME`, the time as GNU writes it, in this computer's zone.
fn header(name: &str, label: Option<&String>) -> String {
    if let Some(label) = label {
        return label.clone();
    }
    let modified = if name == "-" {
        Some(std::time::SystemTime::now())
    } else {
        std::fs::metadata(name).and_then(|m| m.modified()).ok()
    };
    let stamp = modified
        .and_then(|t| jiff::Timestamp::try_from(t).ok())
        .map(|t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y-%m-%d %H:%M:%S.%N %z")
                .to_string()
        })
        .unwrap_or_default();
    format!("{name}\t{stamp}")
}

pub fn run(m: &ArgMatches) -> u8 {
    let say = |message: &str| eprintln!("tw: diff: {message}");
    let files: Vec<&String> = m
        .get_many::<String>("files")
        .map(|f| f.collect())
        .unwrap_or_default();
    let (left, right) = match files.as_slice() {
        [a, b] => (a.as_str(), b.as_str()),
        [] => {
            say("missing operand");
            return TROUBLE;
        }
        [a] => {
            say(&format!("missing operand after '{a}'"));
            return TROUBLE;
        }
        _ => return TROUBLE,
    };
    let (a, b) = match (read(left), read(right)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) => {
            say(&format!("{left}: {}", without_os_error(&e)));
            return TROUBLE;
        }
        (_, Err(e)) => {
            say(&format!("{right}: {}", without_os_error(&e)));
            return TROUBLE;
        }
    };
    let o = Options {
        ignore_case: m.get_flag("ignore_case"),
        ignore_all_space: m.get_flag("ignore_all_space"),
        ignore_space_change: m.get_flag("ignore_space_change"),
    };
    let mut out = io::stdout().lock();
    let identical = |out: &mut io::StdoutLock| {
        if m.get_flag("report_identical") {
            let _ = writeln!(out, "Files {left} and {right} are identical");
        }
        0
    };
    // As GNU, a NUL in either file makes both binary: compared, not shown.
    if a.contains(&0) || b.contains(&0) {
        if a == b {
            return identical(&mut out);
        }
        let _ = writeln!(out, "Binary files {left} and {right} differ");
        return 1;
    }
    let (la, lb) = (diff::lines(&a), diff::lines(&b));
    if m.get_flag("brief") {
        if diff::ops(&la, &lb, o).is_empty() {
            return identical(&mut out);
        }
        let _ = writeln!(out, "Files {left} and {right} differ");
        return 1;
    }
    let context = if m.get_flag("unified_default") {
        Some(3)
    } else {
        m.get_one::<i64>("unified")
            .map(|n| usize::try_from(*n).unwrap_or(3))
    };
    let written = match context {
        Some(context) => {
            let labels: Vec<&String> = m
                .get_many::<String>("label")
                .map(|l| l.collect())
                .unwrap_or_default();
            let headers = (
                header(left, labels.first().copied()),
                header(right, labels.get(1).copied()),
            );
            diff::unified(&la, &lb, o, context, (&headers.0, &headers.1))
        }
        None => diff::normal(&la, &lb, o),
    };
    match written {
        None => identical(&mut out),
        Some(text) => {
            if out.write_all(&text).and_then(|()| out.flush()).is_err() {
                return TROUBLE;
            }
            1
        }
    }
}

fn without_os_error(e: &io::Error) -> String {
    let text = e.to_string();
    text.find(" (os error")
        .map_or(text.clone(), |i| text[..i].to_owned())
}
