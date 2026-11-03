//! `b3sum`: b3sum 1.8.2's command line, written by hand as the coreutils
//! hash commands are (architecture.md): BLAKE3 of files, keyed or for a
//! derived key, any length of output from any offset, raw or as BSD tags,
//! and checking lists. Usage errors exit 2 (b3sum is clap's too); a file
//! that cannot be read or a check that fails exits 1, the others done.
//! Only stdout and the exit code are compared with b3sum (tests/compat).

use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};

use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};
use toolwing_core::{Ctx, Tool as _};
use toolwing_tools::hash::{self, B3SUM, Blake3Mode};

pub const USAGE: u8 = 2;

pub fn command() -> Command {
    let flag = |id: &'static str, help: &'static str| {
        Arg::new(id).long(id).help(help).action(ArgAction::SetTrue)
    };
    Command::new("b3sum")
        .about(B3SUM.meta().summary)
        .arg(
            Arg::new("files")
                .value_name("FILE")
                .num_args(0..)
                .help("Files to hash, or lists to check; stdin when none, or -"),
        )
        .arg(flag("keyed", "Keyed mode, the 32-byte key read from stdin"))
        .arg(
            Arg::new("derive-key")
                .long("derive-key")
                .value_name("CONTEXT")
                .help("Key derivation mode, for the given context string")
                .conflicts_with("keyed"),
        )
        .arg(
            Arg::new("length")
                .short('l')
                .long("length")
                .value_name("LEN")
                .value_parser(value_parser!(u64))
                .default_value("32")
                .help("Bytes of output, before hex"),
        )
        .arg(
            Arg::new("seek")
                .long("seek")
                .value_name("SEEK")
                .value_parser(value_parser!(u64))
                .default_value("0")
                .help("Byte of the output to start from"),
        )
        .arg(
            Arg::new("num-threads")
                .long("num-threads")
                .value_name("NUM")
                .value_parser(value_parser!(u64))
                .help("Accepted as b3sum accepts it; tw hashes on one thread"),
        )
        .arg(flag(
            "no-mmap",
            "Accepted as b3sum accepts it; tw reads files as streams",
        ))
        .arg(flag("no-names", "Leave the file names out"))
        .arg(flag("raw", "The output's bytes, not hex; one input only").conflicts_with("check"))
        .arg(flag("tag", "BSD-style lines: BLAKE3 (FILE) = HASH"))
        .arg(
            Arg::new("check")
                .short('c')
                .long("check")
                .help("Check the sums listed in the FILEs")
                .action(ArgAction::SetTrue)
                // As b3sum: a list is checked plainly, at full length.
                .conflicts_with_all(["keyed", "derive-key", "tag", "no-names", "length", "raw"]),
        )
        .arg(flag("quiet", "With --check, leave out the OK lines").requires("check"))
}

/// Runs `tw b3sum`; the exit code.
pub fn run(m: &ArgMatches) -> u8 {
    let files: Vec<String> = match m.get_many::<String>("files") {
        Some(f) => f.cloned().collect(),
        None => vec!["-".to_owned()],
    };
    let mode = if m.get_flag("keyed") {
        let mut key = Vec::new();
        if let Err(e) = io::stdin().lock().read_to_end(&mut key) {
            eprintln!("Error: {e}");
            return 1;
        }
        match <[u8; 32]>::try_from(key.as_slice()) {
            Ok(key) => Blake3Mode::Keyed(key),
            Err(_) => {
                eprintln!(
                    "Error: expected 32 key bytes from stdin, found {}",
                    key.len()
                );
                return 1;
            }
        }
    } else if let Some(context) = m.get_one::<String>("derive-key") {
        Blake3Mode::DeriveKey(context.clone())
    } else {
        Blake3Mode::Hash
    };
    let length = m.get_one::<u64>("length").copied().unwrap_or(32);
    let Ok(length) = usize::try_from(length) else {
        eprintln!("Error: length too large");
        return 1;
    };
    let seek = m.get_one::<u64>("seek").copied().unwrap_or(0);
    let mut out = BufWriter::new(crate::stdout());
    let code = if m.get_flag("check") {
        check(&files, seek, m.get_flag("quiet"), &mut out)
    } else if m.get_flag("raw") {
        if files.len() > 1 {
            eprintln!("Error: Only one filename can be provided when using --raw");
            return 1;
        }
        match digest(&mode, &files[0], seek, length) {
            Ok(d) => {
                let _ = out.write_all(&d);
                0
            }
            Err(e) => {
                eprintln!("b3sum: {}: {e}", files[0]);
                1
            }
        }
    } else {
        let style = Lines {
            names: !m.get_flag("no-names"),
            tag: m.get_flag("tag"),
        };
        let mut code = 0;
        for file in &files {
            match digest(&mode, file, seek, length) {
                Ok(d) => {
                    let _ = out.write_all(&style.line(&d, file));
                }
                Err(e) => {
                    let _ = out.flush();
                    eprintln!("b3sum: {file}: {e}");
                    code = 1;
                }
            }
        }
        code
    };
    let _ = out.flush();
    code
}

fn digest(mode: &Blake3Mode, file: &str, seek: u64, length: usize) -> io::Result<Vec<u8>> {
    let mut input: Box<dyn Read> = if file == "-" {
        Box::new(io::stdin().lock())
    } else {
        Box::new(File::open(file)?)
    };
    hash::blake3_reader(&Ctx::new(), mode, &mut input, seek, length)
        .map_err(|e| io::Error::other(e.message))
}

struct Lines {
    names: bool,
    tag: bool,
}

impl Lines {
    /// One line, as b3sum writes it: a name with `\` or a newline in it is
    /// escaped, and the line starts with `\`, as coreutils marks it.
    fn line(&self, digest: &[u8], name: &str) -> Vec<u8> {
        let hex = hash::hex(digest);
        if !self.names {
            return format!("{hex}\n").into_bytes();
        }
        let escape = name.contains(['\\', '\n']);
        let shown = if escape {
            name.replace('\\', r"\\").replace('\n', r"\n")
        } else {
            name.to_owned()
        };
        let mark = if escape { "\\" } else { "" };
        if self.tag {
            format!("{mark}BLAKE3 ({shown}) = {hex}\n").into_bytes()
        } else {
            format!("{mark}{hex}  {shown}\n").into_bytes()
        }
    }
}

/// A list's line: the hex and the file it is of.
fn parse_line(line: &str) -> Result<(String, String), &'static str> {
    let (escaped, line) = match line.strip_prefix('\\') {
        Some(rest) => (true, rest),
        None => (false, line),
    };
    let (hex, name) = if let Some(rest) = line.strip_prefix("BLAKE3 (") {
        let (name, hex) = rest
            .rsplit_once(") = ")
            .ok_or("Invalid check line format")?;
        (hex, name)
    } else {
        line.split_once("  ").ok_or("Invalid check line format")?
    };
    if hex.len() != 64 {
        return Err("Invalid hash length");
    }
    // Lowercase, as b3sum writes it and reads it.
    if !hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err("Invalid check line format");
    }
    let name = if escaped {
        unescape(name).ok_or("Invalid check line format")?
    } else {
        name.to_owned()
    };
    Ok((hex.to_owned(), name))
}

fn unescape(name: &str) -> Option<String> {
    let mut out = String::with_capacity(name.len());
    let mut chars = name.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            _ => return None,
        }
    }
    Some(out)
}

/// `name` as b3sum shows it: `\` and newlines escaped, and a `\` before.
fn shown(name: &str) -> String {
    if name.contains(['\\', '\n']) {
        format!("\\{}", name.replace('\\', r"\\").replace('\n', r"\n"))
    } else {
        name.to_owned()
    }
}

/// Checks each list, from byte `seek` of each hash as b3sum does; 1 when a
/// line failed or did not parse.
fn check(lists: &[String], seek: u64, quiet: bool, out: &mut dyn Write) -> u8 {
    let mut failed = 0usize;
    let mut code = 0;
    for list in lists {
        let reader: Box<dyn BufRead> = if list == "-" {
            Box::new(BufReader::new(io::stdin()))
        } else {
            match File::open(list) {
                Ok(f) => Box::new(BufReader::new(f)),
                Err(e) => {
                    eprintln!("b3sum: {list}: {e}");
                    code = 1;
                    continue;
                }
            }
        };
        for line in reader.split(b'\n') {
            let Ok(line) = line else { break };
            let line = String::from_utf8_lossy(&line);
            let line = line.strip_suffix('\r').unwrap_or(&line);
            let (want, name) = match parse_line(line) {
                Ok(p) => p,
                Err(why) => {
                    let _ = out.flush();
                    eprintln!("b3sum: {why}");
                    failed += 1;
                    continue;
                }
            };
            let name_shown = shown(&name);
            let result = match digest(&Blake3Mode::Hash, &name, seek, 32) {
                Ok(d) if hash::hex(&d) == want => (!quiet).then(|| format!("{name_shown}: OK\n")),
                Ok(_) => {
                    failed += 1;
                    Some(format!("{name_shown}: FAILED\n"))
                }
                Err(e) => {
                    failed += 1;
                    Some(format!("{name_shown}: FAILED ({e})\n"))
                }
            };
            if let Some(r) = result {
                let _ = out.write_all(r.as_bytes());
            }
        }
    }
    if failed > 0 {
        let _ = out.flush();
        let s = if failed == 1 { "" } else { "s" };
        eprintln!("b3sum: WARNING: {failed} computed checksum{s} did NOT match");
        code = 1;
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_lines_parse_as_b3sum_reads_them() {
        let hex = "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85";
        assert_eq!(
            parse_line(&format!("{hex}  a")),
            Ok((hex.into(), "a".into()))
        );
        assert_eq!(
            parse_line(&format!("BLAKE3 (a b) = {hex}")),
            Ok((hex.into(), "a b".into()))
        );
        assert_eq!(
            parse_line(&format!("\\{hex}  new\\nline")),
            Ok((hex.into(), "new\nline".into()))
        );
        assert_eq!(parse_line("garbage line"), Err("Invalid check line format"));
        assert_eq!(
            parse_line("6437b3ac38465133  a"),
            Err("Invalid hash length")
        );
    }

    #[test]
    fn lines_escape_names_as_b3sum_does() {
        let d = [0xab; 2];
        let plain = Lines {
            names: true,
            tag: false,
        };
        assert_eq!(plain.line(&d, "new\nline"), b"\\abab  new\\nline\n");
        let tag = Lines {
            names: true,
            tag: true,
        };
        assert_eq!(tag.line(&d, "a"), b"BLAKE3 (a) = abab\n");
        let bare = Lines {
            names: false,
            tag: false,
        };
        assert_eq!(bare.line(&d, "a"), b"abab\n");
    }
}
