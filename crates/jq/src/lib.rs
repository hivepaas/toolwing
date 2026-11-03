//! jq on jaq's engine (docs/design/jq.md): jaq's definitions, then jq's own
//! where jaq's differ (`prelude.jq`), compiled and run as jq runs a program.
//!
//! What the command line does with this (its flags, inputs, outputs and exit
//! codes) is `toolwing-cli`'s `jq`; the app's page will run it in a worker.

use std::path::PathBuf;

pub use jaq_all::data::{Filter, Runner};
pub use jaq_all::fmts::Format;
pub use jaq_all::fmts::{read, write};
pub use jaq_all::jaq_core::Vars;
pub use jaq_all::json::{self, Val};

use jaq_all::data::{Ctx, Data};
use jaq_all::jaq_core::load::{self, Arena, File, Loader, import, parse::Def};
use jaq_all::jaq_core::{ValT as _, compile::Compiler};
use jaq_all::jaq_std::input::RcIter;
use jaq_all::load::{FileReports, FileReportsDisp, compile_errors, load_errors};

/// jq 1.8.2's definitions where jaq's differ or are missing.
pub const PRELUDE: &str = include_str!("prelude.jq");

/// jaq's definitions, then the prelude's, so a program sees the prelude's.
pub fn defs() -> impl Iterator<Item = Def<&'static str>> {
    let prelude = load::parse(PRELUDE, |p| p.defs()).unwrap_or_default();
    jaq_all::defs().chain(prelude)
}

/// Where modules are looked for when no `-L` is given, as jq looks.
fn default_paths() -> Vec<PathBuf> {
    ["~/.jq", "$ORIGIN/../lib/jq", "$ORIGIN/../lib"]
        .map(PathBuf::from)
        .to_vec()
}

/// A compiled program, with the values of its `import "x" as $x` data.
pub struct Program {
    pub filter: Filter,
    pub imported: Vec<Val>,
}

/// Compiles `code` (shown in errors as `path`), with the `$name` variables
/// given by `--arg` and the like, modules looked for in `paths`. A failure
/// is the report jaq writes, without colour.
pub fn compile(
    path: &str,
    code: &str,
    vars: &[String],
    paths: &[PathBuf],
) -> Result<Program, String> {
    let defaults = default_paths();
    let paths = if paths.is_empty() { &defaults } else { paths };
    let vars: Vec<String> = vars.iter().map(|v| format!("${v}")).collect();
    let arena = Arena::default();
    let loader = Loader::new(defs()).with_std_read(paths);
    let file = File {
        path: PathBuf::from(path),
        code,
    };
    let modules = loader
        .load(&arena, file)
        .map_err(|e| report(load_errors(e)))?;
    let mut imported = Vec::new();
    import(&modules, |p| {
        let found = p.find(paths, "json")?;
        imported.push(read::json_array(found).map_err(|e| e.to_string())?);
        Ok(())
    })
    .map_err(|e| report(load_errors(e)))?;
    let filter = Compiler::default()
        .with_funs(jaq_all::data::funs())
        .with_global_vars(vars.iter().map(|v| &**v))
        .compile(modules)
        .map_err(|e| report(compile_errors(e)))?;
    Ok(Program { filter, imported })
}

fn report(reports: Vec<FileReports<PathBuf>>) -> String {
    reports
        .iter()
        .map(|r| {
            FileReportsDisp::new(r)
                .with_path(|p| format!("[{}]", p.display()))
                .to_string()
        })
        .collect()
}

/// Writes what jaq logs as jq writes it to stderr: `debug` as
/// `["DEBUG:",value]` on a line, `stderr` and `halt_error` as they are.
pub fn log_to_stderr() {
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            use std::io::Write as _;
            let mut err = std::io::stderr().lock();
            let _ = match record.level() {
                log::Level::Error => write!(err, "{}", record.args()),
                _ => writeln!(err, "[\"DEBUG:\",{}]", record.args()),
            };
        }
        fn flush(&self) {}
    }
    if log::set_logger(&Stderr).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }
}

/// What ends a run before its inputs do.
#[derive(Debug)]
pub enum Stop {
    /// An input that does not parse (jq exits 5).
    Input(String),
    /// `halt` or `halt_error`, with the exit code to end with.
    Halt(i32),
    /// Writing the output failed.
    Io(std::io::Error),
}

/// How a run went, for its exit code: whether the program failed on the
/// last input (jq exits 5), and the truth of the last output (for `-e`).
#[derive(Debug, Default)]
pub struct Ran {
    pub failed: bool,
    pub last: Option<bool>,
}

/// Runs `program` as jq does: on each input (or once on `null`, with
/// `runner.null_input`), its outputs to `out`. An error the program raises
/// on one input goes to `error`, and the run goes on with the next, as jq's
/// does; `filename` is what `input_filename` gives at the time.
pub fn run(
    runner: &Runner,
    program: &Program,
    vars: Vars<Val>,
    inputs: impl Iterator<Item = std::io::Result<Val>>,
    filename: impl Fn() -> Val,
    mut out: impl FnMut(Val) -> std::io::Result<()>,
    mut error: impl FnMut(String),
) -> Result<Ran, Stop> {
    let inputs: Box<dyn Iterator<Item = Result<Val, String>>> =
        Box::new(inputs.map(|r| r.map_err(|e| e.to_string())));
    let inputs = &RcIter::new(inputs);
    let mut ran = Ran::default();
    let mut process = |input: Val| -> Result<(), Stop> {
        let input_filename = filename();
        let data = Data {
            runner,
            lut: &program.filter.lut,
            inputs,
            input_filename: &input_filename,
        };
        ran.failed = false;
        for v in program
            .filter
            .id
            .run((Ctx::new(&data, vars.clone()), input))
        {
            match v {
                Ok(v) => {
                    ran.last = Some(v.as_bool());
                    out(v).map_err(Stop::Io)?;
                }
                Err(e) => match e.get_err() {
                    Ok(e) => {
                        error(e.to_string());
                        ran.failed = true;
                        break;
                    }
                    Err(e) => return Err(Stop::Halt(e.get_halt().ok().unwrap_or(5))),
                },
            }
        }
        Ok(())
    };
    if runner.null_input {
        process(Val::Null)?;
    } else {
        // `input` in the program takes from the same inputs.
        for input in inputs {
            process(input.map_err(Stop::Input)?)?;
        }
    }
    Ok(ran)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(code: &str, input: &str) -> Result<Vec<String>, String> {
        let program = compile("<test>", code, &[], &[])?;
        let runner = Runner::default();
        let inputs =
            read::json::parse_many(input.as_bytes()).map(|r| r.map_err(std::io::Error::other));
        let (mut outputs, mut errors) = (Vec::new(), Vec::new());
        let out = |v: Val| {
            outputs.push(v.to_string());
            Ok(())
        };
        let vars = Vars::new([]);
        run(
            &runner,
            &program,
            vars,
            inputs,
            || Val::Null,
            out,
            |e| errors.push(e),
        )
        .map_err(|e| format!("{e:?}"))?;
        if let Some(e) = errors.into_iter().next() {
            return Err(e);
        }
        Ok(outputs)
    }

    #[test]
    fn the_prelude_is_what_programs_see() {
        // jq's definitions, not jaq's.
        assert_eq!(
            eval(r#"[1,null,"x"] | join("-")"#, "null"),
            Ok(vec![r#""1--x""#.into()])
        );
        assert_eq!(
            eval(r#"[{"Key":"a","Value":1}] | from_entries"#, "null"),
            Ok(vec![r#"{"a":1}"#.into()])
        );
        assert_eq!(
            eval(r#"[.[] | select(IN("a","c"))]"#, r#"["a","b","c"]"#),
            Ok(vec![r#"["a","c"]"#.into()])
        );
        assert_eq!(eval("del(.[0, 2])", "[1,2,3,4]"), Ok(vec!["[2,4]".into()]));
    }

    #[test]
    fn compile_errors_are_reported() {
        let e = compile("<inline>", ".a |", &[], &[])
            .err()
            .unwrap_or_default();
        assert!(e.contains("<inline>"), "{e}");
    }
}
