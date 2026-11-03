//! `tw jq`: jq 1.8.2's command line (docs/design/jq.md), on `toolwing-jq`.
//!
//! Its arguments are read as jq reads them, by hand: options anywhere, the
//! first other argument the filter (or a file with `-f`), the rest files,
//! or positional values after `--args` and `--jsonargs`. Exit codes are
//! jq's: 2 usage or I/O, 3 a filter that does not compile, 5 a runtime
//! error or an input that does not parse, and with `-e` 1 when the last
//! output is false or null, 4 when there was none. A runtime error on one
//! input does not stop the next; the last input decides. The flow follows
//! jaq's own `main.rs` (MIT).

use std::cell::RefCell;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use toolwing_jq::json::write::{Pp, Styles};
use toolwing_jq::write::{Writer, with_stdout, write};
use toolwing_jq::{Format, Runner, Stop, Val, Vars, read};

/// `tw jq` in `tw --help` and completions; `main` reads its arguments.
pub fn command() -> clap::Command {
    clap::Command::new("jq")
        .about("Query and transform JSON, as jq 1.8 does")
        .disable_help_flag(true)
        .arg(
            clap::Arg::new("args")
                .value_name("ARGS")
                .num_args(0..)
                .allow_hyphen_values(true)
                .trailing_var_arg(true),
        )
}

/// The reference `tw jq` stands in for.
pub const JQ_VERSION: &str = "1.8.2";

#[derive(Default)]
struct Cli {
    from: Option<Format>,
    to: Option<Format>,
    null_input: bool,
    slurp: bool,
    /// `-c`, `--tab` and `--indent` undo each other: the last one counts.
    compact: bool,
    indent: Option<String>,
    join: bool,
    sort_keys: bool,
    color: bool,
    monochrome: bool,
    from_file: bool,
    library_path: Vec<PathBuf>,
    /// `--arg` and the like, in order; a name given twice keeps its first.
    named: Vec<(String, Val)>,
    filter: Option<String>,
    files: Vec<String>,
    positional: Vec<Val>,
    exit_status: bool,
    unbuffered: bool,
    ascii: bool,
    version: bool,
    help: bool,
}

/// What an argument that is not an option is, after the filter.
#[derive(Clone, Copy)]
enum Mode {
    Files,
    Args,
    JsonArgs,
}

/// A usage error: the message, then exit 2.
struct Usage(String);

/// jq's `isoptish`: `-` and `-1` are not options, `-x` and `--x` are.
fn is_option(arg: &str) -> bool {
    let mut chars = arg.chars();
    chars.next() == Some('-')
        && chars
            .next()
            .is_some_and(|c| c == '-' || c.is_ascii_alphabetic())
}

fn parse(args: &[String]) -> Result<Cli, Usage> {
    let mut cli = Cli::default();
    let mut mode = Mode::Files;
    let mut it = args.iter();
    let mut options_done = false;
    while let Some(arg) = it.next() {
        if options_done || !is_option(arg) {
            positional(&mut cli, mode, arg)?;
        } else if arg == "--" {
            options_done = true;
        } else if let Some(long) = arg.strip_prefix("--") {
            parse_long(&mut cli, &mut mode, long, &mut it)?;
        } else {
            let flags = &arg[1..];
            for (i, c) in flags.char_indices() {
                // -L takes the rest of its argument, or the next one.
                if c == 'L' {
                    let attached = &flags[i + 1..];
                    let dir = if attached.is_empty() {
                        next(&mut it, "-L")?
                    } else {
                        attached.to_owned()
                    };
                    cli.library_path.push(dir.into());
                    break;
                }
                parse_short(&mut cli, c)?;
            }
        }
    }
    Ok(cli)
}

fn next<'a>(it: &mut impl Iterator<Item = &'a String>, option: &str) -> Result<String, Usage> {
    it.next()
        .cloned()
        .ok_or_else(|| Usage(format!("{option} takes a parameter")))
}

fn json(text: &str, option: &str) -> Result<Val, Usage> {
    read::json::parse_single(text.as_bytes())
        .map_err(|_| Usage(format!("invalid JSON text passed to --{option}")))
}

fn positional(cli: &mut Cli, mode: Mode, arg: &str) -> Result<(), Usage> {
    if cli.filter.is_none() {
        cli.filter = Some(arg.to_owned());
        return Ok(());
    }
    match mode {
        Mode::Files => cli.files.push(arg.to_owned()),
        Mode::Args => cli.positional.push(Val::from(arg.to_owned())),
        Mode::JsonArgs => cli.positional.push(json(arg, "jsonargs")?),
    }
    Ok(())
}

fn named(
    cli: &mut Cli,
    name: String,
    value: impl FnOnce() -> Result<Val, Usage>,
) -> Result<(), Usage> {
    if !cli.named.iter().any(|(n, _)| *n == name) {
        let value = value()?;
        cli.named.push((name, value));
    }
    Ok(())
}

fn format_of(option: &str, it: &mut std::slice::Iter<String>) -> Result<Format, Usage> {
    let name = next(it, option)?;
    Format::parse(&name).ok_or_else(|| {
        Usage(format!(
            "{option}: no format {name:?} (formats: {})",
            Format::ALL
        ))
    })
}

fn parse_long(
    cli: &mut Cli,
    mode: &mut Mode,
    long: &str,
    it: &mut std::slice::Iter<String>,
) -> Result<(), Usage> {
    match long {
        _ if let Some((c, _)) = SHORT.iter().find(|(_, l)| *l == long) => parse_short(cli, *c)?,
        "raw-output0" => {
            cli.to = Some(Format::Raw0);
            cli.join = true;
        }
        "tab" => {
            cli.compact = false;
            cli.indent = Some("\t".to_owned());
        }
        "indent" => {
            let n = next(it, "--indent")?;
            let n: i8 = n
                .parse()
                .ok()
                .filter(|n| (-1..=7).contains(n))
                .ok_or_else(|| Usage("--indent takes a number between -1 and 7".into()))?;
            cli.compact = false;
            cli.indent = Some(" ".repeat(n.unsigned_abs().into()));
        }
        "unbuffered" => cli.unbuffered = true,
        "library-path" => cli.library_path.push(next(it, "--library-path")?.into()),
        "arg" => {
            let (name, value) = (next(it, "--arg")?, next(it, "--arg")?);
            named(cli, name, || Ok(Val::from(value)))?;
        }
        "argjson" => {
            let (name, text) = (next(it, "--argjson")?, next(it, "--argjson")?);
            named(cli, name, || json(&text, "argjson"))?;
        }
        "rawfile" | "slurpfile" => {
            let (name, path) = (next(it, long)?, next(it, long)?);
            named(cli, name, || {
                let bad = |e: io::Error| Usage(format!("Bad JSON in --{long} {path}: {e}"));
                if long == "rawfile" {
                    Ok(Val::utf8_str(read::load_file(&path).map_err(bad)?))
                } else {
                    read::json_array(&path).map_err(bad)
                }
            })?;
        }
        "args" => *mode = Mode::Args,
        "jsonargs" => *mode = Mode::JsonArgs,
        // jaq's formats, which jq has not.
        "from" => cli.from = Some(format_of("--from", it)?),
        "to" => cli.to = Some(format_of("--to", it)?),
        "seq" | "stream" | "stream-errors" => {
            return Err(Usage(format!(
                "--{long} is not supported yet (docs/design/jq.md)"
            )));
        }
        other => return Err(Usage(format!("Unknown option --{other}"))),
    }
    Ok(())
}

/// The options with a short name, and their long one.
const SHORT: &[(char, &str)] = &[
    ('s', "slurp"),
    ('r', "raw-output"),
    ('j', "join-output"),
    ('c', "compact-output"),
    ('C', "color-output"),
    ('M', "monochrome-output"),
    ('S', "sort-keys"),
    ('R', "raw-input"),
    ('n', "null-input"),
    ('f', "from-file"),
    ('e', "exit-status"),
    ('h', "help"),
    ('V', "version"),
    ('b', "binary"),
    ('a', "ascii-output"),
];

fn parse_short(cli: &mut Cli, c: char) -> Result<(), Usage> {
    match c {
        's' => cli.slurp = true,
        'r' => {
            cli.to.get_or_insert(Format::Raw);
        }
        'j' => {
            cli.to.get_or_insert(Format::Raw);
            cli.join = true;
        }
        'c' => cli.compact = true,
        'C' => cli.color = true,
        'M' => cli.monochrome = true,
        'S' => cli.sort_keys = true,
        'R' => cli.from = Some(Format::Raw),
        'n' => cli.null_input = true,
        'f' => cli.from_file = true,
        'e' => cli.exit_status = true,
        'h' => cli.help = true,
        'V' => cli.version = true,
        // Windows' binary mode: what tw always does.
        'b' => {}
        'a' => cli.ascii = true,
        other => return Err(Usage(format!("Unknown option -{other}"))),
    }
    Ok(())
}

const HELP: &str = "Usage: tw jq [OPTIONS] FILTER [FILES...]
       tw jq [OPTIONS] --args FILTER [ARGUMENTS...]

jq 1.8's command line on jaq's engine: query and transform JSON, and with
--from and --to also YAML, TOML, XML, CBOR and CSV.

  -n, --null-input          use null as the single input
  -R, --raw-input           read each line as a string
  -s, --slurp               read all inputs into one array
  -c, --compact-output      one line per output
  -r, --raw-output          strings without quotes
      --raw-output0         like -r, a NUL after each output
  -j, --join-output         like -r, no newline after each output
  -S, --sort-keys           sort the keys of objects
  -a, --ascii-output        escape characters outside ASCII as \\uXXXX
  -C, --color-output        colour the output; -M, --monochrome-output not
      --tab, --indent N     indent with a tab, or N spaces (up to 7)
  -f, --from-file           read the filter from a file
  -L DIR                    look for modules in DIR
      --arg NAME VALUE      $NAME is the string VALUE
      --argjson NAME JSON   $NAME is the JSON value
      --slurpfile NAME FILE $NAME is an array of the JSON values in FILE
      --rawfile NAME FILE   $NAME is the text of FILE
      --args, --jsonargs    the remaining arguments are $ARGS.positional
  -e, --exit-status         exit 1 if the last output is false or null, 4 if none
      --from FMT, --to FMT  read or write FMT: json, yaml, toml, xml, cbor, csv, tsv, raw
  -V, --version             the version
";

/// Runs `tw jq` with its arguments (those after `jq`); the exit code.
pub fn main(args: &[String]) -> u8 {
    let cli = match parse(args) {
        Ok(cli) => cli,
        Err(Usage(message)) => {
            eprintln!("tw jq: {message}");
            eprintln!("Use tw jq --help for help with command-line options.");
            return 2;
        }
    };
    if cli.help {
        print!("{HELP}");
        return 0;
    }
    if cli.version {
        println!("tw jq {} (jq {JQ_VERSION})", env!("CARGO_PKG_VERSION"));
        return 0;
    }
    // Without a filter jq runs `.`, unless both ends are a terminal.
    if cli.filter.is_none()
        && (cli.from_file || (io::stdin().is_terminal() && io::stdout().is_terminal()))
    {
        eprint!("{HELP}");
        return 2;
    }
    toolwing_jq::log_to_stderr();
    match run(cli) {
        Ok(code) => code,
        Err(Failure::Io(_, e) | Failure::Stop(Stop::Io(e)))
            if e.kind() == io::ErrorKind::BrokenPipe =>
        {
            141
        }
        Err(Failure::Io(file, e)) => {
            eprintln!(
                "tw jq: error: {}{e}",
                file.map(|f| format!("{f}: ")).unwrap_or_default()
            );
            2
        }
        Err(Failure::Compile(report)) => {
            eprint!("{report}");
            3
        }
        Err(Failure::Stop(Stop::Input(e))) => {
            eprintln!("tw jq: parse error: {e}");
            5
        }
        // An exit status is a byte: `halt_error(256)` exits 0, as jq does.
        Err(Failure::Stop(Stop::Halt(code))) => u8::try_from(code.rem_euclid(256)).unwrap_or(5),
        Err(Failure::Stop(Stop::Io(e))) => {
            eprintln!("tw jq: error: {e}");
            2
        }
    }
}

enum Failure {
    Io(Option<String>, io::Error),
    Compile(String),
    Stop(Stop),
}

impl From<io::Error> for Failure {
    fn from(e: io::Error) -> Self {
        Failure::Io(None, e)
    }
}

fn run(cli: Cli) -> Result<u8, Failure> {
    let runner = Runner {
        null_input: cli.null_input,
        color_err: false,
        writer: writer(&cli),
    };

    let (path, code) = match &cli.filter {
        Some(f) if cli.from_file => (
            f.clone(),
            std::fs::read_to_string(f).map_err(|e| Failure::Io(Some(f.clone()), e))?,
        ),
        Some(f) => ("<inline>".to_owned(), f.clone()),
        None => ("<inline>".to_owned(), ".".to_owned()),
    };
    // The named variables, then $ARGS and $ENV, then imported data.
    let mut names: Vec<String> = cli.named.iter().map(|(k, _)| k.clone()).collect();
    names.extend(["ARGS".to_owned(), "ENV".to_owned()]);
    let program =
        toolwing_jq::compile(&path, &code, &names, &cli.library_path).map_err(Failure::Compile)?;
    let key = |k: &str| Val::from(k.to_owned());
    let named = cli.named.iter().map(|(k, v)| (key(k), v.clone()));
    let args = [
        (key("positional"), cli.positional.iter().cloned().collect()),
        (key("named"), Val::obj(named.collect())),
    ];
    let env = std::env::vars().map(|(k, v)| (Val::from(k), Val::from(v)));
    let mut values: Vec<Val> = cli.named.iter().map(|(_, v)| v.clone()).collect();
    values.push(Val::obj(args.into_iter().collect()));
    values.push(Val::obj(env.collect()));
    values.extend(program.imported.iter().cloned());
    let vars = Vars::new(values);

    let mut out = io::stdout();
    // `-a`: as jq, a string is written as JSON even with `-r` or `-j`.
    let json_writer = Writer {
        format: Format::Json,
        pp: writer(&cli).pp,
        join: runner.writer.join,
    };
    let emit = |w: &mut dyn Write, v: &Val| -> io::Result<()> {
        if cli.ascii {
            let raw = matches!(runner.writer.format, Format::Raw | Format::Raw0);
            let as_json = raw && matches!(v, Val::TStr(_) | Val::BStr(_));
            let mut buf = Vec::new();
            write(
                &mut buf,
                if as_json {
                    &json_writer
                } else {
                    &runner.writer
                },
                v,
            )?;
            w.write_all(toolwing_jq::doc::escape_ascii(&String::from_utf8_lossy(&buf)).as_bytes())?;
        } else {
            write(w, &runner.writer, v)?;
        }
        if cli.unbuffered {
            w.flush()?;
        }
        Ok(())
    };
    // jq reads stdin, or its files, as one stream of inputs: `-n '[inputs]'
    // a b` sees both files, and `-s` slurps them into one array. A file that
    // cannot be read ends the files there, and the run with exit 2.
    let stdin = io::stdin();
    let current = RefCell::new("<stdin>".to_owned());
    let mut unreadable = None;
    let mut loaded = Vec::new();
    for f in &cli.files {
        let bytes = if f == "-" {
            let mut all = Vec::new();
            stdin.lock().read_to_end(&mut all).map(|_| all.into())
        } else {
            read::load_file(f)
        };
        match bytes {
            Ok(bytes) => {
                let fmt = cli.from.or_else(|| Format::determine(Path::new(f)));
                let name = if f == "-" { "<stdin>" } else { f.as_str() };
                loaded.push((name, fmt.unwrap_or_default(), bytes));
            }
            Err(e) => {
                unreadable = Some((f, e));
                break;
            }
        }
    }
    let raw_slurp = cli.slurp && matches!(cli.from, Some(Format::Raw));
    if raw_slurp && let Some(&(name, ..)) = loaded.last() {
        // One string, the files' text end to end.
        let all: Vec<u8> = loaded
            .iter()
            .flat_map(|(.., b)| b.iter().copied())
            .collect();
        loaded = vec![(name, Format::Raw, all.into())];
    }
    let texts = loaded
        .iter()
        .map(|(_, fmt, bytes)| read::bytes_str(*fmt, bytes))
        .collect::<io::Result<Vec<_>>>()?;
    let stdin_text;
    let inputs: Box<dyn Iterator<Item = io::Result<Val>> + '_> = if cli.files.is_empty() {
        let fmt = cli.from.unwrap_or_default();
        stdin_text = read::read_string(fmt, stdin.lock())?;
        read::read(fmt, stdin.lock(), &stdin_text, cli.slurp)
    } else {
        let each = loaded
            .iter()
            .zip(&texts)
            .flat_map(|((name, fmt, bytes), text)| {
                current.replace((*name).to_owned());
                read::parse(*fmt, bytes, text, raw_slurp)
            });
        if cli.slurp && !raw_slurp {
            Box::new(std::iter::once(each.collect()))
        } else {
            Box::new(each)
        }
    };
    let ran = with_stdout(&mut out, |o| {
        toolwing_jq::run(
            &runner,
            &program,
            vars,
            inputs,
            || Val::from(current.borrow().clone()),
            |v| emit(o, &v),
            |e| match e {
                Val::TStr(s) | Val::BStr(s) => {
                    let s = String::from_utf8_lossy(&s);
                    eprintln!("tw jq: error (at {}): {s}", current.borrow());
                }
                e => eprintln!("tw jq: error (at {}) (not a string): {e}", current.borrow()),
            },
        )
    })
    .map_err(Failure::Stop)?;
    if let Some((f, e)) = unreadable {
        eprintln!("tw jq: error: Could not open {f}: {e}");
        return Ok(2);
    }
    // As jq's: the last input decides, and with `-e` the last output.
    Ok(match (ran.failed, cli.exit_status, ran.last) {
        (true, ..) => 5,
        (false, false, _) | (false, true, Some(true)) => 0,
        (false, true, Some(false)) => 1,
        (false, true, None) => 4,
    })
}

fn writer(cli: &Cli) -> Writer {
    let no_color = std::env::var("NO_COLOR").is_ok_and(|v| !v.is_empty());
    let color = !cli.monochrome && (cli.color || (!no_color && io::stdout().is_terminal()));
    let styles = match std::env::var("JQ_COLORS") {
        _ if !color => Styles::default(),
        Ok(s) => Styles::ansi().parse(&s),
        Err(_) => Styles::ansi(),
    };
    let indent = || cli.indent.clone().unwrap_or_else(|| "  ".to_owned());
    Writer {
        pp: Pp {
            indent: (!cli.compact).then(indent),
            sort_keys: cli.sort_keys,
            styles,
            sep_space: !cli.compact || matches!(cli.to, Some(Format::Yaml)),
        },
        format: cli.to.unwrap_or_default(),
        join: cli.join,
    }
}
