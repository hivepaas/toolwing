//! `uniq`: adjacent matching lines filtered as GNU coreutils 9.7's uniq
//! filters them in a UTF-8 locale (C.UTF-8): fields skipped over glibc's
//! blanks (`iswblank`), characters skipped as UTF-8 (a byte that is not one
//! is one), `-i` folding ASCII only (`memcasecmp`), and the last line given
//! its delimiter. `-w N` keeps 9.7's reckoning: when the field is longer
//! than N bytes, N times the length of its first character.
//!
//! Ported from GNU coreutils 9.7's uniq.c: Copyright (C) 1986-2025 Free
//! Software Foundation, Inc. GPL-3.0-or-later.

use std::io::{self, BufRead, Write};

use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, InputKind, Meta, Output, Param, ParamKind, Reference, Tool,
    Value, Values, en,
};

use crate::glibc::{self, Decoded};

/// `--all-repeated`'s METHOD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delimit {
    None,
    Prepend,
    Separate,
}

/// `--group`'s METHOD, or no `--group`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grouping {
    None,
    Prepend,
    Append,
    Separate,
    Both,
}

/// What uniq compares and prints.
#[derive(Clone, Debug)]
pub struct Options {
    pub skip_fields: usize,
    pub skip_chars: usize,
    pub check_chars: usize,
    pub ignore_case: bool,
    pub count: bool,
    pub output_unique: bool,
    pub output_first_repeated: bool,
    pub output_later_repeated: bool,
    pub delimit: Delimit,
    pub grouping: Grouping,
    pub delimiter: u8,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            skip_fields: 0,
            skip_chars: 0,
            check_chars: usize::MAX,
            ignore_case: false,
            count: false,
            output_unique: true,
            output_first_repeated: true,
            output_later_repeated: false,
            delimit: Delimit::None,
            grouping: Grouping::None,
            delimiter: b'\n',
        }
    }
}

/// Why uniq stopped: the input or the output failed.
#[derive(Debug)]
pub enum Failure {
    Read(io::Error),
    Write(io::Error),
}

/// `mcel_scan`: the length of the character at the start of `b` (1 for a
/// byte that is not one), and the character.
fn scan(b: &[u8]) -> (usize, Option<u32>) {
    match glibc::mbrtoc32(b) {
        Decoded::Char(c, n) => (n, Some(c)),
        _ => (1, None),
    }
}

/// `find_field`: where the compared part of a line starts, and its length
/// (the line holds its delimiter last).
fn find_field(line: &[u8], o: &Options) -> (usize, usize) {
    let lim = line.len().saturating_sub(1);
    let mut lp = 0;
    let blank = |c: Option<u32>| c.is_some_and(|c| c == u32::from(b'\n') || glibc::is_blank(c));
    for _ in 0..o.skip_fields {
        if lp >= lim {
            break;
        }
        while lp < lim {
            let (n, c) = scan(&line[lp..lim]);
            if !blank(c) {
                break;
            }
            lp += n;
        }
        while lp < lim {
            let (n, c) = scan(&line[lp..lim]);
            if blank(c) {
                break;
            }
            lp += n;
        }
    }
    for _ in 0..o.skip_chars {
        if lp >= lim {
            break;
        }
        lp += scan(&line[lp..lim]).0;
    }
    let len = if lim - lp <= o.check_chars {
        lim - lp
    } else {
        // 9.7 scans the first character again each time.
        let first = scan(&line[lp..lim]).0;
        first.saturating_mul(o.check_chars)
    };
    (lp, len)
}

/// A line buffer as gnulib's `linebuffer`: its allocation, from 128 bytes
/// growing by half (`xpalloc`), and the line at its start. What earlier
/// longer lines left past the line stays, as 9.7's `-w` may compare it.
struct LineBuf {
    data: Vec<u8>,
    len: usize,
}

impl LineBuf {
    fn new() -> LineBuf {
        LineBuf {
            data: Vec::new(),
            len: 0,
        }
    }

    fn line(&self) -> &[u8] {
        &self.data[..self.len]
    }

    /// `readlinebuffer_delim`: the next line with its delimiter (one added
    /// to a last line without it); false at the end.
    fn read(&mut self, input: &mut dyn BufRead, delim: u8) -> Result<bool, Failure> {
        let mut line = Vec::new();
        let n = input.read_until(delim, &mut line).map_err(Failure::Read)?;
        if n == 0 {
            return Ok(false);
        }
        if line.last() != Some(&delim) {
            line.push(delim);
        }
        while self.data.len() < line.len() {
            let size = self.data.len();
            let grown = if size == 0 { 128 } else { size + size / 2 };
            self.data.resize(grown, 0);
        }
        self.data[..line.len()].copy_from_slice(&line);
        self.len = line.len();
        Ok(true)
    }

    /// `len` bytes from `at`, past the line if need be (as far as the
    /// buffer goes; zeros past it, where the C reads beyond its memory).
    fn field(&self, at: usize, len: usize) -> std::borrow::Cow<'_, [u8]> {
        let end = at.saturating_add(len);
        if end <= self.data.len() {
            std::borrow::Cow::Borrowed(&self.data[at..end])
        } else {
            let mut v = self.data[at.min(self.data.len())..].to_vec();
            v.resize(len, 0);
            std::borrow::Cow::Owned(v)
        }
    }
}

/// `different`: lengths, then bytes (ASCII case folded with `-i`).
fn different(
    a: &LineBuf,
    b: &LineBuf,
    (aat, alen): (usize, usize),
    (bat, blen): (usize, usize),
    o: &Options,
) -> bool {
    if alen != blen {
        return true;
    }
    let (x, y) = (a.field(aat, alen), b.field(bat, blen));
    if o.ignore_case {
        !x.eq_ignore_ascii_case(&y)
    } else {
        x != y
    }
}

/// `check_file`: `input`'s lines filtered into `out`.
pub fn run(input: &mut dyn BufRead, out: &mut dyn Write, o: &Options) -> Result<(), Failure> {
    let w = |out: &mut dyn Write, b: &[u8]| out.write_all(b).map_err(Failure::Write);
    let d = [o.delimiter];
    let mut this = LineBuf::new();
    let mut prev = LineBuf::new();
    if o.output_unique && o.output_first_repeated && !o.count {
        let mut prevfield: Option<(usize, usize)> = None;
        let mut first_group_printed = false;
        while this.read(input, o.delimiter)? {
            let field = find_field(this.line(), o);
            let new_group = match prevfield {
                None => true,
                Some(pf) => different(&this, &prev, field, pf, o),
            };
            let g = o.grouping;
            if new_group
                && g != Grouping::None
                && (g == Grouping::Prepend
                    || g == Grouping::Both
                    || (first_group_printed && (g == Grouping::Append || g == Grouping::Separate)))
            {
                w(out, &d)?;
            }
            if new_group || g != Grouping::None {
                w(out, this.line())?;
                std::mem::swap(&mut prev, &mut this);
                prevfield = Some(field);
                first_group_printed = true;
            }
        }
        if matches!(o.grouping, Grouping::Both | Grouping::Append) && first_group_printed {
            w(out, &d)?;
        }
        return Ok(());
    }

    if !prev.read(input, o.delimiter)? {
        return Ok(());
    }
    let mut prevfield = find_field(prev.line(), o);
    let mut match_count: u64 = 0;
    let mut first_delimiter = true;
    let writeline = |out: &mut dyn Write, line: &[u8], matched: bool, count: u64| {
        let shown = if count == 0 {
            o.output_unique
        } else if !matched {
            o.output_first_repeated
        } else {
            o.output_later_repeated
        };
        if !shown {
            return Ok(());
        }
        if o.count {
            w(out, format!("{:7} ", count + 1).as_bytes())?;
        }
        w(out, line)
    };
    while this.read(input, o.delimiter)? {
        let field = find_field(this.line(), o);
        let matched = !different(&this, &prev, field, prevfield, o);
        match_count += u64::from(matched);
        if o.delimit != Delimit::None {
            if !matched {
                if match_count > 0 {
                    first_delimiter = false;
                }
            } else if match_count == 1
                && (o.delimit == Delimit::Prepend
                    || (o.delimit == Delimit::Separate && !first_delimiter))
            {
                w(out, &d)?;
            }
        }
        if !matched || o.output_later_repeated {
            writeline(out, prev.line(), matched, match_count)?;
            std::mem::swap(&mut prev, &mut this);
            prevfield = field;
            if !matched {
                match_count = 0;
            }
        }
    }
    writeline(out, prev.line(), false, match_count)
}

// ---- The app's tool ----------------------------------------------------------

static META: Meta = Meta {
    id: "uniq",
    group: "uniq",
    action: "",
    action_flag: None,
    title: en!("Unique lines"),
    summary: en!("Adjacent repeated lines merged, as uniq does; counted, or only the repeated"),
    category: Category::Text,
    keywords: &["uniq", "unique", "duplicate", "repeated", "lines", "dedupe"],
    input: InputKind::Bytes,
    streams: false,
    reference: Some(Reference {
        name: "uniq",
        version: "9.7",
        compat: Compat::DropIn,
    }),
};

static PARAMS: &[Param] = &[
    Param {
        key: "count",
        label: en!("Count"),
        help: en!("Prefix each line with how many times it occurs"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "count",
            short: Some('c'),
        },
    },
    Param {
        key: "repeated",
        label: en!("Only repeated"),
        help: en!("Only the lines that occur more than once, each once"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "repeated",
            short: Some('d'),
        },
    },
    Param {
        key: "unique",
        label: en!("Only unique"),
        help: en!("Only the lines that occur once"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "unique",
            short: Some('u'),
        },
    },
    Param {
        key: "ignore_case",
        label: en!("Ignore case"),
        help: en!("Compare ASCII letters without their case"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "ignore-case",
            short: Some('i'),
        },
    },
];

pub struct UniqTool;

impl Tool for UniqTool {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let f = |k| values.flag(PARAMS, k);
        let o = Options {
            count: f("count"),
            output_unique: !f("repeated"),
            output_first_repeated: !f("unique"),
            ignore_case: f("ignore_case"),
            ..Options::default()
        };
        let mut out = Vec::new();
        // Reading a slice and writing a vector cannot fail.
        let _ = run(&mut &input[..], &mut out, &o);
        if out.last() == Some(&b'\n') {
            out.pop();
        }
        Ok(Output::Text(String::from_utf8_lossy(&out).into_owned()))
    }

    fn command_args(&self, values: &Values) -> Option<Vec<String>> {
        let mut args = vec!["uniq".to_owned()];
        for (k, flag) in [
            ("count", "-c"),
            ("repeated", "-d"),
            ("unique", "-u"),
            ("ignore_case", "-i"),
        ] {
            if values.flag(PARAMS, k) {
                args.push(flag.to_owned());
            }
        }
        Some(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniq(input: &str, o: &Options) -> String {
        let mut out = Vec::new();
        let _ = run(&mut input.as_bytes(), &mut out, o);
        String::from_utf8_lossy(&out).into_owned()
    }

    #[test]
    fn filters_as_gnu_uniq() {
        let d = Options::default();
        assert_eq!(uniq("a\na\nb\na", &d), "a\nb\na\n");
        let c = Options {
            count: true,
            ..Options::default()
        };
        assert_eq!(uniq("a\na\nb\n", &c), "      2 a\n      1 b\n");
        let f = Options {
            skip_fields: 1,
            ..Options::default()
        };
        assert_eq!(uniq("1 x\n2 x\n3 y\n", &f), "1 x\n3 y\n");
        // 9.7's -w: twice the first character's length here, 4 bytes.
        let w = Options {
            check_chars: 2,
            ..Options::default()
        };
        assert_eq!(uniq("éab\néac\n", &w), "éab\néac\n");
        let g = Options {
            grouping: Grouping::Separate,
            ..Options::default()
        };
        assert_eq!(uniq("a\na\nb\n", &g), "a\na\n\nb\n");
    }
}
