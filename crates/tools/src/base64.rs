//! `base64`, as GNU coreutils' `base64` (drop-in: tools.md). The rules for
//! writing and reading are basen.rs's, shared with `base32`.

use std::io::{Read, Write};

use crate::basen::BASE64;
use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, InputKind, Meta, Output, Param, ParamKind, Reference, Tool,
    Value, Values, en,
};

const REFERENCE: Option<Reference> = Some(Reference {
    name: "GNU coreutils base64",
    version: "9.7",
    compat: Compat::DropIn,
});

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
        BASE64.encode(ctx, input, out, wrap)
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
        BASE64.decode(ctx, input, out, ignore_garbage)
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
        let data: Vec<u8> = (0..crate::basen::CHUNK * 2 + 7)
            .map(|i| (i * 31 % 251) as u8)
            .collect();
        let text = encode(&data, 76);
        assert_eq!(decode(&text, false), (data, true));
    }
}
