//! Integer / decimal numbers.
use super::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};
use num_bigint::{BigInt, BigUint, Sign};
use num_traits::cast::ToPrimitive;

/// Integer / decimal number.
///
/// The speciality of this type is that numbers are distinguished into
/// integers and 64-bit floating-point numbers.
/// This allows using integers to index arrays,
/// while using floating-point numbers to do general math.
///
/// Operations on numbers follow a few principles:
/// * The sum, difference, product, and remainder of two integers is integer.
/// * Any other operation between two numbers yields a float.
#[derive(Clone, Debug)]
pub enum Num {
    /// Machine-size integer
    Int(isize),
    /// Arbitrarily large integer
    BigInt(Rc<BigInt>),
    /// Floating-point number
    Float(f64),
    /// Decimal number
    Dec(Rc<String>),
}

impl Num {
    /// Create a big integer.
    pub fn big_int(i: BigInt) -> Self {
        Self::BigInt(i.into())
    }

    pub(crate) fn from_str(s: &str) -> Self {
        // toolwing: `-0` stays as written, as jq keeps a literal (jaq: 0)
        if s == "-0" {
            return Self::Dec(Rc::new(s.to_string()));
        }
        Self::from_str_radix(s, 10).unwrap_or_else(|| Self::Dec(Rc::new(s.to_string())))
    }

    /// Convert from an integral type to a machine-sized or big integer.
    pub fn from_integral<T: Copy + TryInto<isize> + Into<BigInt>>(x: T) -> Self {
        x.try_into()
            .map_or_else(|_| Num::big_int(x.into()), Num::Int)
    }

    /// Try to parse an integer from a string with given radix.
    pub fn from_str_radix(i: &str, radix: u32) -> Option<Self> {
        let int = || isize::from_str_radix(i, radix).ok().map(Num::Int);
        let big = || bigint_from_str_radix(i, radix).map(Self::big_int);
        int().or_else(big)
    }

    /// Try to parse a decimal string to a [`Self::Float`], else return NaN.
    pub fn from_dec_str(n: &str) -> Self {
        // TODO: changed to NaN!
        n.parse().map_or(Self::Float(f64::NAN), Self::Float)
    }

    pub(crate) fn is_int(&self) -> bool {
        matches!(self, Self::Int(_) | Self::BigInt(_))
    }

    /// If the value is a machine-sized integer, return it, else fail.
    pub fn as_isize(&self) -> Option<isize> {
        match self {
            Self::Int(i) => Some(*i),
            Self::BigInt(i) => i.to_isize(),
            _ => None,
        }
    }

    /// An index: a fraction is truncated toward zero, as jq truncates it
    /// (toolwing: jaq refuses it).
    pub(crate) fn as_pos_usize(&self) -> Option<PosUsize> {
        self.as_pos_usize_by(f64::trunc)
    }

    /// An index, a fraction rounded by `round` (toolwing: jq floors a slice's
    /// start and ceils its end); `NaN` is none.
    pub(crate) fn as_pos_usize_by(&self, round: fn(f64) -> f64) -> Option<PosUsize> {
        match self {
            Self::Int(i) => Some(PosUsize(*i >= 0, i.unsigned_abs())),
            Self::BigInt(i) => i
                .magnitude()
                .to_usize()
                .map(|u| PosUsize(i.sign() != Sign::Minus, u)),
            Self::Float(_) | Self::Dec(_) => {
                let f = round(self.as_f64());
                // -0.0 is the start, as 0 is; `as` saturates.
                (!f.is_nan()).then(|| PosUsize(f >= 0.0, f.abs() as usize))
            }
        }
    }

    /// If the value is or can be converted to float, return it, else fail.
    pub(crate) fn as_f64(&self) -> f64 {
        match self {
            Self::Int(n) => *n as f64,
            Self::BigInt(n) => n.to_f64().unwrap(),
            Self::Float(n) => *n,
            Self::Dec(n) => n.parse().unwrap(),
        }
    }

    pub(crate) fn length(&self) -> Self {
        match self {
            Self::Int(i) => Self::Int(i.abs()),
            Self::BigInt(i) => match i.sign() {
                Sign::Plus | Sign::NoSign => Self::BigInt(i.clone()),
                Sign::Minus => Self::BigInt(BigInt::from(i.magnitude().clone()).into()),
            },
            Self::Dec(n) => Self::from_dec_str(n).length(),
            Self::Float(f) => Self::Float(f.abs()),
        }
    }
}

#[derive(Copy, Clone)]
pub(crate) struct PosUsize(pub(crate) bool, pub(crate) usize);

impl PosUsize {
    pub fn wrap(&self, len: usize) -> Option<usize> {
        self.0.then_some(self.1).or_else(|| len.checked_sub(self.1))
    }
}

#[test]
fn wrap_test() {
    let len = 4;
    let pos = |i| PosUsize(true, i);
    let neg = |i| PosUsize(false, i);
    assert_eq!(pos(0).wrap(len), Some(0));
    assert_eq!(pos(8).wrap(len), Some(8));
    assert_eq!(neg(1).wrap(len), Some(3));
    assert_eq!(neg(4).wrap(len), Some(0));
    assert_eq!(neg(8).wrap(len), None);
}

fn int_or_big<const N: usize>(
    i: Option<isize>,
    x: [isize; N],
    f: fn([BigInt; N]) -> BigInt,
) -> Num {
    i.map_or_else(|| Num::big_int(f(x.map(BigInt::from))), Num::Int)
}

// Do not use `BigInt::parse_bytes`, because it accepts arbitrary underscores between digits.
// https://github.com/rust-num/num-bigint/issues/340
fn bigint_from_str_radix(s: &str, radix: u32) -> Option<BigInt> {
    use num_bigint::Sign::{Minus, Plus};
    let f = |c, sign| s.strip_prefix(c).map(|s| (sign, s));
    let (sign, num) = f('-', Minus).or_else(|| f('+', Plus)).unwrap_or((Plus, s));
    biguint_from_str_radix(num, radix).map(|bu| BigInt::from_biguint(sign, bu))
}

fn biguint_from_str_radix(s: &str, radix: u32) -> Option<BigUint> {
    assert!((2..=36).contains(&radix));
    if s.is_empty() {
        return None;
    }

    // normalize all characters to plain digit values
    let digits = s.bytes().map(|b| {
        Some(match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'z' => b - b'a' + 10,
            b'A'..=b'Z' => b - b'A' + 10,
            _ => return None,
        })
        .filter(|d| *d < radix as u8)
    });
    BigUint::from_radix_be(&digits.collect::<Option<Vec<_>>>()?, radix)
}

// toolwing: arithmetic as jq does it, in doubles (jaq: exact integers of any
// size). Two integers stay an integer while it and they are exact as a
// double, which then gives the same; past 2^53, or for -0, it is the double.

/// The largest integer every smaller one is exact as a double: 2^53.
const EXACT: isize = 1 << 53;

fn exact(i: isize) -> bool {
    (-EXACT..=EXACT).contains(&i)
}

impl Num {
    fn arith(self, rhs: Self, int: fn(isize, isize) -> Option<isize>, float: fn(f64, f64) -> f64) -> Self {
        if let (Self::Int(x), Self::Int(y)) = (&self, &rhs) {
            if let Some(r) = int(*x, *y).filter(|r| exact(*x) && exact(*y) && exact(*r)) {
                return Self::Int(r);
            }
        }
        Self::Float(float(self.as_f64(), rhs.as_f64()))
    }

    /// jq's `dtoi`: a double as an integer, truncated and saturated.
    pub(crate) fn as_int_truncated(&self) -> isize {
        match self {
            Self::Int(i) => *i,
            // `as` truncates toward zero and saturates, as `dtoi` does.
            n => n.as_f64() as isize,
        }
    }
}

impl core::ops::Add for Num {
    type Output = Num;
    fn add(self, rhs: Self) -> Self::Output {
        self.arith(rhs, isize::checked_add, |x, y| x + y)
    }
}

impl core::ops::Sub for Num {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        self.arith(rhs, isize::checked_sub, |x, y| x - y)
    }
}

impl core::ops::Mul for Num {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self::Output {
        // 0 times a negative number is -0, which only a double is.
        let int = |x: isize, y: isize| x.checked_mul(y).filter(|r| *r != 0 || (x < 0) == (y < 0));
        self.arith(rhs, int, |x, y| x * y)
    }
}

impl core::ops::Div for Num {
    type Output = Self;
    /// A double, always; `Val` refuses a zero divisor, as jq does.
    fn div(self, rhs: Self) -> Self::Output {
        Self::Float(self.as_f64() / rhs.as_f64())
    }
}

impl core::ops::Rem for Num {
    type Output = Self;
    /// jq's: both truncated to integers; `Val` refuses NaN and a zero divisor.
    fn rem(self, rhs: Self) -> Self::Output {
        let (x, y) = (self.as_int_truncated(), rhs.as_int_truncated());
        // -1 divides everything, and `isize::MIN % -1` overflows.
        Self::Int(if y == -1 { 0 } else { x.checked_rem(y).unwrap_or(0) })
    }
}

impl core::ops::Neg for Num {
    type Output = Self;
    fn neg(self) -> Self::Output {
        match self {
            Self::Int(x) => int_or_big(x.checked_neg(), [x], |[x]| -x),
            Self::BigInt(x) => Self::big_int(-&*x),
            Self::Float(x) => Self::Float(-x),
            Self::Dec(n) => match n.strip_prefix('-') {
                Some(pos) => Self::Dec(pos.to_string().into()),
                None => Self::Dec(alloc::format!("-{n}").into()),
            },
        }
    }
}

impl Hash for Num {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            // hash machine-sized integers like floating-point numbers,
            // because they are also compared for equality that way
            Self::Int(i) => Self::Float(*i as f64).hash(state),
            // hash all non-finite floats, like NaN and infinity, to the same
            // note that `Val::hash` assumes that `Num::hash` always starts
            // with `state.write_u8(n)`, where `n < 2`
            Self::Float(f) => {
                state.write_u8(0);
                if f.is_finite() {
                    f.to_ne_bytes().hash(state);
                }
            }
            Self::Dec(d) => Self::from_dec_str(d).hash(state),
            Self::BigInt(i) => {
                let f = i.to_f64().unwrap();
                if f.is_finite() {
                    Self::Float(f).hash(state)
                } else {
                    state.write_u8(1);
                    i.hash(state)
                }
            }
        }
    }
}

#[test]
fn hash_nums() {
    use core::f64::consts::PI;
    use core::hash::BuildHasher;
    let h = |n| foldhash::fast::FixedState::with_seed(42).hash_one(n);

    assert_eq!(h(Num::Int(4096)), h(Num::big_int(4096.into())));
    assert_eq!(h(Num::Float(0.0)), h(Num::Int(0)));
    assert_eq!(h(Num::Float(3.0)), h(Num::big_int(3.into())));

    assert_ne!(h(Num::Float(0.2)), h(Num::Float(0.4)));
    assert_ne!(h(Num::Float(PI)), h(Num::big_int(3.into())));
    assert_ne!(h(Num::Float(0.2)), h(Num::Int(1)));
    assert_ne!(h(Num::Int(1)), h(Num::Int(2)));
}

impl PartialEq for Num {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Int(x), Self::Int(y)) => x == y,
            (Self::Int(i), Self::Float(f)) | (Self::Float(f), Self::Int(i)) => {
                f.is_finite() && float_eq(*i as f64, *f)
            }
            (Self::BigInt(x), Self::BigInt(y)) => x == y,
            (Self::Int(i), Self::BigInt(b)) | (Self::BigInt(b), Self::Int(i)) => {
                **b == BigInt::from(*i)
            }
            (Self::BigInt(i), Self::Float(f)) | (Self::Float(f), Self::BigInt(i)) => {
                f.is_finite() && float_eq(i.to_f64().unwrap(), *f)
            }
            (Self::Float(x), Self::Float(y)) => float_eq(*x, *y),
            (Self::Dec(x), Self::Dec(y)) if Rc::ptr_eq(x, y) => true,
            (Self::Dec(n), y) => &Self::from_dec_str(n) == y,
            (x, Self::Dec(n)) => x == &Self::from_dec_str(n),
        }
    }
}

impl Eq for Num {}

impl PartialOrd for Num {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Num {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Int(x), Self::Int(y)) => x.cmp(y),
            (Self::Int(x), Self::BigInt(y)) => BigInt::from(*x).cmp(y),
            (Self::Int(i), Self::Float(f)) => float_cmp(*i as f64, *f),
            (Self::BigInt(x), Self::Int(y)) => (**x).cmp(&BigInt::from(*y)),
            (Self::BigInt(x), Self::BigInt(y)) => x.cmp(y),
            // BigInt::to_f64 always yields Some, large values become f64::INFINITY
            (Self::BigInt(x), Self::Float(y)) => float_cmp(x.to_f64().unwrap(), *y),
            (Self::Float(f), Self::Int(i)) => float_cmp(*f, *i as f64),
            (Self::Float(x), Self::BigInt(y)) => float_cmp(*x, y.to_f64().unwrap()),
            (Self::Float(x), Self::Float(y)) => float_cmp(*x, *y),
            (Self::Dec(x), Self::Dec(y)) if Rc::ptr_eq(x, y) => Ordering::Equal,
            (Self::Dec(n), y) => Self::from_dec_str(n).cmp(y),
            (x, Self::Dec(n)) => x.cmp(&Self::from_dec_str(n)),
        }
    }
}

fn float_eq(left: f64, right: f64) -> bool {
    float_cmp(left, right) == Ordering::Equal
}

fn float_cmp(left: f64, right: f64) -> Ordering {
    if left == 0. && right == 0. {
        // consider negative and positive 0 as equal
        Ordering::Equal
    } else if left.is_nan() {
        // there are more than 50 shades of NaN, and which of these
        // you strike when you perform a calculation is not deterministic (!),
        // therefore `total_cmp` may yield different results for the same calculation
        // so we bite the bullet and handle this like in jq
        Ordering::Less
    } else if right.is_nan() {
        Ordering::Greater
    } else {
        f64::total_cmp(&left, &right)
    }
}

impl fmt::Display for Num {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Int(i) => write!(f, "{i}"),
            Self::BigInt(i) => write!(f, "{i}"),
            // toolwing: as jq writes numbers (jaq: Rust's shortest form, and
            // a literal as it was written)
            Self::Float(x) => f.write_str(&fmt_double(*x)),
            Self::Dec(n) => match canonical(n) {
                Some(c) => f.write_str(&c),
                None => write!(f, "{n}"),
            },
        }
    }
}

/// toolwing: a double as jq writes it (`jvp_dtoa_fmt`): its shortest digits
/// that read back the same, plain from 1e-4 up to 15 places past them, else
/// with an exponent of at least two digits (`1e-05`, `1e+17`). NaN is
/// `null` and the infinities the largest doubles, as JSON has neither.
pub(crate) fn fmt_double(x: f64) -> String {
    use alloc::format;
    if x.is_nan() {
        return "null".into();
    }
    let x = x.clamp(-f64::MAX, f64::MAX);
    // Rust's `{:e}` gives the shortest digits, as dtoa's mode 0 does.
    let sci = format!("{:e}", x.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let decpt = exp.parse::<i32>().unwrap_or(0) + 1;
    let n = digits.len() as i32;
    let mut out = String::new();
    if x.is_sign_negative() {
        out.push('-');
    }
    if decpt <= -4 || decpt > n + 15 {
        out.push_str(&digits[..1]);
        if n > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        let e = decpt - 1;
        out.push_str(&format!("e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs()));
    } else if decpt <= 0 {
        out.push_str("0.");
        out.extend(core::iter::repeat_n('0', (-decpt) as usize));
        out.push_str(&digits);
    } else if n > decpt {
        out.push_str(&digits[..decpt as usize]);
        out.push('.');
        out.push_str(&digits[decpt as usize..]);
    } else {
        out.push_str(&digits);
        out.extend(core::iter::repeat_n('0', (decpt - n) as usize));
    }
    out
}

/// toolwing: a number literal as jq writes it, decNumber's canonical form:
/// the digits as given, plain when the exponent is not positive and the
/// number not below 1e-6, else one digit before the point and `E+n`/`E-n`
/// (`1e3` is `1E+3`, `1.5e1` is `15`, `1.0` stays `1.0`). None if `s` is
/// not a JSON number.
pub(crate) fn canonical(s: &str) -> Option<String> {
    use alloc::format;
    let (neg, s) = s.strip_prefix('-').map_or((false, s), |s| (true, s));
    let (mantissa, exp) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], s[i + 1..].parse::<i64>().ok()?),
        None => (s, 0),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let all_digits = |d: &str| d.bytes().all(|b| b.is_ascii_digit());
    if int.is_empty() || !all_digits(int) || !all_digits(frac) {
        return None;
    }
    let exp = exp - frac.len() as i64;
    let coef = format!("{int}{frac}");
    let coef = match coef.trim_start_matches('0') {
        "" => "0",
        c => c,
    };
    let len = coef.len() as i64;
    let adjusted = exp + len - 1;
    let mut out = String::from(if neg { "-" } else { "" });
    if exp <= 0 && adjusted >= -6 {
        if exp == 0 {
            out.push_str(coef);
        } else if len > -exp {
            let point = (len + exp) as usize;
            out.push_str(&format!("{}.{}", &coef[..point], &coef[point..]));
        } else {
            out.push_str("0.");
            out.extend(core::iter::repeat_n('0', (-exp - len) as usize));
            out.push_str(coef);
        }
    } else {
        out.push_str(&coef[..1]);
        if len > 1 {
            out.push('.');
            out.push_str(&coef[1..]);
        }
        let sign = if adjusted < 0 { '-' } else { '+' };
        out.push_str(&format!("E{sign}{}", adjusted.abs()));
    }
    Some(out)
}

#[test]
fn numbers_as_jq_writes_them() {
    // Each as jq 1.8.2 writes it.
    for (x, jq) in [
        (2.0, "2"),
        (0.1 + 0.2, "0.30000000000000004"),
        (1e-5, "1e-05"),
        (0.0001, "0.0001"),
        (1e17, "1e+17"),
        (1e20, "1e+20"),
        (123456789012000000.0, "123456789012000000"),
        (12345678901234567890.0, "12345678901234567000"),
        (-0.0, "-0"),
        (0.0, "0"),
        (5e-324, "5e-324"),
        (f64::INFINITY, "1.7976931348623157e+308"),
        (f64::NEG_INFINITY, "-1.7976931348623157e+308"),
        (f64::NAN, "null"),
        (1.0 / 3.0, "0.3333333333333333"),
    ] {
        assert_eq!(fmt_double(x), jq, "{x}");
    }
    for (lit, jq) in [
        ("1e3", "1E+3"),
        ("1.0e3", "1.0E+3"),
        ("1.5e1", "15"),
        ("1E+2", "1E+2"),
        ("1e-7", "1E-7"),
        ("2.5e-300", "2.5E-300"),
        ("1.0", "1.0"),
        ("1.50", "1.50"),
        ("0.1", "0.1"),
        ("-0", "-0"),
        ("0.000", "0.000"),
        ("0.000001", "0.000001"),
        ("0.0000001", "1E-7"),
        ("3.14159265358979", "3.14159265358979"),
    ] {
        assert_eq!(canonical(lit).as_deref(), Some(jq), "{lit}");
    }
}
