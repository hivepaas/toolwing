//! `zstd`: Zstandard, by hivepack's port of the zstd command's own library
//! (libzstd 1.5.7, the version Debian 13's zstd is), with the
//! parameters its command line sets, so a frame is byte for byte the
//! command's: its level, a checksum, the content size when it is known, and
//! its multi-threaded mode (whose output does not depend on how many
//! threads). Also what `zstd -l` reads from a file's frames.

use std::io::{self, Read, Write};

use hivepack::zstd::{
    CCtx, CParameter, ContentSize, DCtx, DParameter, EndDirective, FRAMEHEADER_SIZE_MAX,
    FrameHeaderState, InBuffer, OutBuffer, ParamSwitch, ResetDirective, cstream_in_size,
    cstream_out_size, dstream_in_size, dstream_out_size, get_frame_content_size, get_frame_header,
};
use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, InputKind, Meta, Output, Param, ParamKind, Reference, Score,
    Tool, Value, Values, en,
};

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

/// `input` compressed into `out` as the command compresses it
/// (`FIO_compressZstdFrame`): read in pieces of `ZSTD_CStreamInSize`, the
/// last with `ZSTD_e_end`; the bytes read and written.
pub fn compress(input: &mut dyn Read, out: &mut dyn Write, o: &Compress) -> io::Result<(u64, u64)> {
    let mut cctx = CCtx::new();
    // As `FIO_createCResources` sets them, in its order.
    let set = |cctx: &mut CCtx, p: CParameter| cctx.set_parameter(p).map_err(io::Error::other);
    set(&mut cctx, CParameter::ContentSizeFlag(o.content_size))?;
    set(&mut cctx, CParameter::DictIdFlag(true))?;
    set(&mut cctx, CParameter::ChecksumFlag(o.checksum))?;
    set(&mut cctx, CParameter::CompressionLevel(o.level))?;
    if o.size_hint > 0 {
        set(&mut cctx, CParameter::SrcSizeHint(o.size_hint))?;
    }
    if o.long_window_log > 0 {
        set(
            &mut cctx,
            CParameter::EnableLongDistanceMatching(ParamSwitch::Enable),
        )?;
        set(&mut cctx, CParameter::WindowLog(o.long_window_log))?;
    }
    set(&mut cctx, CParameter::EnableDedicatedDictSearch(true))?;
    set(&mut cctx, CParameter::NbWorkers(o.workers))?;
    if o.workers > 0 {
        set(&mut cctx, CParameter::JobSize(o.job_size))?;
    }
    if let Some(d) = &o.dict {
        cctx.load_dictionary(d).map_err(io::Error::other)?;
    }
    if let Some(n) = o.pledged {
        cctx.set_pledged_src_size(n).map_err(io::Error::other)?;
    }
    let mut src = vec![0u8; cstream_in_size()];
    let mut dst = vec![0u8; cstream_out_size()];
    let (mut read, mut written) = (0u64, 0u64);
    loop {
        let n = fill(input, &mut src)?;
        read += n as u64;
        let end = n == 0 || o.pledged == Some(read);
        let directive = if end {
            EndDirective::End
        } else {
            EndDirective::Continue
        };
        let mut inb = InBuffer::new(&src[..n]);
        loop {
            let mut outb = OutBuffer::new(&mut dst);
            let left = cctx
                .compress_stream2(&mut outb, &mut inb, directive)
                .map_err(io::Error::other)?;
            let w = outb.pos;
            out.write_all(&dst[..w])?;
            written += w as u64;
            if inb.pos == n && (!end || left == 0) {
                break;
            }
        }
        if end {
            return Ok((read, written));
        }
    }
}

/// Whether an error of [`compress`] is the library's, not the input's or
/// the output's: the command ends at once on those (error 11).
pub fn is_library_error(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|x| x.is::<hivepack::zstd::Error>())
}

/// As much of `buf` as `input` fills: all of it, unless the input ends.
fn fill(input: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match input.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
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
    /// Fewer than 4 bytes where a frame should start.
    Short,
    /// The input ends inside a frame.
    Premature,
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

/// Every frame of `input` decompressed into `out`, skippable ones skipped,
/// as `FIO_decompressFrames` reads them; the bytes written.
pub fn decompress(
    input: &mut dyn Read,
    out: &mut dyn Write,
    o: &Decompress,
) -> Result<u64, Decode> {
    let corrupt = |e: hivepack::zstd::Error| Decode::Corrupt(e.to_string());
    let mut dctx = DCtx::new();
    // The limit is checked below, in bytes, as the command checks it.
    dctx.set_parameter(DParameter::WindowLogMax(31))
        .map_err(corrupt)?;
    if !o.check {
        dctx.set_parameter(DParameter::ForceIgnoreChecksum(true))
            .map_err(corrupt)?;
    }
    if let Some(d) = &o.dict {
        dctx.load_dictionary(d).map_err(corrupt)?;
    }
    let mut src = Source {
        input,
        buf: vec![0u8; dstream_in_size()],
        start: 0,
        end: 0,
    };
    // The command's output buffer (`ZSTD_DStreamOutSize`): a frame's last
    // piece is written only once its checksum is checked.
    let mut dst = vec![0u8; dstream_out_size()];
    let mut written = 0u64;
    let mut first = true;
    loop {
        src.fill(4).map_err(Decode::Io)?;
        let head = src.loaded();
        if head.is_empty() {
            return if first {
                Err(Decode::Empty)
            } else {
                Ok(written)
            };
        }
        first = false;
        if head.len() < 4 {
            return Err(Decode::Short);
        }
        let magic = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        if magic != MAGIC && !is_skippable(magic) {
            return Err(Decode::Format);
        }
        dctx.reset(ResetDirective::SessionOnly).map_err(corrupt)?;
        src.fill(FRAMEHEADER_SIZE_MAX).map_err(Decode::Io)?;
        if let Some(w) = window_size(src.loaded())
            && w > o.max_window
        {
            return Err(Decode::Window(w));
        }
        loop {
            let mut inb = InBuffer::new(src.loaded());
            let mut outb = OutBuffer::new(&mut dst);
            let hint = dctx
                .decompress_stream(&mut outb, &mut inb)
                .map_err(corrupt)?;
            let (r, w) = (inb.pos, outb.pos);
            out.write_all(&dst[..w]).map_err(Decode::Io)?;
            written += w as u64;
            src.start += r;
            if hint == 0 {
                break;
            }
            let want = hint.min(dstream_in_size());
            if src.loaded().len() < want && src.fill(want).map_err(Decode::Io)? == 0 {
                return Err(Decode::Premature);
            }
        }
    }
}

/// The command's read buffer: what is loaded and not yet decompressed.
struct Source<'a> {
    input: &'a mut dyn Read,
    buf: Vec<u8>,
    start: usize,
    end: usize,
}

impl Source<'_> {
    fn loaded(&self) -> &[u8] {
        &self.buf[self.start..self.end]
    }

    /// At least `n` bytes loaded (`AIO_ReadPool_fillBuffer`), unless the
    /// input ends first; the bytes this call read.
    fn fill(&mut self, n: usize) -> io::Result<usize> {
        if self.end - self.start >= n {
            return Ok(0);
        }
        self.buf.copy_within(self.start..self.end, 0);
        self.end -= self.start;
        self.start = 0;
        let before = self.end;
        let want = n.min(self.buf.len());
        while self.end < want {
            match self.input.read(&mut self.buf[self.end..]) {
                Ok(0) => break,
                Ok(k) => self.end += k,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(self.end - before)
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

/// Why `zstd -l` stopped reading a file, with what the command says of it
/// on stderr (`FIO_analyzeFrames`), when it says something.
#[derive(Debug, PartialEq, Eq)]
pub enum InfoError {
    /// A frame that cannot be read; what was read is still listed. Empty
    /// when the reference would read bytes it does not have (a skippable
    /// frame's size, cut short) and so has nothing to say.
    Frame(&'static str),
    /// Not zstd.
    NotZstd(Option<&'static str>),
    /// A frame's sizes go past the end of the file: where they lead.
    Truncated { position: u64, size: u64 },
}

impl InfoError {
    /// The command's message, as it ends it (with " \n").
    pub fn message(&self) -> Option<String> {
        match self {
            InfoError::Frame("") | InfoError::NotZstd(None) => None,
            InfoError::Frame(m) | InfoError::NotZstd(Some(m)) => Some(format!("{m} \n")),
            InfoError::Truncated { position, size } => Some(format!(
                "Error: seeked to position {position}, which is beyond file size of {size}\n \n"
            )),
        }
    }
}

/// The frames of a file's bytes, as `FIO_analyzeFrames` walks them: it reads
/// up to a largest frame header at a time, and seeks over blocks and
/// skippable frames, which may pass the end of the file (then the file is
/// truncated). A frame that cannot be read is an error after what was read.
/// With it, how many warnings of frames with different dictionaries it gives.
pub fn info(data: &[u8]) -> (Info, Option<InfoError>, u32) {
    let mut info = Info {
        compressed: data.len() as u64,
        ..Info::default()
    };
    let mut dict_warnings = 0u32;
    let len = data.len();
    let mut at = 0usize;
    loop {
        let rest = data.get(at..).unwrap_or_default();
        let h = &rest[..rest.len().min(FRAMEHEADER_SIZE_MAX)];
        if h.len() < 6 {
            if h.is_empty() && len > 0 {
                let e = (at != len).then_some(InfoError::Truncated {
                    position: at as u64,
                    size: len as u64,
                });
                return (info, e, dict_warnings);
            }
            let e = InfoError::NotZstd(Some("Error: reached end of file with incomplete frame"));
            return (info, Some(e), dict_warnings);
        }
        let magic = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
        if is_skippable(magic) {
            // Its size needs 8 bytes; the reference reads past what it has.
            let Some(sz) = h.get(4..8) else {
                return (info, Some(InfoError::Frame("")), dict_warnings);
            };
            let size = u32::from_le_bytes([sz[0], sz[1], sz[2], sz[3]]);
            at += 8 + size as usize;
            info.skippable += 1;
            continue;
        }
        if magic != MAGIC {
            return (info, Some(InfoError::NotZstd(None)), dict_warnings);
        }
        match get_frame_content_size(h) {
            ContentSize::Known(n) => info.decompressed = info.decompressed.wrapping_add(n),
            _ => info.decompressed_unknown = true,
        }
        let header = match get_frame_header(h) {
            Ok(FrameHeaderState::Complete(header)) => header,
            _ => {
                let e = InfoError::Frame("Error: could not decode frame header");
                return (info, Some(e), dict_warnings);
            }
        };
        if info.dict_id != 0 && info.dict_id != header.dict_id {
            dict_warnings += 1;
            info.dict_id = 0;
        } else {
            info.dict_id = header.dict_id;
        }
        info.window = header.window_size;
        let mut q = at + header.header_size as usize;
        loop {
            let Some(b) = data.get(q..q + 3) else {
                let e = InfoError::Frame("Error while reading block header");
                return (info, Some(e), dict_warnings);
            };
            let bh = u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16;
            let ty = (bh >> 1) & 3;
            if ty == 3 {
                let e = InfoError::Frame("Error: unsupported block type");
                return (info, Some(e), dict_warnings);
            }
            q += 3 + if ty == 1 { 1 } else { (bh >> 3) as usize };
            if bh & 1 == 1 {
                break;
            }
        }
        if h[4] & 4 != 0 {
            info.uses_check = true;
            match data.get(q..q + 4) {
                Some(c) => info.checksum.copy_from_slice(c),
                None => {
                    let e = InfoError::Frame("Error: could not read checksum");
                    return (info, Some(e), dict_warnings);
                }
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
            Decode::Format | Decode::Short | Decode::Empty => {
                Error::input("not a zstd frame (nor Base64 of one)")
            }
            Decode::Premature => Error::input("the frame is cut short"),
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
        let (i, err, dict_warnings) = info(&z);
        assert_eq!((err, dict_warnings), (None, 0));
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
