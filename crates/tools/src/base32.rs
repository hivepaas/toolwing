//! `base32`, as GNU coreutils' `base32` (drop-in: tools.md): RFC 4648's
//! alphabet, with the rules for writing and reading of basen.rs, shared
//! with `base64`.

use std::io::{Read, Write};

use crate::basen::BASE32;
use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, InputKind, Meta, Output, Param, ParamKind, Reference, Tool,
    Value, Values, en,
};

const REFERENCE: Option<Reference> = Some(Reference {
    name: "GNU coreutils base32",
    version: "9.7",
    compat: Compat::DropIn,
});

pub struct Encode;
pub struct Decode;

static ENCODE_META: Meta = Meta {
    id: "base32.encode",
    group: "base32",
    action: "encode",
    action_flag: None,
    title: en!("Base32 encode"),
    summary: en!("Encode bytes as Base32 (RFC 4648)"),
    category: Category::Encoding,
    keywords: &["b32", "base32", "encode"],
    input: InputKind::Bytes,
    streams: true,
    reference: REFERENCE,
};

static DECODE_META: Meta = Meta {
    id: "base32.decode",
    group: "base32",
    action: "decode",
    action_flag: Some(('d', "decode")),
    title: en!("Base32 decode"),
    summary: en!("Decode Base32 back to bytes"),
    category: Category::Encoding,
    keywords: &["b32", "base32", "decode"],
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
    help: en!("When decoding, ignore characters outside the Base32 alphabet"),
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
        BASE32.encode(ctx, input, out, wrap)
    }
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
        BASE32.decode(ctx, input, out, ignore_garbage)
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
        assert_eq!(encode(b"foobar", 76), "MZXW6YTBOI======\n");
        assert_eq!(encode(b"hello", 0), "NBSWY3DP");
        assert_eq!(
            encode(&[b'a'; 100], 20).lines().next(),
            Some("MFQWCYLBMFQWCYLBMFQW")
        );
    }

    /// Each case as GNU coreutils 9.7 decodes it (`printf '…' | base32 -d`).
    #[test]
    fn decodes_as_gnu_does() {
        let cases: &[(&str, bool, &[u8], bool)] = &[
            ("MZXW6===", false, b"foo", true),
            ("MZXW6", false, b"foo", true),
            ("MZXW6YQ=", false, b"foob", true),
            ("MZXW6YQ", false, b"foob", true),
            ("MZXW6YTB", false, b"fooba", true),
            ("MZXW6YTBOI======", false, b"foobar", true),
            ("MZXW6YTBOI", false, b"foobar", true),
            ("MY======", false, b"f", true),
            ("MY", false, b"f", true),
            ("MZXQ====", false, b"fo", true),
            ("MZXA====", false, b"fn", true),
            ("MZXW6===MZXW6===", false, b"foofoo", true),
            ("MZXW\n6===\n", false, b"foo", true),
            ("", false, b"", true),
            ("MZ======", false, b"f", false),
            ("M=======", false, b"", false),
            ("MZX", false, b"f", false),
            ("MZXW6Y", false, b"foo", false),
            ("MZXW6YT", false, b"foob", false),
            ("mzxw6===", false, b"", false),
            ("MZXW 6===", false, b"fo", false),
            ("MZXW6===\nextra", false, b"foo", false),
            ("========", false, b"", false),
            ("MZXW61==", false, b"foo", false),
            ("MZXW6==", false, b"", false),
            ("MY=", false, b"", false),
            ("MZXW6YTBOI=", false, b"fooba", false),
            ("MZXW6 junk!==", true, b"", false),
            ("MZXW6 junk!===", true, b"foo", true),
            ("MZ*XW6===", true, b"foo", true),
        ];
        for &(input, garbage, want, valid) in cases {
            assert_eq!(decode(input, garbage), (want.to_vec(), valid), "{input:?}");
        }
    }

    #[test]
    fn round_trips_across_chunks() {
        let data: Vec<u8> = (0..crate::basen::CHUNK * 2 + 7)
            .map(|i| (i * 31 % 251) as u8)
            .collect();
        let text = encode(&data, 76);
        assert_eq!(decode(&text, false), (data, true));
    }
}
