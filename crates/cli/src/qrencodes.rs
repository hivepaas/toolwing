//! `qrencode`: qrencode 4.1.1's command line, its arguments read as glibc's
//! `getopt_long` reads them for its `main`, and its output types as its
//! writers write them (toolwing_tools::qr). Exit codes as its: 1 for an
//! error, 0 otherwise. Not made: Micro QR (`-M`) and structured symbols
//! (`-S`), which are refused.

use std::io::{self, Read, Write};

use toolwing_core::Tool as _;
use toolwing_tools::qr::{self, EncodeError, Look, Options};

const VERSION: &str = "4.1.1";

/// qrencode's input buffer, `7090 * 2`; more is refused.
const MAX_DATA_SIZE: usize = 7090 * 2;

pub fn command() -> clap::Command {
    clap::Command::new("qrencode")
        .about(qr::Encode.meta().summary)
        .disable_help_flag(true)
        .arg(
            clap::Arg::new("args")
                .value_name("ARGS")
                .num_args(0..)
                .allow_hyphen_values(true)
                .trailing_var_arg(true),
        )
}

const SHORT: &str = "ho:r:l:s:v:m:d:t:Skci8MV";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Opt {
    Short(char),
    Rle,
    SvgPath,
    Inline,
    Strict,
    Foreground,
    Background,
    Verbose,
}

const LONG: &[(&str, bool, Opt)] = &[
    ("help", false, Opt::Short('h')),
    ("output", true, Opt::Short('o')),
    ("read-from", true, Opt::Short('r')),
    ("level", true, Opt::Short('l')),
    ("size", true, Opt::Short('s')),
    ("symversion", true, Opt::Short('v')),
    ("margin", true, Opt::Short('m')),
    ("dpi", true, Opt::Short('d')),
    ("type", true, Opt::Short('t')),
    ("structured", false, Opt::Short('S')),
    ("kanji", false, Opt::Short('k')),
    ("casesensitive", false, Opt::Short('c')),
    ("ignorecase", false, Opt::Short('i')),
    ("8bit", false, Opt::Short('8')),
    ("micro", false, Opt::Short('M')),
    ("rle", false, Opt::Rle),
    ("svg-path", false, Opt::SvgPath),
    ("inline", false, Opt::Inline),
    ("strict-version", false, Opt::Strict),
    ("foreground", true, Opt::Foreground),
    ("background", true, Opt::Background),
    ("version", false, Opt::Short('V')),
    ("verbose", false, Opt::Verbose),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Type {
    Png,
    Png32,
    Eps,
    Svg,
    Xpm,
    Ansi,
    Ansi256,
    Ascii,
    AsciiI,
    Utf8,
    AnsiUtf8,
    Ansi256Utf8,
    Utf8I,
    AnsiUtf8I,
}

fn image_type(s: &str) -> Option<Type> {
    Some(match s.to_ascii_lowercase().as_str() {
        "png32" => Type::Png32,
        "png" => Type::Png,
        "eps" => Type::Eps,
        "svg" => Type::Svg,
        "xpm" => Type::Xpm,
        "ansi" => Type::Ansi,
        "ansi256" => Type::Ansi256,
        "asciii" => Type::AsciiI,
        "ascii" => Type::Ascii,
        "utf8" => Type::Utf8,
        "ansiutf8" => Type::AnsiUtf8,
        "ansi256utf8" => Type::Ansi256Utf8,
        "utf8i" => Type::Utf8I,
        "ansiutf8i" => Type::AnsiUtf8I,
        _ => return None,
    })
}

/// C's `atoi`: leading space, a sign, the digits there are; 0 for none.
fn atoi(s: &str) -> i64 {
    let s = s.trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r']);
    let (neg, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let digits: String = s
        .chars()
        .take_while(char::is_ascii_digit)
        .take(18)
        .collect();
    let n: i64 = digits.parse().unwrap_or(0);
    if neg { -n } else { n }
}

/// `--foreground` and `--background`: RRGGBB or RRGGBBAA.
fn color(s: &str) -> Option<[u8; 4]> {
    if !(s.len() == 6 || s.len() == 8) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    Some([
        byte(0)?,
        byte(2)?,
        byte(4)?,
        if s.len() == 8 { byte(6)? } else { 255 },
    ])
}

fn banner() -> String {
    format!(
        "tw qrencode {} (qrencode {VERSION})\n",
        env!("CARGO_PKG_VERSION")
    )
}

/// The reference's `exit(EXIT_FAILURE)` after its message.
struct Fail(String);

/// Runs `tw qrencode`; the exit code. `args` are those after `qrencode`.
pub fn main(args: &[String]) -> u8 {
    match run(args) {
        Ok(code) => code,
        Err(Fail(message)) => {
            eprint!("{message}");
            1
        }
    }
}

fn run(args: &[String]) -> Result<u8, Fail> {
    let fail = |m: String| Err(Fail(m));
    let (opts, operands) = crate::getopt::getopt(args, SHORT, Opt::Short, LONG);
    let mut o = Options::default();
    let mut look = Look {
        margin: usize::MAX,
        ..Look::default()
    };
    let (mut outfile, mut infile) = (None::<String>, None::<String>);
    let mut ty = Type::Png;
    let (mut structured, mut micro, mut strict, mut verbose) = (false, false, false, false);
    let mut version: i64 = 0;
    let mut margin: i64 = -1;
    for got in opts {
        let Some(got) = got else {
            return fail("Try \"qrencode --help\" for more information.\n".into());
        };
        let v = got.value.unwrap_or_default();
        match got.opt {
            Opt::Short('h') => {
                let help = if got.long {
                    include_str!("qrencode_help_long.txt")
                } else {
                    include_str!("qrencode_help_short.txt")
                };
                let mut out = crate::stdout();
                let _ = write!(out, "{}{help}", banner());
                return Ok(0);
            }
            Opt::Short('o') => outfile = Some(v),
            Opt::Short('r') => infile = Some(v),
            Opt::Short('s') => {
                let n = atoi(&v);
                if n <= 0 {
                    return fail(format!("Invalid size: {n}\n"));
                }
                look.size = n as usize;
            }
            Opt::Short('v') => {
                version = atoi(&v);
                if version < 0 {
                    return fail(format!("Invalid version: {version}\n"));
                }
            }
            Opt::Short('l') => {
                o.level = match v.chars().next() {
                    Some('l' | 'L') => qrcode::EcLevel::L,
                    Some('m' | 'M') => qrcode::EcLevel::M,
                    Some('q' | 'Q') => qrcode::EcLevel::Q,
                    Some('h' | 'H') => qrcode::EcLevel::H,
                    _ => return fail(format!("Invalid level: {v}\n")),
                }
            }
            Opt::Short('m') => {
                margin = atoi(&v);
                if margin < 0 {
                    return fail(format!("Invalid margin: {margin}\n"));
                }
            }
            Opt::Short('d') => {
                let n = atoi(&v);
                if n < 0 {
                    return fail(format!("Invalid DPI: {n}\n"));
                }
                look.dpi = n as u32;
            }
            Opt::Short('t') => match image_type(&v) {
                Some(t) => ty = t,
                None => return fail(format!("Invalid image type: {v}\n")),
            },
            Opt::Short('S') => structured = true,
            Opt::Short('k') => o.kanji = true,
            Opt::Short('c') => o.case_sensitive = true,
            Opt::Short('i') => o.case_sensitive = false,
            Opt::Short('8') => o.eight_bit = true,
            Opt::Short('M') => micro = true,
            Opt::Foreground => match color(&v) {
                Some(c) => look.fg = c,
                None => return fail("Invalid foreground color value.\n".into()),
            },
            Opt::Background => match color(&v) {
                Some(c) => look.bg = c,
                None => return fail("Invalid background color value.\n".into()),
            },
            Opt::Short('V') => {
                let mut out = crate::stdout();
                let _ = write!(out, "{}", banner());
                return Ok(0);
            }
            Opt::Rle => look.rle = true,
            Opt::SvgPath => look.svg_path = true,
            Opt::Inline => look.inline_svg = true,
            Opt::Strict => strict = true,
            Opt::Verbose => verbose = true,
            Opt::Short(_) => return fail("Try \"qrencode --help\" for more information.\n".into()),
        }
    }
    if args.is_empty() {
        return fail(format!(
            "{}{}",
            banner(),
            include_str!("qrencode_help_short.txt")
        ));
    }
    if outfile.is_none() && ty == Type::Png {
        return fail("No output filename is given.\n".into());
    }
    let text: Vec<u8> = match operands.first() {
        Some(t) => t.as_bytes().to_vec(),
        None => {
            let mut data = Vec::new();
            let read = match &infile {
                None => io::stdin()
                    .lock()
                    .take(MAX_DATA_SIZE as u64 + 1)
                    .read_to_end(&mut data),
                Some(f) => match std::fs::File::open(f) {
                    Ok(file) => file.take(MAX_DATA_SIZE as u64 + 1).read_to_end(&mut data),
                    Err(_) => return fail(format!("Cannot read input file {f}.\n")),
                },
            };
            if read.is_err() || data.is_empty() {
                return fail("No input data.\n".into());
            }
            // fread fills the buffer before it sees the end: a full one is
            // too large too.
            if data.len() >= MAX_DATA_SIZE {
                return fail("Input data is too large.\n".into());
            }
            data
        }
    };
    if micro && version > 4 {
        return fail("Version number should be less or equal to 4.\n".into());
    } else if !micro && version > 40 {
        return fail("Version number should be less or equal to 40.\n".into());
    }
    if micro && structured {
        return fail("Micro QR Code does not support structured symbols.\n".into());
    }
    if structured && version == 0 {
        return fail("Version number must be specified to encode structured symbols.\n".into());
    }
    if micro || structured {
        return fail(format!(
            "tw qrencode does not make {} yet.\n",
            if micro {
                "Micro QR Codes"
            } else {
                "structured symbols"
            }
        ));
    }
    look.margin = if margin < 0 { 4 } else { margin as usize };
    o.version = version as u32;
    // A string from the command line ends at its NUL, as a C string does.
    let symbol = match qr::encode(&text, &o) {
        Ok(s) => s,
        Err(EncodeError::TooLarge) => {
            return fail("Failed to encode the input data: Input data too large\n".into());
        }
        Err(EncodeError::Empty) => {
            return fail("Failed to encode the input data: Invalid argument\n".into());
        }
    };
    if strict && version > 0 && i64::from(symbol.version) != version {
        return fail("Failed to encode the input data: Input data too large\n".into());
    }
    if verbose {
        eprintln!(
            "File: {}, Version: {}",
            outfile.as_deref().unwrap_or("(stdout)"),
            symbol.version
        );
    }
    let bytes = match ty {
        Type::Png | Type::Png32 => match qr::png(&symbol, &look, ty == Type::Png32) {
            Ok(b) => b,
            Err(e) => return fail(format!("Failed to write the PNG image: {e}\n")),
        },
        Type::Eps => qr::eps(&symbol, &look).into_bytes(),
        Type::Svg => qr::svg(&symbol, &look).into_bytes(),
        Type::Xpm => qr::xpm(&symbol, &look).into_bytes(),
        Type::Ansi | Type::Ansi256 => {
            qr::ansi(&symbol, look.margin, ty == Type::Ansi256).into_bytes()
        }
        Type::Ascii | Type::AsciiI => {
            qr::ascii(&symbol, look.margin, ty == Type::AsciiI).into_bytes()
        }
        Type::Utf8 => qr::utf8(&symbol, look.margin, 0, false).into_bytes(),
        Type::AnsiUtf8 => qr::utf8(&symbol, look.margin, 1, false).into_bytes(),
        Type::Ansi256Utf8 => qr::utf8(&symbol, look.margin, 2, false).into_bytes(),
        Type::Utf8I => qr::utf8(&symbol, look.margin, 0, true).into_bytes(),
        Type::AnsiUtf8I => qr::utf8(&symbol, look.margin, 1, true).into_bytes(),
    };
    match outfile.as_deref() {
        None | Some("-") => {
            let mut out = crate::stdout();
            let _ = out.write_all(&bytes);
            let _ = out.flush();
        }
        Some(f) => {
            if let Err(e) = std::fs::write(f, &bytes) {
                return fail(format!("Failed to create file: {f}\n{e}\n"));
            }
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_colours_read_as_qrencode_reads_them() {
        assert_eq!(atoi(" 12x"), 12);
        assert_eq!(atoi("x"), 0);
        assert_eq!(atoi("-3"), -3);
        assert_eq!(color("ff0000"), Some([255, 0, 0, 255]));
        assert_eq!(color("00ff0080"), Some([0, 255, 0, 128]));
        assert_eq!(color("fff"), None);
        assert_eq!(color("gg0000"), None);
    }
}
