//! The `tw` command that gives a result: the app shows it ("copy as command",
//! docs/design/app.md) and the CLI's tests parse it back.

use toolwing_core::{InputKind, ParamKind, Tool, Value, Values};

use crate::group;

/// The arguments after `tw` for running `tool` with `values`, without its
/// input. Values equal to their default are left out.
pub fn args(tool: &dyn Tool, values: &Values) -> Vec<String> {
    let m = tool.meta();
    let mut args = vec![m.group.to_owned()];
    let tools = group(m.group);
    if tools.len() > 1 {
        match m.action_flag {
            Some((short, _)) => args.push(format!("-{short}")),
            None if !m.drop_in() => args.push(m.action.to_owned()),
            None => {}
        }
    }
    for p in tool.params() {
        let Some(value) = values.get(p.key) else {
            continue;
        };
        if *value == p.default {
            continue;
        }
        let name = match p.cli.short {
            Some(s) => format!("-{s}"),
            None => format!("--{}", p.cli.long),
        };
        match (p.kind, value) {
            (ParamKind::Flag, Value::Bool(true)) => args.push(name),
            (ParamKind::Flag, _) => {}
            (ParamKind::Choice { flags: true, .. }, Value::Text(s)) if !s.is_empty() => {
                args.push(format!("--{s}"))
            }
            (ParamKind::Choice { flags: true, .. }, _) => {}
            (_, Value::Int(n)) => args.extend([name, n.to_string()]),
            (_, Value::Text(s)) => args.extend([name, s.clone()]),
            (_, Value::Bool(_)) => {}
        }
    }
    args
}

/// The whole command line for a shell, with the input: the value itself when
/// it is short text on one line, otherwise read from a file.
pub fn shell_line(tool: &dyn Tool, values: &Values, input: &str) -> String {
    let mut line = String::from("tw");
    for a in args(tool, values) {
        line.push(' ');
        line.push_str(&quote(&a));
    }
    let m = tool.meta();
    match m.input {
        InputKind::None => {}
        _ if m.drop_in() => line.push_str(" < input"),
        _ if input.is_empty() => {}
        _ if input.len() <= 60 && !input.contains('\n') => {
            line.push(' ');
            line.push_str(&quote(input));
        }
        _ => line.push_str(" -i input.txt"),
    }
    line
}

/// Quotes for POSIX shells, only where needed.
pub fn quote(s: &str) -> String {
    let plain = !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./:=@%+,".contains(&b));
    if plain {
        s.to_owned()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::find;

    fn tool(id: &str) -> &'static dyn Tool {
        find(id).unwrap_or_else(|| panic!("no tool {id}"))
    }

    #[test]
    fn commands_read_as_people_type_them() {
        let mut v = Values::new();
        assert_eq!(
            shell_line(tool("base64.encode"), &v, "x"),
            "tw base64 < input"
        );
        v.set("wrap", Value::Int(0));
        assert_eq!(
            shell_line(tool("base64.encode"), &v, "x"),
            "tw base64 -w 0 < input"
        );
        v.set("ignore_garbage", Value::Bool(true));
        assert_eq!(
            shell_line(tool("base64.decode"), &v, ""),
            "tw base64 -d -i < input"
        );

        let mut v = Values::new();
        v.set("form", Value::Bool(true));
        assert_eq!(
            shell_line(tool("url.encode"), &v, "a b"),
            "tw url encode --form 'a b'"
        );
        assert_eq!(
            shell_line(tool("url.decode"), &v, "it's"),
            r"tw url decode --form 'it'\''s'"
        );

        let mut v = Values::new();
        v.set("version", Value::Text("v7".into()));
        v.set("count", Value::Int(5));
        assert_eq!(shell_line(tool("uuid"), &v, ""), "tw uuid --v7 -n 5");
    }
}
