//! `argon2`: the reference C's `argon2` command (phc-winner-argon2
//! 20190702, `src/run.c`), its arguments read as its `main` reads them, by
//! hand: the salt first, then `-i`, `-d`, `-id`, `-t`, `-m`, `-k`, `-p`,
//! `-l`, `-e`, `-r`, `-v`. The password is stdin, at most 127 bytes, newline
//! and all, as the command reads it. Its messages go to stderr with its exit
//! codes: 1 for an error, 226 with no arguments (its ARGON2_MISSING_ARGS).
//!
//! toolwing's own: `tw argon2 verify ENCODED` checks stdin's password (the
//! command has no way to; "verify" is too short to be its salt), and
//! `--help` prints the usage and succeeds.

use std::io::{self, Read, Write};
use std::time::Instant;

use toolwing_core::Tool as _;
use toolwing_tools::hash;
use toolwing_tools::password::{self, Algorithm, Argon2Spec};

/// `argon2` with no arguments: ARGON2_MISSING_ARGS, -30, as a process exit code.
const MISSING_ARGS: u8 = 226;

/// The reference's password buffer; one this long is refused.
const MAX_PASS_LEN: usize = 128;

pub fn command() -> clap::Command {
    clap::Command::new("argon2")
        .about(password::Argon2Hash.meta().summary)
        .disable_help_flag(true)
        .arg(
            clap::Arg::new("args")
                .value_name("ARGS")
                .num_args(0..)
                .allow_hyphen_values(true)
                .trailing_var_arg(true),
        )
}

fn usage(out: &mut impl Write) {
    let cmd = "tw argon2";
    let _ = write!(
        out,
        "Usage:  {cmd} [-h] salt [-i|-d|-id] [-t iterations] \
         [-m log2(memory in KiB) | -k memory in KiB] [-p parallelism] \
         [-l hash length] [-e|-r] [-v (10|13)]\n\
         \tPassword is read from stdin\n\
         Parameters:\n\
         \tsalt\t\tThe salt to use, at least 8 characters\n\
         \t-i\t\tUse Argon2i (this is the default)\n\
         \t-d\t\tUse Argon2d instead of Argon2i\n\
         \t-id\t\tUse Argon2id instead of Argon2i\n\
         \t-t N\t\tSets the number of iterations to N (default = 3)\n\
         \t-m N\t\tSets the memory usage of 2^N KiB (default 12)\n\
         \t-k N\t\tSets the memory usage of N KiB (default 4096)\n\
         \t-p N\t\tSets parallelism to N threads (default 1)\n\
         \t-l N\t\tSets hash output length to N bytes (default 32)\n\
         \t-e\t\tOutput only encoded hash\n\
         \t-r\t\tOutput only the raw bytes of the hash\n\
         \t-v (10|13)\tArgon2 version (defaults to the most recent version, currently 13)\n\
         \t-h\t\tPrint {cmd} usage\n"
    );
}

/// The reference's `fatal`: what was written so far, then the message, 1.
struct Fatal(&'static str);

/// C's `strtoul(s, NULL, 10)` on a 64-bit system: leading white space and a
/// sign, the digits there are (0 if none), ULONG_MAX past it, and a minus
/// sign wrapping the value round.
fn strtoul(s: &str) -> u64 {
    let s = s.trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r']);
    let (negative, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let digits = s.bytes().take_while(u8::is_ascii_digit);
    let mut value: u64 = 0;
    for d in digits {
        match value
            .checked_mul(10)
            .and_then(|v| v.checked_add(u64::from(d - b'0')))
        {
            Some(v) => value = v,
            None => return u64::MAX,
        }
    }
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}

struct Options {
    spec: Argon2Spec,
    encoded_only: bool,
    raw_only: bool,
}

/// `main`'s loop over the options after the salt.
fn options(args: &[String]) -> Result<Result<Options, Fatal>, ()> {
    let mut spec = Argon2Spec::default();
    let (mut types, mut m_given, mut encoded_only, mut raw_only) = (0, false, false, false);
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let value = args.get(i + 1);
        // An option's number: the next argument, taken whatever it is.
        let number = |missing, bad, ok: fn(u64) -> bool| -> Result<u64, Fatal> {
            let v = value.ok_or(Fatal(missing))?;
            let n = strtoul(v);
            if n == 0 || n == u64::MAX || !ok(n) {
                return Err(Fatal(bad));
            }
            Ok(n)
        };
        let step = match a {
            // `-h` anywhere prints the usage, and stops there with 1.
            "-h" => return Err(()),
            "-m" | "-k" if m_given => Err(Fatal("-m or -k can only be used once")),
            "-m" => {
                m_given = true;
                number("missing -m argument", "bad numeric input for -m", |n| {
                    n <= 32
                })
                .map(|n| spec.m_cost = (1u64 << n).min(0xFFFF_FFFF) as u32)
            }
            "-k" => {
                m_given = true;
                number("missing -k argument", "bad numeric input for -k", |_| true)
                    .map(|n| spec.m_cost = n.min(0xFFFF_FFFF) as u32)
            }
            "-t" => number("missing -t argument", "bad numeric input for -t", |n| {
                n <= 0xFFFF_FFFF
            })
            .map(|n| spec.t_cost = n as u32),
            "-p" => number("missing -p argument", "bad numeric input for -p", |n| {
                n <= 0xFF_FFFF
            })
            .map(|n| spec.lanes = n as u32),
            // Taken as it is, cut to 32 bits, as the reference stores it.
            "-l" => match value {
                Some(v) => {
                    spec.out_len = strtoul(v) as u32;
                    Ok(())
                }
                None => Err(Fatal("missing -l argument")),
            },
            "-i" | "-d" | "-id" => {
                spec.algorithm = match a {
                    "-i" => Algorithm::Argon2i,
                    "-d" => Algorithm::Argon2d,
                    _ => Algorithm::Argon2id,
                };
                types += 1;
                Ok(())
            }
            "-e" => {
                encoded_only = true;
                Ok(())
            }
            "-r" => {
                raw_only = true;
                Ok(())
            }
            "-v" => match value.map(String::as_str) {
                Some(v) => match v {
                    "10" => {
                        spec.version = 0x10;
                        Ok(())
                    }
                    "13" => {
                        spec.version = 0x13;
                        Ok(())
                    }
                    _ => Err(Fatal("invalid Argon2 version")),
                },
                None => Err(Fatal("missing -v argument")),
            },
            _ => Err(Fatal("unknown argument")),
        };
        if let Err(f) = step {
            return Ok(Err(f));
        }
        // An option with a value takes the next argument too.
        i += if matches!(a, "-m" | "-k" | "-t" | "-p" | "-l" | "-v") {
            2
        } else {
            1
        };
    }
    if types > 1 {
        return Ok(Err(Fatal("cannot specify multiple Argon2 types")));
    }
    if encoded_only && raw_only {
        return Ok(Err(Fatal("cannot provide both -e and -r")));
    }
    Ok(Ok(Options {
        spec,
        encoded_only,
        raw_only,
    }))
}

/// stdin as `fread(pwd, 1, 128, stdin)` reads it: up to 128 bytes.
fn read_password() -> Result<Vec<u8>, Fatal> {
    let mut pwd = Vec::with_capacity(MAX_PASS_LEN);
    let _ = io::stdin()
        .lock()
        .take(MAX_PASS_LEN as u64)
        .read_to_end(&mut pwd);
    if pwd.is_empty() {
        return Err(Fatal("no password read"));
    }
    if pwd.len() == MAX_PASS_LEN {
        return Err(Fatal(
            "Provided password longer than supported in command line utility",
        ));
    }
    Ok(pwd)
}

/// Runs `tw argon2`; the exit code. `args` are those after `argon2`.
pub fn main(args: &[String]) -> u8 {
    let mut out = io::BufWriter::new(crate::stdout());
    let code = match run(args, &mut out) {
        Ok(code) => code,
        Err(Fatal(message)) => {
            let _ = out.flush();
            eprintln!("Error: {message}");
            1
        }
    };
    let _ = out.flush();
    code
}

fn run(args: &[String], out: &mut impl Write) -> Result<u8, Fatal> {
    match args.first().map(String::as_str) {
        None => {
            usage(out);
            return Ok(MISSING_ARGS);
        }
        Some("-h") => {
            usage(out);
            return Ok(1);
        }
        Some("--help") => {
            usage(out);
            return Ok(0);
        }
        Some("verify") => return verify(&args[1..], out),
        Some(_) => {}
    }
    let pwd = read_password()?;
    let salt = args[0].as_bytes();
    let Options {
        spec,
        encoded_only,
        raw_only,
    } = match options(&args[1..]) {
        Ok(o) => o?,
        Err(()) => {
            usage(out);
            return Ok(1);
        }
    };
    if !encoded_only && !raw_only {
        let _ = write!(
            out,
            "Type:\t\t{}\nIterations:\t{}\nMemory:\t\t{} KiB\nParallelism:\t{}\n",
            password::argon2_title(spec.algorithm),
            spec.t_cost,
            spec.m_cost,
            spec.lanes
        );
    }
    let start = Instant::now();
    let (raw, encoded) = password::argon2_hash(&spec, &pwd, salt).map_err(Fatal)?;
    let took = start.elapsed();
    let hex = hash::hex(&raw);
    if encoded_only {
        let _ = writeln!(out, "{encoded}");
    }
    if raw_only {
        let _ = writeln!(out, "{hex}");
    }
    if encoded_only || raw_only {
        return Ok(0);
    }
    let _ = write!(out, "Hash:\t\t{hex}\nEncoded:\t{encoded}\n");
    // The reference's clock() counts processor time; this is the time taken.
    let _ = writeln!(out, "{:2.3} seconds", took.as_secs_f64());
    if password::argon2_verify(&encoded, &pwd) != Ok(true) {
        return Err(Fatal("The password does not match the supplied hash"));
    }
    let _ = writeln!(out, "Verification ok");
    Ok(0)
}

/// `tw argon2 verify ENCODED`: "Verification ok" and 0 when stdin's
/// password is the hash's, the reference's words for why not and 1 when not.
fn verify(args: &[String], out: &mut impl Write) -> Result<u8, Fatal> {
    let [encoded] = args else {
        return Err(Fatal(
            "verify takes one encoded hash: tw argon2 verify '$argon2id$…'",
        ));
    };
    password::argon2_decode(encoded).map_err(Fatal)?;
    let pwd = read_password()?;
    match password::argon2_verify(encoded, &pwd) {
        Ok(true) => {
            let _ = writeln!(out, "Verification ok");
            Ok(0)
        }
        Ok(false) => Err(Fatal("The password does not match the supplied hash")),
        Err(e) => Err(Fatal(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The app's "copy as command" for Argon2 runs what the app ran.
    #[test]
    fn copied_commands_parse_back() {
        use toolwing_core::{Tool, Value, Values};
        let hash = &password::Argon2Hash;
        let mut samples = Vec::new();
        for (key, value) in [
            ("type", Value::Text("i".into())),
            ("type", Value::Text("d".into())),
            ("type", Value::Text("id".into())),
            ("iterations", Value::Int(1)),
            ("iterations", Value::Int(0xFFFF_FFFF)),
            ("memory", Value::Int(8)),
            ("memory", Value::Int(0xFFFF_FFFF)),
            ("parallelism", Value::Int(0xFF_FFFF)),
            ("length", Value::Int(4)),
            ("length", Value::Int(0xFFFF_FFFF)),
            ("version", Value::Text("10".into())),
            ("version", Value::Text("13".into())),
        ] {
            let mut v = Values::new();
            v.set("salt", Value::Text("some salt".into()));
            v.set(key, value);
            samples.push(v);
        }
        for v in samples {
            let args = hash.command_args(&v).unwrap_or_default();
            assert_eq!(args[..2], ["argon2", "some salt"]);
            let parsed = options(&args[2..])
                .ok()
                .and_then(Result::ok)
                .map(|o| o.spec);
            assert_eq!(parsed, password::argon2_spec(&v).ok(), "{args:?}");
        }
        let mut v = Values::new();
        v.set(
            "encoded",
            Value::Text("$argon2id$v=19$m=8,t=1,p=1$c29tZXNhbHQ$AAAAAA".into()),
        );
        let args = password::Argon2Verify.command_args(&v).unwrap_or_default();
        assert_eq!(
            args,
            [
                "argon2",
                "verify",
                "$argon2id$v=19$m=8,t=1,p=1$c29tZXNhbHQ$AAAAAA"
            ]
        );
    }

    #[test]
    fn numbers_read_as_strtoul_reads_them() {
        assert_eq!(strtoul("12"), 12);
        assert_eq!(strtoul("  +7x"), 7);
        assert_eq!(strtoul("x"), 0);
        assert_eq!(strtoul("0x10"), 0);
        assert_eq!(strtoul("-1"), u64::MAX);
        assert_eq!(strtoul("-2"), u64::MAX - 1);
        assert_eq!(strtoul("99999999999999999999999"), u64::MAX);
        assert_eq!(strtoul("4294967296"), 1 << 32);
    }
}
