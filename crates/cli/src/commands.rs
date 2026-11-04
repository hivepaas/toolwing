//! The commands, made from the registry (docs/design/cli.md): each group a
//! subcommand, each param a flag spelled as its `Cli` says.

use clap::{Arg, ArgAction, ArgGroup, ArgMatches, Command, value_parser};
use toolwing_core::{Error, ErrorKind, InputKind, Param, ParamKind, Tool, Value, Values};
use toolwing_tools::{group, group_info, groups};

/// What a group's command looks like.
pub enum Shape {
    /// One tool: `tw uuid -n 5`.
    Single,
    /// Drop-in with action flags: `tw base64 -d FILE`.
    DropInFlags,
    /// An action per subcommand: `tw url decode 'a%20b'`.
    Actions,
}

pub fn shape(tools: &[&'static dyn Tool]) -> Shape {
    if tools.len() == 1 {
        Shape::Single
    } else if tools[0].meta().drop_in() {
        Shape::DropInFlags
    } else {
        Shape::Actions
    }
}

pub fn root() -> Command {
    // Named as typed, not as the binary is (tw.exe on Windows).
    let mut cmd = Command::new("tw")
        .bin_name("tw")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Fast, native developer tools")
        .subcommand_required(true)
        .arg_required_else_help(true);
    for g in groups() {
        cmd = cmd.subcommand(match crate::sums::find(g) {
            Some(sum) => crate::sums::command(sum),
            None if g == "diff" => crate::diffs::command(),
            None if g == "b3sum" => crate::b3sums::command(),
            None if g == "jq" => crate::jq::command(),
            None => group_command(g),
        });
    }
    cmd.subcommand(
        Command::new("list")
            .about("List every tool")
            .arg(json_flag()),
    )
    .subcommand(
        Command::new("completions")
            .about("Print shell completions")
            .arg(
                Arg::new("shell")
                    .required(true)
                    .value_parser(value_parser!(clap_complete::Shell)),
            ),
    )
}

fn group_command(name: &'static str) -> Command {
    let tools = group(name);
    let about = group_info(name).map_or(tools[0].meta().summary, |g| g.summary);
    match shape(&tools) {
        Shape::Single => tool_args(Command::new(name).about(about), tools[0]),
        Shape::DropInFlags => {
            let mut cmd = Command::new(name).about(about).arg(
                Arg::new("file")
                    .value_name("FILE")
                    .help("Read FILE; - or none for stdin"),
            );
            let mut seen: Vec<&str> = Vec::new();
            for tool in &tools {
                let m = tool.meta();
                if let Some((short, long)) = m.action_flag {
                    cmd = cmd.arg(
                        Arg::new(format!("action:{}", m.action))
                            .short(short)
                            .long(long)
                            .help(m.summary)
                            .action(ArgAction::SetTrue),
                    );
                }
                for p in tool.params() {
                    if !seen.contains(&p.key) {
                        seen.push(p.key);
                        cmd = param_args(cmd, p);
                    }
                }
            }
            cmd
        }
        Shape::Actions => {
            let mut cmd = Command::new(name)
                .about(about)
                .subcommand_required(true)
                .arg_required_else_help(true);
            for tool in tools {
                let m = tool.meta();
                cmd = cmd.subcommand(tool_args(Command::new(m.action).about(m.summary), tool));
            }
            cmd
        }
    }
}

/// A native tool's command: its params, its input, and the output flags.
fn tool_args(mut cmd: Command, tool: &dyn Tool) -> Command {
    // `tw base -1`, `tw time -86400`: a number is the value, not a flag.
    cmd = cmd.allow_negative_numbers(true);
    for p in tool.params() {
        cmd = param_args(cmd, p);
    }
    if tool.meta().input != InputKind::None {
        cmd = cmd
            .arg(
                Arg::new("value")
                    .value_name("VALUE")
                    .help("The input itself; stdin when left out, or -"),
            )
            .arg(
                Arg::new("input")
                    .short('i')
                    .long("input")
                    .value_name("FILE")
                    .help("Read the input from FILE")
                    .conflicts_with("value"),
            );
    }
    cmd.arg(json_flag()).arg(
        Arg::new("no_newline")
            .long("no-newline")
            .help("Do not end text output with a newline")
            .action(ArgAction::SetTrue),
    )
}

fn json_flag() -> Arg {
    Arg::new("json")
        .long("json")
        .help("Write the result as JSON")
        .action(ArgAction::SetTrue)
}

fn param_args(cmd: Command, p: &'static Param) -> Command {
    let named = |arg: Arg| {
        let arg = arg.long(p.cli.long).help(p.help);
        match p.cli.short {
            Some(s) => arg.short(s),
            None => arg,
        }
    };
    match p.kind {
        // A flag that is on unless asked otherwise: `--no-upper`.
        ParamKind::Flag if p.default == Value::Bool(true) => cmd.arg(
            Arg::new(p.key)
                .long(format!("no-{}", p.cli.long))
                .help(format!("Leave out: {}", p.help))
                .action(ArgAction::SetTrue),
        ),
        ParamKind::Flag => cmd.arg(named(Arg::new(p.key)).action(ArgAction::SetTrue)),
        ParamKind::Int { min, max } => cmd.arg(
            named(Arg::new(p.key))
                .value_name("N")
                .value_parser(value_parser!(i64).range(min..=max)),
        ),
        ParamKind::Text => cmd.arg(named(Arg::new(p.key)).value_name("TEXT")),
        // The value, or where to read it: a secret need not be in shell
        // history or `ps` (cli.md).
        ParamKind::Secret => {
            let (file, env) = (format!("{}:file", p.key), format!("{}:env", p.key));
            cmd.arg(named(Arg::new(p.key)).value_name("SECRET"))
                .arg(
                    Arg::new(file.clone())
                        .long(format!("{}-file", p.cli.long))
                        .value_name("FILE")
                        .help(format!("Read the {} from FILE", p.label.to_lowercase())),
                )
                .arg(
                    Arg::new(env.clone())
                        .long(format!("{}-env", p.cli.long))
                        .value_name("NAME")
                        .help(format!(
                            "Read the {} from the environment variable NAME",
                            p.label.to_lowercase()
                        )),
                )
                .group(
                    ArgGroup::new(format!("{}:any", p.key))
                        .args([p.key.to_owned(), file, env])
                        .multiple(false),
                )
        }
        ParamKind::Choice {
            options,
            flags: false,
            ..
        } => cmd.arg(
            named(Arg::new(p.key))
                .value_name(p.key.to_uppercase())
                .value_parser(options.iter().map(|(o, _)| *o).collect::<Vec<_>>()),
        ),
        ParamKind::Choice {
            options,
            flags: true,
            ..
        } => {
            let ids: Vec<String> = options
                .iter()
                .map(|(o, _)| format!("{}={o}", p.key))
                .collect();
            let mut cmd = cmd.group(ArgGroup::new(p.key).args(ids.clone()).multiple(false));
            for (id, (option, help)) in ids.into_iter().zip(options.iter()) {
                cmd = cmd.arg(
                    Arg::new(id)
                        .long(*option)
                        .help(*help)
                        .action(ArgAction::SetTrue),
                );
            }
            cmd
        }
    }
}

/// The values given on the command line for `params`; the rest take their
/// defaults. A secret named by its file or environment variable is read here.
pub fn values(m: &ArgMatches, params: &[Param]) -> Result<Values, Error> {
    let mut v = Values::new();
    for p in params {
        match p.kind {
            ParamKind::Flag => {
                if m.get_flag(p.key) {
                    // `--no-x` turns off what is on by default.
                    v.set(p.key, Value::Bool(p.default != Value::Bool(true)));
                }
            }
            ParamKind::Int { .. } => {
                if let Some(n) = m.get_one::<i64>(p.key) {
                    v.set(p.key, Value::Int(*n));
                }
            }
            ParamKind::Text | ParamKind::Choice { flags: false, .. } => {
                if let Some(s) = m.get_one::<String>(p.key) {
                    v.set(p.key, Value::Text(s.clone()));
                }
            }
            ParamKind::Secret => {
                let file = m.get_one::<String>(&format!("{}:file", p.key));
                let env = m.get_one::<String>(&format!("{}:env", p.key));
                let secret = if let Some(s) = m.get_one::<String>(p.key) {
                    Some(s.clone())
                } else if let Some(path) = file {
                    let text = std::fs::read_to_string(path)
                        .map_err(|e| Error::new(ErrorKind::Io, format!("{path}: {e}")))?;
                    // A file written by `echo` ends with a newline the secret has not.
                    Some(text.strip_suffix('\n').unwrap_or(&text).to_owned())
                } else if let Some(name) = env {
                    Some(std::env::var(name).map_err(|_| {
                        Error::usage(format!("the environment variable {name} is not set"))
                    })?)
                } else {
                    None
                };
                if let Some(s) = secret {
                    v.set(p.key, Value::Text(s));
                }
            }
            ParamKind::Choice {
                options,
                flags: true,
                ..
            } => {
                if let Some((o, _)) = options
                    .iter()
                    .find(|(o, _)| m.get_flag(&format!("{}={o}", p.key)))
                {
                    v.set(p.key, Value::Text((*o).to_owned()));
                }
            }
        }
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_tree_is_valid() {
        root().debug_assert();
    }

    #[test]
    fn values_come_from_flags() {
        let m = root()
            .try_get_matches_from(["tw", "uuid", "generate", "--v7", "-n", "3", "-u"])
            .unwrap_or_else(|e| panic!("{e}"));
        let (_, sub) = m.subcommand().unwrap_or_else(|| panic!("no subcommand"));
        let (_, sub) = sub.subcommand().unwrap_or_else(|| panic!("no action"));
        let v = values(
            sub,
            toolwing_tools::find("uuid.generate").map_or(&[], |t| t.params()),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(v.get("version"), Some(&Value::Text("v7".into())));
        assert_eq!(v.get("count"), Some(&Value::Int(3)));
        assert_eq!(v.get("upper"), Some(&Value::Bool(true)));
    }

    /// The app's "copy as command" gives a command that parses back to the
    /// same values (architecture.md), for every tool and every kind of param.
    #[test]
    fn copied_commands_parse_back() {
        for tool in toolwing_tools::TOOLS {
            for v in sample_values(tool.params()) {
                let args = toolwing_tools::command::args(*tool, &v);
                let all = std::iter::once("tw".to_owned()).chain(args.iter().cloned());
                let m = root()
                    .try_get_matches_from(all)
                    .unwrap_or_else(|e| panic!("{args:?}: {e}"));
                let mut m = m.subcommand().map(|(_, s)| s).unwrap_or(&m);
                if let Some((_, sub)) = m.subcommand() {
                    m = sub;
                }
                let mut want = Values::new();
                for (k, value) in v.iter() {
                    let p = tool.params().iter().find(|p| p.key == k);
                    if p.is_some_and(|p| p.default != *value) {
                        want.set(k, value.clone());
                    }
                }
                assert_eq!(values(m, tool.params()).ok(), Some(want), "{args:?}");
            }
        }
    }

    /// Each param at each of its interesting values, one param at a time, and all at once.
    fn sample_values(params: &[Param]) -> Vec<Values> {
        let mut all = Values::new();
        let mut samples = vec![Values::new()];
        for p in params {
            let choices: Vec<Value> = match p.kind {
                ParamKind::Flag => vec![Value::Bool(p.default != Value::Bool(true))],
                ParamKind::Int { min, max } => vec![Value::Int(min), Value::Int(max)],
                ParamKind::Text => vec![Value::Text("a b".into())],
                // Copied as --x-env TW_SECRET: read from the environment, not
                // given back; secret_params_are_copied_by_name checks them.
                ParamKind::Secret => continue,
                ParamKind::Choice { options, .. } => options
                    .iter()
                    .map(|(o, _)| Value::Text((*o).into()))
                    .collect(),
            };
            for c in &choices {
                let mut v = Values::new();
                v.set(p.key, c.clone());
                samples.push(v);
            }
            if let Some(last) = choices.last() {
                all.set(p.key, last.clone());
            }
        }
        samples.push(all);
        samples
    }

    #[test]
    fn choice_flags_exclude_each_other() {
        assert!(
            root()
                .try_get_matches_from(["tw", "uuid", "generate", "--v4", "--v7"])
                .is_err()
        );
    }
}
