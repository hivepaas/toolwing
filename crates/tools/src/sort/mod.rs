//! `sort`: lines ordered as GNU coreutils 9.7's sort orders them in the
//! C.UTF-8 locale, where its collation is the bytes' order (glibc's
//! `strcoll` and `strxfrm` leave UTF-8 as it is there) and its character
//! classes are ASCII's: keys (`-k`, `-t`), every ordering (`-b -d -f -g -h
//! -i -M -n -R -r -V`), the last-resort comparison, `-s`, `-u`, `-m` as its
//! merge runs, `-c`, and `--debug`'s marks. `-g` compares long doubles
//! (float.rs); `-R` hashes keys with MD5 from a random (or given) seed.
//!
//! Ported from GNU coreutils 9.7's sort.c: Copyright (C) 1988-2025 Free
//! Software Foundation, Inc. GPL-3.0-or-later.

mod app;
pub mod float;
pub mod num;

use md5::{Digest, Md5};

pub use app::SortTool;
use float::{LongDouble, strtold};
use num::{filenvercmp, human_numcompare, strnumcmp, traverse_raw_number, unit_order};

use crate::glibc::{self, Decoded};

/// `SIZE_MAX`: no field (a key from the line's start, or to its end).
pub const NONE: usize = usize::MAX;

/// Characters a key leaves out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ignore {
    #[default]
    None,
    /// `-d`: all but letters, digits and blanks.
    Nondictionary,
    /// `-i`: the non-printing.
    Nonprinting,
}

/// A sort key (`struct keyfield`): where it starts and ends, and how it
/// compares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub sword: usize,
    pub schar: usize,
    pub eword: usize,
    pub echar: usize,
    pub ignore: Ignore,
    /// `-f`: lower case folded to upper.
    pub translate: bool,
    pub skipsblanks: bool,
    pub skipeblanks: bool,
    pub numeric: bool,
    pub random: bool,
    pub general_numeric: bool,
    pub human_numeric: bool,
    pub month: bool,
    pub reverse: bool,
    pub version: bool,
    pub traditional_used: bool,
}

impl Default for Key {
    fn default() -> Self {
        Key {
            sword: 0,
            schar: 0,
            eword: NONE,
            echar: 0,
            ignore: Ignore::None,
            translate: false,
            skipsblanks: false,
            skipeblanks: false,
            numeric: false,
            random: false,
            general_numeric: false,
            human_numeric: false,
            month: false,
            reverse: false,
            version: false,
            traditional_used: false,
        }
    }
}

impl Key {
    /// `key_numeric`.
    pub fn is_numeric(&self) -> bool {
        self.numeric || self.general_numeric || self.human_numeric
    }

    /// `default_key_compare`: no ordering option given.
    pub fn default_compare(&self) -> bool {
        !(self.ignore != Ignore::None
            || self.translate
            || self.skipsblanks
            || self.skipeblanks
            || self.is_numeric()
            || self.month
            || self.version
            || self.random)
    }

    /// `set_ordering`: the ordering letters of `s` applied, `b` to the
    /// start, the end or both; the rest of `s` from the first other.
    pub fn set_ordering<'a>(&mut self, s: &'a str, start: bool, end: bool) -> &'a str {
        for (i, c) in s.char_indices() {
            match c {
                'b' => {
                    self.skipsblanks |= start;
                    self.skipeblanks |= end;
                }
                'd' => self.ignore = Ignore::Nondictionary,
                'f' => self.translate = true,
                'g' => self.general_numeric = true,
                'h' => self.human_numeric = true,
                'i' => {
                    if self.ignore == Ignore::None {
                        self.ignore = Ignore::Nonprinting;
                    }
                }
                'M' => self.month = true,
                'n' => self.numeric = true,
                'R' => self.random = true,
                'r' => self.reverse = true,
                'V' => self.version = true,
                _ => return &s[i..],
            }
        }
        ""
    }

    /// `key_to_opts`: the letters that give this key's ordering.
    pub fn to_opts(&self) -> String {
        let mut o = String::new();
        let mut add = |on: bool, c: char| {
            if on {
                o.push(c);
            }
        };
        add(self.skipsblanks || self.skipeblanks, 'b');
        add(self.ignore == Ignore::Nondictionary, 'd');
        add(self.translate, 'f');
        add(self.general_numeric, 'g');
        add(self.human_numeric, 'h');
        add(self.ignore == Ignore::Nonprinting, 'i');
        add(self.month, 'M');
        add(self.numeric, 'n');
        add(self.random, 'R');
        add(self.reverse, 'r');
        add(self.version, 'V');
        o
    }
}

/// `blanks`: a blank, or a newline (with `-z` a line may hold one).
pub fn blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n')
}

fn ignored(ignore: Ignore, c: u8) -> bool {
    match ignore {
        Ignore::None => false,
        Ignore::Nondictionary => !blank(c) && !c.is_ascii_alphanumeric(),
        Ignore::Nonprinting => !(0x20..0x7f).contains(&c),
    }
}

fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// The month names, in the C.UTF-8 locale's order of `nl_langinfo
/// (ABMON_n)` upper-cased, sorted for the binary search.
const MONTHS: [(&[u8], i32); 12] = [
    (b"APR", 4),
    (b"AUG", 8),
    (b"DEC", 12),
    (b"FEB", 2),
    (b"JAN", 1),
    (b"JUL", 7),
    (b"JUN", 6),
    (b"MAR", 3),
    (b"MAY", 5),
    (b"NOV", 11),
    (b"OCT", 10),
    (b"SEP", 9),
];

/// `getmonth`: the month (1 to 12, 0 for none) at the start of `s`, after
/// blanks, and where its name ends.
pub fn getmonth(s: &[u8]) -> (i32, Option<usize>) {
    let mut m = 0;
    while blank(at(s, m)) {
        m += 1;
    }
    let (mut lo, mut hi) = (0usize, 12usize);
    loop {
        let ix = (lo + hi) / 2;
        let name = MONTHS[ix].0;
        let mut k = 0;
        loop {
            if k == name.len() {
                return (MONTHS[ix].1, Some(m + k));
            }
            let c = at(s, m + k).to_ascii_uppercase();
            if c < name[k] {
                hi = ix;
                break;
            } else if c > name[k] {
                lo = ix + 1;
                break;
            }
            k += 1;
        }
        if lo >= hi {
            return (0, None);
        }
    }
}

/// `nan_compare`: two NaNs as `printf ("%Lf")` writes them.
fn nan_text(v: LongDouble) -> &'static str {
    if v.is_negative() { "-nan" } else { "nan" }
}

/// `general_numcompare`.
fn general_numcompare(a: &[u8], b: &[u8]) -> i32 {
    let (x, ea) = strtold(a);
    let (y, eb) = strtold(b);
    if ea == 0 {
        return if eb == 0 { 0 } else { -1 };
    }
    if eb == 0 {
        return 1;
    }
    match x.partial_cmp(y) {
        Some(o) => o as i32,
        None => {
            let (xn, yn) = (
                matches!(x, LongDouble::Nan { .. }),
                matches!(y, LongDouble::Nan { .. }),
            );
            if !yn {
                -1
            } else if !xn {
                1
            } else {
                nan_text(x).cmp(nan_text(y)) as i32
            }
        }
    }
}

/// `debug_width`: the columns of `s` (`mbsnwidth`), a tab counting one.
fn debug_width(s: &[u8]) -> usize {
    let mut width = 0;
    let mut p = 0;
    while p < s.len() {
        let c = s[p];
        if (0x20..0x7f).contains(&c) {
            width += 1;
            p += 1;
            continue;
        }
        match glibc::mbrtoc32(&s[p..]) {
            Decoded::Invalid => {
                width += 1;
                p += 1;
            }
            Decoded::Incomplete => {
                width += 1;
                p = s.len();
            }
            Decoded::Char(ch, n) => {
                let w = glibc::width(ch);
                if w >= 0 {
                    width += w as usize;
                } else {
                    let cntrl =
                        ch < 0x20 || (0x7f..=0x9f).contains(&ch) || ch == 0x2028 || ch == 0x2029;
                    if !cntrl {
                        width += 1;
                    }
                }
                p += n;
            }
        }
    }
    width + s.iter().filter(|&&c| c == b'\t').count()
}

/// How lines are ordered: the keys, and the global options.
#[derive(Clone, Debug, Default)]
pub struct Sorter {
    pub keys: Vec<Key>,
    /// `-t`: the field separator; fields split at blanks without it.
    pub tab: Option<u8>,
    pub eol: u8,
    /// `-r` given alone: the last-resort comparison reversed.
    pub reverse: bool,
    pub stable: bool,
    pub unique: bool,
    /// `-R`'s seed: the 16 bytes the MD5 of every key starts with.
    pub seed: [u8; 16],
}

impl Sorter {
    /// `begfield`: where KEY starts in LINE.
    pub fn begfield(&self, line: &[u8], key: &Key) -> usize {
        let lim = line.len();
        let mut ptr = 0;
        let mut sword = key.sword;
        match self.tab {
            Some(tab) => {
                while ptr < lim && sword > 0 {
                    sword -= 1;
                    while ptr < lim && line[ptr] != tab {
                        ptr += 1;
                    }
                    if ptr < lim {
                        ptr += 1;
                    }
                }
            }
            None => {
                while ptr < lim && sword > 0 {
                    sword -= 1;
                    while ptr < lim && blank(line[ptr]) {
                        ptr += 1;
                    }
                    while ptr < lim && !blank(line[ptr]) {
                        ptr += 1;
                    }
                }
            }
        }
        if key.skipsblanks {
            while ptr < lim && blank(line[ptr]) {
                ptr += 1;
            }
        }
        lim.min(ptr.saturating_add(key.schar))
    }

    /// `limfield`: where KEY ends in LINE.
    pub fn limfield(&self, line: &[u8], key: &Key) -> usize {
        let lim = line.len();
        let mut ptr = 0;
        let mut eword = key.eword;
        let echar = key.echar;
        if echar == 0 {
            eword = eword.wrapping_add(1);
        }
        match self.tab {
            Some(tab) => {
                while ptr < lim && eword > 0 {
                    eword -= 1;
                    while ptr < lim && line[ptr] != tab {
                        ptr += 1;
                    }
                    if ptr < lim && (eword > 0 || echar > 0) {
                        ptr += 1;
                    }
                }
            }
            None => {
                while ptr < lim && eword > 0 {
                    eword -= 1;
                    while ptr < lim && blank(line[ptr]) {
                        ptr += 1;
                    }
                    while ptr < lim && !blank(line[ptr]) {
                        ptr += 1;
                    }
                }
            }
        }
        if echar != 0 {
            if key.skipeblanks {
                while ptr < lim && blank(line[ptr]) {
                    ptr += 1;
                }
            }
            ptr = lim.min(ptr.saturating_add(echar));
        }
        ptr
    }

    /// A key's start and limit in a line. The first key's start is found
    /// as `fillbuf` finds it: its blanks skipped to the line's end.
    fn key_span(&self, line: &[u8], key: &Key, first: bool) -> (usize, usize) {
        let lim = if key.eword != NONE {
            self.limfield(line, key)
        } else {
            line.len()
        };
        let beg = if key.sword != NONE {
            self.begfield(line, key)
        } else {
            let mut b = 0;
            if key.skipsblanks {
                let bound = if first { line.len() } else { lim };
                while b < bound && blank(line[b]) {
                    b += 1;
                }
            }
            b
        };
        (beg, lim.max(beg))
    }

    /// `compare_random`: the keys' MD5s from the seed.
    fn compare_random(&self, a: &[u8], b: &[u8]) -> i32 {
        // What `strxfrm` gives each NUL-ended piece (the bytes, in
        // C.UTF-8), each with its NUL.
        let hashed = |t: &[u8]| -> Vec<u8> {
            let mut v = t.to_vec();
            if !t.is_empty() && t.last() != Some(&0) {
                v.push(0);
            }
            v
        };
        let (ha, hb) = (hashed(a), hashed(b));
        let digest = |t: &[u8]| {
            let mut m = Md5::new();
            m.update(self.seed);
            m.update(t);
            m.finalize()
        };
        match digest(&ha).as_slice().cmp(digest(&hb).as_slice()) {
            std::cmp::Ordering::Equal => ha.cmp(&hb) as i32,
            o => o as i32,
        }
    }

    /// `keycompare`: the keys in turn until one differs.
    pub fn keycompare(&self, a: &[u8], b: &[u8]) -> i32 {
        for (n, key) in self.keys.iter().enumerate() {
            let (ab, al) = self.key_span(a, key, n == 0);
            let (bb, bl) = self.key_span(b, key, n == 0);
            let (ka, kb) = (&a[ab..al], &b[bb..bl]);
            let copy = |k: &[u8]| -> Vec<u8> {
                k.iter()
                    .filter(|&&c| !ignored(key.ignore, c))
                    .map(|&c| {
                        if key.translate {
                            c.to_ascii_uppercase()
                        } else {
                            c
                        }
                    })
                    .collect()
            };
            let (ta, tb): (std::borrow::Cow<[u8]>, std::borrow::Cow<[u8]>) =
                if key.ignore != Ignore::None || key.translate {
                    (copy(ka).into(), copy(kb).into())
                } else {
                    (ka.into(), kb.into())
                };
            let skip = |s: &[u8]| -> usize { s.iter().take_while(|&&c| blank(c)).count() };
            let diff = if key.numeric {
                strnumcmp(&ta[skip(&ta)..], &tb[skip(&tb)..])
            } else if key.general_numeric {
                general_numcompare(&ta, &tb)
            } else if key.human_numeric {
                human_numcompare(&ta[skip(&ta)..], &tb[skip(&tb)..])
            } else if key.month {
                getmonth(&ta).0 - getmonth(&tb).0
            } else if key.random {
                self.compare_random(&ta, &tb)
            } else if key.version {
                filenvercmp(&ta, &tb)
            } else if ta.is_empty() {
                -i32::from(!tb.is_empty())
            } else if tb.is_empty() {
                1
            } else {
                ta.cmp(&tb) as i32
            };
            if diff != 0 {
                return if key.reverse {
                    -diff.signum()
                } else {
                    diff.signum()
                };
            }
        }
        0
    }

    /// `compare`: the keys, then (unless `-s` or `-u`) the whole lines.
    pub fn compare(&self, a: &[u8], b: &[u8]) -> i32 {
        if !self.keys.is_empty() {
            let diff = self.keycompare(a, b);
            if diff != 0 || self.unique || self.stable {
                return diff;
            }
        }
        let diff = if a.is_empty() {
            -i32::from(!b.is_empty())
        } else if b.is_empty() {
            1
        } else {
            a.cmp(b) as i32
        };
        if self.reverse { -diff } else { diff }
    }

    /// A stable merge sort by `compare`.
    pub fn sort<'a>(&self, lines: &mut Vec<&'a [u8]>) {
        let n = lines.len();
        if n < 2 {
            return;
        }
        let mut buf: Vec<&'a [u8]> = Vec::with_capacity(n);
        let mut width = 1;
        let mut src = std::mem::take(lines);
        while width < n {
            buf.clear();
            let mut i = 0;
            while i < n {
                let mid = (i + width).min(n);
                let end = (i + 2 * width).min(n);
                let (mut l, mut r) = (i, mid);
                while l < mid && r < end {
                    if self.compare(src[l], src[r]) <= 0 {
                        buf.push(src[l]);
                        l += 1;
                    } else {
                        buf.push(src[r]);
                        r += 1;
                    }
                }
                buf.extend_from_slice(&src[l..mid]);
                buf.extend_from_slice(&src[r..end]);
                i = end;
            }
            std::mem::swap(&mut src, &mut buf);
            width *= 2;
        }
        *lines = src;
    }

    /// `write_unique`'s choice: with `-u`, the first of lines that compare
    /// equal.
    pub fn unique_lines<'a>(&self, lines: Vec<&'a [u8]>) -> Vec<&'a [u8]> {
        if !self.unique {
            return lines;
        }
        let mut out: Vec<&'a [u8]> = Vec::with_capacity(lines.len());
        for l in lines {
            if out.last().is_some_and(|s| self.compare(l, s) == 0) {
                continue;
            }
            out.push(l);
        }
        out
    }

    /// `mergefps`: sorted inputs merged as sort merges them (the inputs in
    /// order on ties), with `-u` the first of equal lines.
    pub fn mergefps<'a>(&self, inputs: Vec<Vec<&'a [u8]>>) -> Vec<&'a [u8]> {
        let mut files: Vec<(Vec<&'a [u8]>, usize)> = inputs
            .into_iter()
            .filter(|f| !f.is_empty())
            .map(|f| (f, 0))
            .collect();
        let mut out = Vec::new();
        let mut nfiles = files.len();
        let cur = |files: &Vec<(Vec<&'a [u8]>, usize)>, i: usize| files[i].0[files[i].1];
        let mut ord: Vec<usize> = (0..nfiles).collect();
        let mut i = 1;
        while i < nfiles {
            if 0 < self.compare(cur(&files, ord[i - 1]), cur(&files, ord[i])) {
                ord.swap(i - 1, i);
                i = 0;
            }
            i += 1;
        }
        let mut saved: Option<&'a [u8]> = None;
        while nfiles > 0 {
            let smallest = cur(&files, ord[0]);
            if self.unique {
                if let Some(s) = saved
                    && self.compare(s, smallest) != 0
                {
                    out.push(s);
                    saved = None;
                }
                if saved.is_none() {
                    saved = Some(smallest);
                }
            } else {
                out.push(smallest);
            }
            let o0 = ord[0];
            files[o0].1 += 1;
            if files[o0].1 == files[o0].0.len() {
                for x in ord.iter_mut().skip(1) {
                    if *x > o0 {
                        *x -= 1;
                    }
                }
                files.remove(o0);
                ord.remove(0);
                nfiles -= 1;
                continue;
            }
            let (mut lo, mut hi) = (1usize, nfiles);
            let mut probe = lo;
            while lo < hi {
                let cmp = self.compare(cur(&files, o0), cur(&files, ord[probe]));
                if cmp < 0 || (cmp == 0 && o0 < ord[probe]) {
                    hi = probe;
                } else {
                    lo = probe + 1;
                }
                probe = (lo + hi) / 2;
            }
            let count = lo - 1;
            ord.copy_within(1..=count, 0);
            ord[count] = o0;
        }
        if let Some(s) = saved {
            out.push(s);
        }
        out
    }

    /// `merge`: NMERGE inputs at a time into temporaries, as sort merges
    /// many, then the rest into the output.
    pub fn merge<'a>(&self, mut files: Vec<Vec<&'a [u8]>>, nmerge: usize) -> Vec<&'a [u8]> {
        let nmerge = nmerge.max(2);
        while nmerge < files.len() {
            let nfiles = files.len();
            let mut next: Vec<Vec<&'a [u8]>> = Vec::new();
            let mut taken = 0;
            while nmerge <= nfiles - taken {
                let group = files[taken..taken + nmerge].to_vec();
                next.push(self.mergefps(group));
                taken += nmerge;
            }
            let remainder = nfiles - taken;
            let cheap = nmerge - next.len() % nmerge;
            if cheap < remainder {
                let short = remainder - cheap + 1;
                let group = files[taken..taken + short].to_vec();
                next.push(self.mergefps(group));
                taken += short;
            }
            next.extend(files.drain(taken..));
            files = next;
        }
        self.mergefps(files)
    }

    /// `check`: the 0-based index of the first line out of order.
    pub fn check(&self, lines: &[&[u8]]) -> Option<usize> {
        let nonunique = i32::from(!self.unique);
        (1..lines.len()).find(|&i| nonunique <= self.compare(lines[i - 1], lines[i]))
    }

    /// `debug_key`: the mark under one key (or the whole line).
    fn debug_key(&self, line: &[u8], key: Option<&Key>) -> Vec<u8> {
        let mut beg = 0;
        let mut lim = line.len();
        if let Some(key) = key {
            if key.sword != NONE {
                beg = self.begfield(line, key);
            }
            if key.eword != NONE {
                lim = self.limfield(line, key).max(beg);
            }
            if (key.skipsblanks && key.sword == NONE) || key.month || key.is_numeric() {
                while beg < lim && blank(line[beg]) {
                    beg += 1;
                }
                let s = &line[beg..lim.max(beg)];
                let mut tighter = beg;
                if lim < beg {
                    tighter = lim;
                } else if key.month {
                    if let (_, Some(end)) = getmonth(s) {
                        tighter = beg + end;
                    }
                } else if key.general_numeric {
                    tighter = beg + strtold(s).1;
                } else if key.numeric || key.human_numeric {
                    let p = usize::from(!s.is_empty() && s[0] == b'-');
                    let (max_digit, end) = traverse_raw_number(s, p);
                    if b'0' <= max_digit {
                        tighter = beg
                            + end
                            + usize::from(key.human_numeric && unit_order(at(s, end)) != 0);
                    }
                } else {
                    tighter = lim;
                }
                lim = tighter;
            }
        }
        let offset = debug_width(&line[..beg.min(line.len())]);
        let width =
            debug_width(&line[beg.min(line.len())..lim.min(line.len()).max(beg.min(line.len()))]);
        let mut out = vec![b' '; offset];
        if width == 0 {
            out.extend_from_slice(b"^ no match for key\n");
        } else {
            out.extend(std::iter::repeat_n(b'_', width));
            out.push(b'\n');
        }
        out
    }

    /// `write_line` with `--debug`: the line (tabs as '>'), then its keys
    /// marked, and the whole line's last unless `-s` or `-u`.
    pub fn debug_line(&self, line: &[u8]) -> Vec<u8> {
        let mut out: Vec<u8> = line
            .iter()
            .map(|&c| if c == b'\t' { b'>' } else { c })
            .collect();
        out.push(b'\n');
        if self.keys.is_empty() {
            out.extend(self.debug_key(line, None));
            return out;
        }
        for k in &self.keys {
            out.extend(self.debug_key(line, Some(k)));
        }
        if !(self.unique || self.stable) {
            out.extend(self.debug_key(line, None));
        }
        out
    }
}

/// The lines of an input as sort reads them: split at `eol`, the last one
/// whether or not it ends with it.
pub fn lines(data: &[u8], eol: u8) -> Vec<&[u8]> {
    if data.is_empty() {
        return Vec::new();
    }
    let body = if data.last() == Some(&eol) {
        &data[..data.len() - 1]
    } else {
        data
    };
    body.split(|&c| c == eol).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(input: &str, s: &Sorter) -> String {
        let mut l = lines(input.as_bytes(), b'\n');
        s.sort(&mut l);
        let l = s.unique_lines(l);
        l.iter()
            .map(|x| format!("{}\n", String::from_utf8_lossy(x)))
            .collect()
    }

    #[test]
    fn sorts_as_gnu_sort() {
        let plain = Sorter {
            eol: b'\n',
            ..Sorter::default()
        };
        assert_eq!(sorted("b\na\nB\n\nä\n", &plain), "\nB\na\nb\nä\n");
        let numeric = Sorter {
            keys: vec![Key {
                sword: NONE,
                numeric: true,
                ..Key::default()
            }],
            eol: b'\n',
            ..Sorter::default()
        };
        assert_eq!(sorted("10\n9\n-1\nx\n", &numeric), "-1\nx\n9\n10\n");
        let second = Sorter {
            keys: vec![Key {
                sword: 1,
                eword: 1,
                ..Key::default()
            }],
            eol: b'\n',
            stable: true,
            ..Sorter::default()
        };
        assert_eq!(sorted("a z\nb y\nc y\n", &second), "b y\nc y\na z\n");
        let unique = Sorter {
            eol: b'\n',
            unique: true,
            ..Sorter::default()
        };
        assert_eq!(sorted("b\na\nb\n", &unique), "a\nb\n");
    }

    #[test]
    fn months_and_debug_marks() {
        assert_eq!(getmonth(b"  feb x"), (2, Some(5)));
        assert_eq!(getmonth(b"Foo"), (0, None));
        let s = Sorter {
            eol: b'\n',
            ..Sorter::default()
        };
        assert_eq!(s.debug_line(b"a\tb"), b"a>b\n___\n".to_vec());
        assert_eq!(s.debug_line(b""), b"\n^ no match for key\n".to_vec());
    }
}
