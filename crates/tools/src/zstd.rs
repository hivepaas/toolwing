//! `zstd`: Zstandard, by the zstd command's own library (libzstd 1.5.7, as
//! zstd-sys bundles it, the version Debian 13's zstd is), with the
//! parameters its command line sets, so a frame is byte for byte the
//! command's: its level, a checksum, the content size when it is known, and
//! its multi-threaded mode (whose output does not depend on how many
//! threads). Also what `zstd -l` reads from a file's frames.

use std::io::{self, Read, Write};

use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, InputKind, Meta, Output, Param, ParamKind, Reference, Score,
    Tool, Value, Values, en,
};
use zstd::zstd_safe::{CParameter, DParameter};

/// What the command sets for a compression.
#[derive(Clone, Debug)]
pub struct Compress {
    pub level: i32,
    pub checksum: bool,
    pub content_size: bool,
    /// 0 for `--single-thread`, whose frames differ; any more give the same.
    pub workers: u32,
    /// `--long`'s window log; 0 when not given.
    pub long_window_log: u32,
    pub job_size: u32,
    pub size_hint: u32,
    /// The size to pledge: a file's, or `--stream-size`.
    pub pledged: Option<u64>,
    pub dict: Option<Vec<u8>>,
}

impl Default for Compress {
    fn default() -> Self {
        Compress {
            level: 3,
            checksum: true,
            content_size: true,
            workers: 1,
            long_window_log: 0,
            job_size: 0,
            size_hint: 0,
            pledged: None,
            dict: None,
        }
    }
}

/// `input` compressed into `out` as the command compresses it; the bytes
/// read and written.
pub fn compress(input: &mut dyn Read, out: &mut dyn Write, o: &Compress) -> io::Result<(u64, u64)> {
    let mut counter = Counter { inner: out, n: 0 };
    let read = {
        let mut enc = match &o.dict {
            Some(d) => zstd::stream::Encoder::with_dictionary(&mut counter, o.level, d)?,
            None => zstd::stream::Encoder::new(&mut counter, o.level)?,
        };
        enc.set_parameter(CParameter::ContentSizeFlag(o.content_size))?;
        enc.set_parameter(CParameter::DictIdFlag(true))?;
        enc.set_parameter(CParameter::ChecksumFlag(o.checksum))?;
        if o.size_hint > 0 {
            enc.set_parameter(CParameter::SrcSizeHint(o.size_hint))?;
        }
        if o.long_window_log > 0 {
            enc.set_parameter(CParameter::EnableLongDistanceMatching(true))?;
            enc.set_parameter(CParameter::WindowLog(o.long_window_log))?;
        }
        enc.set_parameter(CParameter::EnableDedicatedDictSearch(true))?;
        enc.set_parameter(CParameter::NbWorkers(o.workers))?;
        if o.workers > 0 {
            enc.set_parameter(CParameter::JobSize(o.job_size))?;
        }
        if let Some(n) = o.pledged {
            enc.set_pledged_src_size(Some(n))?;
        }
        let read = io::copy(input, &mut enc)?;
        enc.finish()?;
        read
    };
    Ok((read, counter.n))
}

struct Counter<'a> {
    inner: &'a mut dyn Write,
    n: u64,
}

impl Write for Counter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.n += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// What decompressing needs: the largest window to accept (`--memory`,
/// `--long`; 128 MiB by default), a dictionary, and the checksum checked.
#[derive(Clone, Debug)]
pub struct Decompress {
    /// The largest window a frame may need, in bytes.
    pub max_window: u64,
    pub dict: Option<Vec<u8>>,
    pub check: bool,
}

impl Default for Decompress {
    fn default() -> Self {
        Decompress {
            max_window: 1 << 27,
            dict: None,
            check: true,
        }
    }
}

/// Why a frame could not be read.
#[derive(Debug)]
pub enum Decode {
    /// Not a zstd frame where one should start.
    Format,
    /// Nothing at all.
    Empty,
    /// The frame needs a larger window than allowed.
    Window(u64),
    Io(io::Error),
    Corrupt(String),
}

const MAGIC: u32 = 0xFD2F_B528;

fn is_skippable(m: u32) -> bool {
    m & 0xFFFF_FFF0 == 0x184D_2A50
}

/// Every frame of `input` decompressed into `out`, skippable ones skipped;
/// the bytes written.
pub fn decompress(
    input: &mut dyn Read,
    out: &mut dyn Write,
    o: &Decompress,
) -> Result<u64, Decode> {
    let mut input = io::BufReader::with_capacity(1 << 17, input);
    let mut written = 0u64;
    let mut first = true;
    loop {
        let head = {
            let buf = std::io::BufRead::fill_buf(&mut input).map_err(Decode::Io)?;
            buf.to_vec()
        };
        if head.is_empty() {
            return if first {
                Err(Decode::Empty)
            } else {
                Ok(written)
            };
        }
        first = false;
        if head.len() < 4 {
            return Err(Decode::Format);
        }
        let magic = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        if magic != MAGIC && !is_skippable(magic) {
            return Err(Decode::Format);
        }
        if let Some(w) = window_size(&head)
            && w > o.max_window
        {
            return Err(Decode::Window(w));
        }
        let mut dec = match &o.dict {
            Some(d) => zstd::stream::read::Decoder::with_dictionary(&mut input, d),
            None => zstd::stream::read::Decoder::with_buffer(&mut input),
        }
        .map_err(Decode::Io)?
        .single_frame();
        // The limit is checked above, in bytes, as the command checks it.
        dec.window_log_max(31).map_err(Decode::Io)?;
        if !o.check {
            dec.set_parameter(DParameter::ForceIgnoreChecksum(true))
                .map_err(Decode::Io)?;
        }
        // In the command's output buffer (ZSTD_DStreamOutSize, 128 KiB): a
        // frame's last piece is written only once its checksum is checked.
        let mut buf = vec![0u8; 1 << 17];
        loop {
            let n = dec.read(&mut buf).map_err(|e| match e.kind() {
                io::ErrorKind::UnexpectedEof => Decode::Corrupt("premature end".into()),
                io::ErrorKind::Other => Decode::Corrupt(e.to_string()),
                _ if e.get_ref().is_some() => Decode::Corrupt(e.to_string()),
                _ => Decode::Io(e),
            })?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(Decode::Io)?;
            written += n as u64;
        }
        dec.finish();
    }
}

/// A frame header's window size, from its first bytes.
fn window_size(h: &[u8]) -> Option<u64> {
    if h.len() < 6 || u32::from_le_bytes([h[0], h[1], h[2], h[3]]) != MAGIC {
        return None;
    }
    let fhd = h[4];
    let single = fhd & 0x20 != 0;
    if single {
        let fcs = match fhd >> 6 {
            0 => 1,
            1 => 2,
            2 => 4,
            _ => 8,
        };
        let dict = [0, 1, 2, 4][usize::from(fhd & 3)];
        let at = 5 + dict;
        let b = h.get(at..at + fcs)?;
        let mut v = 0u64;
        for (i, &x) in b.iter().enumerate() {
            v |= u64::from(x) << (8 * i);
        }
        if fcs == 2 {
            v += 256;
        }
        return Some(v);
    }
    let wd = h[5];
    let exp = u32::from(wd >> 3) + 10;
    let base = 1u64 << exp;
    Some(base + base / 8 * u64::from(wd & 7))
}

/// What `zstd -l` says of a file.
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub frames: u32,
    pub skippable: u32,
    pub compressed: u64,
    pub decompressed: u64,
    pub decompressed_unknown: bool,
    pub uses_check: bool,
    pub checksum: [u8; 4],
    pub dict_id: u32,
    pub window: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum InfoError {
    /// A frame that cannot be read; what was read is still listed.
    Frame,
    NotZstd,
    Truncated,
}

/// The frames of a file's bytes, as `FIO_analyzeFrames` walks them: a seek
/// may pass the end (then the file is truncated), fewer than six bytes left
/// is not zstd, and a frame that cannot be read is an error after what was
/// read.
pub fn info(data: &[u8]) -> (Info, Option<InfoError>) {
    let mut info = Info {
        compressed: data.len() as u64,
        ..Info::default()
    };
    let len = data.len();
    let mut at = 0usize;
    loop {
        let n = len.saturating_sub(at).min(18);
        if n < 6 {
            if n == 0 && len > 0 {
                return (info, (at != len).then_some(InfoError::Truncated));
            }
            return (info, Some(InfoError::NotZstd));
        }
        let rest = &data[at..at + n];
        let magic = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
        if is_skippable(magic) {
            // Its size needs 8 bytes; the reference reads past what it has.
            let Some(sz) = rest.get(4..8) else {
                return (info, Some(InfoError::Frame));
            };
            let size = u32::from_le_bytes([sz[0], sz[1], sz[2], sz[3]]);
            at += 8 + size as usize;
            info.skippable += 1;
            continue;
        }
        if magic != MAGIC {
            return (info, Some(InfoError::NotZstd));
        }
        let fhd = rest[4];
        let single = fhd & 0x20 != 0;
        let dict_len = [0usize, 1, 2, 4][usize::from(fhd & 3)];
        let fcs_len = match fhd >> 6 {
            0 if single => 1,
            0 => 0,
            1 => 2,
            2 => 4,
            _ => 8,
        };
        let header = 5 + usize::from(!single) + dict_len + fcs_len;
        // A header cut short, or its reserved bit set, cannot be read.
        let Some(h) = rest.get(..header).filter(|_| fhd & 8 == 0) else {
            info.decompressed_unknown = true;
            return (info, Some(InfoError::Frame));
        };
        let le = |b: &[u8]| {
            b.iter()
                .enumerate()
                .fold(0u64, |v, (i, &x)| v | u64::from(x) << (8 * i))
        };
        let p = 5 + usize::from(!single);
        let dict_id = le(&h[p..p + dict_len]) as u32;
        if fcs_len == 0 {
            info.decompressed_unknown = true;
        } else {
            let v = le(&h[p + dict_len..header]);
            // As the reference's 64-bit sum: it wraps.
            info.decompressed =
                info.decompressed
                    .wrapping_add(if fcs_len == 2 { v + 256 } else { v });
        }
        info.dict_id = if info.dict_id != 0 && info.dict_id != dict_id {
            0
        } else {
            dict_id
        };
        info.window = window_size(h).unwrap_or(0);
        let mut q = at + header;
        loop {
            let Some(b) = data.get(q..q + 3) else {
                return (info, Some(InfoError::Frame));
            };
            let bh = u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16;
            let ty = (bh >> 1) & 3;
            if ty == 3 {
                return (info, Some(InfoError::Frame));
            }
            q += 3 + if ty == 1 { 1 } else { (bh >> 3) as usize };
            if bh & 1 == 1 {
                break;
            }
        }
        if fhd & 4 != 0 {
            info.uses_check = true;
            match data.get(q..q + 4) {
                Some(c) => info.checksum.copy_from_slice(c),
                None => return (info, Some(InfoError::Frame)),
            }
            q += 4;
        }
        info.frames += 1;
        at = q;
    }
}

/// `UTIL_makeHumanReadableSize`: a value, its precision and suffix.
pub fn human(size: u64, verbose: bool) -> (f64, usize, &'static str) {
    if verbose {
        if size >= 1 << 53 {
            return (size as f64 / f64::from(1u32 << 20), 2, " MiB");
        }
        return (size as f64, 0, " B");
    }
    let units: [(u64, &str); 6] = [
        (1 << 60, " EiB"),
        (1 << 50, " PiB"),
        (1 << 40, " TiB"),
        (1 << 30, " GiB"),
        (1 << 20, " MiB"),
        (1 << 10, " KiB"),
    ];
    let (value, suffix) = units
        .iter()
        .find(|(u, _)| size >= *u)
        .map_or((size as f64, " B"), |(u, s)| (size as f64 / *u as f64, *s));
    let precision = if value >= 100.0 || value as u64 == size {
        0
    } else if value >= 10.0 {
        1
    } else if value > 1.0 {
        2
    } else {
        3
    };
    (value, precision, suffix)
}

// ---- The app's tools --------------------------------------------------------------

static REFERENCE: Option<Reference> = Some(Reference {
    name: "zstd",
    version: "1.5.7",
    compat: Compat::DropIn,
});

const fn meta(
    action: &'static str,
    id: &'static str,
    title: &'static str,
    summary: &'static str,
    input: InputKind,
) -> Meta {
    Meta {
        id,
        group: "zstd",
        action,
        action_flag: None,
        title,
        summary,
        category: Category::Encoding,
        keywords: &["zstd", "zstandard", "compress", "decompress", "zst"],
        input,
        streams: false,
        reference: REFERENCE,
    }
}

static COMPRESS_META: Meta = meta(
    "compress",
    "zstd.compress",
    en!("Zstandard compress"),
    en!("Data compressed as the zstd command compresses it, as Base64 to paste"),
    InputKind::Bytes,
);
static DECOMPRESS_META: Meta = meta(
    "decompress",
    "zstd.decompress",
    en!("Zstandard decompress"),
    en!("A zstd frame, as Base64 or as it is, decompressed"),
    InputKind::Bytes,
);

static COMPRESS_PARAMS: &[Param] = &[Param {
    key: "level",
    label: en!("Level"),
    help: en!("1 (fastest) to 19 (smallest); 3 as zstd"),
    kind: ParamKind::Int { min: 1, max: 19 },
    default: Value::Int(3),
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

    /// The frame `zstd -c -LEVEL < input` writes (the size is not known
    /// from a pipe, so not written), as Base64.
    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let o = Compress {
            level: values.int(COMPRESS_PARAMS, "level").clamp(1, 19) as i32,
            ..Compress::default()
        };
        let mut out = Vec::new();
        compress(&mut &input[..], &mut out, &o).map_err(|e| Error::input(e.to_string()))?;
        Ok(Output::Text(data_encoding::BASE64.encode(&out)))
    }

    fn command_args(&self, values: &Values) -> Option<Vec<String>> {
        let level = values.int(COMPRESS_PARAMS, "level");
        let mut args = vec!["zstd".to_owned(), "-c".to_owned()];
        if level != 3 {
            args.push(format!("-{level}"));
        }
        Some(args)
    }
}

impl Tool for DecompressTool {
    fn meta(&self) -> &'static Meta {
        &DECOMPRESS_META
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], _values: &Values) -> Result<Output, Error> {
        // Base64 as pasted, or the frame's bytes.
        let text: Vec<u8> = input
            .iter()
            .copied()
            .filter(|b| !b.is_ascii_whitespace())
            .collect();
        let bytes = data_encoding::BASE64
            .decode(&text)
            .unwrap_or_else(|_| input.to_vec());
        let mut out = Vec::new();
        decompress(&mut &bytes[..], &mut out, &Decompress::default()).map_err(|e| match e {
            Decode::Format | Decode::Empty => Error::input("not a zstd frame (nor Base64 of one)"),
            Decode::Window(w) => Error::input(format!(
                "the frame's window is {w} bytes, more than 128 MiB"
            )),
            Decode::Io(e) => Error::input(e.to_string()),
            Decode::Corrupt(m) => Error::input(format!("the frame is damaged: {m}")),
        })?;
        Ok(match String::from_utf8(out) {
            Ok(s) => Output::Text(s),
            Err(e) => Output::Bytes(e.into_bytes()),
        })
    }

    fn command_args(&self, _values: &Values) -> Option<Vec<String>> {
        Some(vec!["zstd".to_owned(), "-dc".to_owned()])
    }

    fn detect(&self, sample: &[u8]) -> Option<Score> {
        sample
            .starts_with(&MAGIC.to_le_bytes())
            .then_some(Score(90))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_list() {
        let data = b"hello world, hello world, hello world\n".repeat(100);
        let mut z = Vec::new();
        let o = Compress {
            pledged: Some(data.len() as u64),
            ..Compress::default()
        };
        compress(&mut &data[..], &mut z, &o).unwrap_or_default();
        let mut back = Vec::new();
        let n = decompress(&mut &z[..], &mut back, &Decompress::default())
            .map_err(|e| format!("{e:?}"));
        assert_eq!(n, Ok(data.len() as u64));
        assert_eq!(back, data);
        let (i, err) = info(&z);
        assert_eq!(err, None);
        assert_eq!(
            (i.frames, i.decompressed, i.uses_check),
            (1, data.len() as u64, true)
        );
        let mut two = z.clone();
        two.extend_from_slice(&z);
        let mut back = Vec::new();
        assert!(decompress(&mut &two[..], &mut back, &Decompress::default()).is_ok());
        assert_eq!(back.len(), data.len() * 2);
        assert!(matches!(
            decompress(
                &mut &b"not zstd"[..],
                &mut Vec::new(),
                &Decompress::default()
            ),
            Err(Decode::Format)
        ));
        assert_eq!(human(1 << 20, false), (1.0, 3, " MiB"));
        assert_eq!(human(1536, false), (1.5, 2, " KiB"));
        assert_eq!(human(500, false), (500.0, 0, " B"));
    }
}
