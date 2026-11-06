//! `gzip`: GNU gzip 1.13's command line, ported from its gzip.c: its options
//! as glibc's `getopt_long` reads them (and the GZIP environment variable),
//! its files (names and suffixes, `-S`, the overwrite question, the mode,
//! times and owner kept, `-r`, `-k`), stdin and stdout, `--test`, `--list`
//! and every message. The formats are toolwing_tools::gzip's: gzip's own
//! decoders, and zlib's deflate in gzip's header and trailer to compress.
//!
//! Ported from GNU gzip 1.13: Copyright (C) 1997-2023 Free Software
//! Foundation, Inc.; Copyright (C) 1992-1993 Jean-loup Gailly.
//! GPL-3.0-or-later.

use std::fs::{self, File, Metadata};
use std::io::{self, BufRead, IsTerminal, Read, Write};

use toolwing_tools::gzip::{
    DEFLATED, ERROR, Gzip, Io, MAX_PATH_LEN, OK, R, Stamp, Stop, WARNING, Work, base_start,
    strerror,
};

use crate::getopt::getopt_errors;

/// tw's command for `gzip`: everything after it is gzip's.
pub fn command() -> clap::Command {
    clap::Command::new("gzip")
        .about(toolwing_tools::group_info("gzip").map_or("", |g| g.summary))
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

const SHORT: &str = "ab:cdfhH?klLmMnNqrS:tvVZ123456789";
const PRESUME_INPUT_TTY: char = '\u{1}';
const RSYNCABLE: char = '\u{2}';
const SYNCHRONOUS: char = '\u{3}';

/// `longopts`, in gzip's order (glibc lists ambiguous ones in it).
const LONG: &[(&str, bool, char)] = &[
    ("ascii", false, 'a'),
    ("to-stdout", false, 'c'),
    ("stdout", false, 'c'),
    ("decompress", false, 'd'),
    ("uncompress", false, 'd'),
    ("force", false, 'f'),
    ("help", false, 'h'),
    ("keep", false, 'k'),
    ("list", false, 'l'),
    ("license", false, 'L'),
    ("no-name", false, 'n'),
    ("name", false, 'N'),
    ("-presume-input-tty", false, PRESUME_INPUT_TTY),
    ("quiet", false, 'q'),
    ("silent", false, 'q'),
    ("synchronous", false, SYNCHRONOUS),
    ("recursive", false, 'r'),
    ("suffix", true, 'S'),
    ("test", false, 't'),
    ("verbose", false, 'v'),
    ("version", false, 'V'),
    ("fast", false, '1'),
    ("best", false, '9'),
    ("lzw", false, 'Z'),
    ("bits", true, 'b'),
    ("rsyncable", false, RSYNCABLE),
];

const HELP: &[&str] = &[
    "Compress or uncompress FILEs (by default, compress FILES in-place).",
    "",
    "Mandatory arguments to long options are mandatory for short options too.",
    "",
    "  -c, --stdout      write on standard output, keep original files unchanged",
    "  -d, --decompress  decompress",
    "  -f, --force       force overwrite of output file and compress links",
    "  -h, --help        give this help",
    "  -k, --keep        keep (don't delete) input files",
    "  -l, --list        list compressed file contents",
    "  -L, --license     display software license",
    "  -n, --no-name     do not save or restore the original name and timestamp",
    "  -N, --name        save or restore the original name and timestamp",
    "  -q, --quiet       suppress all warnings",
    "  -r, --recursive   operate recursively on directories",
    "      --rsyncable   make rsync-friendly archive",
    "  -S, --suffix=SUF  use suffix SUF on compressed files",
    "      --synchronous synchronous output (safer if system crashes, but slower)",
    "  -t, --test        test compressed file integrity",
    "  -v, --verbose     verbose mode",
    "  -V, --version     display version number",
    "  -1, --fast        compress faster",
    "  -9, --best        compress better",
    "",
    "With no FILE, or when FILE is -, read standard input.",
    "",
    "Report bugs to <bug-gzip@gnu.org>.",
];

const LICENSE: &[&str] = &[
    "Copyright (C) 2023 Free Software Foundation, Inc.",
    "Copyright (C) 1993 Jean-loup Gailly.",
    "This is free software.  You may redistribute copies of it under the terms of",
    "the GNU General Public License <https://www.gnu.org/licenses/gpl.html>.",
    "There is NO WARRANTY, to the extent permitted by law.",
];

/// The longest suffix (`MAX_SUFFIX`).
const MAX_SUFFIX: usize = 30;
/// Name parts kept at least this long when a name is shortened.
const MIN_PART: usize = 3;

/// Runs `tw gzip`; the exit code. `args` are those after `gzip`.
pub fn main(args: &[String]) -> u8 {
    let mut c = Cmd::default();
    let code = match c.run(args) {
        Ok(()) => c.finish(),
        Err(Stop(code)) => {
            c.remove_output_file();
            let _ = c.stdout.flush();
            code
        }
    };
    u8::try_from(code).unwrap_or(1)
}

/// The command's state beyond the engine's.
struct Cmd {
    gz: Gzip,
    keep: bool,
    recursive: bool,
    ascii: bool,
    synchronous: bool,
    presume_input_tty: bool,
    z_suffix: String,
    /// The output to remove if the command stops (`remove_ofname_fd`).
    remove_ofname: Option<String>,
    stdin_was_read: bool,
    total_in: i64,
    total_out: i64,
    first_time: bool,
    work: Option<Work>,
    stdout: io::BufWriter<crate::Stdout>,
}

impl Default for Cmd {
    fn default() -> Self {
        Cmd {
            gz: Gzip::default(),
            keep: false,
            recursive: false,
            ascii: false,
            synchronous: false,
            presume_input_tty: false,
            z_suffix: ".gz".into(),
            remove_ofname: None,
            stdin_was_read: false,
            total_in: 0,
            total_out: 0,
            first_time: true,
            work: None,
            stdout: io::BufWriter::with_capacity(1 << 16, crate::stdout()),
        }
    }
}

/// An option as gzip's `switch` takes it: its character, its value, the
/// long name it was given by. '?' carries getopt's message.
struct Opt {
    c: char,
    value: Option<String>,
    long: Option<&'static str>,
}

/// What an option does: go on, or end the command now.
enum Next {
    Go,
    Exit(i32),
}

/// `add_envopt`: the GZIP variable's words.
fn env_words() -> Option<Vec<String>> {
    let v = std::env::var_os("GZIP")?;
    let v = v.to_string_lossy().into_owned();
    let words: Vec<String> = v
        .split([' ', '\t'])
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    (!words.is_empty()).then_some(words)
}

/// The file's metadata, for stdin (`fstat (STDIN_FILENO)`).
fn stdin_metadata() -> io::Result<Metadata> {
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        let fd = io::stdin().as_fd().try_clone_to_owned()?;
        File::from(fd).metadata()
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsHandle;
        let h = io::stdin().as_handle().try_clone_to_owned()?;
        File::from(h).metadata()
    }
}

fn mtime(m: &Metadata) -> Stamp {
    match m.modified() {
        Ok(t) => match t.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => Stamp {
                sec: d.as_secs() as i64,
                nsec: i64::from(d.subsec_nanos()),
            },
            Err(e) => {
                let d = e.duration();
                let (mut sec, mut nsec) = (-(d.as_secs() as i64), -i64::from(d.subsec_nanos()));
                if nsec < 0 {
                    sec -= 1;
                    nsec += 1_000_000_000;
                }
                Stamp { sec, nsec }
            }
        },
        Err(_) => Stamp::UNKNOWN,
    }
}

#[cfg(unix)]
fn mode_of(m: &Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt;
    m.mode()
}

#[cfg(not(unix))]
fn mode_of(m: &Metadata) -> u32 {
    if m.permissions().readonly() {
        0o444
    } else {
        0o666
    }
}

#[cfg(unix)]
fn nlink_of(m: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    m.nlink()
}

#[cfg(not(unix))]
fn nlink_of(_: &Metadata) -> u64 {
    1
}

/// `display_ratio`: a ratio on six characters.
fn ratio(num: i64, den: i64) -> String {
    let r = if den == 0 {
        0.0
    } else {
        100.0 * num as f64 / den as f64
    };
    format!("{r:5.1}%")
}

/// `fprint_off`: a number right-aligned on `width`.
fn off(n: i64, width: usize) -> String {
    format!("{n:>width$}")
}

impl Cmd {
    fn prog(&self) -> &str {
        &self.gz.program_name
    }

    fn say(&mut self, text: &str) {
        self.gz.say(text.as_bytes());
    }

    fn warn(&mut self, text: &str) {
        self.gz.warn(text.as_bytes());
    }

    /// `try_help`.
    fn try_help(&mut self) -> Next {
        let m = format!("Try `{} --help' for more information.\n", self.prog());
        self.say(&m);
        Next::Exit(ERROR)
    }

    /// `progerror`: "gzip: NAME: what the system said".
    fn progerror(&mut self, name: &str, e: &io::Error) {
        let m = format!("{}: {name}: {}\n", self.prog(), strerror(e));
        self.say(&m);
        self.gz.exit_code = ERROR;
    }

    fn print(&mut self, text: &str) -> R<()> {
        self.stdout.write_all(text.as_bytes()).map_err(|e| {
            self.gz.ofname = b"stdout".to_vec();
            self.gz.write_error(&e)
        })
    }

    /// The options, then the files or stdin.
    fn run(&mut self, args: &[String]) -> R<()> {
        // The GZIP variable's options first, as gzip takes them there.
        let mut opts: Vec<Opt> = Vec::new();
        if let Some(words) = env_words() {
            let end = words.iter().position(|w| w == "--");
            let (eopts, operands) =
                getopt_errors(&words[..end.unwrap_or(words.len())], SHORT, |c| c, LONG);
            let mut env_quiet = false;
            for o in eopts {
                let o = match o {
                    Ok(g) => Opt {
                        c: g.opt,
                        value: g.value,
                        long: g.name,
                    },
                    Err(e) => {
                        let m = e.message(&self.gz.program_name.clone());
                        self.say(&m);
                        let next = self.try_help();
                        return self.exit_next(next);
                    }
                };
                if let Some(next) = self.env_option(&o) {
                    return self.exit_next(next);
                }
                match o.c {
                    'q' => env_quiet = true,
                    'v' => env_quiet = false,
                    _ => {}
                }
                opts.push(o);
            }
            if end.is_some() {
                let m = format!(
                    "{}: --: option not valid in GZIP environment variable\n",
                    self.prog()
                );
                self.say(&m);
                let next = self.try_help();
                return self.exit_next(next);
            }
            if let Some(first) = operands.first() {
                let m = format!(
                    "{}: {first}: non-option in GZIP environment variable\n",
                    self.prog()
                );
                self.say(&m);
                let next = self.try_help();
                return self.exit_next(next);
            }
            if !env_quiet {
                let m = format!(
                    "{}: warning: GZIP environment variable is deprecated; use an alias or script\n",
                    self.prog()
                );
                self.say(&m);
            }
        }

        // Then the arguments'.
        let (aopts, files) = getopt_errors(args, SHORT, |c| c, LONG);
        for o in aopts {
            match o {
                Ok(g) => opts.push(Opt {
                    c: g.opt,
                    value: g.value,
                    long: g.name,
                }),
                Err(e) => {
                    // The options before it count: -h first gives the help.
                    opts.push(Opt {
                        c: '?',
                        value: Some(e.message(&self.gz.program_name.clone())),
                        long: None,
                    });
                }
            }
        }
        let mut no_name: Option<bool> = None;
        let mut no_time: Option<bool> = None;
        for o in &opts {
            if o.c == '?' {
                if let Some(m) = &o.value {
                    let m = m.clone();
                    self.say(&m);
                }
                let next = self.try_help();
                return self.exit_next(next);
            }
            if let Next::Exit(code) = self.option(o, &mut no_name, &mut no_time)? {
                return self.exit_next(Next::Exit(code));
            }
        }

        // By default, the name and time are saved when compressing, not
        // restored when decompressing.
        self.gz.no_time = no_time.unwrap_or(self.gz.decompress);
        self.gz.no_name = no_name.unwrap_or(self.gz.decompress);

        if self.ascii && !self.gz.quiet {
            let m = format!("{}: option --ascii ignored on this system\n", self.prog());
            self.say(&m);
        }
        if self.z_suffix.is_empty() || self.z_suffix.len() > MAX_SUFFIX {
            let m = format!("{}: invalid suffix '{}'\n", self.prog(), self.z_suffix);
            self.say(&m);
            return Err(Stop(ERROR));
        }

        if files.is_empty() {
            self.treat_stdin()?;
        } else {
            for f in &files {
                self.treat_file(f)?;
            }
        }
        if self.gz.list && !self.gz.quiet && files.len() > 1 {
            self.do_list(-1)?;
        }
        Ok(())
    }

    /// The command ending at an option (`do_exit`): through `Err` so that
    /// `main` gives the code.
    fn exit_next(&mut self, next: Next) -> R<()> {
        match next {
            Next::Go => Ok(()),
            Next::Exit(code) => {
                self.stdout.flush().map_err(|e| {
                    self.gz.ofname = b"stdout".to_vec();
                    self.gz.write_error(&e)
                })?;
                Err(Stop(code))
            }
        }
    }

    /// An option from GZIP: those gzip takes there, or the end.
    fn env_option(&mut self, o: &Opt) -> Option<Next> {
        match o.c {
            'n' | 'N' | 'q' | 'v' | RSYNCABLE | '1'..='9' => None,
            '?' => Some(self.try_help()),
            c => {
                let what = match o.long {
                    Some(name) => format!("--{name}"),
                    None => format!("-{c}"),
                };
                let m = format!(
                    "{}: {what}: option not valid in GZIP environment variable\n",
                    self.prog()
                );
                self.say(&m);
                Some(self.try_help())
            }
        }
    }

    /// One option, as gzip's `switch` takes it.
    fn option(
        &mut self,
        o: &Opt,
        no_name: &mut Option<bool>,
        no_time: &mut Option<bool>,
    ) -> R<Next> {
        let gz = &mut self.gz;
        match o.c {
            'a' => self.ascii = true,
            'b' => {
                let v = o.value.clone().unwrap_or_default();
                if !v.bytes().all(|b| b.is_ascii_digit()) {
                    let m = format!("{}: -b operand is not an integer\n", self.prog());
                    self.say(&m);
                    return Ok(self.try_help());
                }
            }
            'c' => gz.to_stdout = true,
            'd' => gz.decompress = true,
            'f' => gz.force += 1,
            'h' | 'H' => {
                let mut text = format!("Usage: {} [OPTION]... [FILE]...\n", self.prog());
                for l in HELP {
                    text.push_str(l);
                    text.push('\n');
                }
                self.print(&text)?;
                return Ok(Next::Exit(OK));
            }
            'k' => self.keep = true,
            'l' => {
                gz.list = true;
                gz.decompress = true;
                gz.test = true;
                gz.to_stdout = true;
            }
            'L' => {
                let text = self.license();
                self.print(&text)?;
                return Ok(Next::Exit(OK));
            }
            'm' => *no_time = Some(true),
            'M' => *no_time = Some(false),
            'n' => {
                *no_name = Some(true);
                *no_time = Some(true);
            }
            'N' => {
                *no_name = Some(false);
                *no_time = Some(false);
            }
            PRESUME_INPUT_TTY => self.presume_input_tty = true,
            'q' => {
                gz.quiet = true;
                gz.verbose = 0;
            }
            'r' => self.recursive = true,
            RSYNCABLE => {}
            'S' => self.z_suffix = o.value.clone().unwrap_or_default(),
            SYNCHRONOUS => self.synchronous = true,
            't' => {
                gz.test = true;
                gz.decompress = true;
                gz.to_stdout = true;
            }
            'v' => {
                gz.verbose += 1;
                gz.quiet = false;
            }
            'V' => {
                let mut text = self.license();
                text.push_str("\nWritten by Jean-loup Gailly.\n");
                self.print(&text)?;
                return Ok(Next::Exit(OK));
            }
            'Z' => {
                let m = format!("{}: -Z not supported in this version\n", self.prog());
                self.say(&m);
                return Ok(self.try_help());
            }
            c @ '1'..='9' => gz.level = c as i32 - '0' as i32,
            _ => return Ok(self.try_help()),
        }
        Ok(Next::Go)
    }

    /// `license`, naming tw first.
    fn license(&self) -> String {
        let mut text = format!(
            "tw {} {} (GNU gzip 1.13)\n",
            self.prog(),
            env!("CARGO_PKG_VERSION")
        );
        for l in LICENSE {
            text.push_str(l);
            text.push('\n');
        }
        text
    }

    /// The exit code, after stdout is written out.
    fn finish(&mut self) -> i32 {
        if let Err(e) = self.stdout.flush() {
            self.gz.ofname = b"stdout".to_vec();
            let Stop(code) = self.gz.write_error(&e);
            return code;
        }
        self.gz.exit_code
    }

    /// `remove_output_file`.
    fn remove_output_file(&mut self) {
        if let Some(name) = self.remove_ofname.take() {
            let _ = fs::remove_file(name);
        }
    }

    /// `get_input_size_and_time`.
    fn input_size_and_time(&mut self, meta: Option<&Metadata>) {
        self.gz.ifile_size = -1;
        self.gz.time_stamp.nsec = -1;
        if let Some(m) = meta.filter(|m| m.is_file()) {
            self.gz.ifile_size = m.len() as i64;
            if !self.gz.no_time || self.gz.list {
                self.gz.time_stamp = mtime(m);
            }
        }
    }

    /// The work on each member, until the input ends; the last method
    /// (negative after an error or trailing data).
    fn members(&mut self, input: &mut dyn Read, to_file: Option<&mut File>) -> R<(i32, bool)> {
        let mut out_file;
        let output: &mut dyn Write = match to_file {
            Some(f) => {
                out_file = io::BufWriter::with_capacity(1 << 16, f);
                &mut out_file
            }
            None => &mut self.stdout,
        };
        let mut io = Io { input, output };
        let gz = &mut self.gz;
        let mut method = gz.method;
        let mut failed = false;
        let r = (|| -> R<()> {
            loop {
                let Some(w) = self.work else { return Ok(()) };
                if w(gz, &mut io)? != OK {
                    failed = true;
                    return Ok(());
                }
                if gz.input_eof(&mut io)? {
                    return Ok(());
                }
                method = gz.get_method(&mut io, &mut self.work)?;
                if method < 0 {
                    return Ok(());
                }
                gz.bytes_out = 0;
            }
        })();
        let flushed = io.output.flush();
        r?;
        flushed.map_err(|e| self.gz.write_error(&e))?;
        Ok((method, failed))
    }

    /// `treat_stdin`.
    fn treat_stdin(&mut self) -> R<()> {
        let tty = if self.gz.decompress {
            io::stdin().is_terminal()
        } else {
            io::stdout().is_terminal()
        };
        if self.gz.force == 0 && !self.gz.list && (self.presume_input_tty || tty) {
            if !self.gz.quiet {
                let d = self.gz.decompress;
                let m = format!(
                    "{p}: compressed data not {} a terminal. Use -f to force {}compression.\nFor help, type: {p} -h\n",
                    if d { "read from" } else { "written to" },
                    if d { "de" } else { "" },
                    p = self.prog()
                );
                self.say(&m);
            }
            return Err(Stop(ERROR));
        }
        self.gz.ifname = b"stdin".to_vec();
        self.gz.ofname = b"stdout".to_vec();
        let meta = match stdin_metadata() {
            Ok(m) => Some(m),
            Err(e) => {
                self.progerror("standard input", &e);
                return Err(Stop(ERROR));
            }
        };
        self.input_size_and_time(meta.as_ref());
        self.gz.clear_bufs();
        self.gz.to_stdout = true;
        self.gz.part_nb = 0;
        self.stdin_was_read = true;
        self.work = Some(Gzip::zip);

        let stdin = io::stdin();
        let mut input = stdin.lock();
        if self.gz.decompress {
            let method = {
                let mut io = Io {
                    input: &mut input,
                    output: &mut self.stdout,
                };
                self.gz.get_method(&mut io, &mut self.work)?
            };
            self.gz.method = method;
            if method < 0 {
                return Err(Stop(self.gz.exit_code));
            }
        }
        let (method, failed) = self.members(&mut input, None)?;
        if failed || method < 0 {
            return Ok(());
        }
        if self.gz.list {
            return self.do_list(method);
        }
        if self.gz.verbose > 0 {
            if self.gz.test {
                self.say(" OK\n");
            } else if !self.gz.decompress {
                let gz = &self.gz;
                let m = ratio(gz.bytes_in - (gz.bytes_out - gz.header_bytes), gz.bytes_in);
                self.say(&format!("{m}\n"));
            }
        }
        Ok(())
    }

    /// `open_input_file`: the input, with a suffix added when decompressing
    /// a name that does not exist; `ifname` set.
    fn open_input_file(&mut self, iname: &str) -> Option<(Option<File>, Metadata)> {
        if MAX_PATH_LEN - 1 <= iname.len() {
            let m = format!("{}: {iname}: file name too long\n", self.prog());
            self.say(&m);
            self.gz.exit_code = ERROR;
            return None;
        }
        self.gz.ifname = iname.as_bytes().to_vec();
        let e = match self.open_and_stat(iname) {
            Ok(f) => return Some(f),
            Err(e) => e,
        };
        if !self.gz.decompress || e.kind() != io::ErrorKind::NotFound {
            self.progerror(iname, &e);
            return None;
        }
        if self.get_suffix(iname).is_some() {
            self.progerror(iname, &e);
            return None;
        }
        let mut suffixes: Vec<String> = vec![self.z_suffix.clone()];
        for s in [".gz", ".z", "-z", ".Z"] {
            suffixes.push(s.into());
        }
        let skip = usize::from(self.z_suffix == ".gz");
        let mut z_suffix_err: Option<io::Error> = None;
        for s in &suffixes[skip..] {
            let name = format!("{iname}{s}");
            if MAX_PATH_LEN <= name.len() {
                let m = format!("{}: {iname}: file name too long\n", self.prog());
                self.say(&m);
                self.gz.exit_code = ERROR;
                return None;
            }
            self.gz.ifname = name.as_bytes().to_vec();
            match self.open_and_stat(&name) {
                Ok(f) => return Some(f),
                Err(e) if e.kind() != io::ErrorKind::NotFound => {
                    self.progerror(&name, &e);
                    return None;
                }
                Err(e) => {
                    if *s == self.z_suffix {
                        z_suffix_err = Some(e);
                    }
                }
            }
        }
        let name = format!("{iname}{}", self.z_suffix);
        self.gz.ifname = name.as_bytes().to_vec();
        let e = z_suffix_err.unwrap_or_else(|| io::Error::from(io::ErrorKind::NotFound));
        self.progerror(&name, &e);
        None
    }

    /// `open_and_stat`: symbolic links refused unless -c or -f.
    /// A directory is not opened (Windows cannot open one as a file).
    fn open_and_stat(&self, name: &str) -> io::Result<(Option<File>, Metadata)> {
        if !self.gz.to_stdout && self.gz.force == 0 {
            let l = fs::symlink_metadata(name)?;
            if l.file_type().is_symlink() {
                return Err(eloop());
            }
        }
        let m = fs::metadata(name)?;
        if m.is_dir() {
            return Ok((None, m));
        }
        let f = File::open(name)?;
        let m = f.metadata()?;
        Ok((Some(f), m))
    }

    /// `get_suffix`: where a compressed file's suffix starts in `name`.
    fn get_suffix(&self, name: &str) -> Option<usize> {
        let builtin = [".gz", ".z", ".taz", ".tgz", "-gz", "-z", "_z"];
        let z = &self.z_suffix;
        let z_lower = z.to_ascii_lowercase();
        let suffix_of_builtin = builtin
            .iter()
            .any(|b| z.len() < b.len() && b.ends_with(z.as_str()));
        let mut known: Vec<&str> = Vec::new();
        if !suffix_of_builtin {
            known.push(&z_lower);
        }
        known.extend(builtin);
        if suffix_of_builtin {
            known.push(&z_lower);
        }
        let bytes = name.as_bytes();
        let nlen = bytes.len();
        let tail = if nlen <= MAX_SUFFIX + 2 {
            bytes
        } else {
            &bytes[nlen - MAX_SUFFIX - 2..]
        };
        let tail = tail.to_ascii_lowercase();
        let slen = tail.len();
        for s in known {
            let sl = s.len();
            if slen > sl && tail[slen - sl - 1] != b'/' && tail.ends_with(s.as_bytes()) {
                return Some(nlen - sl);
            }
        }
        None
    }

    /// `make_ofname`: the output's name; false when the file is skipped.
    fn make_ofname(&mut self) -> bool {
        let ifname = String::from_utf8_lossy(&self.gz.ifname).into_owned();
        let mut ofname = ifname.clone();
        let suff = self.get_suffix(&ofname);
        if self.gz.decompress {
            let Some(at) = suff else {
                if !self.recursive && self.gz.test {
                    self.gz.ofname = ofname.into_bytes();
                    return true;
                }
                if self.gz.verbose > 0 || (!self.recursive && !self.gz.quiet) {
                    let m = format!("{}: {ifname}: unknown suffix -- ignored\n", self.prog());
                    self.warn(&m);
                }
                return false;
            };
            let lower = ofname[at..].to_ascii_lowercase();
            ofname.truncate(at);
            if lower == ".tgz" || lower == ".taz" {
                ofname.push_str(".tar");
            }
        } else if let (Some(at), 0) = (suff, self.gz.force) {
            if self.gz.verbose > 0 || (!self.recursive && !self.gz.quiet) {
                let m = format!(
                    "{}: {ifname} already has {} suffix -- unchanged\n",
                    self.prog(),
                    &ifname[at..]
                );
                self.say(&m);
            }
            return false;
        } else {
            self.gz.save_orig_name = false;
            if MAX_PATH_LEN <= ofname.len() + self.z_suffix.len() {
                let m = format!("{}: {ifname}: file name too long\n", self.prog());
                self.warn(&m);
                return false;
            }
            ofname.push_str(&self.z_suffix);
        }
        self.gz.ofname = ofname.into_bytes();
        true
    }

    /// `treat_file`.
    fn treat_file(&mut self, iname: &str) -> R<()> {
        if iname == "-" {
            let cflag = self.gz.to_stdout;
            self.treat_stdin()?;
            self.gz.to_stdout = cflag;
            return Ok(());
        }
        let Some((ifile, istat)) = self.open_input_file(iname) else {
            return Ok(());
        };
        let ifname = String::from_utf8_lossy(&self.gz.ifname).into_owned();
        let Some(ifile) = ifile else {
            if self.recursive {
                return self.treat_dir(iname);
            }
            let m = format!("{}: {ifname} is a directory -- ignored\n", self.prog());
            self.warn(&m);
            return Ok(());
        };
        if !self.gz.to_stdout {
            let mode = mode_of(&istat);
            let skip = if !istat.is_file() {
                Some(format!(
                    "{ifname} is not a directory or a regular file - ignored"
                ))
            } else if mode & 0o4000 != 0 {
                Some(format!("{ifname} is set-user-ID on execution - ignored"))
            } else if mode & 0o2000 != 0 {
                Some(format!("{ifname} is set-group-ID on execution - ignored"))
            } else if self.gz.force == 0 && mode & 0o1000 != 0 {
                Some(format!("{ifname} has the sticky bit set - file ignored"))
            } else if self.gz.force == 0 && nlink_of(&istat) >= 2 {
                let n = nlink_of(&istat) - 1;
                Some(format!(
                    "{ifname} has {n} other link{} -- file ignored",
                    if n == 1 { "" } else { "s" }
                ))
            } else {
                None
            };
            if let Some(why) = skip {
                let m = format!("{}: {why}\n", self.prog());
                self.warn(&m);
                return Ok(());
            }
        }
        self.input_size_and_time(Some(&istat));

        if self.gz.to_stdout && !self.gz.test {
            self.gz.ofname = b"stdout".to_vec();
        } else if !self.make_ofname() {
            return Ok(());
        }
        self.gz.clear_bufs();
        self.gz.part_nb = 0;
        self.work = Some(Gzip::zip);

        let mut input = io::BufReader::with_capacity(0, ifile);
        if self.gz.decompress {
            let method = {
                let mut io = Io {
                    input: &mut input,
                    output: &mut self.stdout,
                };
                self.gz.get_method(&mut io, &mut self.work)?
            };
            self.gz.method = method;
            if method < 0 {
                return Ok(());
            }
        }

        let mut ofile = None;
        if !self.gz.to_stdout {
            let Some(f) = self.create_outfile()? else {
                return Ok(());
            };
            ofile = Some(f);
            if !self.gz.decompress
                && self.gz.save_orig_name
                && self.gz.verbose == 0
                && !self.gz.quiet
            {
                let m = format!(
                    "{}: {ifname} compressed to {}\n",
                    self.prog(),
                    String::from_utf8_lossy(&self.gz.ofname)
                );
                self.say(&m);
            }
        }
        if !self.gz.save_orig_name {
            self.gz.save_orig_name = !self.gz.no_name;
        }
        if self.gz.verbose > 0 && !self.gz.list {
            self.say(&format!("{ifname}:\t"));
        }

        let (mut method, failed) = self.members(&mut input, ofile.as_mut())?;
        if failed {
            method = -1;
        }
        drop(input);

        if self.gz.list {
            return self.do_list(method);
        }

        if let Some(f) = ofile {
            self.copy_stat(&f, &istat);
            let synced = if self.synchronous {
                f.sync_all()
            } else {
                Ok(())
            };
            drop(f);
            if let Err(e) = synced {
                return Err(self.gz.write_error(&e));
            }
            if !self.keep {
                self.remove_ofname = None;
                if let Err(e) = fs::remove_file(&ifname) {
                    let m = format!("{}: {ifname}: {}\n", self.prog(), strerror(&e));
                    self.warn(&m);
                }
            }
        }

        if method == -1 {
            if !self.gz.to_stdout {
                self.remove_output_file();
            }
            return Ok(());
        }
        self.remove_ofname = None;

        if self.gz.verbose > 0 {
            let gz = &self.gz;
            let mut m = if gz.test {
                " OK".to_owned()
            } else if gz.decompress {
                ratio(gz.bytes_out - (gz.bytes_in - gz.header_bytes), gz.bytes_out)
            } else {
                ratio(gz.bytes_in - (gz.bytes_out - gz.header_bytes), gz.bytes_in)
            };
            if !gz.test {
                m.push_str(&format!(
                    " -- {} {}",
                    if self.keep {
                        "created"
                    } else {
                        "replaced with"
                    },
                    String::from_utf8_lossy(&gz.ofname)
                ));
            }
            m.push('\n');
            self.say(&m);
        }
        Ok(())
    }

    /// `create_outfile`: the output, new; when it exists, the question (or
    /// not overwritten); `None` when the file is skipped.
    fn create_outfile(&mut self) -> R<Option<File>> {
        let mut shortened = false;
        loop {
            let ofname = String::from_utf8_lossy(&self.gz.ofname).into_owned();
            let mut o = fs::OpenOptions::new();
            o.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                o.mode(0o600);
            }
            match o.open(&ofname) {
                Ok(f) => {
                    self.remove_ofname = Some(ofname.clone());
                    if shortened && self.gz.decompress {
                        let m = format!("{}: {ofname}: warning, name truncated\n", self.prog());
                        self.warn(&m);
                    }
                    return Ok(Some(f));
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    if !self.check_ofname(&ofname) {
                        return Ok(None);
                    }
                }
                Err(e) if is_name_too_long(&e) => {
                    let name = self.shorten_name(&ofname)?;
                    self.gz.ofname = name.into_bytes();
                    shortened = true;
                }
                Err(e) => return Err(self.gz.write_error(&e)),
            }
        }
    }

    /// `check_ofname`: whether an existing output may be replaced (and it
    /// is then removed).
    fn check_ofname(&mut self, ofname: &str) -> bool {
        if self.gz.force == 0 {
            let m = format!("{}: {ofname} already exists;", self.prog());
            self.say(&m);
            let mut ok = false;
            if self.presume_input_tty || io::stdin().is_terminal() {
                self.say(" do you wish to overwrite (y or n)? ");
                let mut line = String::new();
                let _ = io::stdin().lock().read_line(&mut line);
                ok = line.starts_with(['y', 'Y']);
            }
            if !ok {
                self.say("\tnot overwritten\n");
                if self.gz.exit_code == OK {
                    self.gz.exit_code = WARNING;
                }
                return false;
            }
        }
        if let Err(e) = fs::remove_file(ofname) {
            self.progerror(ofname, &e);
            return false;
        }
        true
    }

    /// `shorten_name`: one character less, or a part of the name, for a
    /// name too long for the file system.
    fn shorten_name(&mut self, name: &str) -> R<String> {
        let mut name = name.to_owned();
        if self.gz.decompress {
            if name.len() <= 1 {
                return Err(self.gz.gzip_error("name too short"));
            }
            name.pop();
            return Ok(name);
        }
        let Some(at) = self.get_suffix(&name) else {
            return Err(self.gz.gzip_error("can't recover suffix\n"));
        };
        name.truncate(at);
        self.gz.save_orig_name = true;
        if name.len() > 4 && name.ends_with(".tar") {
            name.truncate(name.len() - 4);
            name.push_str(".tgz");
            return Ok(name);
        }
        let base = base_start(name.as_bytes());
        let mut trunc: Option<usize> = None;
        let mut min_part = MIN_PART;
        while trunc.is_none() && min_part > 0 {
            let mut p = base;
            let b = name.as_bytes();
            while p < b.len() {
                let plen = b[p..]
                    .iter()
                    .position(|&c| c == b'.')
                    .unwrap_or(b.len() - p);
                p += plen;
                if plen > min_part {
                    trunc = Some(p - 1);
                }
                if p < b.len() {
                    p += 1;
                }
            }
            if trunc.is_none() {
                min_part -= 1;
            }
        }
        match trunc {
            Some(t) => {
                name.remove(t);
            }
            None => {
                let Some(mut t) = name.rfind('.') else {
                    return Err(self.gz.gzip_error("internal error in shorten_name"));
                };
                if t + 1 == name.len() {
                    t = t.saturating_sub(1);
                }
                name.truncate(t);
            }
        }
        name.push_str(&self.z_suffix);
        Ok(name)
    }

    /// `copy_stat`: the input's times, mode and owner on the output.
    fn copy_stat(&mut self, f: &File, istat: &Metadata) {
        let ofname = String::from_utf8_lossy(&self.gz.ofname).into_owned();
        let in_m = mtime(istat);
        let ts = self.gz.time_stamp;
        let restoring = self.gz.decompress && ts.known() && in_m != ts;
        let mut times = fs::FileTimes::new();
        if let Ok(a) = istat.accessed() {
            times = times.set_accessed(a);
        }
        let modified = if restoring {
            std::time::UNIX_EPOCH.checked_add(std::time::Duration::new(
                ts.sec.max(0) as u64,
                ts.nsec as u32,
            ))
        } else {
            istat.modified().ok()
        };
        if let Some(m) = modified {
            times = times.set_modified(m);
        }
        match f.set_times(times) {
            Ok(()) => {
                if restoring && self.gz.verbose > 1 {
                    self.say(&format!("{ofname}: timestamp restored\n"));
                }
            }
            Err(e) => {
                let m = format!("{}: {ofname}: {}\n", self.prog(), strerror(&e));
                self.warn(&m);
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt, fchown};
            let _ = fchown(f, None, Some(istat.gid()));
            if let Err(e) = f.set_permissions(fs::Permissions::from_mode(istat.mode() & 0o777)) {
                let m = format!("{}: {ofname}: {}\n", self.prog(), strerror(&e));
                self.warn(&m);
            }
            let _ = fchown(f, Some(istat.uid()), None);
        }
        #[cfg(not(unix))]
        {
            let _ = f.set_permissions(istat.permissions());
        }
    }

    /// `treat_dir`: every entry, in the directory's order.
    fn treat_dir(&mut self, dir: &str) -> R<()> {
        let entries = match fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(e) => {
                self.progerror(dir, &e);
                return Ok(());
            }
        };
        let mut names = Vec::new();
        for e in entries {
            match e {
                Ok(e) => names.push(e.file_name().to_string_lossy().into_owned()),
                Err(e) => {
                    self.progerror(dir, &e);
                    return Ok(());
                }
            }
        }
        for entry in names {
            let base = base_start(dir.as_bytes());
            let mut path = dir.to_owned();
            if base < dir.len() && !dir.ends_with('/') {
                path.push('/');
            }
            path.push_str(&entry);
            if path.len() < MAX_PATH_LEN - 1 {
                self.treat_file(&path)?;
            } else {
                let m = format!("{}: {dir}/{entry}: pathname too long\n", self.prog());
                self.say(&m);
                self.gz.exit_code = ERROR;
            }
        }
        Ok(())
    }

    /// `do_list`: a file's line, the header first; or the totals.
    fn do_list(&mut self, method: i32) -> R<()> {
        const METHODS: [&str; 9] = ["store", "compr", "pack ", "lzh  ", "", "", "", "", "defla"];
        let width = 19;
        let mut text = String::new();
        if self.first_time && method >= 0 {
            self.first_time = false;
            if self.gz.verbose > 0 {
                text.push_str("method  crc     date  time  ");
            }
            if !self.gz.quiet {
                text.push_str(&format!(
                    "{:>width$} {:>width$}  ratio uncompressed_name\n",
                    "compressed", "uncompressed"
                ));
            }
        } else if method < 0 {
            if self.total_in <= 0 || self.total_out <= 0 {
                return self.print(&text);
            }
            if self.gz.verbose > 0 {
                text.push_str("                            ");
            }
            if self.gz.verbose > 0 || !self.gz.quiet {
                text.push_str(&format!(
                    "{} {} ",
                    off(self.total_in, width),
                    off(self.total_out, width)
                ));
            }
            let gz = &self.gz;
            text.push_str(&ratio(
                self.total_out - (self.total_in - gz.header_bytes),
                self.total_out,
            ));
            text.push_str(" (totals)\n");
            return self.print(&text);
        }
        // An unsigned long, all ones when not known.
        let mut crc: u64 = !0;
        if method == DEFLATED && !self.gz.last_member {
            crc = u64::from(self.gz.unzip_crc);
        }
        if self.gz.verbose > 0 {
            let name = METHODS.get(method as usize).copied().unwrap_or("");
            text.push_str(&format!("{name:>5} {crc:08x} "));
            let tz = jiff::tz::TimeZone::system();
            match jiff::Timestamp::from_second(self.gz.time_stamp.sec) {
                Ok(t) => {
                    let z = t.to_zoned(tz);
                    const MONTHS: [&str; 12] = [
                        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct",
                        "Nov", "Dec",
                    ];
                    text.push_str(&format!(
                        "{}{:3} {:02}:{:02} ",
                        MONTHS[(z.month() - 1) as usize],
                        z.day(),
                        z.hour(),
                        z.minute()
                    ));
                }
                Err(_) => text.push_str("??? ?? ??:?? "),
            }
        }
        let gz = &mut self.gz;
        text.push_str(&format!(
            "{} {} ",
            off(gz.bytes_in, width),
            off(gz.bytes_out, width)
        ));
        if gz.bytes_in == -1 {
            self.total_in = -1;
            gz.bytes_in = 0;
            gz.bytes_out = 0;
            gz.header_bytes = 0;
        } else if self.total_in >= 0 {
            self.total_in += gz.bytes_in;
        }
        if gz.bytes_out == -1 {
            self.total_out = -1;
            gz.bytes_in = 0;
            gz.bytes_out = 0;
            gz.header_bytes = 0;
        } else if self.total_out >= 0 {
            self.total_out += gz.bytes_out;
        }
        text.push_str(&ratio(
            gz.bytes_out - (gz.bytes_in - gz.header_bytes),
            gz.bytes_out,
        ));
        text.push(' ');
        text.push_str(&String::from_utf8_lossy(&gz.ofname));
        text.push('\n');
        self.print(&text)
    }
}

/// ELOOP, as `open` with O_NOFOLLOW gives it for a link.
fn eloop() -> io::Error {
    if cfg!(target_os = "linux") {
        io::Error::from_raw_os_error(40)
    } else if cfg!(unix) {
        io::Error::from_raw_os_error(62)
    } else {
        io::Error::other("Too many levels of symbolic links")
    }
}

fn is_name_too_long(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::InvalidFilename
}
