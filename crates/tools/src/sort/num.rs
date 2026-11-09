//! The comparisons of `sort -n`, `-h` and `-V`: gnulib's `strnumcmp` (the
//! numbers compared as strings, digit by digit, never converted), sort's
//! units for `-h`, and gnulib's `filevercmp`. Keys are slices; past their
//! end reads as the C's terminating NUL. In C.UTF-8 the decimal point is
//! '.' and there is no thousands separator.
//!
//! Ported from gnulib's strnumcmp-in.h and filevercmp.c, and coreutils
//! 9.7's sort.c: Copyright (C) 1988-2025 Free Software Foundation, Inc.
//! GPL-3.0-or-later.

use std::cmp::Ordering;

const DECIMAL_POINT: u8 = b'.';
/// No thousands separator: a value no byte has.
const THOUSANDS_SEP: i32 = 256;

fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

fn is_sep(c: u8) -> bool {
    i32::from(c) == THOUSANDS_SEP
}

/// `fraccompare`: two fractions, each from its decimal point.
fn fraccompare(a: &[u8], mut ai: usize, b: &[u8], mut bi: usize) -> i32 {
    if at(a, ai) == DECIMAL_POINT && at(b, bi) == DECIMAL_POINT {
        loop {
            ai += 1;
            bi += 1;
            if at(a, ai) != at(b, bi) {
                break;
            }
            if !at(a, ai).is_ascii_digit() {
                return 0;
            }
        }
        let (ca, cb) = (at(a, ai), at(b, bi));
        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            return i32::from(ca) - i32::from(cb);
        }
        if ca.is_ascii_digit() {
            while at(a, ai) == b'0' {
                ai += 1;
            }
            return i32::from(at(a, ai).is_ascii_digit());
        }
        if cb.is_ascii_digit() {
            while at(b, bi) == b'0' {
                bi += 1;
            }
            return -i32::from(at(b, bi).is_ascii_digit());
        }
        return 0;
    }
    let ca = at(a, ai);
    ai += 1;
    if ca == DECIMAL_POINT {
        while at(a, ai) == b'0' {
            ai += 1;
        }
        return i32::from(at(a, ai).is_ascii_digit());
    }
    let cb = at(b, bi);
    bi += 1;
    if cb == DECIMAL_POINT {
        while at(b, bi) == b'0' {
            bi += 1;
        }
        return -i32::from(at(b, bi).is_ascii_digit());
    }
    0
}

/// `strnumcmp`: two numbers (an optional '-', digits, a fraction) as
/// strings.
pub fn strnumcmp(a: &[u8], b: &[u8]) -> i32 {
    let (mut ai, mut bi) = (0usize, 0usize);
    let mut ta = at(a, 0);
    let mut tb = at(b, 0);
    macro_rules! next_a {
        () => {{
            ai += 1;
            ta = at(a, ai);
        }};
    }
    macro_rules! next_b {
        () => {{
            bi += 1;
            tb = at(b, bi);
        }};
    }
    if ta == b'-' {
        loop {
            next_a!();
            if !(ta == b'0' || is_sep(ta)) {
                break;
            }
        }
        if tb != b'-' {
            if ta == DECIMAL_POINT {
                loop {
                    next_a!();
                    if ta != b'0' {
                        break;
                    }
                }
            }
            if ta.is_ascii_digit() {
                return -1;
            }
            while tb == b'0' || is_sep(tb) {
                next_b!();
            }
            if tb == DECIMAL_POINT {
                loop {
                    next_b!();
                    if tb != b'0' {
                        break;
                    }
                }
            }
            return -i32::from(tb.is_ascii_digit());
        }
        loop {
            next_b!();
            if !(tb == b'0' || is_sep(tb)) {
                break;
            }
        }
        while ta == tb && ta.is_ascii_digit() {
            loop {
                next_a!();
                if !is_sep(ta) {
                    break;
                }
            }
            loop {
                next_b!();
                if !is_sep(tb) {
                    break;
                }
            }
        }
        if (ta == DECIMAL_POINT && !tb.is_ascii_digit())
            || (tb == DECIMAL_POINT && !ta.is_ascii_digit())
        {
            return fraccompare(b, bi, a, ai);
        }
        let tmp = i32::from(tb) - i32::from(ta);
        let mut log_a = 0usize;
        while ta.is_ascii_digit() {
            log_a += 1;
            loop {
                next_a!();
                if !is_sep(ta) {
                    break;
                }
            }
        }
        let mut log_b = 0usize;
        while tb.is_ascii_digit() {
            log_b += 1;
            loop {
                next_b!();
                if !is_sep(tb) {
                    break;
                }
            }
        }
        if log_a != log_b {
            return if log_a < log_b { 1 } else { -1 };
        }
        if log_a == 0 {
            return 0;
        }
        return tmp;
    }
    if tb == b'-' {
        loop {
            next_b!();
            if !(tb == b'0' || is_sep(tb)) {
                break;
            }
        }
        if tb == DECIMAL_POINT {
            loop {
                next_b!();
                if tb != b'0' {
                    break;
                }
            }
        }
        if tb.is_ascii_digit() {
            return 1;
        }
        while ta == b'0' || is_sep(ta) {
            next_a!();
        }
        if ta == DECIMAL_POINT {
            loop {
                next_a!();
                if ta != b'0' {
                    break;
                }
            }
        }
        return i32::from(ta.is_ascii_digit());
    }
    while ta == b'0' || is_sep(ta) {
        next_a!();
    }
    while tb == b'0' || is_sep(tb) {
        next_b!();
    }
    while ta == tb && ta.is_ascii_digit() {
        loop {
            next_a!();
            if !is_sep(ta) {
                break;
            }
        }
        loop {
            next_b!();
            if !is_sep(tb) {
                break;
            }
        }
    }
    if (ta == DECIMAL_POINT && !tb.is_ascii_digit())
        || (tb == DECIMAL_POINT && !ta.is_ascii_digit())
    {
        return fraccompare(a, ai, b, bi);
    }
    let tmp = i32::from(ta) - i32::from(tb);
    let mut log_a = 0usize;
    while ta.is_ascii_digit() {
        log_a += 1;
        loop {
            next_a!();
            if !is_sep(ta) {
                break;
            }
        }
    }
    let mut log_b = 0usize;
    while tb.is_ascii_digit() {
        log_b += 1;
        loop {
            next_b!();
            if !is_sep(tb) {
                break;
            }
        }
    }
    if log_a != log_b {
        return if log_a < log_b { -1 } else { 1 };
    }
    if log_a == 0 {
        return 0;
    }
    tmp
}

/// `unit_order`: the order of magnitude of a unit, 0 for none.
pub fn unit_order(c: u8) -> i32 {
    match c {
        b'K' | b'k' => 1,
        b'M' => 2,
        b'G' => 3,
        b'T' => 4,
        b'P' => 5,
        b'E' => 6,
        b'Z' => 7,
        b'Y' => 8,
        b'R' => 9,
        b'Q' => 10,
        _ => 0,
    }
}

/// `traverse_raw_number`: from `i`, past digits (one thousands separator
/// between them), a decimal point and digits; the highest digit (0 for
/// none), and where the number ends.
pub fn traverse_raw_number(s: &[u8], i: usize) -> (u8, usize) {
    let mut p = i;
    let mut max_digit = 0u8;
    let mut ends_with_sep = false;
    let mut ch;
    loop {
        ch = at(s, p);
        p += 1;
        if !ch.is_ascii_digit() {
            break;
        }
        max_digit = max_digit.max(ch);
        ends_with_sep = is_sep(at(s, p));
        if ends_with_sep {
            p += 1;
        }
    }
    if ends_with_sep {
        return (max_digit, p - 2);
    }
    if ch == DECIMAL_POINT {
        loop {
            ch = at(s, p);
            p += 1;
            if !ch.is_ascii_digit() {
                break;
            }
            max_digit = max_digit.max(ch);
        }
    }
    (max_digit, p - 1)
}

/// `find_unit_order`.
fn find_unit_order(s: &[u8]) -> i32 {
    let minus = at(s, 0) == b'-';
    let (max_digit, end) = traverse_raw_number(s, usize::from(minus));
    if b'0' < max_digit {
        let order = unit_order(at(s, end));
        if minus { -order } else { order }
    } else {
        0
    }
}

/// `human_numcompare` (blanks already skipped by the caller's `skip`).
pub fn human_numcompare(a: &[u8], b: &[u8]) -> i32 {
    let diff = find_unit_order(a) - find_unit_order(b);
    if diff != 0 { diff } else { strnumcmp(a, b) }
}

// ---- filevercmp ---------------------------------------------------------------

/// `file_prefixlen`: the length before a suffix like `(\.[A-Za-z~][A-Za-z0-9~]*)*$`
/// (never all of a non-empty name).
fn file_prefixlen(s: &[u8]) -> usize {
    let n = s.len();
    let mut prefixlen = 0;
    let mut i = 0;
    loop {
        if i == n {
            return prefixlen;
        }
        i += 1;
        prefixlen = i;
        while i + 1 < n && s[i] == b'.' && (s[i + 1].is_ascii_alphabetic() || s[i + 1] == b'~') {
            i += 2;
            while i < n && (s[i].is_ascii_alphanumeric() || s[i] == b'~') {
                i += 1;
            }
        }
    }
}

fn order(s: &[u8], pos: usize) -> i32 {
    if pos >= s.len() {
        return -1;
    }
    let c = s[pos];
    if c.is_ascii_digit() {
        0
    } else if c.is_ascii_alphabetic() {
        i32::from(c)
    } else if c == b'~' {
        -2
    } else {
        i32::from(c) + 256
    }
}

fn verrevcmp(s1: &[u8], s2: &[u8]) -> i32 {
    let (l1, l2) = (s1.len(), s2.len());
    let (mut p1, mut p2) = (0, 0);
    while p1 < l1 || p2 < l2 {
        let mut first_diff = 0;
        while (p1 < l1 && !s1[p1].is_ascii_digit()) || (p2 < l2 && !s2[p2].is_ascii_digit()) {
            let c1 = order(s1, p1);
            let c2 = order(s2, p2);
            if c1 != c2 {
                return c1 - c2;
            }
            p1 += 1;
            p2 += 1;
        }
        while p1 < l1 && s1[p1] == b'0' {
            p1 += 1;
        }
        while p2 < l2 && s2[p2] == b'0' {
            p2 += 1;
        }
        while p1 < l1 && p2 < l2 && s1[p1].is_ascii_digit() && s2[p2].is_ascii_digit() {
            if first_diff == 0 {
                first_diff = i32::from(s1[p1]) - i32::from(s2[p2]);
            }
            p1 += 1;
            p2 += 1;
        }
        if p1 < l1 && s1[p1].is_ascii_digit() {
            return 1;
        }
        if p2 < l2 && s2[p2].is_ascii_digit() {
            return -1;
        }
        if first_diff != 0 {
            return first_diff;
        }
    }
    0
}

/// `filenvercmp`: version order.
pub fn filenvercmp(a: &[u8], b: &[u8]) -> i32 {
    if a.is_empty() {
        return -i32::from(!b.is_empty());
    }
    if b.is_empty() {
        return 1;
    }
    if a[0] == b'.' {
        if b[0] != b'.' {
            return -1;
        }
        let (adot, bdot) = (a.len() == 1, b.len() == 1);
        if adot {
            return -i32::from(!bdot);
        }
        if bdot {
            return 1;
        }
        let adotdot = a[1] == b'.' && a.len() == 2;
        let bdotdot = b[1] == b'.' && b.len() == 2;
        if adotdot {
            return -i32::from(!bdotdot);
        }
        if bdotdot {
            return 1;
        }
    } else if b[0] == b'.' {
        return 1;
    }
    let ap = file_prefixlen(a);
    let bp = file_prefixlen(b);
    let one_pass_only = ap == a.len() && bp == b.len();
    let result = verrevcmp(&a[..ap], &b[..bp]);
    if result != 0 || one_pass_only {
        result
    } else {
        verrevcmp(a, b)
    }
}

/// An `Ordering` from a C comparison's sign.
pub fn sign(d: i32) -> Ordering {
    d.cmp(&0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_as_strnumcmp() {
        assert!(strnumcmp(b"10", b"9") > 0);
        assert!(strnumcmp(b"-10", b"-9") < 0);
        assert_eq!(strnumcmp(b"007", b"7"), 0);
        assert!(strnumcmp(b"1.5", b"1.25") > 0);
        assert_eq!(strnumcmp(b"-0", b"0"), 0);
        assert!(strnumcmp(b"abc", b"1") < 0);
        assert!(human_numcompare(b"2K", b"1000") > 0);
        assert!(human_numcompare(b"1M", b"2K") > 0);
    }

    #[test]
    fn versions_as_filevercmp() {
        assert!(filenvercmp(b"a-1.10", b"a-1.9") > 0);
        assert!(filenvercmp(b"1.0~rc1", b"1.0") < 0);
        assert!(filenvercmp(b".", b"..") < 0);
        assert!(filenvercmp(b"foo.tar.gz", b"foo2.tar.gz") < 0);
    }
}
