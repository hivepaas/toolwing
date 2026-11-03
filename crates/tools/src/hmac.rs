//! `hmac`: HMAC (RFC 2104) of the input with a key, the digest in hex as
//! `openssl dgst -hmac` writes it, or in Base64. The key is a secret: given
//! with `--key`, `--key-file` or `--key-env` (cli.md), and never shown.

use std::io::{Read, Write};

use hmac::{Hmac, KeyInit, Mac};
use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, InputKind, Meta, Output, Param, ParamKind, Reference, Tool,
    Value, Values, en,
};

use crate::basen::{CHUNK, read_some};

pub struct Digest;

static META: Meta = Meta {
    id: "hmac",
    group: "hmac",
    action: "",
    action_flag: None,
    title: en!("HMAC"),
    summary: en!("HMAC-SHA256 (or SHA-1, SHA-384, SHA-512, MD5) of the input with a key"),
    category: Category::Crypto,
    keywords: &[
        "hmac",
        "mac",
        "signature",
        "sign",
        "sha256",
        "webhook",
        "digest",
    ],
    input: InputKind::Bytes,
    streams: true,
    reference: Some(Reference {
        name: "openssl dgst -hmac",
        version: "3",
        compat: Compat::Output,
    }),
};

static PARAMS: &[Param] = &[
    Param {
        key: "key",
        label: en!("Key"),
        help: en!("The secret key"),
        kind: ParamKind::Secret,
        default: Value::Text(String::new()),
        cli: Cli {
            long: "key",
            short: None,
        },
    },
    Param {
        key: "algorithm",
        label: en!("Hash"),
        help: en!("The hash HMAC is made with; SHA-256 when left out"),
        kind: ParamKind::Choice {
            options: &[
                ("sha256", en!("SHA-256")),
                ("sha384", en!("SHA-384")),
                ("sha512", en!("SHA-512")),
                ("sha1", en!("SHA-1")),
                ("md5", en!("MD5")),
            ],
            flags: true,
            unset: None,
        },
        default: Value::Text(String::new()),
        cli: Cli {
            long: "algorithm",
            short: None,
        },
    },
    Param {
        key: "base64",
        label: en!("Base64"),
        help: en!("Write the digest in Base64, not hex"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "base64",
            short: None,
        },
    },
];

/// The digest of everything `input` gives.
fn mac<M: Mac + KeyInit>(key: &[u8], ctx: &Ctx, input: &mut dyn Read) -> Result<Vec<u8>, Error> {
    let mut m = <M as KeyInit>::new_from_slice(key).map_err(|e| Error::usage(e.to_string()))?;
    let mut buf = vec![0; CHUNK];
    loop {
        ctx.check()?;
        let n = read_some(input, &mut buf)?;
        if n == 0 {
            break;
        }
        m.update(&buf[..n]);
    }
    Ok(m.finalize().into_bytes().to_vec())
}

impl Digest {
    fn text(&self, ctx: &Ctx, input: &mut dyn Read, values: &Values) -> Result<String, Error> {
        let key = values.text(PARAMS, "key");
        if key.is_empty() {
            return Err(Error::usage("give the key: --key, --key-file or --key-env"));
        }
        let key = key.as_bytes();
        let digest = match values.text(PARAMS, "algorithm").as_str() {
            "sha1" => mac::<Hmac<sha1::Sha1>>(key, ctx, input)?,
            "sha384" => mac::<Hmac<sha2::Sha384>>(key, ctx, input)?,
            "sha512" => mac::<Hmac<sha2::Sha512>>(key, ctx, input)?,
            "md5" => mac::<Hmac<md5::Md5>>(key, ctx, input)?,
            _ => mac::<Hmac<sha2::Sha256>>(key, ctx, input)?,
        };
        Ok(if values.flag(PARAMS, "base64") {
            data_encoding::BASE64.encode(&digest)
        } else {
            data_encoding::HEXLOWER.encode(&digest)
        })
    }
}

impl Tool for Digest {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn run(&self, ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        Ok(Output::Text(self.text(ctx, &mut &input[..], values)?))
    }

    fn run_stream(
        &self,
        ctx: &Ctx,
        input: &mut dyn Read,
        out: &mut dyn Write,
        values: &Values,
    ) -> Result<(), Error> {
        writeln!(out, "{}", self.text(ctx, input, values)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hmac(key: &[u8], data: &[u8], algorithm: &str) -> String {
        let mut v = Values::new();
        v.set(
            "key",
            Value::Text(String::from_utf8_lossy(key).into_owned()),
        );
        v.set("algorithm", Value::Text(algorithm.into()));
        match Digest.run(&Ctx::new(), data, &v) {
            Ok(Output::Text(s)) => s,
            other => panic!("{other:?}"),
        }
    }

    /// RFC 4231's test case 2, and RFC 2202's for SHA-1 and MD5.
    #[test]
    fn as_the_rfcs_give_them() {
        let (key, data) = (b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hmac(key, data, ""),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(
            hmac(key, data, "sha512"),
            "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737"
        );
        assert_eq!(
            hmac(key, data, "sha384"),
            "af45d2e376484031617f78d2b58a6b1b9c7ef464f5a01b47e42ec3736322445e8e2240ca5e69e2c78b3239ecfab21649"
        );
        assert_eq!(
            hmac(key, data, "sha1"),
            "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79"
        );
        assert_eq!(hmac(key, data, "md5"), "750c783e6ab0b503eaa86e310a5db738");
    }

    #[test]
    fn base64_and_a_missing_key() {
        let mut v = Values::new();
        v.set("key", Value::Text("Jefe".into()));
        v.set("base64", Value::Bool(true));
        let out = Digest.run(&Ctx::new(), b"what do ya want for nothing?", &v);
        assert!(
            matches!(out, Ok(Output::Text(s)) if s == "W9zBRr9gdU5qBCQmCJV1x1oAPwidJzmDnexYuWTsOEM=")
        );
        assert!(Digest.run(&Ctx::new(), b"x", &Values::new()).is_err());
    }
}
