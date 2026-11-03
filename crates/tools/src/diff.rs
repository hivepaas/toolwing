//! `diff`: two texts compared line by line, written as GNU diffutils 3.10
//! writes them: its normal format (`2c2`, `< `, `> `) or unified (`-u`), with
//! "\ No newline at end of file" where a file has none.
//!
//! Lines are bytes: a file need not be UTF-8. The changes are found with
//! Myers' algorithm, as GNU finds them; where several smallest diffs exist,
//! the hunks may be placed otherwise than GNU's (the compatibility cases show
//! where they agree).

use similar::{Algorithm, DiffOp, DiffTag};
use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, InputKind, Meta, Output, Param, ParamKind, Reference, Tool,
    Value, Values, en,
};

/// What counts as the same line.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// `-i`
    pub ignore_case: bool,
    /// `-w`: white space ignored altogether.
    pub ignore_all_space: bool,
    /// `-b`: runs of white space count as one, trailing white space as none.
    pub ignore_space_change: bool,
}

/// A text as lines, without their newlines.
pub struct Lines<'a> {
    pub lines: Vec<&'a [u8]>,
    /// The last line has no newline.
    pub missing_newline: bool,
}

pub fn lines(text: &[u8]) -> Lines<'_> {
    let mut lines: Vec<&[u8]> = text.split(|&b| b == b'\n').collect();
    let missing_newline = !text.is_empty() && !text.ends_with(b"\n");
    if !missing_newline {
        lines.pop();
    }
    Lines {
        lines,
        missing_newline,
    }
}

fn key(line: &[u8], o: Options) -> Vec<u8> {
    let white = |b: &u8| b == &b' ' || b == &b'\t' || b == &b'\r' || b == &0x0b || b == &0x0c;
    let mut out: Vec<u8> = if o.ignore_all_space {
        line.iter().copied().filter(|b| !white(b)).collect()
    } else if o.ignore_space_change {
        let mut out = Vec::with_capacity(line.len());
        let mut in_space = false;
        for &b in line {
            if white(&b) {
                in_space = true;
            } else {
                if in_space && !out.is_empty() {
                    out.push(b' ');
                }
                in_space = false;
                out.push(b);
            }
        }
        out
    } else {
        line.to_vec()
    };
    if o.ignore_case {
        out.make_ascii_lowercase();
    }
    out
}

/// The changes from `a` to `b`; empty when they are the same, as `o` sees it.
pub fn ops(a: &Lines, b: &Lines, o: Options) -> Vec<DiffOp> {
    let ka: Vec<Vec<u8>> = a.lines.iter().map(|l| key(l, o)).collect();
    let kb: Vec<Vec<u8>> = b.lines.iter().map(|l| key(l, o)).collect();
    let ops = similar::capture_diff_slices(Algorithm::Myers, &ka, &kb);
    // A last line with a newline and the same without differ, as in GNU.
    let last_differs = a.missing_newline != b.missing_newline
        && !a.lines.is_empty()
        && !b.lines.is_empty()
        && ka.last() == kb.last();
    if !last_differs {
        return if ops.iter().all(|op| op.tag() == DiffTag::Equal) {
            Vec::new()
        } else {
            ops
        };
    }
    // Compare again with the last lines told apart by their ending.
    let mark = |k: &[Vec<u8>], missing: bool| {
        let mut k = k.to_vec();
        if let (Some(last), true) = (k.last_mut(), missing) {
            last.push(0xff);
        }
        k
    };
    similar::capture_diff_slices(
        Algorithm::Myers,
        &mark(&ka, a.missing_newline),
        &mark(&kb, b.missing_newline),
    )
}

const NO_NEWLINE: &[u8] = b"\\ No newline at end of file\n";

fn push_line(out: &mut Vec<u8>, prefix: &[u8], text: &Lines, i: usize) {
    out.extend_from_slice(prefix);
    out.extend_from_slice(text.lines[i]);
    out.push(b'\n');
    if text.missing_newline && i + 1 == text.lines.len() {
        out.extend_from_slice(NO_NEWLINE);
    }
}

/// `5`, or `5,7`: a range of lines numbered from 1, for the normal format.
fn normal_range(start: usize, len: usize) -> String {
    if len <= 1 {
        (start + 1).to_string()
    } else {
        format!("{},{}", start + 1, start + len)
    }
}

/// GNU's normal format; `None` when the texts are the same.
pub fn normal(a: &Lines, b: &Lines, o: Options) -> Option<Vec<u8>> {
    let ops = ops(a, b, o);
    if ops.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for op in ops {
        let (tag, old, new) = op.as_tag_tuple();
        match tag {
            DiffTag::Equal => continue,
            DiffTag::Delete => {
                out.extend_from_slice(
                    format!("{}d{}\n", normal_range(old.start, old.len()), new.start).as_bytes(),
                );
                old.for_each(|i| push_line(&mut out, b"< ", a, i));
            }
            DiffTag::Insert => {
                out.extend_from_slice(
                    format!("{}a{}\n", old.start, normal_range(new.start, new.len())).as_bytes(),
                );
                new.for_each(|i| push_line(&mut out, b"> ", b, i));
            }
            DiffTag::Replace => {
                let header = format!(
                    "{}c{}\n",
                    normal_range(old.start, old.len()),
                    normal_range(new.start, new.len())
                );
                out.extend_from_slice(header.as_bytes());
                old.for_each(|i| push_line(&mut out, b"< ", a, i));
                out.extend_from_slice(b"---\n");
                new.for_each(|i| push_line(&mut out, b"> ", b, i));
            }
        }
    }
    Some(out)
}

/// `-3,5`: a hunk's range in unified format, a count of 1 left out and an
/// empty range at the line before.
fn unified_range(start: usize, len: usize) -> String {
    match len {
        0 => format!("{start},0"),
        1 => (start + 1).to_string(),
        n => format!("{},{n}", start + 1),
    }
}

/// GNU's unified format with `context` lines around each change, under the
/// two header lines; `None` when the texts are the same.
pub fn unified(
    a: &Lines,
    b: &Lines,
    o: Options,
    context: usize,
    headers: (&str, &str),
) -> Option<Vec<u8>> {
    let ops = ops(a, b, o);
    if ops.is_empty() {
        return None;
    }
    let mut out = format!("--- {}\n+++ {}\n", headers.0, headers.1).into_bytes();
    for group in similar::group_diff_ops(ops, context) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        let old = first.old_range().start..last.old_range().end;
        let new = first.new_range().start..last.new_range().end;
        out.extend_from_slice(
            format!(
                "@@ -{} +{} @@\n",
                unified_range(old.start, old.len()),
                unified_range(new.start, new.len())
            )
            .as_bytes(),
        );
        for op in group {
            let (tag, old, new) = op.as_tag_tuple();
            match tag {
                DiffTag::Equal => old.for_each(|i| push_line(&mut out, b" ", a, i)),
                DiffTag::Delete => old.for_each(|i| push_line(&mut out, b"-", a, i)),
                DiffTag::Insert => new.for_each(|i| push_line(&mut out, b"+", b, i)),
                DiffTag::Replace => {
                    old.for_each(|i| push_line(&mut out, b"-", a, i));
                    new.for_each(|i| push_line(&mut out, b"+", b, i));
                }
            }
        }
    }
    Some(out)
}

/// Lines taken out and put in: the `+3 −2` the app shows.
pub fn counts(a: &Lines, b: &Lines, o: Options) -> (usize, usize) {
    ops(a, b, o).iter().fold((0, 0), |(add, del), op| {
        let (tag, old, new) = op.as_tag_tuple();
        match tag {
            DiffTag::Equal => (add, del),
            DiffTag::Delete => (add, del + old.len()),
            DiffTag::Insert => (add + new.len(), del),
            DiffTag::Replace => (add + new.len(), del + old.len()),
        }
    })
}

/// The registry's diff: the app's page and `tw list`. Its command line is
/// written by hand (`toolwing-cli`'s `diffs`), as GNU's takes two files.
pub struct Compare;

static META: Meta = Meta {
    id: "diff",
    group: "diff",
    action: "",
    action_flag: None,
    title: en!("Diff"),
    summary: en!("Compare two texts or files line by line, as diff does"),
    category: Category::Text,
    keywords: &["diff", "compare", "changes", "patch", "unified"],
    input: InputKind::Bytes,
    streams: false,
    reference: Some(Reference {
        name: "GNU diffutils",
        version: "3.10",
        compat: Compat::DropIn,
    }),
};

/// The keys are those of the `tw diff` arguments, so a copied command reads
/// back (cli.md).
pub static PARAMS: &[Param] = &[
    Param {
        key: "unified",
        label: en!("Context lines"),
        help: en!("Lines of context around each change in unified format"),
        kind: ParamKind::Int { min: 0, max: 1_000 },
        default: Value::Int(3),
        cli: Cli {
            long: "unified",
            short: Some('U'),
        },
    },
    Param {
        key: "ignore_case",
        label: en!("Ignore case"),
        help: en!("Lines that differ in case only are the same"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "ignore-case",
            short: Some('i'),
        },
    },
    Param {
        key: "ignore_all_space",
        label: en!("Ignore all white space"),
        help: en!("White space is not compared at all"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "ignore-all-space",
            short: Some('w'),
        },
    },
    Param {
        key: "ignore_space_change",
        label: en!("Ignore changes in white space"),
        help: en!("Runs of white space are one space, and trailing white space is none"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "ignore-space-change",
            short: Some('b'),
        },
    },
];

pub fn options(values: &Values) -> Options {
    Options {
        ignore_case: values.flag(PARAMS, "ignore_case"),
        ignore_all_space: values.flag(PARAMS, "ignore_all_space"),
        ignore_space_change: values.flag(PARAMS, "ignore_space_change"),
    }
}

impl Tool for Compare {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    /// The input is the text before; `other`, a value the app sets, the text after.
    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let after = values.text(PARAMS, "other");
        let (a, b) = (lines(input), lines(after.as_bytes()));
        let context = usize::try_from(values.int(PARAMS, "unified")).unwrap_or(3);
        let diff =
            unified(&a, &b, options(values), context, ("before", "after")).unwrap_or_default();
        Ok(Output::Text(String::from_utf8_lossy(&diff).into_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
    const B: &str = "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nk\n";

    fn norm(a: &str, b: &str, o: Options) -> String {
        String::from_utf8_lossy(
            &normal(&lines(a.as_bytes()), &lines(b.as_bytes()), o).unwrap_or_default(),
        )
        .into_owned()
    }

    fn uni(a: &str, b: &str, context: usize) -> String {
        let d = unified(
            &lines(a.as_bytes()),
            &lines(b.as_bytes()),
            Options::default(),
            context,
            ("a", "b"),
        );
        String::from_utf8_lossy(&d.unwrap_or_default()).into_owned()
    }

    /// Each as GNU diffutils 3.10 gives it.
    #[test]
    fn normal_format() {
        assert_eq!(
            norm(A, B, Options::default()),
            "2c2\n< b\n---\n> B\n10a11\n> k\n"
        );
        assert_eq!(
            norm("x", "y", Options::default()),
            "1c1\n< x\n\\ No newline at end of file\n---\n> y\n\\ No newline at end of file\n"
        );
        assert_eq!(
            norm("a\nb\nc\n", "a\n", Options::default()),
            "2,3d1\n< b\n< c\n"
        );
        assert_eq!(norm(A, A, Options::default()), "");
    }

    #[test]
    fn unified_format() {
        assert_eq!(
            uni(A, B, 3),
            "--- a\n+++ b\n@@ -1,5 +1,5 @@\n a\n-b\n+B\n c\n d\n e\n@@ -8,3 +8,4 @@\n h\n i\n j\n+k\n"
        );
        assert_eq!(
            uni(A, B, 1),
            "--- a\n+++ b\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n@@ -10 +10,2 @@\n j\n+k\n"
        );
        assert!(uni("", A, 3).starts_with("--- a\n+++ b\n@@ -0,0 +1,10 @@\n+a\n"));
        assert_eq!(
            uni("x", "y", 3),
            "--- a\n+++ b\n@@ -1 +1 @@\n-x\n\\ No newline at end of file\n+y\n\\ No newline at end of file\n"
        );
    }

    #[test]
    fn a_missing_newline_alone_is_a_change() {
        assert_eq!(
            norm("a\n", "a", Options::default()),
            "1c1\n< a\n---\n> a\n\\ No newline at end of file\n"
        );
    }

    #[test]
    fn white_space_and_case() {
        let i = Options {
            ignore_case: true,
            ..Options::default()
        };
        assert_eq!(norm("A\n", "a\n", i), "");
        let w = Options {
            ignore_all_space: true,
            ..Options::default()
        };
        assert_eq!(norm("a b\n", "ab\n", w), "");
        let b = Options {
            ignore_space_change: true,
            ..Options::default()
        };
        assert_eq!(norm("a b\n", "a  b \n", b), "");
        assert_ne!(norm("a b\n", "ab\n", b), "");
        assert_eq!(
            counts(
                &lines(A.as_bytes()),
                &lines(B.as_bytes()),
                Options::default()
            ),
            (2, 1)
        );
    }
}
