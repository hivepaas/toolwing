//! `wc`: lines, words, characters, bytes and the widest line, counted as GNU
//! coreutils 9.7's wc counts them in a UTF-8 locale (C.UTF-8): a word is a
//! run of characters that are not white space (glibc's `iswspace`, and the
//! no-break spaces unless POSIXLY_CORRECT), bytes that are not UTF-8 count
//! as word characters but not as characters, and the width is glibc's
//! `wcwidth`, a tab to the next multiple of 8, a carriage return or form
//! feed starting the line again.
//!
//! Ported from GNU coreutils 9.7's wc.c: Copyright (C) 1985-2025 Free
//! Software Foundation, Inc. GPL-3.0-or-later.

use std::io::{self, Read};

use toolwing_core::{
    Category, Compat, Ctx, Error, Field, InputKind, Meta, Output, Reference, Tool, Values, en,
};

use crate::glibc;

/// What is counted of an input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub lines: u64,
    pub words: u64,
    pub chars: u64,
    pub bytes: u64,
    /// The widest line, in columns.
    pub width: u64,
}

/// `iswnbspace`: the no-break spaces GNU wc takes as spaces too.
fn is_nbspace(c: u32) -> bool {
    matches!(c, 0xa0 | 0x2007 | 0x202f | 0x2060)
}

/// A counter fed a piece of input at a time.
#[derive(Default)]
pub struct Counter {
    counts: Counts,
    in_word: bool,
    linepos: u64,
    /// The start of a character cut by the end of a piece.
    carry: Vec<u8>,
    /// Without the no-break spaces as spaces (POSIXLY_CORRECT).
    pub posixly_correct: bool,
}

impl Counter {
    pub fn new(posixly_correct: bool) -> Counter {
        Counter {
            posixly_correct,
            ..Counter::default()
        }
    }

    /// The next piece of input.
    pub fn feed(&mut self, piece: &[u8]) {
        self.counts.bytes += piece.len() as u64;
        let joined;
        let data: &[u8] = if self.carry.is_empty() {
            piece
        } else {
            let mut v = std::mem::take(&mut self.carry);
            v.extend_from_slice(piece);
            joined = v;
            &joined
        };
        self.scan(data, false);
    }

    /// The counts, once all is fed.
    pub fn finish(mut self) -> Counts {
        let rest = std::mem::take(&mut self.carry);
        self.scan(&rest, true);
        if self.linepos > self.counts.width {
            self.counts.width = self.linepos;
        }
        self.counts
    }

    fn scan(&mut self, data: &[u8], at_end: bool) {
        let mut p = 0;
        while p < data.len() {
            let b = data[p];
            if b < 0x80 {
                self.ascii(b);
                p += 1;
                continue;
            }
            let (c, len) = match glibc::mbrtoc32(&data[p..]) {
                glibc::Decoded::Char(c, n) => (Some(c), n),
                glibc::Decoded::Incomplete if !at_end => {
                    // Cut by the end of this piece: wait for the rest.
                    self.carry = data[p..].to_vec();
                    return;
                }
                _ => (None, 1),
            };
            match c {
                Some(c) => self.wide(c),
                None => {
                    // Not a character: a byte, counted as part of a word.
                    self.counts.words += u64::from(!self.in_word);
                    self.in_word = true;
                }
            }
            p += len;
        }
    }

    fn end_line(&mut self) {
        if self.linepos > self.counts.width {
            self.counts.width = self.linepos;
        }
        self.linepos = 0;
        self.in_word = false;
    }

    fn ascii(&mut self, b: u8) {
        self.counts.chars += 1;
        match b {
            b'\n' => {
                self.counts.lines += 1;
                self.end_line();
            }
            b'\r' | 0x0c => self.end_line(),
            b'\t' => {
                self.linepos += 8 - self.linepos % 8;
                self.in_word = false;
            }
            b' ' => {
                self.linepos += 1;
                self.in_word = false;
            }
            0x0b => self.in_word = false,
            _ => {
                self.linepos += u64::from((0x20..0x7f).contains(&b));
                self.counts.words += u64::from(!self.in_word);
                self.in_word = true;
            }
        }
    }

    fn wide(&mut self, c: u32) {
        self.counts.chars += 1;
        let w = glibc::width(c);
        if w > 0 {
            self.linepos += w as u64;
        }
        let word = !glibc::is_space(c) && !(is_nbspace(c) && !self.posixly_correct);
        self.counts.words += u64::from(!self.in_word && word);
        self.in_word = word;
    }
}

/// Everything `input` gives, counted; the error that stopped the reading,
/// if one did (what was read is counted).
pub fn count(input: &mut dyn Read, posixly_correct: bool) -> (Counts, Option<io::Error>) {
    let mut c = Counter::new(posixly_correct);
    let mut buf = vec![0u8; 1 << 16];
    loop {
        match input.read(&mut buf) {
            Ok(0) => return (c.finish(), None),
            Ok(n) => c.feed(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return (c.finish(), Some(e)),
        }
    }
}

// ---- The app's tool ----------------------------------------------------------

static META: Meta = Meta {
    id: "wc",
    group: "wc",
    action: "",
    action_flag: None,
    title: en!("Count"),
    summary: en!("Lines, words, characters, bytes and the widest line, as wc counts them"),
    category: Category::Text,
    keywords: &[
        "wc",
        "count",
        "lines",
        "words",
        "characters",
        "bytes",
        "length",
    ],
    input: InputKind::Bytes,
    streams: false,
    reference: Some(Reference {
        name: "wc",
        version: "9.7",
        compat: Compat::DropIn,
    }),
};

pub struct CountTool;

impl Tool for CountTool {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], _values: &Values) -> Result<Output, Error> {
        let (c, _) = count(&mut &input[..], false);
        let n = |v: u64| v.to_string();
        Ok(Output::Fields(vec![
            Field::new("lines", "Lines", n(c.lines)),
            Field::new("words", "Words", n(c.words)),
            Field::new("chars", "Characters", n(c.chars)),
            Field::new("bytes", "Bytes", n(c.bytes)),
            Field::new("max_line_length", "Widest line", n(c.width)),
        ]))
    }

    fn command_args(&self, _values: &Values) -> Option<Vec<String>> {
        Some(vec!["wc".into(), "-lwmcL".into()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(s: &[u8]) -> Counts {
        count(&mut &s[..], false).0
    }

    #[test]
    fn counts_as_gnu_wc() {
        let c = of(b"hello world\nsecond line here\n");
        assert_eq!(
            (c.lines, c.words, c.chars, c.bytes, c.width),
            (2, 5, 29, 29, 16)
        );
        // UTF-8: characters, wide ones two columns, no-break space a space.
        let c = of("中文 é\u{a0}x\n".as_bytes());
        assert_eq!((c.words, c.chars, c.width), (3, 7, 8));
        // A byte that is not UTF-8: a word character, not a character.
        let c = of(b"\xff \xfe\xfd");
        assert_eq!((c.words, c.chars, c.bytes), (2, 1, 4));
        // Tabs to the next multiple of 8; a carriage return starts again.
        assert_eq!(of(b"a\tb\r\nxy").width, 9);
    }

    #[test]
    fn pieces_cut_inside_a_character() {
        let text = "aé中\u{1f600} b\n".repeat(50);
        let mut c = Counter::new(false);
        for chunk in text.as_bytes().chunks(3) {
            c.feed(chunk);
        }
        assert_eq!(c.finish(), of(text.as_bytes()));
    }
}
