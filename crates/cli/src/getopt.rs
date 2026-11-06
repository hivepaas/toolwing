//! glibc's `getopt_long`, as the reference commands written in C read their
//! arguments with it (ipcalc, qrencode): options and operands in any order
//! (glibc permutes them), short options clustered (`-nb`) and their values
//! joined or next (`-S24`, `-S 24`), long ones abbreviated to any unique
//! prefix (`--no-d`), `--name=value` or `--name value`, and `--` ending the
//! options. What `getopt_long` returns '?' for (an unknown option, an
//! ambiguous one, a value missing or given where none is taken) is `None`,
//! or with `getopt_errors`, the error glibc prints for it.

/// An option, as the command's own `switch` names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Got<T> {
    pub opt: T,
    pub value: Option<String>,
    /// Given by its long name (qrencode's `--help` and `-h` differ).
    pub long: bool,
    /// The long option's full name, as matched (glibc's `longind`).
    pub name: Option<&'static str>,
}

/// Why `getopt_long` returned '?', as glibc says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptError {
    /// `-x` not among the short options.
    Invalid(char),
    /// `-x` without the value it takes.
    ShortNeedsValue(char),
    /// `--name...` matching no long option: the argument after `--`.
    Unrecognized(String),
    /// `--name...` matching several: the argument after `--`, and the
    /// options it could be.
    Ambiguous(String, Vec<String>),
    /// `--name=value` for an option without a value: its full name.
    NoValueTaken(String),
    /// `--name` without the value it takes: its full name.
    LongNeedsValue(String),
}

impl OptError {
    /// glibc's message, with the program's name first.
    pub fn message(&self, prog: &str) -> String {
        match self {
            OptError::Invalid(c) => format!("{prog}: invalid option -- '{c}'\n"),
            OptError::ShortNeedsValue(c) => {
                format!("{prog}: option requires an argument -- '{c}'\n")
            }
            OptError::Unrecognized(a) => format!("{prog}: unrecognized option '--{a}'\n"),
            OptError::Ambiguous(a, names) => {
                let mut m = format!("{prog}: option '--{a}' is ambiguous; possibilities:");
                for n in names {
                    m.push_str(&format!(" '--{n}'"));
                }
                m.push('\n');
                m
            }
            OptError::NoValueTaken(n) => {
                format!("{prog}: option '--{n}' doesn't allow an argument\n")
            }
            OptError::LongNeedsValue(n) => {
                format!("{prog}: option '--{n}' requires an argument\n")
            }
        }
    }
}

/// The options in order (`None` for each that is '?'), then the operands.
pub fn getopt<T: Copy + PartialEq>(
    args: &[String],
    short: &str,
    short_opt: impl Fn(char) -> T,
    long: &[(&'static str, bool, T)],
) -> (Vec<Option<Got<T>>>, Vec<String>) {
    let (opts, operands) = getopt_errors(args, short, short_opt, long);
    (opts.into_iter().map(Result::ok).collect(), operands)
}

/// The options in order, each '?' with glibc's reason, then the operands.
pub fn getopt_errors<T: Copy + PartialEq>(
    args: &[String],
    short: &str,
    short_opt: impl Fn(char) -> T,
    long: &[(&'static str, bool, T)],
) -> (Vec<Result<Got<T>, OptError>>, Vec<String>) {
    let (mut opts, mut operands) = (Vec::new(), Vec::new());
    let got = |opt, value, name: Option<&'static str>| {
        Ok(Got {
            opt,
            value,
            long: name.is_some(),
            name,
        })
    };
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        if a == "--" {
            operands.extend(args[i..].iter().cloned());
            break;
        }
        if let Some(body) = a.strip_prefix("--") {
            let (name, value) = match body.split_once('=') {
                Some((n, v)) => (n, Some(v.to_owned())),
                None => (body, None),
            };
            let found = match long.iter().find(|(n, ..)| *n == name) {
                Some(o) => Some(o),
                None => {
                    let matches: Vec<_> =
                        long.iter().filter(|(n, ..)| n.starts_with(name)).collect();
                    match matches.as_slice() {
                        [] => {
                            opts.push(Err(OptError::Unrecognized(body.to_owned())));
                            continue;
                        }
                        [first, rest @ ..] => {
                            // Ambiguous unless every match is the same
                            // option; the possibilities are the first and
                            // those that differ from it.
                            let differ: Vec<_> = rest
                                .iter()
                                .filter(|o| o.1 != first.1 || o.2 != first.2)
                                .collect();
                            if differ.is_empty() {
                                Some(*first)
                            } else {
                                let mut names = vec![first.0.to_owned()];
                                names.extend(differ.iter().map(|o| o.0.to_owned()));
                                opts.push(Err(OptError::Ambiguous(body.to_owned(), names)));
                                continue;
                            }
                        }
                    }
                }
            };
            let Some((full, takes, opt)) = found else {
                continue;
            };
            match (takes, value) {
                (false, Some(_)) => opts.push(Err(OptError::NoValueTaken((*full).to_owned()))),
                (false, None) => opts.push(got(*opt, None, Some(*full))),
                (true, Some(v)) => opts.push(got(*opt, Some(v), Some(*full))),
                (true, None) => match args.get(i) {
                    Some(v) => {
                        i += 1;
                        opts.push(got(*opt, Some(v.clone()), Some(*full)));
                    }
                    None => opts.push(Err(OptError::LongNeedsValue((*full).to_owned()))),
                },
            }
            continue;
        }
        if a.len() > 1 && a.starts_with('-') {
            let chars: Vec<char> = a[1..].chars().collect();
            let mut k = 0;
            while k < chars.len() {
                let c = chars[k];
                k += 1;
                let Some(pos) = short.find(c).filter(|_| c != ':') else {
                    opts.push(Err(OptError::Invalid(c)));
                    continue;
                };
                if !short[pos + c.len_utf8()..].starts_with(':') {
                    opts.push(got(short_opt(c), None, None));
                    continue;
                }
                if k < chars.len() {
                    opts.push(got(short_opt(c), Some(chars[k..].iter().collect()), None));
                } else if let Some(v) = args.get(i) {
                    i += 1;
                    opts.push(got(short_opt(c), Some(v.clone()), None));
                } else {
                    opts.push(Err(OptError::ShortNeedsValue(c)));
                }
                break;
            }
            continue;
        }
        operands.push(a.clone());
    }
    (opts, operands)
}
