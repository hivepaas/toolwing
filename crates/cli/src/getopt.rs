//! glibc's `getopt_long`, as the reference commands written in C read their
//! arguments with it (ipcalc, qrencode): options and operands in any order
//! (glibc permutes them), short options clustered (`-nb`) and their values
//! joined or next (`-S24`, `-S 24`), long ones abbreviated to any unique
//! prefix (`--no-d`), `--name=value` or `--name value`, and `--` ending the
//! options. What `getopt_long` returns '?' for (an unknown option, an
//! ambiguous one, a value missing or given where none is taken) is `None`.

/// An option, as the command's own `switch` names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Got<T> {
    pub opt: T,
    pub value: Option<String>,
    /// Given by its long name (qrencode's `--help` and `-h` differ).
    pub long: bool,
}

/// The options in order (`None` for each that is '?'), then the operands.
pub fn getopt<T: Copy + PartialEq>(
    args: &[String],
    short: &str,
    short_opt: impl Fn(char) -> T,
    long: &[(&str, bool, T)],
) -> (Vec<Option<Got<T>>>, Vec<String>) {
    let (mut opts, mut operands) = (Vec::new(), Vec::new());
    let got = |opt, value, long| Some(Got { opt, value, long });
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
                    // Ambiguous unless every match is the same option.
                    match matches.as_slice() {
                        [] => None,
                        [first, rest @ ..]
                            if rest.iter().all(|o| o.1 == first.1 && o.2 == first.2) =>
                        {
                            Some(*first)
                        }
                        _ => None,
                    }
                }
            };
            let Some((_, takes, opt)) = found else {
                opts.push(None);
                continue;
            };
            match (takes, value) {
                (false, Some(_)) => opts.push(None),
                (false, None) => opts.push(got(*opt, None, true)),
                (true, Some(v)) => opts.push(got(*opt, Some(v), true)),
                (true, None) => match args.get(i) {
                    Some(v) => {
                        i += 1;
                        opts.push(got(*opt, Some(v.clone()), true));
                    }
                    None => opts.push(None),
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
                    opts.push(None);
                    continue;
                };
                if !short[pos + c.len_utf8()..].starts_with(':') {
                    opts.push(got(short_opt(c), None, false));
                    continue;
                }
                if k < chars.len() {
                    opts.push(got(short_opt(c), Some(chars[k..].iter().collect()), false));
                } else if let Some(v) = args.get(i) {
                    i += 1;
                    opts.push(got(short_opt(c), Some(v.clone()), false));
                } else {
                    opts.push(None);
                }
                break;
            }
            continue;
        }
        operands.push(a.clone());
    }
    (opts, operands)
}
