//! `color`: a CSS color (CSS Color 4: hex, the named colors, `rgb()`,
//! `hsl()`, `hwb()`, `lab()`, `lch()`, `oklab()`, `oklch()` and `color()` in
//! its predefined spaces, `none`, `calc()`) read as browsers read it, its
//! computed value written as they write it (web-platform-tests' css-color
//! cases), and the color in every other form: hex, rgb, hsl, hwb, lab, lch,
//! oklab, oklch, mapped into sRGB as CSS Color 4 maps it (§13.2), with its
//! WCAG 2.1 contrast against white, black or another color. Conversions are
//! the spec's own matrices, checked against Color.js, the spec editors'
//! library.
//!
//! Not read: relative colors (`rgb(from …)`), `color-mix()`, `currentcolor`
//! and system colors, which need a page around them, nor lengths in
//! `calc()`.

use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, Field, InputKind, Meta, Output, Param, ParamKind, Reference,
    Score, Tool, Value, Values, en,
};

// ---- Tokens ---------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Function(String),
    Number(f64),
    Percent(f64),
    Dimension(f64, String),
    Hash(String),
    Comma,
    Slash,
    Delim(char),
    Open,
    Close,
    Space,
}

fn tokenize(s: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out: Vec<Tok> = Vec::new();
    let is_name_start = |ch: char| ch.is_ascii_alphabetic() || ch == '_' || !ch.is_ascii();
    let is_name = |ch: char| is_name_start(ch) || ch.is_ascii_digit() || ch == '-';
    let valid_escape =
        |i: usize, c: &[char]| c.get(i) == Some(&'\\') && c.get(i + 1).is_some_and(|&n| n != '\n');
    fn escape(c: &[char], i: &mut usize) -> char {
        // After the backslash.
        let mut hex = String::new();
        while hex.len() < 6 && c.get(*i).is_some_and(|ch| ch.is_ascii_hexdigit()) {
            hex.push(c[*i]);
            *i += 1;
        }
        if hex.is_empty() {
            let ch = c[*i];
            *i += 1;
            return ch;
        }
        if c.get(*i).is_some_and(|ch| ch.is_whitespace()) {
            *i += 1;
        }
        let n = u32::from_str_radix(&hex, 16).unwrap_or(0xFFFD);
        if n == 0 || (0xD800..=0xDFFF).contains(&n) || n > 0x10FFFF {
            '\u{FFFD}'
        } else {
            char::from_u32(n).unwrap_or('\u{FFFD}')
        }
    }
    let name = |c: &[char], i: &mut usize| -> String {
        let mut n = String::new();
        loop {
            if *i < c.len() && is_name(c[*i]) {
                n.push(c[*i]);
                *i += 1;
            } else if valid_escape(*i, c) {
                *i += 1;
                n.push(escape(c, i));
            } else {
                return n;
            }
        }
    };
    let starts_ident = |i: usize, c: &[char]| -> bool {
        match c.get(i) {
            Some('-') => {
                c.get(i + 1).is_some_and(|&n| is_name_start(n) || n == '-')
                    || valid_escape(i + 1, c)
            }
            Some(&ch) if is_name_start(ch) => true,
            Some('\\') => valid_escape(i, c),
            _ => false,
        }
    };
    let starts_number = |i: usize, c: &[char]| -> bool {
        match c.get(i) {
            Some('+' | '-') => {
                c.get(i + 1).is_some_and(char::is_ascii_digit)
                    || (c.get(i + 1) == Some(&'.')
                        && c.get(i + 2).is_some_and(char::is_ascii_digit))
            }
            Some('.') => c.get(i + 1).is_some_and(char::is_ascii_digit),
            Some(ch) => ch.is_ascii_digit(),
            None => false,
        }
    };
    while i < c.len() {
        let ch = c[i];
        if c[i..].starts_with(&['/', '*']) {
            match (i + 2..c.len().saturating_sub(1)).find(|&j| c[j] == '*' && c[j + 1] == '/') {
                Some(j) => i = j + 2,
                None => i = c.len(),
            }
            continue;
        }
        if ch.is_whitespace() {
            while i < c.len() && c[i].is_whitespace() {
                i += 1;
            }
            out.push(Tok::Space);
            continue;
        }
        if starts_number(i, &c) {
            let start = i;
            if matches!(c[i], '+' | '-') {
                i += 1;
            }
            while i < c.len() && c[i].is_ascii_digit() {
                i += 1;
            }
            if c.get(i) == Some(&'.') && c.get(i + 1).is_some_and(char::is_ascii_digit) {
                i += 1;
                while i < c.len() && c[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if matches!(c.get(i), Some('e' | 'E'))
                && (c.get(i + 1).is_some_and(char::is_ascii_digit)
                    || (matches!(c.get(i + 1), Some('+' | '-'))
                        && c.get(i + 2).is_some_and(char::is_ascii_digit)))
            {
                i += 2;
                while i < c.len() && c[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text: String = c[start..i].iter().collect();
            let v: f64 = text
                .parse()
                .map_err(|_| format!("{text} is not a number"))?;
            if c.get(i) == Some(&'%') {
                i += 1;
                out.push(Tok::Percent(v));
            } else if starts_ident(i, &c) {
                let unit = name(&c, &mut i);
                out.push(Tok::Dimension(v, unit.to_ascii_lowercase()));
            } else {
                out.push(Tok::Number(v));
            }
            continue;
        }
        if starts_ident(i, &c) {
            let n = name(&c, &mut i);
            if c.get(i) == Some(&'(') {
                i += 1;
                out.push(Tok::Function(n.to_ascii_lowercase()));
            } else {
                out.push(Tok::Ident(n.to_ascii_lowercase()));
            }
            continue;
        }
        i += 1;
        match ch {
            '#' => {
                let n = name(&c, &mut i);
                out.push(Tok::Hash(n));
            }
            ',' => out.push(Tok::Comma),
            '/' => out.push(Tok::Slash),
            '(' => out.push(Tok::Open),
            ')' => out.push(Tok::Close),
            ch => out.push(Tok::Delim(ch)),
        }
    }
    Ok(out)
}

// ---- calc() and the values in a color -------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Number,
    Percent,
    Angle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Num {
    v: f64,
    kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Arg {
    Num(Num),
    None,
}

struct P {
    t: Vec<Tok>,
    i: usize,
}

type R<T> = Result<T, String>;

fn angle(v: f64, unit: &str) -> Option<f64> {
    Some(match unit {
        "deg" => v,
        "grad" => v * 0.9,
        "rad" => v.to_degrees(),
        "turn" => v * 360.0,
        _ => return None,
    })
}

impl P {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }

    fn ws(&mut self) -> bool {
        let mut any = false;
        while self.peek() == Some(&Tok::Space) {
            self.i += 1;
            any = true;
        }
        any
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.t.get(self.i).cloned();
        self.i += 1;
        t
    }

    /// A number, percentage, angle, `none` or math function at this place.
    fn arg(&mut self) -> R<Arg> {
        self.ws();
        match self.next() {
            Some(Tok::Number(v)) => Ok(Arg::Num(Num {
                v,
                kind: Kind::Number,
            })),
            Some(Tok::Percent(v)) => Ok(Arg::Num(Num {
                v,
                kind: Kind::Percent,
            })),
            Some(Tok::Dimension(v, u)) => angle(v, &u)
                .map(|v| {
                    Arg::Num(Num {
                        v,
                        kind: Kind::Angle,
                    })
                })
                .ok_or_else(|| format!("{u} is not a unit a color takes")),
            Some(Tok::Ident(n)) if n == "none" => Ok(Arg::None),
            Some(Tok::Function(f)) => self.math(&f).map(Arg::Num),
            Some(t) => Err(format!("{t:?} is not a value a color takes")),
            None => Err("a color's value is missing".into()),
        }
    }

    /// A math function after its name: calc(), min(), max(), clamp(),
    /// abs(), sign().
    fn math(&mut self, f: &str) -> R<Num> {
        let r = match f {
            "calc" => self.sum()?,
            "min" | "max" => {
                let mut vals = vec![self.sum()?];
                while self.peek() == Some(&Tok::Comma) {
                    self.i += 1;
                    vals.push(self.sum()?);
                }
                let kind = vals[0].kind;
                if vals.iter().any(|v| v.kind != kind) {
                    return Err(format!("{f}() mixes kinds of value"));
                }
                let pick = if f == "min" { f64::min } else { f64::max };
                let v = vals.iter().map(|n| n.v).reduce(|a, b| {
                    if a.is_nan() || b.is_nan() {
                        f64::NAN
                    } else {
                        pick(a, b)
                    }
                });
                Num {
                    v: v.unwrap_or(f64::NAN),
                    kind,
                }
            }
            "clamp" => {
                let lo = self.sum()?;
                self.expect(&Tok::Comma)?;
                let v = self.sum()?;
                self.expect(&Tok::Comma)?;
                let hi = self.sum()?;
                if lo.kind != v.kind || hi.kind != v.kind {
                    return Err("clamp() mixes kinds of value".into());
                }
                Num {
                    v: v.v.min(hi.v).max(lo.v),
                    kind: v.kind,
                }
            }
            "abs" => {
                let v = self.sum()?;
                Num {
                    v: v.v.abs(),
                    kind: v.kind,
                }
            }
            "sign" => {
                let v = self.sum()?;
                let s = if v.v.is_nan() {
                    f64::NAN
                } else if v.v == 0.0 {
                    v.v
                } else {
                    v.v.signum()
                };
                Num {
                    v: s,
                    kind: Kind::Number,
                }
            }
            _ => return Err(format!("{f}() is not read in a color")),
        };
        self.ws();
        self.expect(&Tok::Close)?;
        Ok(r)
    }

    fn expect(&mut self, t: &Tok) -> R<()> {
        self.ws();
        if self.next().as_ref() == Some(t) {
            Ok(())
        } else {
            Err(format!("{t:?} expected"))
        }
    }

    fn sum(&mut self) -> R<Num> {
        let mut acc = self.product()?;
        loop {
            let save = self.i;
            let spaced = self.ws();
            match self.peek() {
                Some(Tok::Delim(op @ ('+' | '-'))) if spaced => {
                    let op = *op;
                    self.i += 1;
                    if !self.ws() {
                        return Err(format!("'{op}' needs space after it in calc()"));
                    }
                    let rhs = self.product()?;
                    if rhs.kind != acc.kind {
                        return Err("calc() adds different kinds of value".into());
                    }
                    acc.v = if op == '+' {
                        acc.v + rhs.v
                    } else {
                        acc.v - rhs.v
                    };
                }
                _ => {
                    self.i = save;
                    return Ok(acc);
                }
            }
        }
    }

    fn product(&mut self) -> R<Num> {
        let mut acc = self.term()?;
        loop {
            let save = self.i;
            self.ws();
            match self.peek() {
                Some(Tok::Delim('*')) => {
                    self.i += 1;
                    let rhs = self.term()?;
                    acc = match (acc.kind, rhs.kind) {
                        (Kind::Number, k) => Num {
                            v: acc.v * rhs.v,
                            kind: k,
                        },
                        (k, Kind::Number) => Num {
                            v: acc.v * rhs.v,
                            kind: k,
                        },
                        _ => return Err("calc() multiplies two values with units".into()),
                    };
                }
                Some(Tok::Slash) => {
                    self.i += 1;
                    let rhs = self.term()?;
                    if rhs.kind != Kind::Number {
                        return Err("calc() divides by a value with a unit".into());
                    }
                    acc.v /= rhs.v;
                }
                _ => {
                    self.i = save;
                    return Ok(acc);
                }
            }
        }
    }

    fn term(&mut self) -> R<Num> {
        self.ws();
        match self.next() {
            Some(Tok::Number(v)) => Ok(Num {
                v,
                kind: Kind::Number,
            }),
            Some(Tok::Percent(v)) => Ok(Num {
                v,
                kind: Kind::Percent,
            }),
            Some(Tok::Dimension(v, u)) => angle(v, &u)
                .map(|v| Num {
                    v,
                    kind: Kind::Angle,
                })
                .ok_or_else(|| format!("{u} is not read in calc() here")),
            Some(Tok::Open) => {
                let v = self.sum()?;
                self.expect(&Tok::Close)?;
                Ok(v)
            }
            Some(Tok::Function(f)) => self.math(&f),
            Some(Tok::Ident(n)) => {
                let v = match n.as_str() {
                    "e" => std::f64::consts::E,
                    "pi" => std::f64::consts::PI,
                    "infinity" => f64::INFINITY,
                    "-infinity" => f64::NEG_INFINITY,
                    "nan" => f64::NAN,
                    _ => return Err(format!("{n} is not a constant calc() knows")),
                };
                Ok(Num {
                    v,
                    kind: Kind::Number,
                })
            }
            Some(t) => Err(format!("{t:?} is not read in calc()")),
            None => Err("calc() is not closed".into()),
        }
    }
}

// ---- Colors ---------------------------------------------------------------------

/// A color space a color is held in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    /// rgb(), hex, named: 0 to 255.
    Rgb,
    Hsl,
    Hwb,
    Lab,
    Lch,
    Oklab,
    Oklch,
    Srgb,
    SrgbLinear,
    DisplayP3,
    DisplayP3Linear,
    A98Rgb,
    ProphotoRgb,
    Rec2020,
    XyzD50,
    XyzD65,
}

impl Space {
    fn name(self) -> &'static str {
        match self {
            Space::Rgb => "rgb",
            Space::Hsl => "hsl",
            Space::Hwb => "hwb",
            Space::Lab => "lab",
            Space::Lch => "lch",
            Space::Oklab => "oklab",
            Space::Oklch => "oklch",
            Space::Srgb => "srgb",
            Space::SrgbLinear => "srgb-linear",
            Space::DisplayP3 => "display-p3",
            Space::DisplayP3Linear => "display-p3-linear",
            Space::A98Rgb => "a98-rgb",
            Space::ProphotoRgb => "prophoto-rgb",
            Space::Rec2020 => "rec2020",
            Space::XyzD50 => "xyz-d50",
            Space::XyzD65 => "xyz-d65",
        }
    }

    fn predefined(name: &str) -> Option<Space> {
        Some(match name {
            "srgb" => Space::Srgb,
            "srgb-linear" => Space::SrgbLinear,
            "display-p3" => Space::DisplayP3,
            "display-p3-linear" => Space::DisplayP3Linear,
            "a98-rgb" => Space::A98Rgb,
            "prophoto-rgb" => Space::ProphotoRgb,
            "rec2020" => Space::Rec2020,
            "xyz" | "xyz-d65" => Space::XyzD65,
            "xyz-d50" => Space::XyzD50,
            _ => return None,
        })
    }
}

/// A color as CSS computes it: its space, three components and alpha,
/// `None` where the color says `none`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub space: Space,
    pub c: [Option<f64>; 3],
    pub alpha: Option<f64>,
}

const NAMED: &[(&str, u32)] = &[
    ("aliceblue", 0xf0f8ff),
    ("antiquewhite", 0xfaebd7),
    ("aqua", 0x00ffff),
    ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("bisque", 0xffe4c4),
    ("black", 0x000000),
    ("blanchedalmond", 0xffebcd),
    ("blue", 0x0000ff),
    ("blueviolet", 0x8a2be2),
    ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887),
    ("cadetblue", 0x5f9ea0),
    ("chartreuse", 0x7fff00),
    ("chocolate", 0xd2691e),
    ("coral", 0xff7f50),
    ("cornflowerblue", 0x6495ed),
    ("cornsilk", 0xfff8dc),
    ("crimson", 0xdc143c),
    ("cyan", 0x00ffff),
    ("darkblue", 0x00008b),
    ("darkcyan", 0x008b8b),
    ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xa9a9a9),
    ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b),
    ("darkolivegreen", 0x556b2f),
    ("darkorange", 0xff8c00),
    ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000),
    ("darksalmon", 0xe9967a),
    ("darkseagreen", 0x8fbc8f),
    ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f),
    ("darkslategrey", 0x2f4f4f),
    ("darkturquoise", 0x00ced1),
    ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493),
    ("deepskyblue", 0x00bfff),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff),
    ("firebrick", 0xb22222),
    ("floralwhite", 0xfffaf0),
    ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff),
    ("gainsboro", 0xdcdcdc),
    ("ghostwhite", 0xf8f8ff),
    ("gold", 0xffd700),
    ("goldenrod", 0xdaa520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xadff2f),
    ("grey", 0x808080),
    ("honeydew", 0xf0fff0),
    ("hotpink", 0xff69b4),
    ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082),
    ("ivory", 0xfffff0),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5),
    ("lawngreen", 0x7cfc00),
    ("lemonchiffon", 0xfffacd),
    ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080),
    ("lightcyan", 0xe0ffff),
    ("lightgoldenrodyellow", 0xfafad2),
    ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90),
    ("lightgrey", 0xd3d3d3),
    ("lightpink", 0xffb6c1),
    ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa),
    ("lightskyblue", 0x87cefa),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de),
    ("lightyellow", 0xffffe0),
    ("lime", 0x00ff00),
    ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6),
    ("magenta", 0xff00ff),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd),
    ("mediumorchid", 0xba55d3),
    ("mediumpurple", 0x9370db),
    ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee),
    ("mediumspringgreen", 0x00fa9a),
    ("mediumturquoise", 0x48d1cc),
    ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xf5fffa),
    ("mistyrose", 0xffe4e1),
    ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead),
    ("navy", 0x000080),
    ("oldlace", 0xfdf5e6),
    ("olive", 0x808000),
    ("olivedrab", 0x6b8e23),
    ("orange", 0xffa500),
    ("orangered", 0xff4500),
    ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa),
    ("palegreen", 0x98fb98),
    ("paleturquoise", 0xafeeee),
    ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5),
    ("peachpuff", 0xffdab9),
    ("peru", 0xcd853f),
    ("pink", 0xffc0cb),
    ("plum", 0xdda0dd),
    ("powderblue", 0xb0e0e6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xff0000),
    ("rosybrown", 0xbc8f8f),
    ("royalblue", 0x4169e1),
    ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072),
    ("sandybrown", 0xf4a460),
    ("seagreen", 0x2e8b57),
    ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d),
    ("silver", 0xc0c0c0),
    ("skyblue", 0x87ceeb),
    ("slateblue", 0x6a5acd),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xfffafa),
    ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4),
    ("tan", 0xd2b48c),
    ("teal", 0x008080),
    ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347),
    ("turquoise", 0x40e0d0),
    ("violet", 0xee82ee),
    ("wheat", 0xf5deb3),
    ("white", 0xffffff),
    ("whitesmoke", 0xf5f5f5),
    ("yellow", 0xffff00),
    ("yellowgreen", 0x9acd32),
];

fn rgb_hex(v: u32, alpha: f64) -> Color {
    let ch = |s: u32| Some(f64::from((v >> s) & 0xff));
    Color {
        space: Space::Rgb,
        c: [ch(16), ch(8), ch(0)],
        alpha: Some(alpha),
    }
}

/// NaN made 0, as CSS censors it.
fn censor(v: f64) -> f64 {
    if v.is_nan() { 0.0 } else { v }
}

/// A CSS color, as the CSS parser reads one.
pub fn parse(s: &str) -> Result<Color, String> {
    let toks = tokenize(s.trim())?;
    let mut p = P { t: toks, i: 0 };
    p.ws();
    let color = match p.next() {
        Some(Tok::Hash(h)) => {
            let n = h.len();
            if !matches!(n, 3 | 4 | 6 | 8) || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("#{h} is not a hex color"));
            }
            let digit = |i: usize| u32::from_str_radix(&h[i..i + 1], 16).unwrap_or(0);
            let pair = |i: usize| u32::from_str_radix(&h[i..i + 2], 16).unwrap_or(0);
            let (rgb, a) = match n {
                3 | 4 => {
                    let v = ((digit(0) * 17) << 16) | ((digit(1) * 17) << 8) | (digit(2) * 17);
                    (v, if n == 4 { digit(3) * 17 } else { 255 })
                }
                _ => (
                    pair(0) << 16 | pair(2) << 8 | pair(4),
                    if n == 8 { pair(6) } else { 255 },
                ),
            };
            rgb_hex(rgb, f64::from(a) / 255.0)
        }
        Some(Tok::Ident(n)) => {
            if n == "transparent" {
                Color {
                    space: Space::Rgb,
                    c: [Some(0.0); 3],
                    alpha: Some(0.0),
                }
            } else if let Some((_, v)) = NAMED.iter().find(|(k, _)| *k == n) {
                rgb_hex(*v, 1.0)
            } else if n == "currentcolor" {
                return Err("currentcolor needs an element to take its color from".into());
            } else {
                return Err(format!("{n} is not a color name"));
            }
        }
        Some(Tok::Function(f)) => function(&mut p, &f)?,
        _ => return Err("not a color".into()),
    };
    p.ws();
    if p.i < p.t.len() {
        return Err("more after the color".into());
    }
    Ok(color)
}

fn function(p: &mut P, f: &str) -> R<Color> {
    if matches!(p.t.get(p.i..p.i + 2), Some([Tok::Space, Tok::Ident(n)] | [Tok::Ident(n), _]) if n == "from")
        || matches!(p.peek(), Some(Tok::Ident(n)) if n == "from")
    {
        return Err("relative colors (from …) are not read".into());
    }
    let space = match f {
        "rgb" | "rgba" => Space::Rgb,
        "hsl" | "hsla" => Space::Hsl,
        "hwb" => Space::Hwb,
        "lab" => Space::Lab,
        "lch" => Space::Lch,
        "oklab" => Space::Oklab,
        "oklch" => Space::Oklch,
        "color" => {
            p.ws();
            match p.next() {
                Some(Tok::Ident(n)) => {
                    Space::predefined(&n).ok_or_else(|| format!("{n} is not a color space"))?
                }
                _ => return Err("color() names its space first".into()),
            }
        }
        _ => return Err(format!("{f}() is not a color function")),
    };
    let mut args = Vec::new();
    let mut commas = 0;
    let mut alpha = None;
    let mut slash = false;
    loop {
        p.ws();
        match p.peek() {
            Some(Tok::Close) => {
                p.i += 1;
                break;
            }
            Some(Tok::Comma) => {
                p.i += 1;
                commas += 1;
                if commas == 3 && args.len() == 3 {
                    alpha = Some(p.arg()?);
                }
                continue;
            }
            Some(Tok::Slash) => {
                p.i += 1;
                if slash || commas > 0 {
                    return Err("'/' in the wrong place".into());
                }
                slash = true;
                alpha = Some(p.arg()?);
                continue;
            }
            None => return Err(format!("{f}() is not closed")),
            _ => {}
        }
        if slash || (commas > 0 && commas != args.len()) {
            return Err(format!("{f}() takes its values another way"));
        }
        args.push(p.arg()?);
    }
    if args.len() != 3 {
        return Err(format!("{f}() takes 3 values and an alpha"));
    }
    let legacy = commas > 0;
    if legacy && (commas != 2 && commas != 3 || !matches!(space, Space::Rgb | Space::Hsl)) {
        return Err(format!("{f}() takes its values another way"));
    }
    if legacy && (args.iter().chain(alpha.iter()).any(|a| *a == Arg::None)) {
        return Err("none is not read with commas".into());
    }
    let kinds: Vec<Option<Kind>> = args
        .iter()
        .map(|a| match a {
            Arg::Num(n) => Some(n.kind),
            Arg::None => None,
        })
        .collect();
    let ok = |i: usize, allowed: &[Kind]| kinds[i].is_none_or(|k| allowed.contains(&k));
    use Kind::{Angle, Number, Percent};
    let valid = match space {
        Space::Rgb if legacy => {
            kinds.iter().all(|k| *k == Some(Number)) || kinds.iter().all(|k| *k == Some(Percent))
        }
        Space::Hsl if legacy => {
            ok(0, &[Number, Angle]) && kinds[1] == Some(Percent) && kinds[2] == Some(Percent)
        }
        Space::Hsl | Space::Hwb => {
            ok(0, &[Number, Angle]) && ok(1, &[Number, Percent]) && ok(2, &[Number, Percent])
        }
        Space::Lch | Space::Oklch => {
            ok(0, &[Number, Percent]) && ok(1, &[Number, Percent]) && ok(2, &[Number, Angle])
        }
        _ => (0..3).all(|i| ok(i, &[Number, Percent])),
    };
    if !valid {
        return Err(format!("{f}() takes other kinds of value"));
    }
    let alpha = match alpha {
        None => Some(1.0),
        Some(Arg::None) => None,
        Some(Arg::Num(n)) => match n.kind {
            Kind::Number => Some(censor(n.v).clamp(0.0, 1.0)),
            Kind::Percent => Some((censor(n.v) / 100.0).clamp(0.0, 1.0)),
            Kind::Angle => return Err("an alpha has no angle".into()),
        },
    };
    let val = |i: usize, pct: f64| -> Option<f64> {
        match args[i] {
            Arg::None => None,
            Arg::Num(n) => Some(match n.kind {
                Kind::Percent => n.v / 100.0 * pct,
                _ => n.v,
            }),
        }
    };
    let hue = |i: usize| {
        val(i, 1.0).map(|h| {
            let h = if h.is_finite() { h } else { 0.0 };
            // A hue just under 0 is just under 360, and 360 is 0.
            let h = h.rem_euclid(360.0);
            if h >= 360.0 { 0.0 } else { h }
        })
    };
    let c = match space {
        Space::Rgb => [val(0, 255.0), val(1, 255.0), val(2, 255.0)]
            .map(|v| v.map(|v| censor(v).clamp(0.0, 255.0))),
        Space::Hsl | Space::Hwb => [hue(0), val(1, 100.0).map(censor), val(2, 100.0).map(censor)],
        Space::Lab => [
            val(0, 100.0).map(|l| censor(l).clamp(0.0, 100.0)),
            val(1, 125.0).map(censor),
            val(2, 125.0).map(censor),
        ],
        Space::Lch => [
            val(0, 100.0).map(|l| censor(l).clamp(0.0, 100.0)),
            val(1, 150.0).map(|c| censor(c).max(0.0)),
            hue(2),
        ],
        Space::Oklab => [
            val(0, 1.0).map(|l| censor(l).clamp(0.0, 1.0)),
            val(1, 0.4).map(censor),
            val(2, 0.4).map(censor),
        ],
        Space::Oklch => [
            val(0, 1.0).map(|l| censor(l).clamp(0.0, 1.0)),
            val(1, 0.4).map(|c| censor(c).max(0.0)),
            hue(2),
        ],
        _ => [
            val(0, 1.0).map(censor),
            val(1, 1.0).map(censor),
            val(2, 1.0).map(censor),
        ],
    };
    // hsl() and hwb() with no `none` compute to sRGB, as rgb().
    let color = Color { space, c, alpha };
    match space {
        Space::Hsl | Space::Hwb if c.iter().all(Option::is_some) && alpha.is_some() => {
            let rgb = to_srgb(&color);
            Ok(Color {
                space: Space::Rgb,
                c: rgb.map(|v| Some((v * 255.0).clamp(0.0, 255.0))),
                alpha,
            })
        }
        _ => Ok(color),
    }
}

// ---- Serializing ------------------------------------------------------------------

/// A number as browsers serialize one: six significant digits at most.
fn num(v: f64) -> String {
    if v.is_infinite() {
        return if v > 0.0 {
            "calc(infinity)".into()
        } else {
            "calc(-infinity)".into()
        };
    }
    if v == 0.0 {
        return "0".into();
    }
    // Past six digits, in exponent form as browsers write it: 5.55556e+17.
    if v.abs() >= 1e6 {
        let e = format!("{v:.5e}");
        let (m, x) = e.split_once('e').unwrap_or((&e, "0"));
        let m = if m.contains('.') {
            m.trim_end_matches('0').trim_end_matches('.')
        } else {
            m
        };
        let x: i32 = x.parse().unwrap_or(0);
        return format!("{m}e{}{}", if x < 0 { '-' } else { '+' }, x.abs());
    }
    let digits = 6 - 1 - v.abs().log10().floor() as i32;
    if digits < 0 {
        // Six significant digits of a large number too.
        let scale = 10f64.powi(-digits);
        return format!("{:.0}", (v / scale).round() * scale);
    }
    let digits = digits.min(20) as usize;
    let s = format!("{v:.digits$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    };
    if s == "-0" { "0".into() } else { s }
}

fn opt(v: Option<f64>) -> String {
    v.map_or("none".into(), num)
}

impl Color {
    /// The computed value, as `getComputedStyle` gives it.
    pub fn computed(&self) -> String {
        let a = self.alpha;
        let tail = match a {
            Some(x) if x >= 1.0 => String::new(),
            Some(x) => format!(" / {}", num(x)),
            None => " / none".into(),
        };
        match self.space {
            Space::Rgb if self.c.iter().all(Option::is_some) && a.is_some() => {
                // To 8 bits, as browsers keep them, after the floating
                // point's noise (127.49999… from hwb() is 127.5).
                let ch: Vec<String> = self
                    .c
                    .iter()
                    .map(|v| {
                        format!(
                            "{}",
                            ((v.unwrap_or(0.0) * 1e6).round() / 1e6).round() as i64
                        )
                    })
                    .collect();
                match a {
                    Some(x) if x < 1.0 => {
                        format!("rgba({}, {}, {}, {})", ch[0], ch[1], ch[2], num(x))
                    }
                    _ => format!("rgb({}, {}, {})", ch[0], ch[1], ch[2]),
                }
            }
            Space::Rgb => {
                let c = self.c.map(|v| v.map(|v| v / 255.0));
                format!(
                    "color(srgb {} {} {}{tail})",
                    opt(c[0]),
                    opt(c[1]),
                    opt(c[2])
                )
            }
            Space::Hsl | Space::Hwb => {
                let pc = |v: Option<f64>| v.map_or("none".into(), |v| format!("{}%", num(v)));
                format!(
                    "{}({} {} {}{tail})",
                    self.space.name(),
                    opt(self.c[0]),
                    pc(self.c[1]),
                    pc(self.c[2])
                )
            }
            Space::Lab | Space::Lch | Space::Oklab | Space::Oklch => {
                format!(
                    "{}({} {} {}{tail})",
                    self.space.name(),
                    opt(self.c[0]),
                    opt(self.c[1]),
                    opt(self.c[2])
                )
            }
            s => format!(
                "color({} {} {} {}{tail})",
                s.name(),
                opt(self.c[0]),
                opt(self.c[1]),
                opt(self.c[2])
            ),
        }
    }
}

// ---- Converting -------------------------------------------------------------------

type M = [[f64; 3]; 3];

fn mul(m: &M, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

const SRGB_TO_XYZ: M = [
    [506752.0 / 1228815.0, 87881.0 / 245763.0, 12673.0 / 70218.0],
    [87098.0 / 409605.0, 175762.0 / 245763.0, 12673.0 / 175545.0],
    [7918.0 / 409605.0, 87881.0 / 737289.0, 1001167.0 / 1053270.0],
];
const XYZ_TO_SRGB: M = [
    [12831.0 / 3959.0, -329.0 / 214.0, -1974.0 / 3959.0],
    [
        -851781.0 / 878810.0,
        1648619.0 / 878810.0,
        36519.0 / 878810.0,
    ],
    [705.0 / 12673.0, -2585.0 / 12673.0, 705.0 / 667.0],
];
const P3_TO_XYZ: M = [
    [
        608311.0 / 1250200.0,
        189793.0 / 714400.0,
        198249.0 / 1000160.0,
    ],
    [
        35783.0 / 156275.0,
        247089.0 / 357200.0,
        198249.0 / 2500400.0,
    ],
    [0.0, 32229.0 / 714400.0, 5220557.0 / 5000800.0],
];
const A98_TO_XYZ: M = [
    [
        573536.0 / 994567.0,
        263643.0 / 1420810.0,
        187206.0 / 994567.0,
    ],
    [
        591459.0 / 1989134.0,
        6239551.0 / 9945670.0,
        374412.0 / 4972835.0,
    ],
    [
        53769.0 / 1989134.0,
        351524.0 / 4972835.0,
        4929758.0 / 4972835.0,
    ],
];
const PROPHOTO_TO_XYZ_D50: M = [
    [
        0.797_766_644_900_642_3,
        0.135_181_297_400_533_08,
        0.031_347_734_128_392_2,
    ],
    [
        0.288_074_828_819_401_3,
        0.711_835_234_241_873,
        0.000_089_936_938_725_64,
    ],
    [0.0, 0.0, 0.825_104_602_510_460_2],
];
const REC2020_TO_XYZ: M = [
    [
        63426534.0 / 99577255.0,
        20160776.0 / 139408157.0,
        47086771.0 / 278816314.0,
    ],
    [
        26158966.0 / 99577255.0,
        472592308.0 / 697040785.0,
        8267143.0 / 139408157.0,
    ],
    [0.0, 19567812.0 / 697040785.0, 295819943.0 / 278816314.0],
];
const D65_TO_D50: M = [
    [
        1.047_929_792_544_996_9,
        0.022_946_870_601_609_652,
        -0.050_192_266_289_205_24,
    ],
    [
        0.029_627_808_770_055_99,
        0.990_434_426_753_879_9,
        -0.017_073_799_063_418_826,
    ],
    [
        -0.009_243_040_646_204_504,
        0.015_055_191_490_298_152,
        0.751_874_281_428_137_1,
    ],
];
const D50_TO_D65: M = [
    [
        0.955_473_421_488_075,
        -0.023_098_454_948_764_71,
        0.063_259_243_200_570_72,
    ],
    [
        -0.028_369_709_333_863_7,
        1.009_995_398_081_304_1,
        0.021_041_441_191_917_323,
    ],
    [
        0.012_314_014_864_481_998,
        -0.020_507_649_298_898_964,
        1.330_365_926_242_124,
    ],
];
const XYZ_TO_LMS: M = [
    [
        0.819_022_437_996_703,
        0.361_906_260_052_890_4,
        -0.128_873_781_520_987_9,
    ],
    [
        0.032_983_653_932_388_5,
        0.929_286_861_586_343_4,
        0.036_144_666_350_642_4,
    ],
    [
        0.048_177_189_359_624_2,
        0.264_239_531_752_730_8,
        0.633_547_828_469_430_9,
    ],
];
const LMS_TO_OKLAB: M = [
    [
        0.210_454_268_309_314,
        0.793_617_774_702_305_4,
        -0.004_072_043_011_619_3,
    ],
    [
        1.977_998_532_431_168_4,
        -2.428_592_242_048_58,
        0.450_593_709_617_411,
    ],
    [
        0.025_904_042_465_547_8,
        0.782_771_712_457_529_6,
        -0.808_675_754_923_077_4,
    ],
];
const LMS_TO_XYZ: M = [
    [
        1.226_879_875_845_924_3,
        -0.557_814_994_460_217_1,
        0.281_391_045_665_964_7,
    ],
    [
        -0.040_575_745_214_800_8,
        1.112_286_803_280_317,
        -0.071_711_058_065_516_4,
    ],
    [
        -0.076_372_936_674_660_1,
        -0.421_493_332_402_243_2,
        1.586_924_019_836_781_6,
    ],
];
const OKLAB_TO_LMS: M = [
    [1.0, 0.396_337_777_376_174_9, 0.215_803_757_309_913_6],
    [1.0, -0.105_561_345_815_658_6, -0.063_854_172_825_813_3],
    [1.0, -0.089_484_177_529_811_9, -1.291_485_548_019_409_2],
];

fn srgb_lin(c: f64) -> f64 {
    let a = c.abs();
    if a <= 0.04045 {
        c / 12.92
    } else {
        c.signum() * ((a + 0.055) / 1.055).powf(2.4)
    }
}

fn srgb_gam(c: f64) -> f64 {
    let a = c.abs();
    if a > 0.003_130_8 {
        c.signum() * (1.055 * a.powf(1.0 / 2.4) - 0.055)
    } else {
        12.92 * c
    }
}

const D50: [f64; 3] = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];

fn lab_to_xyz50(l: f64, a: f64, b: f64) -> [f64; 3] {
    let k = 24389.0 / 27.0;
    let e = 216.0 / 24389.0;
    let f1 = (l + 16.0) / 116.0;
    let f0 = a / 500.0 + f1;
    let f2 = f1 - b / 200.0;
    let x = if f0.powi(3) > e {
        f0.powi(3)
    } else {
        (116.0 * f0 - 16.0) / k
    };
    let y = if l > k * e {
        ((l + 16.0) / 116.0).powi(3)
    } else {
        l / k
    };
    let z = if f2.powi(3) > e {
        f2.powi(3)
    } else {
        (116.0 * f2 - 16.0) / k
    };
    [x * D50[0], y * D50[1], z * D50[2]]
}

fn xyz50_to_lab(v: [f64; 3]) -> [f64; 3] {
    let k = 24389.0 / 27.0;
    let e = 216.0 / 24389.0;
    let f = [0, 1, 2].map(|i| {
        let x = v[i] / D50[i];
        if x > e {
            x.cbrt()
        } else {
            (k * x + 16.0) / 116.0
        }
    });
    [
        116.0 * f[1] - 16.0,
        500.0 * (f[0] - f[1]),
        200.0 * (f[1] - f[2]),
    ]
}

fn oklab_to_xyz(v: [f64; 3]) -> [f64; 3] {
    let lms = mul(&OKLAB_TO_LMS, v).map(|x| x.powi(3));
    mul(&LMS_TO_XYZ, lms)
}

fn xyz_to_oklab(v: [f64; 3]) -> [f64; 3] {
    let lms = mul(&XYZ_TO_LMS, v).map(f64::cbrt);
    mul(&LMS_TO_OKLAB, lms)
}

fn polar(l: f64, c: f64, h: f64) -> [f64; 3] {
    let r = h.to_radians();
    [l, c * r.cos(), c * r.sin()]
}

fn unpolar(v: [f64; 3]) -> [f64; 3] {
    let c = (v[1] * v[1] + v[2] * v[2]).sqrt();
    let h = v[2].atan2(v[1]).to_degrees().rem_euclid(360.0);
    [v[0], c, h]
}

fn hsl_to_srgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    let (s, l) = (s / 100.0, l / 100.0);
    let f = |n: f64| {
        let k = (n + h / 30.0).rem_euclid(12.0);
        let a = s * l.min(1.0 - l);
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [f(0.0), f(8.0), f(4.0)]
}

fn hwb_to_srgb(h: f64, w: f64, b: f64) -> [f64; 3] {
    let (w, b) = (w / 100.0, b / 100.0);
    if w + b >= 1.0 {
        let g = w / (w + b);
        return [g; 3];
    }
    hsl_to_srgb(h, 100.0, 50.0).map(|c| c * (1.0 - w - b) + w)
}

/// XYZ (D65) of a color, `none` as 0.
/// The color with no component past ±10⁹: an infinite one (calc(infinity)
/// chroma), which CSS keeps, or a huge one would overflow converting it;
/// mapping into sRGB brings it back in.
fn bounded(c: &Color) -> Color {
    Color {
        c: c.c
            .map(|x| x.map(|x| if x.is_nan() { 0.0 } else { x.clamp(-1e9, 1e9) })),
        ..*c
    }
}

pub fn to_xyz(c: &Color) -> [f64; 3] {
    let c = &bounded(c);
    let v = c.c.map(|x| x.unwrap_or(0.0));
    let lin_srgb = |v: [f64; 3]| mul(&SRGB_TO_XYZ, v.map(srgb_lin));
    match c.space {
        Space::Rgb => lin_srgb(v.map(|x| x / 255.0)),
        Space::Hsl => lin_srgb(hsl_to_srgb(v[0], v[1], v[2])),
        Space::Hwb => lin_srgb(hwb_to_srgb(v[0], v[1], v[2])),
        Space::Srgb => lin_srgb(v),
        Space::SrgbLinear => mul(&SRGB_TO_XYZ, v),
        Space::DisplayP3 => mul(&P3_TO_XYZ, v.map(srgb_lin)),
        Space::DisplayP3Linear => mul(&P3_TO_XYZ, v),
        Space::A98Rgb => mul(
            &A98_TO_XYZ,
            v.map(|x| x.signum() * x.abs().powf(563.0 / 256.0)),
        ),
        Space::ProphotoRgb => {
            let lin = v.map(|x| {
                if x.abs() <= 16.0 / 512.0 {
                    x / 16.0
                } else {
                    x.signum() * x.abs().powf(1.8)
                }
            });
            mul(&D50_TO_D65, mul(&PROPHOTO_TO_XYZ_D50, lin))
        }
        Space::Rec2020 => {
            let (al, be) = (1.099_296_826_809_44, 0.018_053_968_510_807);
            let lin = v.map(|x| {
                let a = x.abs();
                if a < be * 4.5 {
                    x / 4.5
                } else {
                    x.signum() * ((a + al - 1.0) / al).powf(1.0 / 0.45)
                }
            });
            mul(&REC2020_TO_XYZ, lin)
        }
        Space::XyzD65 => v,
        Space::XyzD50 => mul(&D50_TO_D65, v),
        Space::Lab => mul(&D50_TO_D65, lab_to_xyz50(v[0], v[1], v[2])),
        Space::Lch => {
            let p = polar(v[0], v[1], v[2]);
            mul(&D50_TO_D65, lab_to_xyz50(p[0], p[1], p[2]))
        }
        Space::Oklab => oklab_to_xyz(v),
        Space::Oklch => oklab_to_xyz(polar(v[0], v[1], v[2])),
    }
}

/// Gamma-encoded sRGB, 0 to 1, not mapped.
pub fn to_srgb(c: &Color) -> [f64; 3] {
    let c = &bounded(c);
    match c.space {
        Space::Rgb => c.c.map(|x| x.unwrap_or(0.0) / 255.0),
        Space::Hsl => {
            let v = c.c.map(|x| x.unwrap_or(0.0));
            hsl_to_srgb(v[0], v[1], v[2])
        }
        Space::Hwb => {
            let v = c.c.map(|x| x.unwrap_or(0.0));
            hwb_to_srgb(v[0], v[1], v[2])
        }
        Space::Srgb => c.c.map(|x| x.unwrap_or(0.0)),
        _ => mul(&XYZ_TO_SRGB, to_xyz(c)).map(srgb_gam),
    }
}

fn srgb_to_oklch(rgb: [f64; 3]) -> [f64; 3] {
    unpolar(xyz_to_oklab(mul(&SRGB_TO_XYZ, rgb.map(srgb_lin))))
}

fn oklch_to_srgb(v: [f64; 3]) -> [f64; 3] {
    mul(&XYZ_TO_SRGB, oklab_to_xyz(polar(v[0], v[1], v[2]))).map(srgb_gam)
}

fn in_gamut(rgb: [f64; 3]) -> bool {
    rgb.iter().all(|&x| (-0.000_075..=1.000_075).contains(&x))
}

fn delta_eok(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (pa, pb) = (polar(a[0], a[1], a[2]), polar(b[0], b[1], b[2]));
    ((pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2) + (pa[2] - pb[2]).powi(2)).sqrt()
}

/// sRGB, mapped into its gamut as CSS Color 4 maps a color (§13.2.2: the
/// chroma reduced in OKLCH until clipping it is not noticeable).
pub fn to_srgb_mapped(c: &Color) -> [f64; 3] {
    let rgb = to_srgb(c);
    if in_gamut(rgb) {
        return rgb.map(|x| x.clamp(0.0, 1.0));
    }
    let origin = unpolar(xyz_to_oklab(to_xyz(c)));
    if origin[0] >= 1.0 {
        return [1.0; 3];
    }
    if origin[0] <= 0.0 {
        return [0.0; 3];
    }
    const JND: f64 = 0.02;
    const EPS: f64 = 0.0001;
    let clip = |rgb: [f64; 3]| rgb.map(|x| x.clamp(0.0, 1.0));
    let mut current = origin;
    let mut clipped = clip(oklch_to_srgb(current));
    if delta_eok(srgb_to_oklch(clipped), current) < JND {
        return clipped;
    }
    let (mut min, mut max) = (0.0, origin[1]);
    let mut min_in_gamut = true;
    while max - min > EPS {
        let chroma = (min + max) / 2.0;
        current[1] = chroma;
        let rgb = oklch_to_srgb(current);
        if min_in_gamut && in_gamut(rgb) {
            min = chroma;
            continue;
        }
        clipped = clip(rgb);
        let e = delta_eok(srgb_to_oklch(clipped), current);
        if e < JND {
            if JND - e < EPS {
                return clipped;
            }
            min_in_gamut = false;
            min = chroma;
        } else {
            max = chroma;
        }
    }
    clipped
}

/// WCAG 2.1's relative luminance: Y of XYZ, never below 0.
pub fn luminance(c: &Color) -> f64 {
    to_xyz(c)[1].max(0.0)
}

/// WCAG 2.1's contrast ratio, 1 to 21.
pub fn contrast(a: &Color, b: &Color) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

fn srgb_to_hsl(rgb: [f64; 3]) -> [f64; 3] {
    let max = rgb.iter().copied().fold(f64::MIN, f64::max);
    let min = rgb.iter().copied().fold(f64::MAX, f64::min);
    let l = (max + min) / 2.0;
    let d = max - min;
    let mut h = f64::NAN;
    let mut s = 0.0;
    if d != 0.0 {
        s = if l == 0.0 || l == 1.0 {
            0.0
        } else {
            (max - l) / l.min(1.0 - l)
        };
        h = if max == rgb[0] {
            (rgb[1] - rgb[2]) / d + if rgb[1] < rgb[2] { 6.0 } else { 0.0 }
        } else if max == rgb[1] {
            (rgb[2] - rgb[0]) / d + 2.0
        } else {
            (rgb[0] - rgb[1]) / d + 4.0
        };
        h *= 60.0;
    }
    [h, s * 100.0, l * 100.0]
}

fn srgb_to_hwb(rgb: [f64; 3]) -> [f64; 3] {
    let h = srgb_to_hsl(rgb)[0];
    let w = rgb.iter().copied().fold(f64::MAX, f64::min);
    let b = 1.0 - rgb.iter().copied().fold(f64::MIN, f64::max);
    [h, w * 100.0, b * 100.0]
}

/// Every form of a color, for its fields.
#[derive(Clone, Debug)]
pub struct Forms {
    pub srgb: [f64; 3],
    pub in_srgb: bool,
    pub alpha: f64,
    pub hsl: [f64; 3],
    pub hwb: [f64; 3],
    pub lab: [f64; 3],
    pub lch: [f64; 3],
    pub oklab: [f64; 3],
    pub oklch: [f64; 3],
}

pub fn forms(c: &Color) -> Forms {
    let xyz = to_xyz(c);
    let srgb = to_srgb_mapped(c);
    let lab = xyz50_to_lab(mul(&D65_TO_D50, xyz));
    let oklab = xyz_to_oklab(xyz);
    Forms {
        srgb,
        in_srgb: in_gamut(to_srgb(c)),
        alpha: c.alpha.unwrap_or(1.0),
        hsl: srgb_to_hsl(srgb),
        hwb: srgb_to_hwb(srgb),
        lab,
        lch: unpolar(lab),
        oklab,
        oklch: unpolar(oklab),
    }
}

// ---- Writing forms ----------------------------------------------------------------

fn fixed(v: f64, places: usize) -> String {
    let v = if v.is_nan() { 0.0 } else { v };
    let s = format!("{v:.places$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    };
    if s == "-0" { "0".into() } else { s }
}

impl Forms {
    fn byte(&self, i: usize) -> u8 {
        (self.srgb[i] * 255.0).round().clamp(0.0, 255.0) as u8
    }

    fn alpha_tail(&self) -> String {
        if self.alpha < 1.0 {
            format!(" / {}", fixed(self.alpha, 3))
        } else {
            String::new()
        }
    }

    pub fn hex(&self) -> String {
        let mut s = format!(
            "#{:02x}{:02x}{:02x}",
            self.byte(0),
            self.byte(1),
            self.byte(2)
        );
        if self.alpha < 1.0 {
            s.push_str(&format!("{:02x}", (self.alpha * 255.0).round() as u8));
        }
        s
    }

    pub fn rgb(&self) -> String {
        format!(
            "rgb({} {} {}{})",
            self.byte(0),
            self.byte(1),
            self.byte(2),
            self.alpha_tail()
        )
    }

    pub fn hsl(&self) -> String {
        let [h, s, l] = self.hsl;
        format!(
            "hsl({} {}% {}%{})",
            fixed(h, 2),
            fixed(s, 2),
            fixed(l, 2),
            self.alpha_tail()
        )
    }

    pub fn hwb(&self) -> String {
        let [h, w, b] = self.hwb;
        format!(
            "hwb({} {}% {}%{})",
            fixed(h, 2),
            fixed(w, 2),
            fixed(b, 2),
            self.alpha_tail()
        )
    }

    pub fn lab(&self) -> String {
        let [l, a, b] = self.lab;
        format!(
            "lab({}% {} {}{})",
            fixed(l, 2),
            fixed(a, 2),
            fixed(b, 2),
            self.alpha_tail()
        )
    }

    pub fn lch(&self) -> String {
        let [l, c, h] = self.lch;
        let h = if c < 0.0015 { 0.0 } else { h };
        format!(
            "lch({}% {} {}{})",
            fixed(l, 2),
            fixed(c, 2),
            fixed(h, 2),
            self.alpha_tail()
        )
    }

    pub fn oklab(&self) -> String {
        let [l, a, b] = self.oklab;
        format!(
            "oklab({}% {} {}{})",
            fixed(l * 100.0, 2),
            fixed(a, 4),
            fixed(b, 4),
            self.alpha_tail()
        )
    }

    pub fn oklch(&self) -> String {
        let [l, c, h] = self.oklch;
        let h = if c < 0.000_004 { 0.0 } else { h };
        format!(
            "oklch({}% {} {}{})",
            fixed(l * 100.0, 2),
            fixed(c, 4),
            fixed(h, 2),
            self.alpha_tail()
        )
    }

    /// The CSS name of this exact sRGB color, if it has one.
    pub fn name(&self) -> Option<&'static str> {
        if self.alpha < 1.0 {
            return (self.alpha == 0.0 && self.srgb == [0.0; 3]).then_some("transparent");
        }
        let v =
            u32::from(self.byte(0)) << 16 | u32::from(self.byte(1)) << 8 | u32::from(self.byte(2));
        NAMED.iter().find(|(_, x)| *x == v).map(|(n, _)| *n)
    }
}

/// WCAG 2.1's levels a ratio passes, for normal text.
fn level(ratio: f64) -> &'static str {
    if ratio >= 7.0 {
        "AAA"
    } else if ratio >= 4.5 {
        "AA"
    } else if ratio >= 3.0 {
        "AA for large text"
    } else {
        "fails AA"
    }
}

// ---- The tool -------------------------------------------------------------------------

static META: Meta = Meta {
    id: "color",
    group: "color",
    action: "",
    action_flag: None,
    title: en!("Color"),
    summary: en!(
        "A CSS color in every form: hex, rgb, hsl, hwb, lab, lch, oklab, oklch, its contrast"
    ),
    category: Category::Web,
    keywords: &[
        "color", "colour", "css", "hex", "rgb", "hsl", "oklch", "contrast", "wcag", "lab", "p3",
    ],
    input: InputKind::Text { trim_newline: true },
    streams: false,
    reference: Some(Reference {
        name: "CSS Color 4",
        version: "2024",
        compat: Compat::Standard,
    }),
};

static PARAMS: &[Param] = &[
    Param {
        key: "against",
        label: en!("Against"),
        help: en!("Another color to give the WCAG contrast with; white and black when left out"),
        kind: ParamKind::Text,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "against",
            short: None,
        },
    },
    Param {
        key: "to",
        label: en!("Only"),
        help: en!("Write this one form alone"),
        kind: ParamKind::Choice {
            options: &[
                ("css", en!("The CSS computed value")),
                ("hex", en!("#rrggbb")),
                ("rgb", en!("rgb()")),
                ("hsl", en!("hsl()")),
                ("hwb", en!("hwb()")),
                ("lab", en!("lab()")),
                ("lch", en!("lch()")),
                ("oklab", en!("oklab()")),
                ("oklch", en!("oklch()")),
            ],
            flags: false,
            unset: Some(en!("All")),
        },
        default: Value::Text(String::new()),
        cli: Cli {
            long: "to",
            short: None,
        },
    },
];

pub struct Convert;

impl Tool for Convert {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let text = String::from_utf8_lossy(input);
        let color = parse(&text).map_err(|e| Error::input(format!("not a CSS color: {e}")))?;
        let f = forms(&color);
        let one = match values.text(PARAMS, "to").as_str() {
            "css" => Some(color.computed()),
            "hex" => Some(f.hex()),
            "rgb" => Some(f.rgb()),
            "hsl" => Some(f.hsl()),
            "hwb" => Some(f.hwb()),
            "lab" => Some(f.lab()),
            "lch" => Some(f.lch()),
            "oklab" => Some(f.oklab()),
            "oklch" => Some(f.oklch()),
            _ => None,
        };
        if let Some(one) = one {
            return Ok(Output::Text(one));
        }
        let mut out = vec![Field::new("css", "CSS value", color.computed())];
        if let Some(n) = f.name() {
            out.push(Field::new("name", "Name", n));
        }
        out.push(Field::new("hex", "Hex", f.hex()));
        out.push(Field::new("rgb", "RGB", f.rgb()));
        out.push(Field::new("hsl", "HSL", f.hsl()));
        out.push(Field::new("hwb", "HWB", f.hwb()));
        out.push(Field::new("lab", "Lab", f.lab()));
        out.push(Field::new("lch", "LCH", f.lch()));
        out.push(Field::new("oklab", "OKLab", f.oklab()));
        out.push(Field::new("oklch", "OKLCH", f.oklch()));
        out.push(Field::new(
            "in_srgb",
            "In sRGB",
            if f.in_srgb {
                "yes"
            } else {
                "no: hex, rgb, hsl and hwb are mapped into it"
            },
        ));
        let ratio = |r: f64| format!("{}:1, {}", fixed(r, 2), level(r));
        let against = values.text(PARAMS, "against");
        if against.trim().is_empty() {
            let white = rgb_hex(0xffffff, 1.0);
            let black = rgb_hex(0x000000, 1.0);
            out.push(Field::new(
                "contrast_white",
                "Contrast with white",
                ratio(contrast(&color, &white)),
            ));
            out.push(Field::new(
                "contrast_black",
                "Contrast with black",
                ratio(contrast(&color, &black)),
            ));
        } else {
            let other = parse(&against)
                .map_err(|e| Error::usage(format!("--against is not a CSS color: {e}")))?;
            out.push(Field::new(
                "contrast",
                "Contrast",
                ratio(contrast(&color, &other)),
            ));
        }
        Ok(Output::Fields(out))
    }

    fn detect(&self, sample: &[u8]) -> Option<Score> {
        let s = std::str::from_utf8(sample).ok()?.trim();
        if s.len() > 200 {
            return None;
        }
        let strong = s.starts_with('#') || s.contains('(');
        (strong && parse(s).is_ok()).then_some(Score(70))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Numbers in two serializations agree within WPT's fuzzy epsilon, and
    /// the rest is the same.
    fn fuzzy_eq(a: &str, b: &str) -> bool {
        let nums = |s: &str| -> (String, Vec<f64>) {
            let mut text = String::new();
            let mut v = Vec::new();
            let mut cur = String::new();
            for ch in s.chars().chain(std::iter::once(' ')) {
                if ch.is_ascii_digit()
                    || ch == '.'
                    || ch == 'e' && !cur.is_empty()
                    || (ch == '-' && cur.is_empty())
                {
                    cur.push(ch);
                } else {
                    if !cur.is_empty() {
                        match cur.parse::<f64>() {
                            Ok(n) => {
                                v.push(n);
                                text.push('#');
                            }
                            Err(_) => text.push_str(&cur),
                        }
                        cur.clear();
                    }
                    text.push(ch);
                }
            }
            (text, v)
        };
        let (ta, va) = nums(a);
        let (tb, vb) = nums(b);
        ta == tb && va.len() == vb.len() && va.iter().zip(&vb).all(|(x, y)| (x - y).abs() <= 0.01)
    }

    fn data(file: &str) -> Vec<serde_json::Value> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/color")
            .join(file);
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{l}: {e}")))
            .collect()
    }

    /// web-platform-tests' css-color computed values (testdata/color/SOURCE),
    /// each as a browser writes it, or one of the forms a test allows.
    #[test]
    fn computes_as_browsers_do() {
        let mut failures = Vec::new();
        let cases = data("wpt-computed.jsonl");
        for case in &cases {
            let input = case[0].as_str().unwrap_or_default();
            let wants: Vec<&str> = case[1]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            match parse(input) {
                Ok(c) => {
                    let got = c.computed();
                    if !wants.iter().any(|w| fuzzy_eq(&got, w)) {
                        failures.push(format!("{input}: {got}, want {wants:?}"));
                    }
                }
                Err(e) => failures.push(format!("{input}: refused ({e}), want {wants:?}")),
            }
        }
        for case in data("wpt-invalid.jsonl") {
            let input = case.as_str().unwrap_or_default();
            if let Ok(c) = parse(input) {
                failures.push(format!("{input}: read as {}, want refused", c.computed()));
            }
        }
        assert!(cases.len() > 4000, "{}", cases.len());
        assert!(
            failures.is_empty(),
            "{} differ:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// Color.js 0.5.2's conversions (testdata/color/SOURCE): every form,
    /// sRGB as CSS maps it, WCAG 2.1 contrast.
    #[test]
    fn converts_as_colorjs_does() {
        let mut failures = Vec::new();
        for case in data("colorjs.jsonl") {
            let input = case["input"].as_str().unwrap_or_default();
            let Ok(c) = parse(input) else {
                failures.push(format!("{input}: refused"));
                continue;
            };
            let f = forms(&c);
            let close = |name: &str, got: &[f64], tol: f64| {
                let want = case[name].as_array().cloned().unwrap_or_default();
                for (i, w) in want.iter().enumerate() {
                    let Some(w) = w.as_f64() else { continue };
                    // A hue where the chroma is all but none says little.
                    if (name.ends_with("ch") || name == "hsl" || name == "hwb")
                        && i == 2
                        && name != "hsl"
                        && name != "hwb"
                    {
                        let chroma = got[1];
                        if chroma < 0.0005 {
                            continue;
                        }
                    }
                    if (name == "hsl" || name == "hwb")
                        && i == 0
                        && (got[1] < 0.01 || f.srgb.iter().all(|x| (x - f.srgb[0]).abs() < 1e-9))
                    {
                        continue;
                    }
                    let mut d = (got[i] - w).abs();
                    if (name.ends_with("ch") && i == 2)
                        || ((name == "hsl" || name == "hwb") && i == 0)
                    {
                        d = d.min(360.0 - d);
                    }
                    if d.is_nan() || d > tol {
                        return Some(format!("{name}[{i}] {} want {w}", got[i]));
                    }
                }
                None
            };
            let checks = [
                close("srgb", &f.srgb, 0.002),
                close("hsl", &f.hsl, 0.3),
                close("hwb", &f.hwb, 0.3),
                close("lab", &f.lab, 0.001),
                close("lch", &f.lch, 0.001),
                close("oklab", &f.oklab, 0.00001),
                close("oklch", &f.oklch, 0.0001),
            ];
            for msg in checks.into_iter().flatten() {
                failures.push(format!("{input}: {msg}"));
            }
            if case["in_srgb"].as_bool() != Some(f.in_srgb) {
                failures.push(format!("{input}: in sRGB {}", f.in_srgb));
            }
            for (key, other) in [("contrast_white", 0xffffff), ("contrast_black", 0)] {
                let want = case[key].as_f64().unwrap_or_default();
                let got = contrast(&c, &rgb_hex(other, 1.0));
                if (got - want).abs() > 1e-6 {
                    failures.push(format!("{input}: {key} {got} want {want}"));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} differ:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    #[test]
    fn forms_read_as_people_write_them() {
        let f = forms(&parse("rebeccapurple").unwrap_or_else(|e| panic!("{e}")));
        assert_eq!(f.hex(), "#663399");
        assert_eq!(f.rgb(), "rgb(102 51 153)");
        assert_eq!(f.hsl(), "hsl(270 50% 40%)");
        assert_eq!(f.name(), Some("rebeccapurple"));
        let half = forms(&parse("#ff000080").unwrap_or_else(|e| panic!("{e}")));
        assert_eq!(half.hex(), "#ff000080");
        assert_eq!(half.rgb(), "rgb(255 0 0 / 0.502)");
        let white = rgb_hex(0xffffff, 1.0);
        let black = rgb_hex(0, 1.0);
        assert_eq!(fixed(contrast(&white, &black), 2), "21");
    }
}
