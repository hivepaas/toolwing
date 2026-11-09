//! `strtold` as glibc does it on x86-64, where a long double is the x87's
//! 80-bit extended format: a 64-bit significand and exponents from -16382
//! to 16383 (subnormals below), every decimal or hexadecimal number rounded
//! to the nearest, ties to even, from its exact value; infinities and NaNs
//! by name. `sort -g` compares these, so two numbers that differ past a
//! double's precision but within a long double's still differ.

use std::cmp::Ordering;

use num_bigint::BigUint;

/// A long double.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LongDouble {
    /// `mant * 2^exp`, `mant` 0 (a zero) or with its top bit set.
    Num {
        neg: bool,
        mant: u64,
        exp: i32,
    },
    Inf {
        neg: bool,
    },
    Nan {
        neg: bool,
    },
}

/// The lowest exponent of a significand's last bit: subnormals' 2^-16445.
const MIN_LSB: i64 = -16445;
/// The highest exponent of a significand's top bit.
const MAX_TOP: i64 = 16383;

impl LongDouble {
    /// `<`, `>` and `==` as C compares long doubles: -0 == +0, NaNs
    /// unordered.
    pub fn partial_cmp(self, other: LongDouble) -> Option<Ordering> {
        use LongDouble::*;
        let key = |v: LongDouble| -> Option<(i8, i64, u64)> {
            // Sign, then magnitude (top bit's exponent, significand).
            match v {
                Nan { .. } => None,
                Inf { neg } => Some((if neg { -1 } else { 1 }, i64::MAX, u64::MAX)),
                Num { mant: 0, .. } => Some((0, 0, 0)),
                Num { neg, mant, exp } => {
                    Some((if neg { -1 } else { 1 }, i64::from(exp) + 63, mant))
                }
            }
        };
        let (a, b) = (key(self)?, key(other)?);
        if a.0 != b.0 {
            return Some(a.0.cmp(&b.0));
        }
        let mag = (a.1, a.2).cmp(&(b.1, b.2));
        Some(if a.0 < 0 { mag.reverse() } else { mag })
    }

    /// Whether the sign bit is set (a NaN's too), as `printf ("%Lf")`
    /// shows it.
    pub fn is_negative(self) -> bool {
        match self {
            LongDouble::Num { neg, .. } | LongDouble::Inf { neg } | LongDouble::Nan { neg } => neg,
        }
    }
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

fn starts_with_ci(s: &[u8], i: usize, word: &[u8]) -> bool {
    s.len() >= i + word.len() && s[i..i + word.len()].eq_ignore_ascii_case(word)
}

/// `strtold`: the value, and how many bytes it took (0 when there is no
/// number, as when `endptr` is left at the start).
pub fn strtold(s: &[u8]) -> (LongDouble, usize) {
    let mut i = 0;
    while is_space(at(s, i)) {
        i += 1;
    }
    let neg = match at(s, i) {
        b'-' => {
            i += 1;
            true
        }
        b'+' => {
            i += 1;
            false
        }
        _ => false,
    };
    if starts_with_ci(s, i, b"inf") {
        let len = if starts_with_ci(s, i, b"infinity") {
            8
        } else {
            3
        };
        return (LongDouble::Inf { neg }, i + len);
    }
    if starts_with_ci(s, i, b"nan") {
        let mut end = i + 3;
        if at(s, end) == b'(' {
            let mut j = end + 1;
            while at(s, j).is_ascii_alphanumeric() || at(s, j) == b'_' {
                j += 1;
            }
            if at(s, j) == b')' {
                end = j + 1;
            }
        }
        return (LongDouble::Nan { neg }, end);
    }
    if at(s, i) == b'0' && matches!(at(s, i + 1), b'x' | b'X') {
        if let Some((v, end)) = hex(s, i + 2, neg) {
            return (v, end);
        }
        // "0x" with no hex digit: the 0.
        return (
            LongDouble::Num {
                neg,
                mant: 0,
                exp: 0,
            },
            i + 1,
        );
    }
    match decimal(s, i, neg) {
        Some(r) => r,
        None => (
            LongDouble::Num {
                neg: false,
                mant: 0,
                exp: 0,
            },
            0,
        ),
    }
}

/// The digits of a number, its exponent read after them.
fn exponent(s: &[u8], i: usize, marks: &[u8]) -> (i64, usize) {
    if !marks.contains(&at(s, i)) {
        return (0, i);
    }
    let mut j = i + 1;
    let neg = match at(s, j) {
        b'-' => {
            j += 1;
            true
        }
        b'+' => {
            j += 1;
            false
        }
        _ => false,
    };
    if !at(s, j).is_ascii_digit() {
        return (0, i);
    }
    let mut e: i64 = 0;
    while at(s, j).is_ascii_digit() {
        e = e
            .saturating_mul(10)
            .saturating_add(i64::from(at(s, j) - b'0'));
        j += 1;
    }
    (if neg { -e } else { e }, j)
}

fn decimal(s: &[u8], start: usize, neg: bool) -> Option<(LongDouble, usize)> {
    let mut i = start;
    let mut digits: Vec<u8> = Vec::new();
    let mut seen = false;
    let mut scale: i64 = 0;
    while at(s, i).is_ascii_digit() {
        seen = true;
        digits.push(at(s, i) - b'0');
        i += 1;
    }
    if at(s, i) == b'.' {
        let mut j = i + 1;
        let mut frac = false;
        while at(s, j).is_ascii_digit() {
            frac = true;
            digits.push(at(s, j) - b'0');
            scale -= 1;
            j += 1;
        }
        if frac || seen {
            i = j;
            seen = true;
        }
    }
    if !seen {
        return None;
    }
    let (e, end) = exponent(s, i, b"eE");
    let first = digits.iter().position(|&d| d != 0);
    let Some(first) = first else {
        return Some((
            LongDouble::Num {
                neg,
                mant: 0,
                exp: 0,
            },
            end,
        ));
    };
    let last = digits.iter().rposition(|&d| d != 0).unwrap_or(first);
    let trailing = (digits.len() - 1 - last) as i64;
    let sig = &digits[first..=last];
    let e10 = e.saturating_add(scale).saturating_add(trailing);
    let n = sig.len() as i64;
    // Far past either end: infinity, or zero.
    if (n - 1).saturating_add(e10) > 4933 {
        return Some((LongDouble::Inf { neg }, end));
    }
    if n.saturating_add(e10) < -4952 {
        return Some((
            LongDouble::Num {
                neg,
                mant: 0,
                exp: 0,
            },
            end,
        ));
    }
    let mut d = BigUint::from(0u8);
    for &x in sig {
        d = d * 10u8 + x;
    }
    let ten = BigUint::from(10u8);
    let v = if e10 >= 0 {
        round(&(d * ten.pow(e10 as u32)), 0, false, neg)
    } else {
        let den = ten.pow((-e10) as u32);
        let k = (66 + den.bits() as i64 - d.bits() as i64).max(0) as usize + 2;
        let num = d << k;
        let q = &num / &den;
        let sticky = &num % &den != BigUint::from(0u8);
        round(&q, k as i64, sticky, neg)
    };
    Some((v, end))
}

fn hex(s: &[u8], start: usize, neg: bool) -> Option<(LongDouble, usize)> {
    let mut i = start;
    let mut m = BigUint::from(0u8);
    let mut seen = false;
    let mut scale: i64 = 0;
    while at(s, i).is_ascii_hexdigit() {
        seen = true;
        m = (m << 4usize) + (at(s, i) as char).to_digit(16).unwrap_or(0);
        i += 1;
    }
    if at(s, i) == b'.' {
        let mut j = i + 1;
        let mut frac = false;
        while at(s, j).is_ascii_hexdigit() {
            frac = true;
            m = (m << 4usize) + (at(s, j) as char).to_digit(16).unwrap_or(0);
            scale -= 4;
            j += 1;
        }
        if frac || seen {
            i = j;
            seen = true;
        }
    }
    if !seen {
        return None;
    }
    let (p, end) = exponent(s, i, b"pP");
    if m == BigUint::from(0u8) {
        return Some((
            LongDouble::Num {
                neg,
                mant: 0,
                exp: 0,
            },
            end,
        ));
    }
    let e2 = p.saturating_add(scale);
    let top = (m.bits() as i64 - 1).saturating_add(e2);
    if top > MAX_TOP {
        return Some((LongDouble::Inf { neg }, end));
    }
    if top < MIN_LSB - 2 {
        return Some((
            LongDouble::Num {
                neg,
                mant: 0,
                exp: 0,
            },
            end,
        ));
    }
    // value = m * 2^e2 = m * 2^-k with k = -e2.
    if e2 >= 0 {
        Some((round(&(m << e2 as usize), 0, false, neg), end))
    } else {
        Some((round(&m, -e2, false, neg), end))
    }
}

/// `q * 2^-k` (and a bit more when `sticky`) rounded to a long double.
fn round(q: &BigUint, k: i64, sticky: bool, neg: bool) -> LongDouble {
    let bits = q.bits() as i64;
    if bits == 0 {
        // Only the sticky part: below the smallest subnormal's half.
        return LongDouble::Num {
            neg,
            mant: 0,
            exp: 0,
        };
    }
    let top = bits - 1 - k;
    if top > MAX_TOP {
        return LongDouble::Inf { neg };
    }
    let lsb = (top - 63).max(MIN_LSB);
    let shift = lsb + k;
    let (mut mant, mut lsb) = if shift <= 0 {
        let m: u64 = (q << (-shift) as usize).try_into().unwrap_or(u64::MAX);
        (m, lsb)
    } else {
        let shift_u = shift as u64;
        let kept = q >> shift_u as usize;
        let mut m: u64 = (&kept).try_into().unwrap_or(u64::MAX);
        let guard = q.bit(shift_u - 1);
        let below = shift_u > 1 && q.trailing_zeros().is_some_and(|tz| tz < shift_u - 1);
        if guard && (below || sticky || m & 1 == 1) {
            m = m.wrapping_add(1);
            if m == 0 {
                // Carried past 64 bits.
                return finish(neg, 1 << 63, lsb + 1);
            }
        }
        (m, lsb)
    };
    if mant == 0 {
        return LongDouble::Num {
            neg,
            mant: 0,
            exp: 0,
        };
    }
    // Normalized: the top bit set (the value is the same).
    let lz = mant.leading_zeros();
    mant <<= lz;
    lsb -= i64::from(lz);
    finish(neg, mant, lsb)
}

fn finish(neg: bool, mant: u64, lsb: i64) -> LongDouble {
    if lsb + 63 > MAX_TOP {
        return LongDouble::Inf { neg };
    }
    LongDouble::Num {
        neg,
        mant,
        exp: lsb as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ld(s: &str) -> LongDouble {
        strtold(s.as_bytes()).0
    }

    #[test]
    fn rounds_as_an_x87_long_double() {
        let one = ld("1");
        assert_eq!(
            one,
            LongDouble::Num {
                neg: false,
                mant: 1 << 63,
                exp: -63
            }
        );
        // 1 + 1e-19 is above 1 in 64 bits; 1 + 1e-20 is not.
        assert_eq!(
            ld("1.0000000000000000001").partial_cmp(one),
            Some(Ordering::Greater)
        );
        assert_eq!(
            ld("1.00000000000000000001").partial_cmp(one),
            Some(Ordering::Equal)
        );
        assert_eq!(ld("0x10").partial_cmp(ld("16")), Some(Ordering::Equal));
        assert_eq!(ld("0x1.8p1").partial_cmp(ld("3")), Some(Ordering::Equal));
        assert_eq!(ld("-0").partial_cmp(ld("0")), Some(Ordering::Equal));
        assert_eq!(ld("1e400").partial_cmp(ld("inf")), Some(Ordering::Less));
        assert_eq!(ld("1e5000"), LongDouble::Inf { neg: false });
        assert_eq!(
            ld("1e-5000"),
            LongDouble::Num {
                neg: false,
                mant: 0,
                exp: 0
            }
        );
        assert!(ld("3.6e-4951").partial_cmp(ld("0")) == Some(Ordering::Greater));
        assert_eq!(ld("nan").partial_cmp(ld("1")), None);
        assert_eq!(strtold(b"  -12.5e1x").1, 9);
        assert_eq!(strtold(b"1e").1, 1);
        assert_eq!(strtold(b"0x").1, 1);
        assert_eq!(strtold(b"nan(12)z").1, 7);
        assert_eq!(strtold(b"x").1, 0);
        assert_eq!(strtold(b"infinityx").1, 8);
        // Exponents past any range.
        assert_eq!(
            ld("12e99999999999999999999"),
            LongDouble::Inf { neg: false }
        );
        assert_eq!(
            ld("12e-99999999999999999999").partial_cmp(ld("0")),
            Some(Ordering::Equal)
        );
        assert_eq!(
            ld("0x1p99999999999999999999"),
            LongDouble::Inf { neg: false }
        );
    }
}
