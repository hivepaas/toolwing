//! `base64`, as GNU coreutils' `base64` (drop-in: tools.md).
//!
//! Decoding follows GNU's rules, checked against GNU coreutils 9.7: newlines
//! are skipped (no other whitespace), padding may end any quantum and a new
//! one may follow, a last quantum of two or three characters needs no
//! padding, and unused bits must be zero. On invalid input, the bytes decoded
//! before it are written, then the error.

use std::io::{Read, Write};

use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, InputKind, Meta, Output, Param, ParamKind, Reference, Tool,
    Value, Values, en,
};

const REFERENCE: Option<Reference> = Some(Reference {
    name: "GNU coreutils base64",
    version: "9.7",
    compat: Compat::DropIn,
});

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// How many bytes are read at a time when streaming: a multiple of 3 and 4.
const CHUNK: usize = 3 * 4 * 16 * 1024;

pub struct Encode;
pub struct Decode;

static ENCODE_META: Meta = Meta {
    id: "base64.encode",
    group: "base64",
    action: "encode",
    action_flag: None,
    title: en!("Base64 encode"),
    summary: en!("Encode bytes as Base64 (RFC 4648)"),
    category: Category::Encoding,
    keywords: &["b64", "base64", "encode"],
    input: InputKind::Bytes,
    streams: true,
    reference: REFERENCE,
};

static DECODE_META: Meta = Meta {
    id: "base64.decode",
    group: "base64",
    action: "decode",
    action_flag: Some(('d', "decode")),
    title: en!("Base64 decode"),
    summary: en!("Decode Base64 back to bytes"),
    category: Category::Encoding,
    keywords: &["b64", "base64", "decode"],
    input: InputKind::Bytes,
    streams: true,
    reference: REFERENCE,
};

static ENCODE_PARAMS: &[Param] = &[Param {
    key: "wrap",
    label: en!("Wrap lines at"),
    help: en!("Wrap encoded lines after this many characters; 0 for no wrapping"),
    kind: ParamKind::Int {
        min: 0,
        max: i64::MAX,
    },
    default: Value::Int(76),
    cli: Cli {
        long: "wrap",
        short: Some('w'),
    },
}];

static DECODE_PARAMS: &[Param] = &[Param {
    key: "ignore_garbage",
    label: en!("Ignore other characters"),
    help: en!("When decoding, ignore characters outside the Base64 alphabet"),
    kind: ParamKind::Flag,
    default: Value::Bool(false),
    cli: Cli {
        long: "ignore-garbage",
        short: Some('i'),
    },
}];

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
        Ok(Output::Text(String::from_utf8_lossy(&out).into_owned()))
    }

    fn run_stream(
        &self,
        ctx: &Ctx,
        input: &mut dyn Read,
        out: &mut dyn Write,
        values: &Values,
    ) -> Result<(), Error> {
        let wrap = usize::try_from(values.int(ENCODE_PARAMS, "wrap"))
            .map_err(|_| Error::usage("invalid wrap size"))?;
        let mut buf = vec![0; CHUNK];
        let mut held = 0; // bytes at the start of `buf` not yet encoded
        let mut encoded = Vec::new();
        let mut column = 0;
        let mut wrote = false;
        loop {
            ctx.check()?;
            let n = read_some(input, &mut buf[held..])?;
            let end = held + n;
            let last = n == 0;
            let take = if last { end } else { end - end % 3 };
            encoded.clear();
            encoded.resize(data_encoding::BASE64.encode_len(take), 0);
            data_encoding::BASE64.encode_mut(&buf[..take], &mut encoded);
            wrote |= !encoded.is_empty();
            write_wrapped(out, &encoded, wrap, &mut column)?;
            buf.copy_within(take..end, 0);
            held = end - take;
            if last {
                break;
            }
        }
        if wrap > 0 && wrote && column > 0 {
            out.write_all(b"\n")?;
        }
        Ok(())
    }
}

fn write_wrapped(
    out: &mut dyn Write,
    mut text: &[u8],
    wrap: usize,
    column: &mut usize,
) -> std::io::Result<()> {
    if wrap == 0 {
        return out.write_all(text);
    }
    while !text.is_empty() {
        let room = wrap - *column;
        let n = room.min(text.len());
        out.write_all(&text[..n])?;
        *column += n;
        text = &text[n..];
        if *column == wrap {
            out.write_all(b"\n")?;
            *column = 0;
        }
    }
    Ok(())
}

impl Tool for Decode {
    fn meta(&self) -> &'static Meta {
        &DECODE_META
    }

    fn params(&self) -> &'static [Param] {
        DECODE_PARAMS
    }

    fn run(&self, ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let mut out = Vec::new();
        self.run_stream(ctx, &mut &input[..], &mut out, values)?;
        Ok(Output::Bytes(out))
    }

    fn run_stream(
        &self,
        ctx: &Ctx,
        input: &mut dyn Read,
        out: &mut dyn Write,
        values: &Values,
    ) -> Result<(), Error> {
        let ignore_garbage = values.flag(DECODE_PARAMS, "ignore_garbage");
        let mut buf = vec![0; CHUNK];
        let mut quantum = Quantum::default();
        let mut decoded = Vec::with_capacity(CHUNK);
        loop {
            ctx.check()?;
            let n = read_some(input, &mut buf)?;
            if n == 0 {
                return quantum.finish(out);
            }
            decoded.clear();
            let result = buf[..n]
                .iter()
                .filter(|&&c| c != b'\n' && (!ignore_garbage || c == b'=' || value(c).is_some()))
                .try_for_each(|&c| quantum.push(c, &mut decoded));
            out.write_all(&decoded)?;
            result?;
        }
    }
}

fn value(c: u8) -> Option<u8> {
    ALPHABET.iter().position(|&a| a == c).map(|i| i as u8)
}

fn invalid() -> Error {
    Error::input("invalid input")
}

/// The characters of the quantum being read: up to four.
#[derive(Default)]
struct Quantum {
    chars: [u8; 4],
    len: usize,
}

impl Quantum {
    fn push(&mut self, c: u8, out: &mut Vec<u8>) -> Result<(), Error> {
        self.chars[self.len] = c;
        self.len += 1;
        if self.len == 4 {
            let complete = self.decode(out);
            self.len = 0;
            complete?;
        }
        Ok(())
    }

    /// The end of the input: a quantum of two or three characters is decoded
    /// as if padded.
    fn finish(&mut self, out: &mut dyn Write) -> Result<(), Error> {
        let mut decoded = Vec::new();
        let result = match self.len {
            0 => Ok(()),
            1 => Err(invalid()),
            _ => self.decode(&mut decoded),
        };
        out.write_all(&decoded)?;
        result
    }

    /// Writes what the quantum's leading alphabet characters give, then says
    /// whether the quantum was valid: complete, or padded (or at the end) with
    /// its unused bits zero.
    fn decode(&self, out: &mut Vec<u8>) -> Result<(), Error> {
        let q = &self.chars[..self.len];
        let values: Vec<u8> = q.iter().map_while(|&c| value(c)).collect();
        let bits = values.iter().fold(0u32, |acc, &v| acc << 6 | u32::from(v));
        let bytes = match values.len() {
            4 => [(bits >> 16) as u8, (bits >> 8) as u8, bits as u8].to_vec(),
            3 => [(bits >> 10) as u8, (bits >> 2) as u8].to_vec(),
            2 => [(bits >> 4) as u8].to_vec(),
            _ => Vec::new(),
        };
        out.extend_from_slice(&bytes);
        let padding = &q[values.len()..];
        let padded = padding.iter().all(|&c| c == b'=') && q.len() == 4 || padding.is_empty();
        let unused_zero = match values.len() {
            3 => bits & 0b11 == 0,
            2 => bits & 0b1111 == 0,
            _ => true,
        };
        if values.len() >= 2 && padded && unused_zero {
            Ok(())
        } else {
            Err(invalid())
        }
    }
}

fn read_some(input: &mut dyn Read, buf: &mut [u8]) -> std::io::Result<usize> {
    loop {
        match input.read(buf) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(input: &[u8], wrap: i64) -> String {
        let mut v = Values::new();
        v.set("wrap", Value::Int(wrap));
        match Encode.run(&Ctx::new(), input, &v) {
            Ok(Output::Text(s)) => s,
            other => panic!("{other:?}"),
        }
    }

    /// (decoded bytes, valid)
    fn decode(input: &str, ignore_garbage: bool) -> (Vec<u8>, bool) {
        let mut v = Values::new();
        v.set("ignore_garbage", Value::Bool(ignore_garbage));
        let mut out = Vec::new();
        let ok = Decode
            .run_stream(&Ctx::new(), &mut input.as_bytes(), &mut out, &v)
            .is_ok();
        (out, ok)
    }

    #[test]
    fn encodes_and_wraps_as_gnu_does() {
        assert_eq!(encode(b"", 76), "");
        assert_eq!(encode(b"a", 76), "YQ==\n");
        assert_eq!(encode(b"abc", 0), "YWJj");
        assert_eq!(encode(b"abc", 2), "YW\nJj\n");
        assert_eq!(encode(b"ab", 3), "YWI\n=\n");
        let zeros = encode(&[0; 60], 76);
        assert_eq!(zeros, format!("{}\n{}\n", "A".repeat(76), "A".repeat(4)));
    }

    /// Each case as GNU coreutils 9.7 decodes it (`printf '…' | base64 -d`).
    #[test]
    fn decodes_as_gnu_does() {
        let cases: &[(&str, bool, &[u8], bool)] = &[
            ("YQ==", false, b"a", true),
            ("YQ", false, b"a", true),
            ("YWJj", false, b"abc", true),
            ("YW\nJj\n\n", false, b"abc", true),
            ("YQ==YQ==", false, b"aa", true),
            ("YQ==YQ", false, b"aa", true),
            ("YQ=\n=", false, b"a", true),
            ("YWJjZA", false, b"abcd", true),
            ("YWI", false, b"ab", true),
            ("YWI=", false, b"ab", true),
            ("", false, b"", true),
            ("\n\n", false, b"", true),
            ("Y", false, b"", false),
            ("YQ=", false, b"a", false),
            ("YWJ", false, b"ab", false),
            ("YWJ=", false, b"ab", false),
            ("YR==", false, b"a", false),
            ("YR", false, b"a", false),
            ("YQ===", false, b"a", false),
            ("=", false, b"", false),
            ("YQ==Y", false, b"a", false),
            ("YWJjZG*", false, b"abcd", false),
            ("YW Jj", false, b"a", false),
            ("YW\rJj", false, b"a", false),
            ("YQ== ", false, b"a", false),
            ("YWJj=", false, b"abc", false),
            ("_-", false, b"", false),
            ("YW*Jj", true, b"abc", true),
            ("YQ=*=", true, b"a", true),
            ("*Y*Q*", true, b"a", true),
            ("Y=Q=", true, b"", false),
            ("====", true, b"", false),
            ("YWJj====", true, b"abc", false),
        ];
        for &(input, garbage, want, valid) in cases {
            assert_eq!(decode(input, garbage), (want.to_vec(), valid), "{input:?}");
        }
    }

    #[test]
    fn round_trips_across_chunks() {
        let data: Vec<u8> = (0..CHUNK * 2 + 7).map(|i| (i * 31 % 251) as u8).collect();
        let text = encode(&data, 76);
        assert_eq!(decode(&text, false), (data, true));
    }
}
