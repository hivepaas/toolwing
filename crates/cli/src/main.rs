//! `tw`: toolwing's tools on the command line (docs/design/cli.md).

mod commands;
mod diffs;
mod jq;
mod sums;

/// stdout, as a command line tool's should be: when the reader has gone
/// (`tw … | head`), the process ends quietly with 141, as SIGPIPE would end
/// it, which Rust's runtime ignores.
pub struct Stdout(io::StdoutLock<'static>);

pub fn stdout() -> Stdout {
    Stdout(io::stdout().lock())
}

impl Stdout {
    fn check<T>(r: io::Result<T>) -> io::Result<T> {
        if r.as_ref()
            .is_err_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
        {
            std::process::exit(141);
        }
        r
    }

    pub fn is_terminal(&self) -> bool {
        self.0.is_terminal()
    }
}

impl Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Self::check(self.0.write(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        Self::check(self.0.flush())
    }
}

use std::fs::File;
use std::io::{self, BufWriter, IsTerminal, Read, Write};
use std::process::ExitCode;

use clap::ArgMatches;
use commands::Shape;
use toolwing_core::{Ctx, Error, ErrorKind, InputKind, Output, Tool};
use toolwing_tools::{TOOLS, group};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    // jq's arguments are read as jq reads them (jq.rs), not by clap.
    if args.get(1).is_some_and(|a| a == "jq") {
        // On a large stack, as the app's jq worker: values nested 10,000
        // deep, jq's limit, are read, compared and written by recursion.
        let jq_args = args[2..].to_vec();
        let run = std::thread::Builder::new()
            .stack_size(jq::STACK)
            .spawn(move || jq::main(&jq_args));
        return ExitCode::from(match run.map(|t| t.join()) {
            Ok(Ok(code)) => code,
            _ => 101,
        });
    }
    let matches = match commands::root().try_get_matches_from(&args) {
        Ok(m) => m,
        Err(e) => {
            let _ = e.print();
            if !e.use_stderr() {
                return ExitCode::SUCCESS; // --help, --version
            }
            // A drop-in command exits as its reference does on a usage error.
            let drop_in = args
                .get(1)
                .is_some_and(|g| group(g).first().is_some_and(|t| t.meta().drop_in()));
            // GNU diff's usage errors are trouble, 2; coreutils' are 1.
            let code = match args.get(1).map(String::as_str) {
                Some("diff") => diffs::TROUBLE,
                _ if drop_in => 1,
                _ => 2,
            };
            return ExitCode::from(code);
        }
    };
    let Some((name, sub)) = matches.subcommand() else {
        return ExitCode::from(2);
    };
    let result = match name {
        "list" => list(sub.get_flag("json")),
        "completions" => {
            if let Some(shell) = sub.get_one::<clap_complete::Shell>("shell") {
                clap_complete::generate(*shell, &mut commands::root(), "tw", &mut io::stdout());
            }
            Ok(())
        }
        "diff" => return ExitCode::from(diffs::run(sub)),
        _ => match sums::find(name) {
            Some(sum) => return ExitCode::from(sums::run(sum, sub)),
            None => run_group(name, sub),
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure {
            command,
            error,
            drop_in,
        }) => {
            report(&command, &error);
            ExitCode::from(exit_code(error.kind, drop_in))
        }
    }
}

struct Failure {
    command: String,
    error: Error,
    drop_in: bool,
}

/// Exit codes, cli.md: drop-in commands as their reference (coreutils: 1).
fn exit_code(kind: ErrorKind, drop_in: bool) -> u8 {
    if drop_in {
        return 1;
    }
    match kind {
        ErrorKind::Check => 1,
        ErrorKind::Usage => 2,
        ErrorKind::Input => 3,
        ErrorKind::Io | ErrorKind::Cancelled => 4,
    }
}

fn report(command: &str, e: &Error) {
    let at = e
        .span
        .map(|s| format!(" at line {}, column {}", s.line, s.column))
        .unwrap_or_default();
    eprintln!("tw: {command}: {}{at}", e.message);
}

fn run_group(name: &str, m: &ArgMatches) -> Result<(), Failure> {
    let tools = group(name);
    match commands::shape(&tools) {
        Shape::Single => run_native(name, tools[0], m),
        Shape::Actions => {
            let Some((action, sub)) = m.subcommand() else {
                return Ok(());
            };
            let tool = tools
                .iter()
                .find(|t| t.meta().action == action)
                .copied()
                .unwrap_or(tools[0]);
            run_native(&format!("{name} {action}"), tool, sub)
        }
        Shape::DropInFlags => {
            let tool = tools
                .iter()
                .find(|t| {
                    let meta = t.meta();
                    meta.action_flag.is_some() && m.get_flag(&format!("action:{}", meta.action))
                })
                .copied()
                .unwrap_or(tools[0]);
            run_drop_in(name, tool, m)
        }
    }
}

/// A drop-in command: input from FILE or stdin, streamed where the tool can.
fn run_drop_in(name: &str, tool: &dyn Tool, m: &ArgMatches) -> Result<(), Failure> {
    let fail = |error: Error| Failure {
        command: name.to_owned(),
        error,
        drop_in: true,
    };
    let values = commands::values(m, tool.params()).map_err(fail)?;
    let mut input: Box<dyn Read> = match m.get_one::<String>("file").map(String::as_str) {
        None | Some("-") => Box::new(io::stdin().lock()),
        Some(path) => Box::new(
            File::open(path)
                .map_err(|e| fail(Error::new(ErrorKind::Io, format!("{path}: {e}"))))?,
        ),
    };
    let mut out = BufWriter::new(stdout());
    let ran = tool.run_stream(&Ctx::new(), &mut input, &mut out, &values);
    // What was decoded before an error is written, as the reference does.
    let flushed = out.flush().map_err(Error::from);
    ran.and(flushed).map_err(fail)
}

/// A native command: the value, `-i FILE` or stdin; `--json` and `--no-newline`.
fn run_native(command: &str, tool: &dyn Tool, m: &ArgMatches) -> Result<(), Failure> {
    let fail = |error: Error| Failure {
        command: command.to_owned(),
        error,
        drop_in: false,
    };
    let values = commands::values(m, tool.params()).map_err(fail)?;
    let input = read_input(tool.meta().input, m).map_err(fail)?;
    let output = tool.run(&Ctx::new(), &input, &values).map_err(fail)?;
    let mut out = stdout();
    let newline = !m.get_flag("no_newline");
    let written = if m.get_flag("json") {
        let json = match &output {
            Output::Text(s) => serde_json::Value::from(s.as_str()),
            Output::Lines(lines) => serde_json::Value::from(lines.clone()),
            Output::Fields(fields) => serde_json::Value::Object(
                fields
                    .iter()
                    .map(|f| (f.key.clone(), serde_json::Value::from(f.value.as_str())))
                    .collect(),
            ),
            Output::Record { json, .. } => serde_json::from_str(json)
                .unwrap_or_else(|_| serde_json::Value::from(json.as_str())),
            Output::Bytes(b) => serde_json::Value::from(String::from_utf8_lossy(b).as_ref()),
        };
        writeln!(out, "{json}")
    } else {
        match &output {
            Output::Text(s) if newline => writeln!(out, "{s}"),
            Output::Text(s) => write!(out, "{s}"),
            Output::Fields(fields) | Output::Record { fields, .. } => {
                write_fields(&mut out, fields)
            }
            Output::Lines(lines) => {
                let text = lines.join("\n");
                if newline {
                    writeln!(out, "{text}")
                } else {
                    write!(out, "{text}")
                }
            }
            Output::Bytes(_) if out.is_terminal() => {
                return Err(fail(Error::usage(
                    "binary output is not written to a terminal; redirect it to a file",
                )));
            }
            Output::Bytes(b) => out.write_all(b),
        }
    };
    written
        .and_then(|()| out.flush())
        .map_err(|e| fail(e.into()))
}

/// Aligned `label  value` lines, under their sections' headings when they
/// have them.
fn write_fields(out: &mut dyn Write, fields: &[toolwing_core::Field]) -> io::Result<()> {
    let width = fields
        .iter()
        .map(|f| f.label.chars().count())
        .max()
        .unwrap_or(0);
    let mut section: Option<&str> = None;
    for f in fields {
        if f.section.as_deref() != section {
            section = f.section.as_deref();
            if let Some(s) = section {
                writeln!(out, "{s}")?;
            }
        }
        let indent = if section.is_some() { "  " } else { "" };
        let pad = width - f.label.chars().count();
        writeln!(out, "{indent}{}{}  {}", f.label, " ".repeat(pad), f.value)?;
    }
    Ok(())
}

fn read_input(kind: InputKind, m: &ArgMatches) -> Result<Vec<u8>, Error> {
    if kind == InputKind::None {
        return Ok(Vec::new());
    }
    let value = m.get_one::<String>("value").map(String::as_str);
    let mut bytes = match (value, m.get_one::<String>("input")) {
        (Some(v), _) if v != "-" => return Ok(v.as_bytes().to_vec()),
        (_, Some(path)) => {
            std::fs::read(path).map_err(|e| Error::new(ErrorKind::Io, format!("{path}: {e}")))?
        }
        // Nothing given and nothing piped in: empty input ("now" for `tw time`)
        // rather than waiting on the terminal.
        _ if io::stdin().is_terminal() => Vec::new(),
        _ => {
            let mut all = Vec::new();
            io::stdin().lock().read_to_end(&mut all)?;
            all
        }
    };
    if let InputKind::Text { trim_newline: true } = kind {
        if bytes.ends_with(b"\r\n") {
            bytes.truncate(bytes.len() - 2);
        } else if bytes.ends_with(b"\n") {
            bytes.pop();
        }
    }
    Ok(bytes)
}

fn list(json: bool) -> Result<(), Failure> {
    let rows: Vec<_> = TOOLS.iter().map(|t| t.meta()).collect();
    let mut out = stdout();
    let written = if json {
        let items: Vec<serde_json::Value> = rows
            .iter()
            .map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "group": m.group,
                    "category": m.category.id(),
                    "title": m.title,
                    "summary": m.summary,
                    "reference": m.reference.map(|r| format!("{} {}", r.name, r.version)),
                })
            })
            .collect();
        writeln!(out, "{}", serde_json::Value::from(items))
    } else {
        let width = rows.iter().map(|m| m.id.len()).max().unwrap_or(0);
        rows.iter()
            .try_for_each(|m| writeln!(out, "{:width$}  {}", m.id, m.summary))
    };
    written.map_err(|e| Failure {
        command: "list".into(),
        error: e.into(),
        drop_in: false,
    })
}
