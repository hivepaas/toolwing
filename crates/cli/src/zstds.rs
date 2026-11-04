//! `zstd`: zstd 1.5.7's command line, its arguments read as its own parser
//! (zstdcli.c) reads them, aggregated short options and all, and its files
//! handled as fileio.c handles them: names (`.zst` added, `.zst` and
//! `.tzst` taken off), overwriting, `--rm`, a file's mode and time kept,
//! several inputs into one `-o` output, `--test` and `--list`. Frames come
//! from libzstd itself, with its parameters (toolwing_tools::zstd), so they
//! are the command's byte for byte.
//!
//! Not done, and refused: benchmarking (`-b`), dictionary training
//! (`--train`), `--patch-from`, `--zstd=`, other formats (`--format=gzip`
//! and the like; nor are `.gz`, `.xz`, `.lz4` read), `--adapt`,
//! `--rsyncable`, `-r`, `--filelist`, `--output-dir-*`.

use std::fs::File;
use std::io::{self, BufRead, IsTerminal, Read, Write};

use toolwing_tools::zstd::{self as z, Compress, Decode, Decompress, InfoError};

const STDIN: &str = "/*stdin*\\";
const STDOUT: &str = "/*stdout*\\";
const NULL: &str = "/dev/null";

pub fn command() -> clap::Command {
    clap::Command::new("zstd")
        .about(toolwing_tools::group_info("zstd").map_or("", |g| g.summary))
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Compress,
    Decompress,
    Test,
    List,
}

/// The command's exit: a status, the message already written.
struct Exit(u8);

struct Cli {
    op: Op,
    level: i64,
    ultra: bool,
    out: Option<String>,
    force: bool,
    force_stdout: bool,
    remove: bool,
    display: i32,
    check: Option<bool>,
    content_size: bool,
    workers: i64,
    single_thread: bool,
    long_log: Option<u32>,
    mem_limit: u32,
    dict: Option<String>,
    stream_size: u64,
    size_hint: u64,
    block_size: u64,
    pass_through: i8,
    exclude_compressed: bool,
    files: Vec<String>,
}

fn err(display: i32, level: i32, msg: &str) {
    if display >= level {
        eprint!("{msg}");
    }
}

fn usage_short() -> String {
    "Compress or decompress the INPUT file(s); reads from STDIN if INPUT is `-` or not provided.\n\n\
     Usage: zstd [OPTIONS...] [INPUT... | -] [-o OUTPUT]\n\n\
     Options:\n  -o OUTPUT                     Write output to a single file, OUTPUT.\n  \
     -k, --keep                    Preserve INPUT file(s). [Default] \n  \
     --rm                          Remove INPUT file(s) after successful (de)compression.\n\n  \
     -#                            Desired compression level, where `#` is a number between 1 and 19;\n                                \
     lower numbers provide faster compression, higher numbers yield\n                                \
     better compression ratios. [Default: 3]\n\n  \
     -d, --decompress              Perform decompression.\n  \
     -D DICT                       Use DICT as the dictionary for compression or decompression.\n\n  \
     -f, --force                   Disable input and output checks. Allows overwriting existing files,\n                                \
     receiving input from the console, printing output to STDOUT, and\n                                \
     operating on links, block devices, etc. Unrecognized formats will be\n                                \
     passed-through through as-is.\n\n  \
     -h                            Display short usage and exit.\n  \
     -H, --help                    Display full help and exit.\n  \
     -V, --version                 Display the program version and exit.\n\n"
        .to_owned()
}

fn bad_usage(arg: &str) -> Exit {
    eprintln!("Incorrect parameter: {arg}");
    eprint!("{}", usage_short());
    Exit(1)
}

/// zstdcli's `readU32FromChar`: digits, then K or M (with an optional `i`
/// and `B`); an overflow ends the command.
fn read_u32(s: &str) -> Result<(u32, &str), Exit> {
    let overflow = || {
        eprintln!("error: numeric value overflows 32-bit unsigned int");
        Exit(1)
    };
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let mut v: u32 = 0;
    for d in s[..end].bytes() {
        v = v
            .checked_mul(10)
            .and_then(|v| v.checked_add(u32::from(d - b'0')))
            .ok_or_else(overflow)?;
    }
    let mut rest = &s[end..];
    let mult = match rest.as_bytes().first() {
        Some(b'K') => Some(1u32 << 10),
        Some(b'M') => Some(1u32 << 20),
        _ => None,
    };
    if let Some(m) = mult {
        v = v.checked_mul(m).ok_or_else(overflow)?;
        rest = &rest[1..];
        if let Some(r) = rest.strip_prefix('i') {
            rest = r;
        }
        if let Some(r) = rest.strip_prefix('B') {
            rest = r;
        }
    }
    Ok((v, rest))
}

/// `NEXT_FIELD`: `=value`, or the next argument (which may not start with
/// `-`).
fn next_field(rest: &str, args: &[String], i: &mut usize) -> Result<String, Exit> {
    if let Some(v) = rest.strip_prefix('=') {
        return Ok(v.to_owned());
    }
    *i += 1;
    let Some(v) = args.get(*i) else {
        eprintln!("error: missing command argument ");
        return Err(Exit(1));
    };
    if v.starts_with('-') {
        eprintln!("error: command cannot be separated from its argument by another command ");
        return Err(Exit(1));
    }
    Ok(v.clone())
}

/// `NEXT_UINT32`: a field that must be a number and nothing else.
fn next_u32(rest: &str, args: &[String], i: &mut usize) -> Result<u32, Exit> {
    let f = next_field(rest, args, i)?;
    let (v, tail) = read_u32(&f)?;
    if !tail.is_empty() {
        eprintln!(
            "error: only numeric values with optional suffixes K, KB, KiB, M, MB, MiB are allowed"
        );
        return Err(Exit(1));
    }
    Ok(v)
}

fn unsupported(what: &str) -> Exit {
    eprintln!("zstd: {what} is not done by tw zstd");
    Exit(1)
}

fn parse(args: &[String]) -> Result<Cli, Exit> {
    let mut c = Cli {
        op: Op::Compress,
        level: 3,
        ultra: false,
        out: None,
        force: false,
        force_stdout: false,
        remove: false,
        display: 2,
        check: None,
        content_size: true,
        workers: -1,
        single_thread: false,
        long_log: None,
        mem_limit: 0,
        dict: None,
        stream_size: 0,
        size_hint: 0,
        block_size: 0,
        pass_through: -1,
        exclude_compressed: false,
        files: Vec::new(),
    };
    let mut only_files = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if only_files {
            c.files.push(a.to_owned());
            i += 1;
            continue;
        }
        if a == "-" {
            c.files.push(STDIN.to_owned());
            i += 1;
            continue;
        }
        if let Some(long) = a.strip_prefix("--") {
            let arg = a;
            match arg {
                "--" => only_files = true,
                "--list" => c.op = Op::List,
                "--compress" => c.op = Op::Compress,
                "--decompress" | "--uncompress" => c.op = Op::Decompress,
                "--force" => {
                    c.force = true;
                    c.force_stdout = true;
                }
                "--version" => {
                    version();
                    return Err(Exit(0));
                }
                "--help" => {
                    print!("{}", usage_short());
                    return Err(Exit(0));
                }
                "--verbose" => c.display += 1,
                "--quiet" => c.display -= 1,
                "--stdout" => {
                    c.force_stdout = true;
                    c.out = Some(STDOUT.into());
                }
                "--ultra" => c.ultra = true,
                "--check" => c.check = Some(true),
                "--no-check" => c.check = Some(false),
                "--sparse"
                | "--no-sparse"
                | "--asyncio"
                | "--no-asyncio"
                | "--no-progress"
                | "--progress"
                | "--no-dictID"
                | "--priority=rt"
                | "--row-match-finder"
                | "--no-row-match-finder"
                | "--compress-literals"
                | "--no-compress-literals"
                | "--mmap-dict"
                | "--no-mmap-dict"
                | "--format=zstd" => {}
                "--pass-through" => c.pass_through = 1,
                "--no-pass-through" => c.pass_through = 0,
                "--test" => c.op = Op::Test,
                "--keep" => c.remove = false,
                "--rm" => c.remove = true,
                "--content-size" => c.content_size = true,
                "--no-content-size" => c.content_size = false,
                "--single-thread" => {
                    c.workers = 0;
                    c.single_thread = true;
                }
                "--exclude-compressed" => c.exclude_compressed = true,
                "--max" => {
                    c.ultra = true;
                    c.level = 22;
                    c.long_log = Some(31);
                }
                _ => {
                    let with = |p: &str| long.strip_prefix(&p[2..]);
                    if let Some(r) = with("--threads") {
                        c.workers = i64::from(next_u32(r, args, &mut i)?);
                    } else if let Some(r) = with("--memlimit-decompress")
                        .or_else(|| with("--memlimit"))
                        .or_else(|| with("--memory"))
                    {
                        c.mem_limit = next_u32(r, args, &mut i)?;
                    } else if let Some(r) = with("--block-size") {
                        c.block_size = u64::from(next_u32(r, args, &mut i)?);
                    } else if let Some(r) = with("--stream-size") {
                        c.stream_size = u64::from(next_u32(r, args, &mut i)?);
                    } else if let Some(r) = with("--size-hint") {
                        c.size_hint = u64::from(next_u32(r, args, &mut i)?);
                    } else if let Some(r) = with("--long") {
                        c.long_log = Some(match r.strip_prefix('=') {
                            Some(v) => read_u32(v)?.0,
                            None if r.is_empty() => 27,
                            None => return Err(bad_usage(arg)),
                        });
                        c.ultra = true;
                    } else if let Some(r) = with("--fast") {
                        c.level = match r.strip_prefix('=') {
                            Some(v) => {
                                let n = read_u32(v)?.0.min(131_072);
                                if n == 0 {
                                    return Err(bad_usage(arg));
                                }
                                -i64::from(n)
                            }
                            None if r.is_empty() => -1,
                            None => return Err(bad_usage(arg)),
                        };
                    } else if [
                        "--train",
                        "--patch-from",
                        "--zstd=",
                        "--adapt",
                        "--rsyncable",
                        "--filelist",
                        "--output-dir",
                        "--format=",
                        "--show-default-cparams",
                        "--trace",
                        "--maxdict",
                        "--dictID",
                        "--target-compressed-block-size",
                        "--auto-threads",
                    ]
                    .iter()
                    .any(|p| arg.starts_with(p))
                    {
                        return Err(unsupported(arg));
                    } else {
                        return Err(bad_usage(arg));
                    }
                }
            }
            i += 1;
            continue;
        }
        if let Some(flags) = a.strip_prefix('-') {
            let mut rest = flags;
            while let Some(ch) = rest.chars().next() {
                if ch.is_ascii_digit() {
                    let (v, r) = read_u32(rest)?;
                    c.level = i64::from(v);
                    rest = r;
                    continue;
                }
                rest = &rest[ch.len_utf8()..];
                match ch {
                    'V' => {
                        version();
                        return Err(Exit(0));
                    }
                    'H' | 'h' => {
                        print!("{}", usage_short());
                        return Err(Exit(0));
                    }
                    'z' => c.op = Op::Compress,
                    'd' => c.op = Op::Decompress,
                    'c' => {
                        c.force_stdout = true;
                        c.out = Some(STDOUT.into());
                    }
                    'o' => c.out = Some(next_field(rest, args, &mut i)?),
                    'n' => {}
                    'D' => c.dict = Some(next_field(rest, args, &mut i)?),
                    'f' => {
                        c.force = true;
                        c.force_stdout = true;
                    }
                    'v' => c.display += 1,
                    'q' => c.display -= 1,
                    'k' => c.remove = false,
                    'C' => c.check = Some(true),
                    't' => c.op = Op::Test,
                    'l' => c.op = Op::List,
                    'M' => {
                        let (v, r) = read_u32(rest)?;
                        c.mem_limit = v;
                        rest = r;
                    }
                    'T' => {
                        let (v, r) = read_u32(rest)?;
                        c.workers = i64::from(v);
                        rest = r;
                    }
                    'B' => {
                        let (v, r) = read_u32(rest)?;
                        c.block_size = u64::from(v);
                        rest = r;
                    }
                    'b' | 'e' | 'i' | 'S' | 's' | 'p' | 'P' | 'r' => {
                        return Err(unsupported(&format!("-{ch}")));
                    }
                    _ => return Err(bad_usage(&format!("-{ch}"))),
                }
                // `-o` and `-D` take a value from the next argument but
                // read on in this one, as zstdcli does.
            }
            i += 1;
            continue;
        }
        c.files.push(a.to_owned());
        i += 1;
    }
    Ok(c)
}

fn version() {
    println!(
        "*** tw zstd {} (Zstandard CLI v1.5.7, by Yann Collet) ***",
        env!("CARGO_PKG_VERSION")
    );
}

/// Runs `tw zstd`; the exit code. `args` are those after `zstd`.
pub fn main(args: &[String]) -> u8 {
    match run(args) {
        Ok(code) | Err(Exit(code)) => code,
    }
}

fn run(args: &[String]) -> Result<u8, Exit> {
    let mut c = parse(args)?;
    if c.op == Op::Decompress && c.workers > 1 && c.display >= 2 {
        eprintln!("Warning : decompression does not support multi-threading");
    }
    if c.op == Op::List {
        return Ok(list(&c.files, c.display));
    }
    if c.op == Op::Test {
        c.out = Some(NULL.into());
        c.remove = false;
    }
    if c.files.is_empty() {
        c.files.push(STDIN.into());
    }
    if c.files.len() == 1 && c.files[0] == STDIN && c.out.is_none() {
        c.out = Some(STDOUT.into());
    }
    let has_stdin = c.files.iter().any(|f| f == STDIN);
    if !c.force && has_stdin && io::stdin().is_terminal() {
        err(c.display, 1, "stdin is a console, aborting\n");
        return Err(Exit(1));
    }
    let to_stdout = c.out.as_deref().is_none_or(|o| o == STDOUT);
    if to_stdout
        && io::stdout().is_terminal()
        && has_stdin
        && !c.force_stdout
        && c.op != Op::Decompress
    {
        err(c.display, 1, "stdout is a console, aborting\n");
        return Err(Exit(1));
    }
    let max = if c.ultra { 22 } else { 19 };
    if c.level > max {
        err(
            c.display,
            2,
            &format!("Warning : compression level higher than max, reduced to {max} \n"),
        );
        c.level = max;
    }
    let has_stdout = c.out.as_deref() == Some(STDOUT);
    if has_stdout && c.display == 2 {
        c.display = 1;
    }
    if has_stdout && c.remove {
        c.remove = false;
    }
    if c.mem_limit == 0 {
        c.mem_limit = 1u32 << c.long_log.unwrap_or(27).min(31);
    }
    let dict = match &c.dict {
        Some(d) => match std::fs::read(d) {
            Ok(b) => Some(b),
            Err(e) => {
                err(
                    c.display,
                    1,
                    &format!("zstd: error 32 : Dictionary file {d} : {e}\n"),
                );
                return Err(Exit(1));
            }
        },
        None => None,
    };
    let mut ctx = Ctx {
        c: &c,
        dict,
        has_stdin,
        failed: false,
    };
    match c.op {
        Op::Compress => ctx.compress_all(),
        _ => ctx.decompress_all(),
    }
    Ok(u8::from(ctx.failed))
}

struct Ctx<'a> {
    c: &'a Cli,
    dict: Option<Vec<u8>>,
    has_stdin: bool,
    failed: bool,
}

/// Where output goes: stdout, nowhere (`--test`), or a file being written.
enum Sink {
    Stdout(io::BufWriter<crate::Stdout>),
    Null,
    File(io::BufWriter<File>, String),
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Sink::Stdout(w) => w.write(buf),
            Sink::Null => Ok(buf.len()),
            Sink::File(w, _) => w.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Sink::Stdout(w) => w.flush(),
            Sink::Null => Ok(()),
            Sink::File(w, _) => w.flush(),
        }
    }
}

fn shown(name: &str) -> &str {
    match name {
        STDIN => "*stdin*",
        STDOUT => "*stdout*",
        n => n,
    }
}

impl Ctx<'_> {
    fn say(&self, level: i32, msg: &str) {
        err(self.c.display, level, msg);
    }

    /// `FIO_openSrcFile`: stdin, or a regular file.
    fn open_src(&self, name: &str) -> Option<(Box<dyn Read>, Option<std::fs::Metadata>)> {
        if name == STDIN {
            return Some((Box::new(io::stdin().lock()), None));
        }
        let meta = match std::fs::metadata(name) {
            Ok(m) => m,
            Err(e) => {
                self.say(
                    1,
                    &format!("zstd: can't stat {name} : {} -- ignored \n", strerror(&e)),
                );
                return None;
            }
        };
        if meta.is_dir() {
            self.say(1, &format!("zstd: {name} is a directory -- ignored \n"));
            return None;
        }
        match File::open(name) {
            Ok(f) => Some((Box::new(f), Some(meta))),
            Err(e) => {
                self.say(1, &format!("zstd: {name}: {} \n", strerror(&e)));
                None
            }
        }
    }

    /// `FIO_openDstFile`: stdout, nowhere, or a file, asking before one is
    /// overwritten.
    fn open_dst(&self, src: Option<&str>, dst: &str) -> Option<Sink> {
        match dst {
            STDOUT => return Some(Sink::Stdout(io::BufWriter::new(crate::stdout()))),
            NULL => return Some(Sink::Null),
            _ => {}
        }
        if let Some(src) = src
            && src != STDIN
            && same_file(src, dst)
        {
            self.say(
                1,
                "zstd: Refusing to open an output file which will overwrite the input file \n",
            );
            return None;
        }
        if std::fs::metadata(dst).is_ok_and(|m| m.is_file()) && !self.c.force {
            if self.c.display <= 1 {
                self.say(
                    1,
                    &format!("zstd: {dst} already exists; not overwritten  \n"),
                );
                return None;
            }
            eprint!("zstd: {dst} already exists; ");
            if !confirm("overwrite (y/n) ? ", "Not overwritten  \n", self.has_stdin) {
                return None;
            }
            let _ = std::fs::remove_file(dst);
        }
        match File::create(dst) {
            Ok(f) => Some(Sink::File(io::BufWriter::new(f), dst.to_owned())),
            Err(e) => {
                self.say(1, &format!("zstd: {dst}: {} \n", strerror(&e)));
                None
            }
        }
    }

    fn compress_options(&self, meta: Option<&std::fs::Metadata>) -> Compress {
        let c = self.c;
        // Any number of workers gives the same frames, so one is enough;
        // -T0 is every core, and only --single-thread is none.
        let workers = u32::from(!c.single_thread);
        let pledged = match meta {
            Some(m) if m.is_file() => Some(m.len()),
            _ if c.stream_size > 0 => Some(c.stream_size),
            _ => None,
        };
        Compress {
            level: c.level as i32,
            checksum: c.check.unwrap_or(true),
            content_size: c.content_size,
            workers,
            long_window_log: c.long_log.unwrap_or(0),
            job_size: c.block_size as u32,
            size_hint: c.size_hint as u32,
            pledged,
            dict: self.dict.clone(),
        }
    }

    fn compress_all(&mut self) {
        let c = self.c;
        let files = c.files.clone();
        if let Some(out) = c.out.clone() {
            if files.len() > 1 && !self.concat_ok(&out) {
                self.failed = true;
                return;
            }
            let Some(mut sink) = self.open_dst(
                if files.len() == 1 {
                    files.first().map(String::as_str)
                } else {
                    None
                },
                &out,
            ) else {
                self.failed = true;
                return;
            };
            for f in &files {
                if self.compress_one(f, &mut sink).is_err() {
                    self.failed = true;
                }
            }
            if self
                .close(
                    sink,
                    files
                        .first()
                        .filter(|_| files.len() == 1)
                        .map(String::as_str),
                )
                .is_err()
            {
                self.failed = true;
            }
            if files.len() == 1 && !self.failed && c.remove && files[0] != STDIN {
                let _ = std::fs::remove_file(&files[0]);
            }
            return;
        }
        for f in &files {
            if c.exclude_compressed
                && [
                    "zst", "tzst", "gz", "tgz", "xz", "txz", "lzma", "lz4", "tlz4",
                ]
                .iter()
                .any(|e| f.ends_with(&format!(".{e}")))
            {
                continue;
            }
            let dst = format!("{f}.zst");
            let Some(mut sink) = self.open_dst(Some(f), &dst) else {
                self.failed = true;
                continue;
            };
            let ok = self.compress_one(f, &mut sink).is_ok();
            if !ok {
                drop(sink);
                let _ = std::fs::remove_file(&dst);
                self.failed = true;
                continue;
            }
            if self.close(sink, Some(f)).is_err() {
                self.failed = true;
                continue;
            }
            if c.remove {
                let _ = std::fs::remove_file(f);
            }
        }
    }

    fn compress_one(&self, src: &str, sink: &mut Sink) -> Result<(), ()> {
        let Some((mut input, meta)) = self.open_src(src) else {
            return Err(());
        };
        let o = self.compress_options(meta.as_ref());
        match z::compress(&mut input, sink, &o) {
            Ok((read, wrote)) => {
                let dst = match sink {
                    Sink::File(_, n) => n.clone(),
                    Sink::Stdout(_) => "/*stdout*\\".into(),
                    Sink::Null => NULL.into(),
                };
                if self.c.display >= 2 {
                    let pct = if read == 0 {
                        0.0
                    } else {
                        wrote as f64 / read as f64 * 100.0
                    };
                    let (rv, rp, rs) = z::human(read, false);
                    let (wv, wp, ws) = z::human(wrote, false);
                    eprintln!(
                        "{:<20} :{pct:6.2}%   ({rv:6.rp$}{rs} => {wv:6.wp$}{ws}, {}) ",
                        shown(src),
                        shown(&dst)
                    );
                }
                Ok(())
            }
            Err(e) => {
                self.say(1, &format!("zstd: {}: {e} \n", shown(src)));
                Err(())
            }
        }
    }

    /// Several inputs into one output named with -o: a warning, and a
    /// question unless forced.
    fn concat_ok(&self, out: &str) -> bool {
        if self.c.op == Op::Test {
            return true;
        }
        let stdout = out == STDOUT;
        if stdout {
            self.say(
                2,
                "zstd: WARNING: all input files will be processed and concatenated into stdout. \n",
            );
        } else {
            self.say(2, &format!("zstd: WARNING: all input files will be processed and concatenated into a single output file: {out} \n"));
        }
        self.say(2, "The concatenated output CANNOT regenerate original file names nor directory structure. \n");
        if stdout || self.c.force {
            return true;
        }
        if self.c.display <= 1 {
            self.say(1, "Concatenating multiple processed inputs into a single output loses file metadata. \n");
            self.say(1, "Aborting. \n");
            return false;
        }
        confirm("Proceed? (y/n): ", "Aborting...", self.has_stdin)
    }

    /// Ends a written file, with the source's mode and time.
    fn close(&self, sink: Sink, src: Option<&str>) -> io::Result<()> {
        match sink {
            Sink::Stdout(mut w) => w.flush(),
            Sink::Null => Ok(()),
            Sink::File(w, name) => {
                let f = w.into_inner().map_err(io::IntoInnerError::into_error)?;
                if let Some(src) = src.filter(|s| *s != STDIN)
                    && let Ok(meta) = std::fs::metadata(src)
                {
                    let _ = f.set_permissions(meta.permissions());
                    if let Ok(t) = meta.modified() {
                        let _ = f.set_modified(t);
                    }
                }
                let _ = name;
                Ok(())
            }
        }
    }

    fn decompress_all(&mut self) {
        let c = self.c;
        let files = c.files.clone();
        let opts = Decompress {
            max_window: u64::from(c.mem_limit),
            dict: self.dict.clone(),
            check: c.check.unwrap_or(true),
        };
        if let Some(out) = c.out.clone() {
            if files.len() > 1 && !self.concat_ok(&out) {
                self.failed = true;
                return;
            }
            let Some(mut sink) = self.open_dst(
                if files.len() == 1 {
                    files.first().map(String::as_str)
                } else {
                    None
                },
                &out,
            ) else {
                self.failed = true;
                return;
            };
            for f in &files {
                if self.decompress_one(f, &mut sink, &out, &opts).is_err() {
                    self.failed = true;
                }
            }
            if self
                .close(
                    sink,
                    files
                        .first()
                        .filter(|_| files.len() == 1)
                        .map(String::as_str),
                )
                .is_err()
            {
                self.failed = true;
            }
            if files.len() == 1 && !self.failed && c.remove && files[0] != STDIN {
                let _ = std::fs::remove_file(&files[0]);
            }
            return;
        }
        for f in &files {
            let Some(dst) = self.dst_name(f) else {
                self.failed = true;
                continue;
            };
            if std::fs::metadata(f).is_err() {
                // As `FIO_openSrcFile` says it, before any output is made.
                let e = File::open(f)
                    .err()
                    .map(|e| strerror(&e))
                    .unwrap_or_default();
                self.say(1, &format!("zstd: can't stat {f} : {e} -- ignored \n"));
                self.failed = true;
                continue;
            }
            let Some(mut sink) = self.open_dst(Some(f), &dst) else {
                self.failed = true;
                continue;
            };
            if self.decompress_one(f, &mut sink, &dst, &opts).is_err() {
                drop(sink);
                let _ = std::fs::remove_file(&dst);
                self.failed = true;
                continue;
            }
            if self.close(sink, Some(f)).is_err() {
                self.failed = true;
                continue;
            }
            if c.remove {
                let _ = std::fs::remove_file(f);
            }
        }
    }

    /// `FIO_determineDstName`: the name without its .zst (or .tzst as .tar).
    fn dst_name(&self, src: &str) -> Option<String> {
        if src == STDIN {
            return Some(STDOUT.into());
        }
        let list = ".zst/.tzst/.gz/.tgz/.xz/.txz/.lzma/.lz4/.tlz4";
        let ignore = || {
            self.say(1, &format!("zstd: {src}: unknown suffix ({list} expected). Can't derive the output file name. Specify it with -o dstFileName. Ignoring.\n"));
            None
        };
        let Some(dot) = src.rfind('.') else {
            return ignore();
        };
        let suffix = &src[dot..];
        if dot == 0 || !list.split('/').any(|s| s == suffix) {
            return ignore();
        }
        let base = &src[..dot];
        Some(if suffix.starts_with(".t") {
            format!("{base}.tar")
        } else {
            base.to_owned()
        })
    }

    fn decompress_one(
        &self,
        src: &str,
        sink: &mut Sink,
        dst: &str,
        o: &Decompress,
    ) -> Result<(), ()> {
        let Some((input, _)) = self.open_src(src) else {
            return Err(());
        };
        let pass = match self.c.pass_through {
            -1 => self.c.force && dst == STDOUT,
            p => p == 1,
        };
        let mut input = io::BufReader::new(input);
        if pass {
            let head = input.fill_buf().map(<[u8]>::to_vec).unwrap_or_default();
            let zstd = head.len() >= 4 && {
                let m = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
                m == 0xFD2F_B528 || m & 0xFFFF_FFF0 == 0x184D_2A50
            };
            if !zstd && !head.is_empty() {
                return io::copy(&mut input, sink).map(|_| ()).map_err(|_| ());
            }
        }
        match z::decompress(&mut input, sink, o) {
            Ok(n) => {
                if self.c.display >= 2 && dst != STDOUT && self.c.op != Op::Test {
                    eprintln!("{:<20}: {n} bytes ", shown(src));
                }
                Ok(())
            }
            Err(e) => {
                let s = shown(src);
                self.say(
                    1,
                    &match e {
                        Decode::Empty => format!("zstd: {s}: unexpected end of file \n"),
                        Decode::Format => format!("zstd: {s}: unsupported format \n"),
                        Decode::Window(w) => format!(
                            "{s} : Window size larger than maximum : {w} > {} \n{s} : Use --long={} or --memory={}MB \n",
                            o.max_window,
                            64 - (w - 1).leading_zeros(),
                            w.div_ceil(1 << 20)
                        ),
                        Decode::Io(e) => format!("zstd: {s}: {e} \n"),
                        Decode::Corrupt(m) => format!("{s} : Decoding error (36) : {m} \n"),
                    },
                );
                Err(())
            }
        }
    }
}

/// `UTIL_requireUserConfirmation`: yes when the first character read from
/// stdin is y or Y; never when stdin is an input.
fn confirm(prompt: &str, abort: &str, has_stdin: bool) -> bool {
    if has_stdin {
        eprintln!("stdin is an input - not proceeding.");
        return false;
    }
    eprint!("{prompt}");
    let mut line = String::new();
    let _ = io::stdin().lock().read_line(&mut line);
    if line.starts_with(['y', 'Y']) {
        true
    } else {
        eprintln!("{abort} ");
        false
    }
}

fn same_file(a: &str, b: &str) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// The C library's words for an error, as the reference prints them.
fn strerror(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => "No such file or directory".into(),
        io::ErrorKind::PermissionDenied => "Permission denied".into(),
        _ => e.to_string(),
    }
}

/// `--list`: the frames of each file, as a table (or, with -v, in full).
fn list(files: &[String], display: i32) -> u8 {
    if files.iter().any(|f| f == STDIN) {
        err(
            display,
            1,
            "zstd: --list does not support reading from standard input\n",
        );
        return 1;
    }
    if files.is_empty() {
        if !io::stdin().is_terminal() {
            err(
                display,
                1,
                "zstd: --list does not support reading from standard input \n",
            );
        }
        err(display, 1, "No files given \n");
        return 1;
    }
    let mut out = io::BufWriter::new(crate::stdout());
    let table = display <= 2;
    if table {
        let _ = writeln!(
            out,
            "Frames  Skips  Compressed  Uncompressed  Ratio  Check  Filename"
        );
    }
    let mut total = z::Info {
        uses_check: true,
        ..z::Info::default()
    };
    let mut nb_files = 0u32;
    let mut error = 0u8;
    for f in files {
        if !std::fs::metadata(f).is_ok_and(|m| m.is_file()) {
            err(display, 1, &format!("Error : {f} is not a file\n"));
            if !table {
                let _ = writeln!(out);
            }
            error = 1;
            continue;
        }
        let data = match std::fs::read(f) {
            Ok(d) => d,
            Err(_) => {
                err(
                    display,
                    1,
                    &format!("Error: could not open source file {f}\n"),
                );
                error = 1;
                continue;
            }
        };
        let (info, e) = z::info(&data);
        match e {
            Some(InfoError::NotZstd) => {
                let _ = writeln!(out, "File \"{f}\" not compressed by zstd ");
                if !table {
                    let _ = writeln!(out);
                }
                error = 1;
                continue;
            }
            Some(InfoError::Truncated) => {
                let _ = writeln!(out, "File \"{f}\" is truncated ");
                if !table {
                    let _ = writeln!(out);
                }
                error = 1;
                continue;
            }
            Some(InfoError::Frame) => {
                err(display, 1, &format!("Error while parsing \"{f}\" \n"));
                error = 1;
            }
            None => {}
        }
        display_info(&mut out, f, &info, display);
        total.frames += info.frames;
        total.skippable += info.skippable;
        total.compressed += info.compressed;
        total.decompressed = total.decompressed.wrapping_add(info.decompressed);
        total.decompressed_unknown |= info.decompressed_unknown;
        total.uses_check &= info.uses_check;
        nb_files += 1;
    }
    if files.len() > 1 && table {
        let (cv, cp, cs) = z::human(total.compressed, false);
        let (dv, dp, ds) = z::human(total.decompressed, false);
        let ratio = if total.compressed == 0 {
            0.0
        } else {
            total.decompressed as f64 / total.compressed as f64
        };
        let check = if total.uses_check { "XXH64" } else { "" };
        let _ = writeln!(
            out,
            "----------------------------------------------------------------- "
        );
        if total.decompressed_unknown {
            let _ = writeln!(
                out,
                "{:6}  {:5}  {cv:6.cp$}{cs:>4}                       {check:>5}  {nb_files} files",
                total.frames + total.skippable,
                total.skippable
            );
        } else {
            let _ = writeln!(
                out,
                "{:6}  {:5}  {cv:6.cp$}{cs:>4}  {dv:8.dp$}{ds:>4}  {ratio:5.3}  {check:>5}  {nb_files} files",
                total.frames + total.skippable,
                total.skippable
            );
        }
    }
    let _ = out.flush();
    error
}

fn display_info(out: &mut dyn Write, name: &str, i: &z::Info, display: i32) {
    let verbose = display > 3;
    let (cv, cp, cs) = z::human(i.compressed, verbose);
    let (dv, dp, ds) = z::human(i.decompressed, verbose);
    let ratio = if i.compressed == 0 {
        0.0
    } else {
        i.decompressed as f64 / i.compressed as f64
    };
    let check = if i.uses_check { "XXH64" } else { "None" };
    if display <= 2 {
        if i.decompressed_unknown {
            let _ = writeln!(
                out,
                "{:6}  {:5}  {cv:6.cp$}{cs:>4}                       {check:>5}  {name}",
                i.frames + i.skippable,
                i.skippable
            );
        } else {
            let _ = writeln!(
                out,
                "{:6}  {:5}  {cv:6.cp$}{cs:>4}  {dv:8.dp$}{ds:>4}  {ratio:5.3}  {check:>5}  {name}",
                i.frames + i.skippable,
                i.skippable
            );
        }
        return;
    }
    let (wv, wp, ws) = z::human(i.window, verbose);
    let _ = writeln!(out, "{name} ");
    let _ = writeln!(out, "# Zstandard Frames: {}", i.frames);
    if i.skippable > 0 {
        let _ = writeln!(out, "# Skippable Frames: {}", i.skippable);
    }
    let _ = writeln!(out, "DictID: {}", i.dict_id);
    let _ = writeln!(out, "Window Size: {wv:.wp$}{ws} ({} B)", i.window);
    let _ = writeln!(out, "Compressed Size: {cv:.cp$}{cs} ({} B)", i.compressed);
    if !i.decompressed_unknown {
        let _ = writeln!(
            out,
            "Decompressed Size: {dv:.dp$}{ds} ({} B)",
            i.decompressed
        );
        let _ = writeln!(out, "Ratio: {ratio:.4}");
    }
    if i.uses_check && i.frames == 1 {
        let _ = writeln!(
            out,
            "Check: {check} {:02x}{:02x}{:02x}{:02x}",
            i.checksum[3], i.checksum[2], i.checksum[1], i.checksum[0]
        );
    } else {
        let _ = writeln!(out, "Check: {check}");
    }
    let _ = writeln!(out);
}
