//! `qrencode`: QR codes as libqrencode 4.1.1 makes them, module for module:
//! its split of the input into numeric, alphanumeric, 8-bit and kanji
//! segments (split.c), its choice of version (qrinput.c) and of mask, by its
//! own penalty (mask.c). The symbol itself (function patterns, error
//! correction, placement) is ISO/IEC 18004's, by the `qrcode` crate. Written
//! out as qrencode's writers write it: ASCII, UTF-8 half blocks, ANSI, SVG,
//! EPS, XPM and PNG.

use qrcode::bits::Bits;
use qrcode::canvas::{Canvas, MaskPattern};
use qrcode::{Color, EcLevel, Version};
use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, InputKind, Meta, Output, Param, ParamKind, Reference, Tool,
    Value, Values, en,
};

// ---- Splitting the input ----------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Num,
    An,
    Eight,
    Kanji,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub mode: Mode,
    pub data: Vec<u8>,
}

/// QR's alphanumeric set: 0-9 A-Z space $ % * + - . / :
fn is_an(c: u8) -> bool {
    c.is_ascii_digit() || c.is_ascii_uppercase() || b" $%*+-./:".contains(&c)
}

fn length_bits(mode: Mode, version: u32) -> usize {
    let l = match version {
        0..=9 => 0,
        10..=26 => 1,
        _ => 2,
    };
    [[10, 12, 14], [9, 11, 13], [8, 16, 16], [8, 10, 12]][mode as usize][l]
}

fn bits_num(n: usize) -> usize {
    n / 3 * 10 + [0, 4, 7][n % 3]
}

fn bits_an(n: usize) -> usize {
    n / 2 * 11 + if n % 2 == 1 { 6 } else { 0 }
}

fn bits_8(n: usize) -> usize {
    n * 8
}

fn bits_kanji(n: usize) -> usize {
    n / 2 * 13
}

/// libqrencode's `Split_identifyMode` at `s[i]`.
fn identify(s: &[u8], i: usize, kanji: bool) -> Option<Mode> {
    let c = *s.get(i)?;
    if c == 0 {
        return None;
    }
    if c.is_ascii_digit() {
        return Some(Mode::Num);
    }
    if is_an(c) {
        return Some(Mode::An);
    }
    if kanji
        && let Some(&d) = s.get(i + 1)
        && d != 0
    {
        let w = u16::from(c) << 8 | u16::from(d);
        if (0x8140..=0x9ffc).contains(&w) || (0xe040..=0xebbf).contains(&w) {
            return Some(Mode::Kanji);
        }
    }
    Some(Mode::Eight)
}

struct Splitter<'a> {
    s: &'a [u8],
    version: u32,
    kanji: bool,
    out: Vec<Segment>,
}

impl Splitter<'_> {
    fn digit(&self, i: usize) -> bool {
        self.s.get(i).is_some_and(u8::is_ascii_digit)
    }

    fn an(&self, i: usize) -> bool {
        self.s.get(i).copied().is_some_and(is_an)
    }

    fn push(&mut self, mode: Mode, at: usize, run: usize) -> usize {
        self.out.push(Segment {
            mode,
            data: self.s[at..at + run].to_vec(),
        });
        run
    }

    fn eat_num(&mut self, at: usize) -> usize {
        let ln = length_bits(Mode::Num, self.version) as isize;
        let mut p = at;
        while self.digit(p) {
            p += 1;
        }
        let run = p - at;
        let mode = identify(self.s, p, self.kanji);
        if mode == Some(Mode::Eight) {
            let dif =
                bits_num(run) as isize + 4 + ln + bits_8(1) as isize - bits_8(run + 1) as isize;
            if dif > 0 {
                return self.eat_8(at);
            }
        }
        if mode == Some(Mode::An) {
            let dif =
                bits_num(run) as isize + 4 + ln + bits_an(1) as isize - bits_an(run + 1) as isize;
            if dif > 0 {
                return self.eat_an(at);
            }
        }
        self.push(Mode::Num, at, run)
    }

    fn eat_an(&mut self, at: usize) -> usize {
        let la = length_bits(Mode::An, self.version) as isize;
        let ln = length_bits(Mode::Num, self.version) as isize;
        let mut p = at;
        while self.an(p) {
            if self.digit(p) {
                let mut q = p;
                while self.digit(q) {
                    q += 1;
                }
                let dif = bits_an(p - at) as isize
                    + bits_num(q - p) as isize
                    + 4
                    + ln
                    + if self.an(q) { 4 + ln } else { 0 }
                    - bits_an(q - at) as isize;
                if dif < 0 {
                    break;
                }
                p = q;
            } else {
                p += 1;
            }
        }
        let run = p - at;
        if self.s.get(p).is_some_and(|&c| c != 0) && !self.an(p) {
            let dif =
                bits_an(run) as isize + 4 + la + bits_8(1) as isize - bits_8(run + 1) as isize;
            if dif > 0 {
                return self.eat_8(at);
            }
        }
        self.push(Mode::An, at, run)
    }

    fn eat_kanji(&mut self, at: usize) -> usize {
        let mut p = at;
        while identify(self.s, p, self.kanji) == Some(Mode::Kanji) {
            p += 2;
        }
        self.push(Mode::Kanji, at, p - at)
    }

    fn eat_8(&mut self, at: usize) -> usize {
        let la = length_bits(Mode::An, self.version) as isize;
        let ln = length_bits(Mode::Num, self.version) as isize;
        let l8 = length_bits(Mode::Eight, self.version) as isize;
        let mut p = at + 1;
        while self.s.get(p).is_some_and(|&c| c != 0) {
            let mode = identify(self.s, p, self.kanji);
            match mode {
                Some(Mode::Kanji) => break,
                Some(Mode::Num) => {
                    let mut q = p;
                    while self.digit(q) {
                        q += 1;
                    }
                    let swcost = if identify(self.s, q, self.kanji) == Some(Mode::Eight) {
                        4 + l8
                    } else {
                        0
                    };
                    let dif = bits_8(p - at) as isize + bits_num(q - p) as isize + 4 + ln + swcost
                        - bits_8(q - at) as isize;
                    if dif < 0 {
                        break;
                    }
                    p = q;
                }
                Some(Mode::An) => {
                    let mut q = p;
                    while self.an(q) {
                        q += 1;
                    }
                    let swcost = if identify(self.s, q, self.kanji) == Some(Mode::Eight) {
                        4 + l8
                    } else {
                        0
                    };
                    let dif = bits_8(p - at) as isize + bits_an(q - p) as isize + 4 + la + swcost
                        - bits_8(q - at) as isize;
                    if dif < 0 {
                        break;
                    }
                    p = q;
                }
                _ => p += 1,
            }
        }
        self.push(Mode::Eight, at, p - at)
    }
}

/// libqrencode's `Split_splitStringToQRinput`: `s` up to its first NUL, cut
/// into segments for the length indicators of `version` (0: as version 1).
pub fn split(s: &[u8], version: u32, kanji: bool, case_sensitive: bool) -> Vec<Segment> {
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    let mut text = s[..end].to_vec();
    if !case_sensitive {
        let mut i = 0;
        while i < text.len() {
            if identify(&text, i, kanji) == Some(Mode::Kanji) {
                i += 2;
            } else {
                text[i] = text[i].to_ascii_uppercase();
                i += 1;
            }
        }
    }
    let mut sp = Splitter {
        s: &text,
        version,
        kanji,
        out: Vec::new(),
    };
    let mut at = 0;
    while at < text.len() {
        let n = match identify(&text, at, kanji) {
            Some(Mode::Num) => sp.eat_num(at),
            Some(Mode::An) => sp.eat_an(at),
            Some(Mode::Kanji) if kanji => sp.eat_kanji(at),
            _ => sp.eat_8(at),
        };
        if n == 0 {
            break;
        }
        at += n;
    }
    sp.out
}

// ---- Versions -----------------------------------------------------------------

/// Data codewords of a version at a level.
fn data_words(version: u32, level: EcLevel) -> usize {
    Bits::new(Version::Normal(version as i16))
        .max_len(level)
        .unwrap_or(0)
        / 8
}

/// The smallest version that holds `bytes`; 41 when none does.
fn minimum_version(bytes: usize, level: EcLevel) -> u32 {
    (1..=40)
        .find(|&v| data_words(v, level) >= bytes)
        .unwrap_or(41)
}

/// Characters a segment of `mode` holds at `version`.
fn maximum_words(mode: Mode, version: u32) -> usize {
    let w = (1 << length_bits(mode, version)) - 1;
    if mode == Mode::Kanji { w * 2 } else { w }
}

fn estimate_bits(segments: &[Segment], version: u32) -> usize {
    let version = version.max(1);
    segments
        .iter()
        .map(|s| {
            let n = s.data.len();
            let bits = match s.mode {
                Mode::Num => bits_num(n),
                Mode::An => bits_an(n),
                Mode::Eight => bits_8(n),
                Mode::Kanji => bits_kanji(n),
            };
            let l = length_bits(s.mode, version);
            let m = 1usize << l;
            let count = if s.mode == Mode::Kanji { n / 2 } else { n };
            bits + count.div_ceil(m) * (4 + l)
        })
        .sum()
}

/// libqrencode's `QRinput_estimateVersion`, its off-by-one first step too.
fn estimate_version(segments: &[Segment], level: EcLevel) -> u32 {
    let mut version = 0;
    loop {
        let prev = version;
        let bits = estimate_bits(segments, prev);
        version = minimum_version(bits.div_ceil(8), level);
        if prev == 0 && version > 1 {
            version -= 1;
        }
        if version <= prev {
            return version;
        }
    }
}

/// The segments as bits for `version`, a segment longer than one may hold
/// cut in two, as `QRinput_encodeBitStream` does.
fn bitstream(segments: &[Segment], version: u32) -> Option<Bits> {
    let mut bits = Bits::new(Version::Normal(version as i16));
    for s in segments {
        let max = maximum_words(s.mode, version);
        for chunk in s.data.chunks(max.max(1)) {
            match s.mode {
                Mode::Num => bits.push_numeric_data(chunk).ok()?,
                Mode::An => bits.push_alphanumeric_data(chunk).ok()?,
                Mode::Eight => bits.push_byte_data(chunk).ok()?,
                Mode::Kanji => bits.push_kanji_data(chunk).ok()?,
            }
        }
    }
    Some(bits)
}

// ---- Masks ----------------------------------------------------------------------

/// libqrencode's `Mask_calcN1N3` over one row's or column's runs, the first
/// a light one (-1 when the line starts dark).
fn n1n3(runs: &[i32]) -> i32 {
    let mut demerit = 0;
    let len = runs.len();
    for i in 0..len {
        if runs[i] >= 5 {
            demerit += 3 + (runs[i] - 5);
        }
        if i & 1 == 1 && i >= 3 && i + 2 < len && runs[i] % 3 == 0 {
            let fact = runs[i] / 3;
            // A 1:1:3:1:1 finder-like pattern with light room on a side.
            let finder = runs[i - 2] == fact
                && runs[i - 1] == fact
                && runs[i + 1] == fact
                && runs[i + 2] == fact;
            let room = i == 3 || runs[i - 3] >= 4 * fact || i + 4 >= len || runs[i + 3] >= 4 * fact;
            if finder && room {
                demerit += 40;
            }
        }
    }
    demerit
}

fn runs(line: impl Iterator<Item = bool>) -> Vec<i32> {
    let mut out = Vec::new();
    let mut prev = None;
    for dark in line {
        if prev.is_none() && dark {
            out.push(-1);
        }
        if prev == Some(dark) {
            if let Some(last) = out.last_mut() {
                *last += 1;
            }
        } else {
            out.push(1);
        }
        prev = Some(dark);
    }
    out
}

/// libqrencode's `Mask_mask` penalty of a symbol: N4 by the dark ratio, N2,
/// and N1 and N3 by rows and columns.
fn penalty(width: usize, m: &[bool]) -> i32 {
    let w2 = (width * width) as i32;
    let blacks = m.iter().filter(|&&d| d).count() as i32;
    let bratio = (200 * blacks + w2) / w2 / 2;
    let mut demerit = (bratio - 50).abs() / 5 * 10;
    for y in 1..width {
        for x in 1..width {
            let q = [
                m[y * width + x],
                m[y * width + x - 1],
                m[(y - 1) * width + x],
                m[(y - 1) * width + x - 1],
            ];
            if q.iter().all(|&d| d) || q.iter().all(|&d| !d) {
                demerit += 3;
            }
        }
    }
    for y in 0..width {
        demerit += n1n3(&runs((0..width).map(|x| m[y * width + x])));
    }
    for x in 0..width {
        demerit += n1n3(&runs((0..width).map(|y| m[y * width + x])));
    }
    demerit
}

// ---- Encoding -------------------------------------------------------------------

/// A QR code: `width` × `width` modules, row by row, true for dark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub version: u32,
    pub width: usize,
    pub modules: Vec<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EncodeError {
    /// No data (EINVAL).
    Empty,
    /// More than version 40 holds (ERANGE).
    TooLarge,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// The smallest version to use; 0 for the smallest that holds the data.
    pub version: u32,
    pub level: EcLevel,
    /// Every byte as 8-bit data (`-8`), not split.
    pub eight_bit: bool,
    /// Shift JIS kanji in kanji mode (`-k`).
    pub kanji: bool,
    /// Lower case kept (`-c`, the default), not made upper (`-i`).
    pub case_sensitive: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            version: 0,
            level: EcLevel::L,
            eight_bit: false,
            kanji: false,
            case_sensitive: true,
        }
    }
}

/// `data` as qrencode encodes it.
pub fn encode(data: &[u8], o: &Options) -> Result<Symbol, EncodeError> {
    let segments = if o.eight_bit {
        if data.is_empty() {
            return Err(EncodeError::Empty);
        }
        vec![Segment {
            mode: Mode::Eight,
            data: data.to_vec(),
        }]
    } else {
        if data.first().is_none_or(|&c| c == 0) {
            return Err(EncodeError::Empty);
        }
        split(data, o.version, o.kanji, o.case_sensitive)
    };
    let mut version = estimate_version(&segments, o.level).max(o.version);
    let bits = loop {
        if version > 40 {
            return Err(EncodeError::TooLarge);
        }
        let bits = bitstream(&segments, version).ok_or(EncodeError::TooLarge)?;
        let needed = minimum_version(bits.len().div_ceil(8), o.level);
        if needed > version {
            version = needed;
        } else {
            break bits;
        }
    };
    let mut bits = bits;
    bits.push_terminator(o.level)
        .map_err(|_| EncodeError::TooLarge)?;
    let v = Version::Normal(version as i16);
    let data = bits.into_bytes();
    let (encoded, ec) =
        qrcode::ec::construct_codewords(&data, v, o.level).map_err(|_| EncodeError::TooLarge)?;
    let mut canvas = Canvas::new(v, o.level);
    canvas.draw_all_functional_patterns();
    canvas.draw_data(&encoded, &ec);
    let width = v.width() as usize;
    let mut best: Option<(i32, Vec<bool>)> = None;
    for pattern in [
        MaskPattern::Checkerboard,
        MaskPattern::HorizontalLines,
        MaskPattern::VerticalLines,
        MaskPattern::DiagonalLines,
        MaskPattern::LargeCheckerboard,
        MaskPattern::Fields,
        MaskPattern::Diamonds,
        MaskPattern::Meadow,
    ] {
        let mut c = canvas.clone();
        c.apply_mask(pattern);
        let modules: Vec<bool> = c
            .into_colors()
            .into_iter()
            .map(|m| m == Color::Dark)
            .collect();
        let p = penalty(width, &modules);
        if best.as_ref().is_none_or(|(b, _)| p < *b) {
            best = Some((p, modules));
        }
    }
    let modules = best.map(|b| b.1).unwrap_or_default();
    Ok(Symbol {
        version,
        width,
        modules,
    })
}

// ---- Writing it out ---------------------------------------------------------------

/// How a symbol is drawn: qrencode's `-s`, `-m`, `-d`, colours and SVG options.
#[derive(Clone, Debug)]
pub struct Look {
    pub size: usize,
    pub margin: usize,
    pub dpi: u32,
    /// RGBA.
    pub fg: [u8; 4],
    pub bg: [u8; 4],
    pub rle: bool,
    pub svg_path: bool,
    pub inline_svg: bool,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            size: 3,
            margin: 4,
            dpi: 72,
            fg: [0, 0, 0, 255],
            bg: [255; 4],
            rle: false,
            svg_path: false,
            inline_svg: false,
        }
    }
}

impl Symbol {
    fn dark(&self, x: usize, y: usize) -> bool {
        self.modules[y * self.width + x]
    }
}

/// `-t ASCII` (`ASCIIi` inverted): two characters a module.
pub fn ascii(s: &Symbol, margin: usize, invert: bool) -> String {
    let (black, white) = if invert { (' ', '#') } else { ('#', ' ') };
    let realwidth = (s.width + margin * 2) * 2;
    let mut out = String::new();
    let edge: String = std::iter::repeat_n(if invert { '#' } else { ' ' }, realwidth).collect();
    for _ in 0..margin {
        out.push_str(&edge);
        out.push('\n');
    }
    for y in 0..s.width {
        out.extend(std::iter::repeat_n(white, margin * 2));
        for x in 0..s.width {
            let c = if s.dark(x, y) { black } else { white };
            out.push(c);
            out.push(c);
        }
        out.extend(std::iter::repeat_n(white, margin * 2));
        out.push('\n');
    }
    for _ in 0..margin {
        out.push_str(&edge);
        out.push('\n');
    }
    out
}

/// `-t UTF8` and its kin: two rows a line in half blocks, light modules
/// drawn, for a terminal's dark background; `ansi` 1 or 2 adds its colours.
pub fn utf8(s: &Symbol, margin: usize, ansi: u8, invert: bool) -> String {
    let (mut empty, mut low, mut up, mut full) = (" ", "\u{2584}", "\u{2580}", "\u{2588}");
    if invert {
        std::mem::swap(&mut empty, &mut full);
        std::mem::swap(&mut low, &mut up);
    }
    let (white, reset) = match ansi {
        2 => ("\x1b[38;5;231m\x1b[48;5;16m", "\x1b[0m"),
        1 => ("\x1b[40;37;1m", "\x1b[0m"),
        _ => ("", ""),
    };
    let realwidth = s.width + margin * 2;
    let mut out = String::new();
    let edge = |out: &mut String| {
        for _ in 0..margin / 2 {
            out.push_str(white);
            for _ in 0..realwidth {
                out.push_str(full);
            }
            out.push_str(reset);
            out.push('\n');
        }
    };
    edge(&mut out);
    for y in (0..s.width).step_by(2) {
        out.push_str(white);
        for _ in 0..margin {
            out.push_str(full);
        }
        for x in 0..s.width {
            let below = y + 1 < s.width && s.dark(x, y + 1);
            out.push_str(match (s.dark(x, y), below) {
                (true, true) => empty,
                (true, false) => low,
                (false, true) => up,
                (false, false) => full,
            });
        }
        for _ in 0..margin {
            out.push_str(full);
        }
        out.push_str(reset);
        out.push('\n');
    }
    edge(&mut out);
    out
}

/// `-t ANSI` (`ANSI256`): background colours, two spaces a module.
pub fn ansi(s: &Symbol, margin: usize, colors_256: bool) -> String {
    let (white, black) = if colors_256 {
        ("\x1b[48;5;231m", "\x1b[48;5;16m")
    } else {
        ("\x1b[47m", "\x1b[40m")
    };
    let realwidth = s.width + margin * 2;
    let mut out = String::new();
    let edge = |out: &mut String| {
        for _ in 0..margin {
            out.push_str(white);
            out.extend(std::iter::repeat_n(' ', realwidth * 2));
            out.push_str("\x1b[0m\n");
        }
    };
    edge(&mut out);
    for y in 0..s.width {
        out.push_str(white);
        out.extend(std::iter::repeat_n(' ', margin * 2));
        let mut last = false;
        for x in 0..s.width {
            let d = s.dark(x, y);
            if d && !last {
                out.push_str(black);
            } else if !d && last {
                out.push_str(white);
            }
            last = d;
            out.push_str("  ");
        }
        if last {
            out.push_str(white);
        }
        out.extend(std::iter::repeat_n(' ', margin * 2));
        out.push_str("\x1b[0m\n");
    }
    edge(&mut out);
    out
}

fn hex(c: [u8; 4]) -> String {
    format!("{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// `-t SVG`, with `--rle`, `--svg-path` and `--inline`.
pub fn svg(s: &Symbol, l: &Look) -> String {
    let scale = (f64::from(l.dpi) * (100.0 / 2.54) / 100.0) as f32;
    let symwidth = s.width + l.margin * 2;
    let realwidth = symwidth * l.size;
    let (fg, bg) = (hex(l.fg), hex(l.bg));
    let fg_opacity = f32::from(l.fg[3]) / 255.0;
    let bg_opacity = f32::from(l.bg[3]) / 255.0;
    let mut out = String::new();
    if !l.inline_svg {
        out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    }
    out.push_str(
        "<!-- Created with qrencode 4.1.1 (https://fukuchi.org/works/qrencode/index.html) -->\n",
    );
    let cm = f64::from(realwidth as f32 / scale);
    out.push_str(&format!(
        "<svg width=\"{cm:.2}cm\" height=\"{cm:.2}cm\" viewBox=\"0 0 {symwidth} {symwidth}\" preserveAspectRatio=\"none\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">\n"
    ));
    out.push_str("\t<g id=\"QRcode\">\n");
    if l.bg[3] != 255 {
        out.push_str(&format!(
            "\t\t<rect x=\"0\" y=\"0\" width=\"{symwidth}\" height=\"{symwidth}\" fill=\"#{bg}\" fill-opacity=\"{:.6}\"/>\n",
            f64::from(bg_opacity)
        ));
    } else {
        out.push_str(&format!("\t\t<rect x=\"0\" y=\"0\" width=\"{symwidth}\" height=\"{symwidth}\" fill=\"#{bg}\"/>\n"));
    }
    let m = l.margin;
    if l.svg_path {
        if l.fg[3] != 255 {
            out.push_str(&format!(
                "\t\t<path style=\"stroke:#{fg};stroke-opacity:{:.6}\" transform=\"translate({m},{m}.5)\" d=\"",
                f64::from(fg_opacity)
            ));
        } else {
            out.push_str(&format!(
                "\t\t<path style=\"stroke:#{fg}\" transform=\"translate({m},{m}.5)\" d=\""
            ));
        }
    } else {
        out.push_str(&format!(
            "\t\t<g id=\"Pattern\" transform=\"translate({m},{m})\">\n"
        ));
    }
    let draw = |out: &mut String, x: usize, y: usize, w: usize| {
        if l.svg_path {
            out.push_str(&format!("M{x},{y}h{w}"));
        } else if l.fg[3] != 255 {
            out.push_str(&format!(
                "\t\t\t<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"1\" fill=\"#{fg}\" fill-opacity=\"{:.6}\"/>\n",
                f64::from(fg_opacity)
            ));
        } else {
            out.push_str(&format!(
                "\t\t\t<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"1\" fill=\"#{fg}\"/>\n"
            ));
        }
    };
    for y in 0..s.width {
        if !l.rle {
            for x in 0..s.width {
                if s.dark(x, y) {
                    draw(&mut out, x, y, 1);
                }
            }
        } else {
            let (mut pen, mut x0) = (false, 0);
            for x in 0..s.width {
                if !pen {
                    pen = s.dark(x, y);
                    x0 = x;
                } else if !s.dark(x, y) {
                    draw(&mut out, x0, y, x - x0);
                    pen = false;
                }
            }
            if pen {
                draw(&mut out, x0, y, s.width - x0);
            }
        }
    }
    if l.svg_path {
        out.push_str("\"/>\n");
    } else {
        out.push_str("\t\t</g>\n");
    }
    out.push_str("\t</g>\n</svg>\n");
    out
}

/// `-t EPS`.
pub fn eps(s: &Symbol, l: &Look) -> String {
    let realwidth = (s.width + l.margin * 2) * l.size;
    let c = |v: u8| f64::from(f32::from(v) / 255.0);
    let mut out = format!(
        "%!PS-Adobe-2.0 EPSF-1.2\n%%BoundingBox: 0 0 {realwidth} {realwidth}\n%%Pages: 1 1\n%%EndComments\n"
    );
    out.push_str("/p { moveto 0 1 rlineto 1 0 rlineto 0 -1 rlineto fill } bind def\n");
    out.push_str("gsave\n");
    out.push_str(&format!(
        "{:.6} {:.6} {:.6} setrgbcolor\n",
        c(l.bg[0]),
        c(l.bg[1]),
        c(l.bg[2])
    ));
    out.push_str(&format!("{realwidth} {realwidth} scale\n"));
    out.push_str("0 0 p\ngrestore\n");
    out.push_str(&format!(
        "{:.6} {:.6} {:.6} setrgbcolor\n",
        c(l.fg[0]),
        c(l.fg[1]),
        c(l.fg[2])
    ));
    out.push_str(&format!("{} {} scale\n", l.size, l.size));
    for y in 0..s.width {
        let yy = l.margin + s.width - y - 1;
        for x in 0..s.width {
            if s.dark(x, y) {
                out.push_str(&format!("{} {yy} p ", l.margin + x));
            }
        }
    }
    out.push_str("\n%%EOF\n");
    out
}

/// `-t XPM`, its last line as qrencode ends it (by the size, not the margin).
pub fn xpm(s: &Symbol, l: &Look) -> String {
    let realwidth = (s.width + l.margin * 2) * l.size;
    let realmargin = l.margin * l.size;
    let mut out = String::from(
        "/* XPM */\nstatic const char *const qrcode_xpm[] = {\n/* width height ncolors chars_per_pixel */\n",
    );
    out.push_str(&format!("\"{realwidth} {realwidth} 2 1\",\n/* colors */\n"));
    out.push_str(&format!(
        "\"F c #{}\",\n\"B c #{}\",\n/* pixels */\n",
        hex(l.fg),
        hex(l.bg)
    ));
    let blank: String = "B".repeat(realwidth);
    for _ in 0..realmargin {
        out.push_str(&format!("\"{blank}\",\n"));
    }
    for y in 0..s.width {
        let mut row = "B".repeat(realmargin);
        for x in 0..s.width {
            row.push_str(&(if s.dark(x, y) { "F" } else { "B" }).repeat(l.size));
        }
        row.push_str(&"B".repeat(realmargin));
        for _ in 0..l.size {
            out.push_str(&format!("\"{row}\",\n"));
        }
    }
    for y in 0..realmargin {
        out.push_str(&format!(
            "\"{blank}\"{}\n",
            if y + 1 < l.size { "," } else { "};" }
        ));
    }
    out
}

/// `-t PNG` (1-bit, a palette of the two colours) or `PNG32` (RGBA), with
/// the DPI as its pHYs. The pixels are qrencode's; the compressed bytes are
/// this encoder's.
pub fn png(s: &Symbol, l: &Look, rgba: bool) -> Result<Vec<u8>, String> {
    let realwidth = (s.width + l.margin * 2) * l.size;
    let w = u32::try_from(realwidth).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, w);
        if rgba {
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
        } else {
            enc.set_color(png::ColorType::Indexed);
            enc.set_depth(png::BitDepth::One);
            enc.set_palette(vec![l.fg[0], l.fg[1], l.fg[2], l.bg[0], l.bg[1], l.bg[2]]);
            enc.set_trns(vec![l.fg[3], l.bg[3]]);
        }
        let ppm = (f64::from(l.dpi) * (100.0 / 2.54)) as u32;
        enc.set_pixel_dims(Some(png::PixelDimensions {
            xppu: ppm,
            yppu: ppm,
            unit: png::Unit::Meter,
        }));
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        let mut data = Vec::new();
        let dark_at = |px: usize, py: usize| {
            let (mx, my) = (px / l.size, py / l.size);
            mx >= l.margin
                && my >= l.margin
                && mx < l.margin + s.width
                && my < l.margin + s.width
                && s.dark(mx - l.margin, my - l.margin)
        };
        for py in 0..realwidth {
            if rgba {
                for px in 0..realwidth {
                    data.extend_from_slice(if dark_at(px, py) { &l.fg } else { &l.bg });
                }
            } else {
                let mut row = vec![0xffu8; realwidth.div_ceil(8)];
                for px in 0..realwidth {
                    if dark_at(px, py) {
                        row[px / 8] &= !(0x80 >> (px % 8));
                    }
                }
                data.extend_from_slice(&row);
            }
        }
        writer.write_image_data(&data).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

// ---- The app's tool ---------------------------------------------------------------

static META: Meta = Meta {
    id: "qrencode",
    group: "qrencode",
    action: "",
    action_flag: None,
    title: en!("QR code"),
    summary: en!("Text as a QR code, as qrencode makes it: in the terminal, SVG or PNG"),
    category: Category::Encoding,
    keywords: &["qr", "qrcode", "qrencode", "barcode", "2d", "png", "svg"],
    input: InputKind::Bytes,
    streams: false,
    reference: Some(Reference {
        name: "qrencode",
        version: "4.1.1",
        compat: Compat::DropIn,
    }),
};

static PARAMS: &[Param] = &[
    Param {
        key: "level",
        label: en!("Error correction"),
        help: en!("How much of the code may be lost: L 7%, M 15%, Q 25%, H 30%"),
        kind: ParamKind::Choice {
            options: &[
                ("L", en!("L, 7%")),
                ("M", en!("M, 15%")),
                ("Q", en!("Q, 25%")),
                ("H", en!("H, 30%")),
            ],
            flags: false,
            unset: None,
        },
        default: Value::Text(String::new()),
        cli: Cli {
            long: "level",
            short: Some('l'),
        },
    },
    Param {
        key: "margin",
        label: en!("Margin"),
        help: en!("Light modules around it; 4 as the standard asks"),
        kind: ParamKind::Int { min: 0, max: 40 },
        default: Value::Int(4),
        cli: Cli {
            long: "margin",
            short: Some('m'),
        },
    },
    Param {
        key: "version",
        label: en!("Version"),
        help: en!("The smallest version (size) to use, 1 to 40; 0 for the smallest that holds it"),
        kind: ParamKind::Int { min: 0, max: 40 },
        default: Value::Int(0),
        cli: Cli {
            long: "symversion",
            short: Some('v'),
        },
    },
    Param {
        key: "eight_bit",
        label: en!("8-bit"),
        help: en!("Every byte as 8-bit data, not split into digits and letters"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "8bit",
            short: Some('8'),
        },
    },
];

pub struct Encode;

pub fn level_of(s: &str) -> EcLevel {
    match s {
        "M" => EcLevel::M,
        "Q" => EcLevel::Q,
        "H" => EcLevel::H,
        _ => EcLevel::L,
    }
}

impl Encode {
    pub fn symbol(input: &[u8], values: &Values) -> Result<Symbol, Error> {
        let o = Options {
            version: values.int(PARAMS, "version").clamp(0, 40) as u32,
            level: level_of(&values.text(PARAMS, "level")),
            eight_bit: values.flag(PARAMS, "eight_bit"),
            ..Options::default()
        };
        encode(input, &o).map_err(|e| match e {
            EncodeError::Empty => Error::usage("give the text to encode"),
            EncodeError::TooLarge => {
                Error::input("Input data too large: a QR code holds 2,953 bytes at most (level L)")
            }
        })
    }

    pub fn margin(values: &Values) -> usize {
        values.int(PARAMS, "margin").clamp(0, 40) as usize
    }
}

impl Tool for Encode {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    /// The code as a picture, eight pixels a module, as `qrencode -s 8`.
    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let s = Self::symbol(input, values)?;
        let look = Look {
            size: 8,
            margin: Self::margin(values),
            ..Look::default()
        };
        png(&s, &look, false)
            .map(Output::Image)
            .map_err(Error::input)
    }

    /// `qrencode -t UTF8 [-l L] [-m N] [-v N] [-8] TEXT`.
    fn command_args(&self, values: &Values) -> Option<Vec<String>> {
        let mut args = vec!["qrencode".to_owned(), "-t".to_owned(), "UTF8".to_owned()];
        let level = values.text(PARAMS, "level");
        if !level.is_empty() && level != "L" {
            args.extend(["-l".to_owned(), level]);
        }
        let margin = values.int(PARAMS, "margin");
        if margin != 4 {
            args.extend(["-m".to_owned(), margin.to_string()]);
        }
        let version = values.int(PARAMS, "version");
        if version != 0 {
            args.extend(["-v".to_owned(), version.to_string()]);
        }
        if values.flag(PARAMS, "eight_bit") {
            args.push("-8".to_owned());
        }
        args.push("--".to_owned());
        Some(args)
    }

    fn input_in_args(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(s: &Symbol) -> String {
        ascii(s, 0, false)
    }

    /// What `qrencode -t ASCII -m 0` drew (qrencode 4.1.1, Debian 13): the
    /// split of mixed text, and masks the crate's own penalty picks otherwise.
    #[test]
    fn symbols_are_qrencodes() {
        let hello = encode(b"hello", &Options::default()).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(hello.version, 1);
        assert!(md(&hello).starts_with("##############    ##  ####  ##############\n"));
        let mixed =
            encode(b"HELLO WORLD 123", &Options::default()).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(
            md(&mixed).starts_with("##############  ##      ##  ##############\n"),
            "{}",
            md(&mixed)
        );
        let random = encode(
            b"tYgj?mUh_Bel31iEl&2hpChYgCfrL1s=pNxnyVm&",
            &Options {
                eight_bit: true,
                ..Options::default()
            },
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(
            md(&random).starts_with("##############      ##  ######  ##########  ##############\n"),
            "{}",
            md(&random)
        );
    }

    #[test]
    fn splits_as_libqrencode_does() {
        let s = split(b"HELLO WORLD 123", 0, false, true);
        assert_eq!(s.iter().map(|s| s.mode).collect::<Vec<_>>(), [Mode::An]);
        let s = split(b"abc12345678def", 0, false, true);
        assert_eq!(
            s.iter().map(|s| s.mode).collect::<Vec<_>>(),
            [Mode::Eight, Mode::Num, Mode::Eight]
        );
        let s = split(b"abc", 0, false, false);
        assert_eq!(
            s,
            [Segment {
                mode: Mode::An,
                data: b"ABC".to_vec()
            }]
        );
    }

    /// qrencode's PNGs (recorded as compatibility cases) decode to what
    /// tw's do: size, palette and transparency, DPI, every pixel.
    #[test]
    fn pngs_are_qrencodes_but_for_their_compression() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../cli/tests/compat/qrencode");
        let decode = |bytes: &[u8]| {
            let mut d = png::Decoder::new(std::io::Cursor::new(bytes.to_vec()));
            d.set_transformations(png::Transformations::IDENTITY);
            let mut r = d.read_info().unwrap_or_else(|e| panic!("{e}"));
            let mut buf = vec![0; r.output_buffer_size().unwrap_or(0)];
            let frame = r.next_frame(&mut buf).unwrap_or_else(|e| panic!("{e}"));
            let info = r.info();
            let meta = (
                frame.width,
                frame.color_type,
                frame.bit_depth,
                info.palette.as_ref().map(|p| p.to_vec()),
                info.trns.as_ref().map(|t| t.to_vec()),
                info.pixel_dims.map(|d| (d.xppu, d.yppu)),
            );
            buf.truncate(frame.buffer_size());
            (format!("{meta:?}"), buf)
        };
        for (case, look, rgba) in [
            ("png-stdout", Look::default(), false),
            (
                "png32-stdout",
                Look {
                    size: 2,
                    ..Look::default()
                },
                true,
            ),
        ] {
            let want = std::fs::read(dir.join(case).join("stdout"))
                .unwrap_or_else(|e| panic!("{case}: {e}"));
            let s = encode(b"hello", &Options::default()).unwrap_or_else(|e| panic!("{e:?}"));
            let got = png(&s, &look, rgba).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(decode(&got), decode(&want), "{case}");
        }
    }

    #[test]
    fn too_much_is_refused() {
        let big = vec![b'a'; 3000];
        let o = Options {
            eight_bit: true,
            ..Options::default()
        };
        assert_eq!(encode(&big, &o).map(|_| ()), Err(EncodeError::TooLarge));
        assert_eq!(
            encode(b"", &Options::default()).map(|_| ()),
            Err(EncodeError::Empty)
        );
        assert!(encode(&vec![b'a'; 2953], &o).is_ok());
    }
}
