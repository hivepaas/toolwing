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

/// Whether a long option takes a value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Has {
    No,
    Required,
    /// Only as `--name=value` (`optional_argument`).
    Optional,
}

/// What `getopt_long` gives in order when its option string starts with
/// `-` (`RETURN_IN_ORDER`): options, operands where they stand, and `--`.
pub enum Event<T> {
    Opt(Got<T>),
    Operand(String),
    Err(OptError),
    /// `--`: getopt returns -1; what follows is left at `optind`.
    End(Vec<String>),
}

/// `getopt_long` in `RETURN_IN_ORDER` mode, with optional values: the
/// events in order.
pub fn getopt_in_order<T: Copy + PartialEq>(
    args: &[String],
    short: &str,
    short_opt: impl Fn(char) -> T,
    long: &[(&'static str, Has, T)],
) -> Vec<Event<T>> {
    let mut g = InOrder::new(args, short, long);
    let mut events = Vec::new();
    while let Some(e) = g.next(&short_opt) {
        events.push(e);
    }
    events
}

/// `getopt_long` in `RETURN_IN_ORDER` mode, an event at a time, for a
/// caller that also takes arguments itself (`optind`), as sort does.
pub struct InOrder<'a, T> {
    args: &'a [String],
    short: &'a str,
    long: &'a [(&'static str, Has, T)],
    /// The next argument (`optind`).
    pub optind: usize,
    /// Short options left in the current argument.
    cluster: Vec<char>,
    /// Whether the last option's value was the next argument (`optarg ==
    /// argv[optind - 1]`), which sort's `-y` may give back.
    pub value_was_next: bool,
}

impl<'a, T: Copy + PartialEq> InOrder<'a, T> {
    pub fn new(args: &'a [String], short: &'a str, long: &'a [(&'static str, Has, T)]) -> Self {
        InOrder {
            args,
            short,
            long,
            optind: 0,
            cluster: Vec::new(),
            value_was_next: false,
        }
    }

    /// The argument at `optind`, if any.
    pub fn peek(&self) -> Option<&'a String> {
        self.args.get(self.optind)
    }

    /// The next event; `None` when the arguments are all read.
    pub fn next(&mut self, short_opt: &impl Fn(char) -> T) -> Option<Event<T>> {
        self.value_was_next = false;
        if !self.cluster.is_empty() {
            return Some(self.next_short(short_opt));
        }
        let a = self.args.get(self.optind)?;
        self.optind += 1;
        if a == "--" {
            let rest = self.args[self.optind..].to_vec();
            self.optind = self.args.len();
            return Some(Event::End(rest));
        }
        if let Some(body) = a.strip_prefix("--") {
            return Some(self.next_long(body));
        }
        if a.len() > 1 && a.starts_with('-') {
            self.cluster = a[1..].chars().rev().collect();
            return Some(self.next_short(short_opt));
        }
        Some(Event::Operand(a.clone()))
    }

    fn next_long(&mut self, body: &str) -> Event<T> {
        let (name, value) = match body.split_once('=') {
            Some((n, v)) => (n, Some(v.to_owned())),
            None => (body, None),
        };
        let long = self.long;
        let found: &(&'static str, Has, T) = match long.iter().find(|(n, ..)| *n == name) {
            Some(o) => o,
            None => {
                let matches: Vec<&(&'static str, Has, T)> =
                    long.iter().filter(|(n, ..)| n.starts_with(name)).collect();
                match matches.as_slice() {
                    [] => return Event::Err(OptError::Unrecognized(body.to_owned())),
                    [first, rest @ ..] => {
                        let differ: Vec<_> = rest
                            .iter()
                            .filter(|o| o.1 != first.1 || o.2 != first.2)
                            .collect();
                        if !differ.is_empty() {
                            let mut names = vec![first.0.to_owned()];
                            names.extend(differ.iter().map(|o| o.0.to_owned()));
                            return Event::Err(OptError::Ambiguous(body.to_owned(), names));
                        }
                        first
                    }
                }
            }
        };
        let (full, has, opt) = *found;
        let got = |value| {
            Event::Opt(Got {
                opt,
                value,
                long: true,
                name: Some(full),
            })
        };
        match (has, value) {
            (Has::No, Some(_)) => Event::Err(OptError::NoValueTaken(full.to_owned())),
            (Has::No, None) | (Has::Optional, None) => got(None),
            (_, Some(v)) => got(Some(v)),
            (Has::Required, None) => match self.args.get(self.optind) {
                Some(v) => {
                    self.optind += 1;
                    self.value_was_next = true;
                    got(Some(v.clone()))
                }
                None => Event::Err(OptError::LongNeedsValue(full.to_owned())),
            },
        }
    }

    fn next_short(&mut self, short_opt: &impl Fn(char) -> T) -> Event<T> {
        let Some(c) = self.cluster.pop() else {
            return Event::Err(OptError::Invalid('-'));
        };
        let Some(pos) = self.short.find(c).filter(|_| c != ':' && c != '-') else {
            return Event::Err(OptError::Invalid(c));
        };
        let got = |value| {
            Event::Opt(Got {
                opt: short_opt(c),
                value,
                long: false,
                name: None,
            })
        };
        if !self.short[pos + c.len_utf8()..].starts_with(':') {
            return got(None);
        }
        if !self.cluster.is_empty() {
            let v: String = self.cluster.iter().rev().collect();
            self.cluster.clear();
            return got(Some(v));
        }
        match self.args.get(self.optind) {
            Some(v) => {
                self.optind += 1;
                self.value_was_next = true;
                got(Some(v.clone()))
            }
            None => Event::Err(OptError::ShortNeedsValue(c)),
        }
    }
}
