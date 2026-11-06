//! What GNU coreutils' commands share, as tw's drop-ins for them need it:
//! `error`'s "prog: message", `emit_try_help`, `argmatch` and its list of
//! valid arguments, the help's last lines, and the version text, which names
//! tw.

use std::io::{self, Write};

use crate::quotearg::quote;

/// `error (0, errnum, ...)`: "prog: message" on stderr.
pub fn error(prog: &str, message: &str) {
    let mut e = io::stderr().lock();
    let _ = writeln!(e, "{prog}: {message}");
}

/// `emit_try_help`.
pub fn try_help(prog: &str) {
    let mut e = io::stderr().lock();
    let _ = writeln!(e, "Try '{prog} --help' for more information.");
}

/// The C library's words for an error, as `strerror` gives them.
pub fn strerror(e: &io::Error) -> String {
    toolwing_tools::gzip::strerror(e)
}

/// `emit_ancillary_info`: the help's last lines.
pub fn ancillary_info(prog: &str) -> String {
    format!(
        "\nGNU coreutils online help: <https://www.gnu.org/software/coreutils/>\n\
         Report any translation bugs to <https://translationproject.org/team/>\n\
         Full documentation <https://www.gnu.org/software/coreutils/{prog}>\n\
         or available locally via: info '(coreutils) {prog} invocation'\n"
    )
}

/// The help's `--help` and `--version` lines.
pub const HELP_VERSION: &str = "      --help        display this help and exit\n      --version     output version information and exit\n";

/// `--version`: tw's name first, then coreutils' copyright and authors.
pub fn version(prog: &str, authors: &str) -> String {
    format!(
        "{prog} (tw {}) GNU coreutils 9.7\n\
         Copyright (C) 2025 Free Software Foundation, Inc.\n\
         License GPLv3+: GNU GPL version 3 or later <https://gnu.org/licenses/gpl.html>.\n\
         This is free software: you are free to change and redistribute it.\n\
         There is NO WARRANTY, to the extent permitted by law.\n\n\
         Written by {authors}.\n",
        env!("CARGO_PKG_VERSION")
    )
}

/// `argmatch`: `arg` as one of `args` (or a prefix of only one, or of
/// several with the same value); otherwise the messages of `XARGMATCH`
/// for option `opt`, and `None`.
pub fn argmatch<T: Copy + PartialEq>(
    prog: &str,
    opt: &str,
    arg: &str,
    args: &[(&str, T)],
) -> Option<T> {
    let mut found: Option<T> = None;
    let mut ambiguous = false;
    for (name, value) in args {
        if *name == arg {
            return Some(*value);
        }
        if name.starts_with(arg) {
            match found {
                None => found = Some(*value),
                Some(v) if v != *value => ambiguous = true,
                Some(_) => {}
            }
        }
    }
    if let (Some(v), false) = (found, ambiguous) {
        return Some(v);
    }
    let what = if ambiguous { "ambiguous" } else { "invalid" };
    error(
        prog,
        &format!("{what} argument {} for {}", quote(arg), quote(opt)),
    );
    let mut list = String::from("Valid arguments are:");
    let mut last: Option<T> = None;
    for (name, value) in args {
        if last == Some(*value) {
            list.push_str(&format!(", {}", quote(name)));
        } else {
            list.push_str(&format!("\n  - {}", quote(name)));
            last = Some(*value);
        }
    }
    let _ = writeln!(io::stderr().lock(), "{list}");
    try_help(prog);
    None
}

/// stdin's metadata (`fstat (STDIN_FILENO)`).
pub fn stdin_metadata() -> io::Result<std::fs::Metadata> {
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        let fd = io::stdin().as_fd().try_clone_to_owned()?;
        std::fs::File::from(fd).metadata()
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsHandle;
        let h = io::stdin().as_handle().try_clone_to_owned()?;
        std::fs::File::from(h).metadata()
    }
}

/// `S_ISREG`. On Windows a pipe's metadata says "file" too (`is_file` is
/// only "not a directory nor a link"), with no times at all: a file has them.
pub fn is_regular(m: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        m.is_file() && (m.creation_time() != 0 || m.last_write_time() != 0)
    }
    #[cfg(not(windows))]
    {
        m.is_file()
    }
}
