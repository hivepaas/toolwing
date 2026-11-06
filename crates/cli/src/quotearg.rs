//! gnulib's quotearg, as GNU coreutils quote names and arguments in their
//! messages, in a UTF-8 locale (C.UTF-8): `quotef` (a file name, quoted for
//! the shell only when it needs it, `:` included), `quoteaf` (always
//! quoted for the shell) and `quote` (‘like this’). Ported from
//! `quotearg_buffer_restyled`, branch for branch.
//!
//! Ported from gnulib's quotearg.c: Copyright (C) 1998-2025 Free Software
//! Foundation, Inc. GPL-3.0-or-later.

use toolwing_tools::glibc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    C,
    Locale,
    /// The shell styles, all one once set up (`shell_always_quoting_style`).
    ShellAlways,
}

/// The arguments of one pass: the style as set up, and whether the outer
/// quotes may be left out.
#[derive(Clone, Copy)]
struct Pass {
    style: Style,
    backslash_escapes: bool,
    elide_outer_quotes: bool,
    /// `:` among `quote_these_too` (quotef's).
    colon: bool,
}

/// `goto force_outer_quoting_style`.
struct Force;

const LEFT: &str = "\u{2018}";
const RIGHT: &str = "\u{2019}";

/// A file name for a message, quoted for the shell when it needs it.
pub fn quotef(name: &str) -> String {
    let p = Pass {
        style: Style::ShellAlways,
        backslash_escapes: true,
        elide_outer_quotes: true,
        colon: true,
    };
    run(name.as_bytes(), p)
}

/// A file name quoted for the shell, always.
pub fn quoteaf(name: &str) -> String {
    let p = Pass {
        style: Style::ShellAlways,
        backslash_escapes: true,
        elide_outer_quotes: false,
        colon: false,
    };
    run(name.as_bytes(), p)
}

/// An argument ‘like this’.
pub fn quote(arg: &str) -> String {
    let p = Pass {
        style: Style::Locale,
        backslash_escapes: true,
        elide_outer_quotes: false,
        colon: false,
    };
    run(arg.as_bytes(), p)
}

fn run(arg: &[u8], p: Pass) -> String {
    let bytes = match restyled(arg, p) {
        Ok(b) => b,
        Err(Force) => {
            // The outer quotes, then; no more `quote_these_too`.
            let again = Pass {
                elide_outer_quotes: false,
                colon: false,
                ..p
            };
            restyled(arg, again).unwrap_or_default()
        }
    };
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The character at `i`, as `mbrtoc32` reads it: its length, and whether
/// it is printable; `None` for an invalid or cut sequence.
fn char_at(arg: &[u8], i: usize) -> Option<(usize, bool)> {
    match glibc::mbrtoc32(&arg[i..]) {
        glibc::Decoded::Char(c, n) => Some((n, glibc::is_print(c))),
        _ => None,
    }
}

fn restyled(arg: &[u8], p: Pass) -> Result<Vec<u8>, Force> {
    let mut out: Vec<u8> = Vec::new();
    let style = p.style;
    let backslash_escapes = p.backslash_escapes;
    let elide = p.elide_outer_quotes;
    let quote_string: &[u8] = match style {
        Style::C => {
            if !elide {
                out.push(b'"');
            }
            b"\""
        }
        Style::Locale => {
            if !elide {
                out.extend_from_slice(LEFT.as_bytes());
            }
            RIGHT.as_bytes()
        }
        Style::ShellAlways => {
            if !elide {
                out.push(b'\'');
            }
            b"'"
        }
    };
    let mut pending_shell_escape_end = false;
    let mut encountered_single_quote = false;
    let mut all_compat = true;

    let mut i = 0usize;
    while i < arg.len() {
        let mut escaping = false;
        let mut compat = false;
        let mut is_right_quote = false;
        if backslash_escapes && style != Style::ShellAlways && arg[i..].starts_with(quote_string) {
            if elide {
                return Err(Force);
            }
            is_right_quote = true;
        }
        let mut c = arg[i];

        // What to do with c: store it, escape it first, or both done.
        enum Then {
            Store,
            Escape,
        }
        let then: Then;

        macro_rules! start_esc {
            () => {{
                if elide {
                    return Err(Force);
                }
                escaping = true;
                if style == Style::ShellAlways && !pending_shell_escape_end {
                    out.extend_from_slice(b"'$'");
                    pending_shell_escape_end = true;
                }
                out.push(b'\\');
            }};
        }
        macro_rules! end_esc {
            () => {{
                if pending_shell_escape_end && !escaping {
                    out.extend_from_slice(b"''");
                    pending_shell_escape_end = false;
                }
            }};
        }

        let esc = match c {
            0 => {
                if backslash_escapes {
                    start_esc!();
                    if style != Style::ShellAlways
                        && i + 1 < arg.len()
                        && arg[i + 1].is_ascii_digit()
                    {
                        out.extend_from_slice(b"00");
                    }
                    c = b'0';
                }
                None
            }
            b'?' => {
                if style == Style::ShellAlways && elide {
                    return Err(Force);
                }
                None
            }
            7 => Some((b'a', false)),
            8 => Some((b'b', false)),
            12 => Some((b'f', false)),
            b'\n' => Some((b'n', true)),
            b'\r' => Some((b'r', true)),
            b'\t' => Some((b't', true)),
            11 => Some((b'v', false)),
            b'\\' => {
                if style == Style::ShellAlways {
                    if elide {
                        return Err(Force);
                    }
                    // Stored as it is.
                    end_esc!();
                    out.push(c);
                    i += 1;
                    all_compat = false;
                    continue;
                }
                if backslash_escapes && elide && !quote_string.is_empty() {
                    end_esc!();
                    out.push(c);
                    i += 1;
                    all_compat = false;
                    continue;
                }
                Some((b'\\', true))
            }
            b'{' | b'}' | b'#' | b'~' | b' ' | b'!' | b'"' | b'$' | b'&' | b'(' | b')' | b'*'
            | b';' | b'<' | b'=' | b'>' | b'[' | b'^' | b'`' | b'|' => {
                let special = match c {
                    b'{' | b'}' => arg.len() == 1 && i == 0,
                    b'#' | b'~' => i == 0,
                    _ => true,
                };
                if special {
                    if matches!(c, b'{' | b'}' | b'#' | b'~' | b' ') {
                        compat = true;
                    }
                    if style == Style::ShellAlways && elide {
                        return Err(Force);
                    }
                }
                None
            }
            b'\'' => {
                encountered_single_quote = true;
                compat = true;
                if style == Style::ShellAlways {
                    if elide {
                        return Err(Force);
                    }
                    out.extend_from_slice(b"'\\'");
                    pending_shell_escape_end = false;
                }
                None
            }
            b'%'
            | b'+'
            | b','
            | b'-'
            | b'.'
            | b'/'
            | b'0'..=b'9'
            | b':'
            | b'A'..=b'Z'
            | b']'
            | b'_'
            | b'a'..=b'z' => {
                compat = true;
                None
            }
            _ => {
                // A multibyte sequence, or a byte that is not a character.
                let (m, printable) = match char_at(arg, i) {
                    Some((len, pr)) => (len, pr),
                    None => {
                        // Cut short at the end: every byte left; invalid: none.
                        let rest = arg.len() - i;
                        if is_cut(&arg[i..]) {
                            (rest, false)
                        } else {
                            (0, false)
                        }
                    }
                };
                compat = printable;
                if 1 < m || (backslash_escapes && !printable) {
                    let ilim = i + m;
                    loop {
                        if backslash_escapes && !printable {
                            start_esc!();
                            out.push(b'0' + (c >> 6));
                            out.push(b'0' + ((c >> 3) & 7));
                            c = b'0' + (c & 7);
                        } else if is_right_quote {
                            out.push(b'\\');
                            is_right_quote = false;
                        }
                        if ilim <= i + 1 {
                            break;
                        }
                        end_esc!();
                        out.push(c);
                        i += 1;
                        c = arg[i];
                    }
                    end_esc!();
                    out.push(c);
                    if !compat {
                        all_compat = false;
                    }
                    i += 1;
                    continue;
                }
                None
            }
        };

        if let Some((e, shell_too)) = esc {
            if shell_too && style == Style::ShellAlways && elide {
                return Err(Force);
            }
            if backslash_escapes {
                c = e;
                then = Then::Escape;
            } else {
                then = Then::Store;
            }
        } else {
            let quote_too = c == b':' && p.colon;
            let check = (backslash_escapes && style != Style::ShellAlways) || elide;
            then = if !(check && quote_too) && !is_right_quote {
                Then::Store
            } else {
                Then::Escape
            };
        }
        if let Then::Escape = then {
            start_esc!();
        }
        end_esc!();
        out.push(c);
        if !compat {
            all_compat = false;
        }
        i += 1;
    }

    if out.is_empty() && style == Style::ShellAlways && elide {
        return Err(Force);
    }
    if style == Style::ShellAlways && !elide && encountered_single_quote && all_compat {
        let c = Pass {
            style: Style::C,
            backslash_escapes: true,
            elide_outer_quotes: false,
            colon: p.colon,
        };
        return restyled(arg, c);
    }
    if !elide {
        out.extend_from_slice(quote_string);
    }
    Ok(out)
}

/// Whether `rest` is the start of a UTF-8 sequence cut short by the end.
fn is_cut(rest: &[u8]) -> bool {
    glibc::mbrtoc32(rest) == glibc::Decoded::Incomplete
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_coreutils_quote() {
        assert_eq!(quotef("plain.txt"), "plain.txt");
        assert_eq!(quotef("a b"), "'a b'");
        assert_eq!(quotef("a:b"), "'a:b'");
        assert_eq!(quotef("x\ny"), "'x'$'\\n''y'");
        assert_eq!(quotef(""), "''");
        assert_eq!(quoteaf("nope"), "'nope'");
        assert_eq!(quoteaf("a'b"), "\"a'b\"");
        assert_eq!(quotef("é"), "é");
        assert_eq!(quote("xyz"), "\u{2018}xyz\u{2019}");
        assert_eq!(quote("--total"), "\u{2018}--total\u{2019}");
    }
}
