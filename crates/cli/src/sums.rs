//! `md5sum`, `sha1sum`, `sha256sum`, `sha512sum` and `b2sum`: GNU coreutils
//! 9.7's command lines, written by hand rather than made from the registry
//! (architecture.md), with their lines, their checking and their exit codes.
//! What goes to stderr follows GNU's wording; only stdout and the exit code
//! are compared with it (tests/compat).

use std::fs::File;
use std::io::{self, BufWriter, Read, Write};

use clap::{Arg, ArgAction, ArgMatches, Command};
use toolwing_core::{Ctx, Tool as _};
use toolwing_tools::hash::{self, Algorithm, Line, SUMS, Separators, Style, Sum};

pub fn find(name: &str) -> Option<&'static Sum> {
    SUMS.iter().copied().find(|s| s.meta().id == name)
}

pub fn command(sum: &'static Sum) -> Command {
    let flag = |id: &'static str, long: &'static str, help: &'static str| {
        Arg::new(id)
            .long(long)
            .help(help)
            .action(ArgAction::SetTrue)
    };
    let mut cmd = Command::new(sum.meta().id)
        .about(sum.meta().summary)
        .arg(
            Arg::new("files")
                .value_name("FILE")
                .num_args(0..)
                .help("The files; - or none for stdin"),
        )
        .arg(flag("binary", "binary", "Read in binary mode: * before the name").short('b'))
        .arg(
            flag(
                "check",
                "check",
                "Read checksums from the FILEs and check them",
            )
            .short('c'),
        )
        .arg(flag(
            "tag",
            "tag",
            "Write BSD-style lines: TAG (NAME) = HEX",
        ))
        .arg(flag("text", "text", "Read in text mode (the default)").short('t'))
        .arg(
            flag(
                "zero",
                "zero",
                "End each line with NUL, not newline, and do not escape names",
            )
            .short('z'),
        )
        .arg(flag(
            "ignore_missing",
            "ignore-missing",
            "Checking: skip files that do not exist",
        ))
        .arg(flag(
            "quiet",
            "quiet",
            "Checking: do not print OK for each file",
        ))
        .arg(flag(
            "status",
            "status",
            "Checking: print nothing; the exit code tells",
        ))
        .arg(flag(
            "strict",
            "strict",
            "Checking: fail on improperly formatted lines",
        ))
        .arg(
            flag(
                "warn",
                "warn",
                "Checking: warn about improperly formatted lines",
            )
            .short('w'),
        );
    if matches!(sum.algorithm(), Algorithm::Blake2b(_)) {
        cmd = cmd.arg(
            Arg::new("length")
                .short('l')
                .long("length")
                .value_name("BITS")
                .help("The digest length in bits: a multiple of 8, up to 512")
                .allow_hyphen_values(true),
        );
    }
    cmd
}

/// Runs the command; the exit code, as GNU gives it.
pub fn run(sum: &'static Sum, m: &ArgMatches) -> u8 {
    let name = sum.meta().id;
    let say = |message: &str| eprintln!("tw: {name}: {message}");
    let on = |id: &str| m.get_flag(id);
    let usage = |message: &str| {
        say(message);
        eprintln!("Try 'tw {name} --help' for more information.");
        1
    };
    if on("check") {
        if on("tag") {
            return usage("the --tag option is meaningless when verifying checksums");
        }
        if on("zero") {
            return usage("the --zero option is not supported when verifying checksums");
        }
    } else {
        for (id, long) in [
            ("ignore_missing", "ignore-missing"),
            ("quiet", "quiet"),
            ("status", "status"),
            ("strict", "strict"),
            ("warn", "warn"),
        ] {
            if on(id) {
                return usage(&format!(
                    "the --{long} option is meaningful only when verifying checksums"
                ));
            }
        }
    }
    if on("tag") && on("text") {
        return usage("--tag does not support --text mode");
    }
    let mut algorithm = sum.algorithm();
    if let Some(text) = m.try_get_one::<String>("length").ok().flatten() {
        match length_bits(text) {
            Some(bits) if bits.is_multiple_of(8) => algorithm = Algorithm::Blake2b(bits),
            Some(_) => {
                say(&format!("invalid length: '{text}'"));
                say("length is not a multiple of 8");
                return 1;
            }
            None => {
                say(&format!("invalid length: '{text}'"));
                return 1;
            }
        }
    }
    let files: Vec<String> = m
        .get_many::<String>("files")
        .map(|f| f.cloned().collect())
        .unwrap_or_else(|| vec!["-".into()]);
    let mut out = BufWriter::new(io::stdout().lock());
    let ok = if on("check") {
        let options = CheckOptions {
            ignore_missing: on("ignore_missing"),
            quiet: on("quiet"),
            status: on("status"),
            strict: on("strict"),
            warn: on("warn"),
        };
        // As GNU's bsd_reversed: one for the whole run.
        let mut separators = Separators::default();
        files
            .iter()
            .map(|list| {
                check_list(
                    name,
                    sum.algorithm(),
                    list,
                    options,
                    &mut separators,
                    &mut out,
                )
            })
            // Every file, even after a failure, as GNU goes on: no short circuit.
            .filter(|ok| !ok)
            .count()
            == 0
    } else {
        let style = Style {
            tag: on("tag"),
            binary: on("binary") && !on("text"),
            zero: on("zero"),
        };
        files
            .iter()
            .map(|file| match digest_file(algorithm, file) {
                Ok(digest) => out
                    .write_all(&hash::format_line(algorithm, &digest, file, style))
                    .is_ok(),
                Err(e) => {
                    say(&format!("{file}: {}", io_message(&e)));
                    false
                }
            })
            // Every file, even after a failure, as GNU goes on: no short circuit.
            .filter(|ok| !ok)
            .count()
            == 0
    };
    let flushed = out.flush().is_ok();
    if ok && flushed { 0 } else { 1 }
}

/// `-l` as GNU reads it (`xnumtoumax` with `XTOINT_MAX_QUIET`): a decimal
/// number, leading white space and `+` allowed; more than 512 is 512, and 0
/// is the default, 512.
fn length_bits(text: &str) -> Option<u16> {
    let digits = text.trim_start();
    let digits = digits.strip_prefix('+').unwrap_or(digits);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let bits = digits.trim_start_matches('0');
    if bits.is_empty() || bits.len() > 3 {
        return Some(512);
    }
    let bits: u16 = bits.parse().ok()?;
    Some(if bits > 512 { 512 } else { bits })
}

fn digest_file(algorithm: Algorithm, file: &str) -> io::Result<Vec<u8>> {
    let mut input: Box<dyn Read> = if file == "-" {
        Box::new(io::stdin().lock())
    } else {
        Box::new(File::open(file)?)
    };
    hash::digest_reader(&Ctx::new(), algorithm, &mut input).map_err(|e| io::Error::other(e.message))
}

/// The system's message without Rust's "(os error N)", as GNU words it.
fn io_message(e: &io::Error) -> String {
    let text = e.to_string();
    match text.find(" (os error") {
        Some(i) => text[..i].to_owned(),
        None => text,
    }
}

#[derive(Clone, Copy)]
struct CheckOptions {
    ignore_missing: bool,
    quiet: bool,
    status: bool,
    strict: bool,
    warn: bool,
}

/// Checks one list; whether everything in it checked out.
fn check_list(
    command: &str,
    family: Algorithm,
    list: &str,
    o: CheckOptions,
    separators: &mut Separators,
    out: &mut dyn Write,
) -> bool {
    let say = |message: &str| eprintln!("tw: {command}: {message}");
    let shown_list = if list == "-" {
        "'standard input'".to_owned()
    } else {
        list.to_owned()
    };
    let mut bytes = Vec::new();
    let read = if list == "-" {
        io::stdin().lock().read_to_end(&mut bytes)
    } else {
        File::open(list).and_then(|mut f| f.read_to_end(&mut bytes))
    };
    if let Err(e) = read {
        say(&format!("{list}: {}", io_message(&e)));
        return false;
    }
    let (mut proper, mut improper, mut unreadable, mut mismatched, mut verified) = (0, 0, 0, 0, 0);
    for (i, line) in bytes.split(|&b| b == b'\n').enumerate() {
        let parsed = match hash::parse_line(line, family, separators) {
            // A list read from stdin cannot name stdin.
            Line::Checksum(c) if list == "-" && c.name == "-" => Line::Improper,
            other => other,
        };
        let checksum = match parsed {
            Line::Skip => continue,
            Line::Improper => {
                improper += 1;
                if o.warn {
                    say(&format!(
                        "{list}: {}: improperly formatted {} checksum line",
                        i + 1,
                        family.tag()
                    ));
                }
                continue;
            }
            Line::Checksum(c) => c,
        };
        proper += 1;
        let shown = if hash::needs_escape(&checksum.name) {
            format!("\\{}", hash::escape_name(&checksum.name))
        } else {
            checksum.name.clone()
        };
        let mut print = |status: &str| {
            let _ = writeln!(out, "{shown}: {status}");
        };
        match digest_file(checksum.algorithm, &checksum.name) {
            Err(e) if o.ignore_missing && e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                say(&format!("{}: {}", checksum.name, io_message(&e)));
                if !o.status {
                    print("FAILED open or read");
                }
                unreadable += 1;
            }
            Ok(digest) if digest == checksum.digest => {
                verified += 1;
                if !o.quiet && !o.status {
                    print("OK");
                }
            }
            Ok(_) => {
                mismatched += 1;
                if !o.status {
                    print("FAILED");
                }
            }
        }
    }
    if proper == 0 {
        say(&format!(
            "{shown_list}: no properly formatted checksum lines found"
        ));
        return false;
    }
    if !o.status {
        let plural = |n: i32, one: &str, many: &str| {
            if n == 1 {
                one.to_owned()
            } else {
                many.to_owned()
            }
        };
        if improper > 0 {
            say(&format!(
                "WARNING: {improper} {}",
                plural(
                    improper,
                    "line is improperly formatted",
                    "lines are improperly formatted"
                )
            ));
        }
        if unreadable > 0 {
            say(&format!(
                "WARNING: {unreadable} {}",
                plural(
                    unreadable,
                    "listed file could not be read",
                    "listed files could not be read"
                )
            ));
        }
        if mismatched > 0 {
            say(&format!(
                "WARNING: {mismatched} {}",
                plural(
                    mismatched,
                    "computed checksum did NOT match",
                    "computed checksums did NOT match"
                )
            ));
        }
    }
    let mut ok = mismatched == 0 && unreadable == 0 && !(o.strict && improper > 0);
    if o.ignore_missing && verified == 0 && ok {
        say(&format!("{shown_list}: no file was verified"));
        ok = false;
    }
    ok
}
