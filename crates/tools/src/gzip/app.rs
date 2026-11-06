//! The app's gzip tools: compress to Base64 to paste, and decompress what is
//! pasted (Base64 or the bytes), every format `gunzip` reads.

use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, InputKind, Meta, Output, Param, ParamKind, Reference, Score,
    Tool, Value, Values, en,
};

use super::{Compress, Diag, Gzip, Io, Work};

static REFERENCE: Option<Reference> = Some(Reference {
    name: "gzip",
    version: "1.13",
    compat: Compat::DropIn,
});

const fn meta(
    action: &'static str,
    id: &'static str,
    title: &'static str,
    summary: &'static str,
) -> Meta {
    Meta {
        id,
        group: "gzip",
        action,
        action_flag: None,
        title,
        summary,
        category: Category::Encoding,
        keywords: &[
            "gzip",
            "gunzip",
            "zcat",
            "deflate",
            "gz",
            "compress",
            "decompress",
        ],
        input: InputKind::Bytes,
        streams: false,
        reference: REFERENCE,
    }
}

static COMPRESS_META: Meta = meta(
    "compress",
    "gzip.compress",
    en!("gzip compress"),
    en!("Data compressed as gzip, as Base64 to paste"),
);
static DECOMPRESS_META: Meta = meta(
    "decompress",
    "gzip.decompress",
    en!("gzip decompress"),
    en!("A .gz (or .Z, pack, LZH, single-entry zip), as Base64 or as it is, decompressed"),
);

static COMPRESS_PARAMS: &[Param] = &[Param {
    key: "level",
    label: en!("Level"),
    help: en!("1 (fastest) to 9 (smallest); 6 as gzip"),
    kind: ParamKind::Int { min: 1, max: 9 },
    default: Value::Int(6),
    cli: Cli {
        long: "level",
        short: None,
    },
}];

pub struct CompressTool;
pub struct DecompressTool;

impl Tool for CompressTool {
    fn meta(&self) -> &'static Meta {
        &COMPRESS_META
    }

    fn params(&self) -> &'static [Param] {
        COMPRESS_PARAMS
    }

    /// What `gzip -c -LEVEL < input` writes from a pipe (no name, no
    /// time), as Base64.
    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let c = Compress {
            level: values.int(COMPRESS_PARAMS, "level").clamp(1, 9) as i32,
            ..Compress::default()
        };
        Ok(Output::Text(data_encoding::BASE64.encode(&c.run(input))))
    }

    fn command_args(&self, values: &Values) -> Option<Vec<String>> {
        let level = values.int(COMPRESS_PARAMS, "level");
        let mut args = vec!["gzip".to_owned(), "-c".to_owned()];
        if level != 6 {
            args.push(format!("-{level}"));
        }
        Some(args)
    }
}

/// Every member of `data` decompressed, as `gzip -dc` does from stdin; the
/// command's message when it fails.
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut gz = Gzip {
        decompress: true,
        to_stdout: true,
        no_name: true,
        no_time: true,
        diag: Diag::Kept(Vec::new()),
        ..Gzip::default()
    };
    gz.ifname = b"stdin".to_vec();
    gz.ofname = b"stdout".to_vec();
    let mut out = Vec::new();
    let mut input = data;
    let mut io = Io {
        input: &mut input,
        output: &mut out,
    };
    let said = |gz: &Gzip| match &gz.diag {
        Diag::Kept(v) => String::from_utf8_lossy(v).trim().to_owned(),
        Diag::Stderr => String::new(),
    };
    let mut work: Option<Work> = None;
    let r = (|| -> super::R<bool> {
        let mut method = gz.get_method(&mut io, &mut work)?;
        if method < 0 {
            return Ok(false);
        }
        loop {
            let Some(w) = work else { return Ok(false) };
            if w(&mut gz, &mut io)? != super::OK {
                return Ok(false);
            }
            if gz.input_eof(&mut io)? {
                return Ok(true);
            }
            method = gz.get_method(&mut io, &mut work)?;
            if method < 0 {
                // Trailing garbage or zeros: a warning, the data is fine.
                return Ok(method != -1);
            }
            gz.bytes_out = 0;
        }
    })();
    match r {
        Ok(true) => Ok(out),
        Ok(false) | Err(_) => Err(said(&gz)),
    }
}

impl Tool for DecompressTool {
    fn meta(&self) -> &'static Meta {
        &DECOMPRESS_META
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], _values: &Values) -> Result<Output, Error> {
        let text: Vec<u8> = input
            .iter()
            .copied()
            .filter(|b| !b.is_ascii_whitespace())
            .collect();
        let bytes = data_encoding::BASE64
            .decode(&text)
            .unwrap_or_else(|_| input.to_vec());
        let out = decompress(&bytes).map_err(|m| {
            let m = m.strip_prefix("gzip: stdin: ").unwrap_or(&m).to_owned();
            Error::input(if m.is_empty() {
                "not in gzip format (nor Base64 of it)".to_owned()
            } else {
                m
            })
        })?;
        Ok(match String::from_utf8(out) {
            Ok(s) => Output::Text(s),
            Err(e) => Output::Bytes(e.into_bytes()),
        })
    }

    fn command_args(&self, _values: &Values) -> Option<Vec<String>> {
        Some(vec!["gzip".to_owned(), "-dc".to_owned()])
    }

    fn detect(&self, sample: &[u8]) -> Option<Score> {
        sample.starts_with(&[0x1f, 0x8b]).then_some(Score(90))
    }
}
