//! `hex encode` and `hex decode`: bytes as hexadecimal, as `xxd -p` writes
//! it (30 bytes a line, lowercase) and `xxd -r -p` reads it (digits of
//! either case, white space anywhere). Where xxd guesses at what is not
//! hex, skipping a character or a last lone digit, `hex decode` says where
//! it is and fails.

use std::io::{Read, Write};

use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, InputKind, Meta, Output, Param, ParamKind, Reference, Span,
    Tool, Value, Values, en,
};

use crate::basen::{CHUNK, read_some};

const REFERENCE: Option<Reference> = Some(Reference {
    name: "xxd -p",
    version: "2024-12-07",
    compat: Compat::Output,
});

pub struct Encode;
pub struct Decode;

static ENCODE_META: Meta = Meta {
    id: "hex.encode",
    group: "hex",
    action: "encode",
    action_flag: None,
    title: en!("Hex encode"),
    summary: en!("Bytes as hexadecimal, as xxd -p writes them"),
    category: Category::Encoding,
    keywords: &["hex", "hexadecimal", "xxd", "hexdump", "base16", "encode"],
    input: InputKind::Bytes,
    streams: true,
    reference: REFERENCE,
};

static DECODE_META: Meta = Meta {
    id: "hex.decode",
    group: "hex",
    action: "decode",
    action_flag: None,
    title: en!("Hex decode"),
    summary: en!("Hexadecimal back to bytes, as xxd -r -p reads it"),
    category: Category::Encoding,
    keywords: &["hex", "hexadecimal", "xxd", "unhex", "base16", "decode"],
    input: InputKind::Bytes,
    streams: true,
    reference: REFERENCE,
};

static ENCODE_PARAMS: &[Param] = &[
    Param {
        key: "cols",
        label: en!("Bytes a line"),
        help: en!("Bytes on each line, as xxd's -c; 0 writes one line"),
        kind: ParamKind::Int {
            min: 0,
            max: 1 << 20,
        },
        default: Value::Int(30),
        cli: Cli {
            long: "cols",
            short: Some('c'),
        },
    },
    Param {
        key: "upper",
        label: en!("Uppercase"),
        help: en!("Write A-F in uppercase, as xxd's -u"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "upper",
            short: Some('u'),
        },
    },
];

impl Tool for Encode {
    fn meta(&self) -> &'static Meta {
        &ENCODE_META
    }

    fn params(&self) -> &'static [Param] {
        ENCODE_PARAMS
    }

    fn run(&self, ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let mut out = Vec::new();
        self.run_stream(ctx, &mut &input[..], &mut out, values)?;
        // The CLI ends text output with a newline itself.
        out.pop_if(|b| *b == b'\n');
        Ok(Output::Text(String::from_utf8_lossy(&out).into_owned()))
    }

    /// Each line of `cols` bytes ends with a newline, the last one too;
    /// nothing for no input, as xxd.
    fn run_stream(
        &self,
        ctx: &Ctx,
        input: &mut dyn Read,
        out: &mut dyn Write,
        values: &Values,
    ) -> Result<(), Error> {
        let cols = usize::try_from(values.int(ENCODE_PARAMS, "cols")).unwrap_or(30);
        let digits: &[u8; 16] = if values.flag(ENCODE_PARAMS, "upper") {
            b"0123456789ABCDEF"
        } else {
            b"0123456789abcdef"
        };
        let mut buf = vec![0; CHUNK];
        let mut line = Vec::with_capacity(CHUNK * 3);
        let mut column = 0;
        loop {
            ctx.check()?;
            let n = read_some(input, &mut buf)?;
            if n == 0 {
                break;
            }
            line.clear();
            for &b in &buf[..n] {
                line.push(digits[usize::from(b >> 4)]);
                line.push(digits[usize::from(b & 0xf)]);
                column += 1;
                if cols > 0 && column == cols {
                    line.push(b'\n');
                    column = 0;
                }
            }
            out.write_all(&line)?;
        }
        if column > 0 {
            out.write_all(b"\n")?;
        }
        Ok(())
    }
}

impl Tool for Decode {
    fn meta(&self) -> &'static Meta {
        &DECODE_META
    }

    fn run(&self, ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let mut out = Vec::new();
        self.run_stream(ctx, &mut &input[..], &mut out, values)?;
        Ok(Output::Bytes(out))
    }

    /// Two digits a byte; white space between any of them. The bytes before
    /// an error are written, then the error.
    fn run_stream(
        &self,
        ctx: &Ctx,
        input: &mut dyn Read,
        out: &mut dyn Write,
        _values: &Values,
    ) -> Result<(), Error> {
        let mut buf = vec![0; CHUNK];
        let mut decoded = Vec::with_capacity(CHUNK / 2);
        // The first digit of a byte, waiting for its second; where it was.
        let mut high: Option<(u8, Position)> = None;
        let mut at = Position::default();
        loop {
            ctx.check()?;
            let n = read_some(input, &mut buf)?;
            if n == 0 {
                break;
            }
            decoded.clear();
            let mut failed = None;
            for &c in &buf[..n] {
                let digit = char::from(c).to_digit(16);
                match (digit, high) {
                    (Some(d), None) => high = Some((d as u8, at)),
                    (Some(d), Some((h, _))) => {
                        decoded.push(h << 4 | d as u8);
                        high = None;
                    }
                    (None, _) if c.is_ascii_whitespace() => {}
                    (None, _) => {
                        failed = Some(at.error(format!("{:?} is not a hex digit", char::from(c))));
                        break;
                    }
                }
                at.step(c);
            }
            out.write_all(&decoded)?;
            if let Some(e) = failed {
                return Err(e);
            }
        }
        match high {
            Some((_, at)) => Err(at.error("a hex digit without its pair: bytes take two")),
            None => Ok(()),
        }
    }
}

/// Where a character is: streamed input has no slice to point into.
#[derive(Clone, Copy, Debug)]
struct Position {
    offset: usize,
    line: usize,
    column: usize,
}

impl Default for Position {
    fn default() -> Self {
        Position {
            offset: 0,
            line: 1,
            column: 1,
        }
    }
}

impl Position {
    fn step(&mut self, c: u8) {
        self.offset += 1;
        if c == b'\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
    }

    fn error(self, message: impl Into<String>) -> Error {
        Error::input(message).at(Span {
            offset: self.offset,
            line: self.line,
            column: self.column,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(input: &[u8], cols: i64, upper: bool) -> Vec<u8> {
        let mut v = Values::new();
        v.set("cols", Value::Int(cols));
        v.set("upper", Value::Bool(upper));
        let mut out = Vec::new();
        let _ = Encode.run_stream(&Ctx::new(), &mut &input[..], &mut out, &v);
        out
    }

    fn decode(input: &str) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        Decode
            .run_stream(&Ctx::new(), &mut input.as_bytes(), &mut out, &Values::new())
            .map(|()| out)
    }

    /// Each as xxd 2024-12-07 writes it (`xxd -p`, `-c 8`, `-c 0`, `-u`).
    #[test]
    fn encodes_as_xxd_p() {
        assert_eq!(
            encode(b"hello, world\n", 30, false),
            b"68656c6c6f2c20776f726c640a\n"
        );
        let forty = encode(&[b'A'; 40], 30, false);
        assert_eq!(
            forty,
            format!("{}\n{}\n", "41".repeat(30), "41".repeat(10)).into_bytes()
        );
        assert_eq!(
            encode(&[b'A'; 16], 8, false),
            format!("{0}\n{0}\n", "41".repeat(8)).into_bytes()
        );
        assert_eq!(
            encode(&[b'A'; 10], 0, false),
            format!("{}\n", "41".repeat(10)).into_bytes()
        );
        assert_eq!(encode(&[0xab, 0xcd], 30, true), b"ABCD\n");
        assert_eq!(encode(b"", 30, false), b"");
    }

    #[test]
    fn decodes_as_xxd_r_p_and_says_where_it_cannot() {
        assert_eq!(decode("68656c6c6f").ok(), Some(b"hello".to_vec()));
        assert_eq!(decode("68 65 6c").ok(), Some(b"hel".to_vec()));
        assert_eq!(decode("6865\n6c6c6f\n").ok(), Some(b"hello".to_vec()));
        assert_eq!(decode("4A4b").ok(), Some(b"JK".to_vec()));
        assert_eq!(decode("6 8").ok(), Some(b"h".to_vec()));
        let e = decode("6865\n6czz").err();
        assert_eq!(
            e.as_ref().and_then(|e| e.span).map(|s| (s.line, s.column)),
            Some((2, 3))
        );
        let e = decode("686").err();
        assert_eq!(e.as_ref().and_then(|e| e.span).map(|s| s.column), Some(3));
    }

    #[test]
    fn round_trips_across_chunks() {
        let data: Vec<u8> = (0..CHUNK * 2 + 7).map(|i| (i * 31 % 251) as u8).collect();
        let text = String::from_utf8(encode(&data, 30, false)).unwrap_or_default();
        assert_eq!(decode(&text).ok(), Some(data));
    }
}
