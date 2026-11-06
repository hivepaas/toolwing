//! `gzip`: GNU gzip 1.13, its decompression ported from its own decoders
//! (inflate, and the formats it also reads: compress's LZW `.Z`, pack, SCO
//! LZH and a zip archive's first entry), so a file decompresses, and fails,
//! as with the command: the same output written before an error, the same
//! messages. Compression is zlib's deflate (hivepack's port, zlib 1.3.2) in
//! gzip's header and trailer: a valid `.gz` that any gunzip reads, with the
//! command's name, time, flags and sizes, but not its deflate stream byte
//! for byte (gzip has its own encoder).
//!
//! The state is the command's: its buffers (`inbuf`, `window`, `outbuf` and
//! the others) and counters live as long as the command, from one file to
//! the next, as its globals do.
//!
//! Ported from GNU gzip 1.13: Copyright (C) 1997-2023 Free Software
//! Foundation, Inc.; Copyright (C) 1992-1993 Jean-loup Gailly; inflate and
//! unzip derived from code put in the public domain by Mark Adler; unlzh
//! from 'ar002' by Haruhiko Okumura. GPL-3.0-or-later, as toolwing.

mod app;
mod inflate;
mod unlzh;
mod unlzw;
mod unpack;
mod zip;

use std::io::{self, Read, Write};

pub use app::{CompressTool, DecompressTool, decompress};
pub use zip::Compress;

/// `INBUFSIZ`: how much is read at a time.
pub const INBUFSIZ: usize = 0x40000;
const INBUF_EXTRA: usize = 64;
/// `OUTBUFSIZ`.
pub const OUTBUFSIZ: usize = 0x40000;
const OUTBUF_EXTRA: usize = 2048;
/// `WSIZE`: inflate's window, written out each time it fills.
pub const WSIZE: usize = 0x8000;
/// LZW's largest code size.
const BITS: u32 = 16;

pub const OK: i32 = 0;
pub const ERROR: i32 = 1;
pub const WARNING: i32 = 2;

pub const STORED: i32 = 0;
pub const COMPRESSED: i32 = 1;
pub const PACKED: i32 = 2;
pub const LZHED: i32 = 3;
pub const DEFLATED: i32 = 8;

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];
const OLD_GZIP_MAGIC: [u8; 2] = [0x1f, 0x9e];
const PACK_MAGIC: [u8; 2] = [0x1f, 0x1e];
const LZW_MAGIC: [u8; 2] = [0x1f, 0x9d];
const LZH_MAGIC: [u8; 2] = [0x1f, 0xa0];
const PKZIP_MAGIC: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];

const HEADER_CRC: u8 = 0x02;
const EXTRA_FIELD: u8 = 0x04;
const ORIG_NAME: u8 = 0x08;
const COMMENT: u8 = 0x10;
const ENCRYPTED: u8 = 0x20;
const RESERVED: u8 = 0xC0;

/// The command stops (`abort_gzip`, `write_error`): its exit code. What it
/// had to say is said; the caller removes the output file it was writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stop(pub i32);

pub type R<T> = Result<T, Stop>;

/// Where a file is read from and written to.
pub struct Io<'a> {
    pub input: &'a mut dyn Read,
    pub output: &'a mut dyn Write,
}

/// A timestamp, as the command keeps one: unknown when `nsec` is negative.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub sec: i64,
    pub nsec: i64,
}

impl Stamp {
    pub const UNKNOWN: Stamp = Stamp { sec: 0, nsec: -1 };

    pub fn known(self) -> bool {
        self.nsec >= 0
    }
}

/// The command's state: its options, the file at hand, its buffers.
pub struct Gzip {
    // Options.
    pub program_name: String,
    pub to_stdout: bool,
    pub decompress: bool,
    pub force: u32,
    pub list: bool,
    pub no_name: bool,
    pub no_time: bool,
    pub verbose: u32,
    pub quiet: bool,
    pub test: bool,
    pub level: i32,
    pub exit_code: i32,

    // The file at hand.
    pub ifname: Vec<u8>,
    pub ofname: Vec<u8>,
    pub method: i32,
    pub last_member: bool,
    pub part_nb: u32,
    pub header_bytes: i64,
    pub time_stamp: Stamp,
    pub ifile_size: i64,
    pub save_orig_name: bool,
    pub unzip_crc: u32,
    pub bytes_in: i64,
    pub bytes_out: i64,

    // Buffers, as long-lived as the command's.
    pub insize: usize,
    pub inptr: usize,
    pub outcnt: usize,
    inbuf: Vec<u8>,
    outbuf: Vec<u8>,
    window: Vec<u8>,
    prev: Vec<u16>,
    crc: u32,

    // inflate.
    bb: u64,
    bk: u32,
    fresh: bool,
    // unzip: a zip archive's entry.
    pkzip: bool,
    ext_header: bool,
    // unpack and unlzh keep theirs in their modules' structs.
    unpack: unpack::State,
    unlzh: unlzh::State,

    /// What goes to stderr: written there, or kept (the app's).
    pub diag: Diag,
}

/// Where the command's messages go.
pub enum Diag {
    Stderr,
    Kept(Vec<u8>),
}

impl Default for Gzip {
    fn default() -> Self {
        Gzip {
            program_name: "gzip".into(),
            to_stdout: false,
            decompress: false,
            force: 0,
            list: false,
            no_name: false,
            no_time: false,
            verbose: 0,
            quiet: false,
            test: false,
            level: 6,
            exit_code: OK,
            ifname: Vec::new(),
            ofname: Vec::new(),
            method: DEFLATED,
            last_member: false,
            part_nb: 0,
            header_bytes: 0,
            time_stamp: Stamp::UNKNOWN,
            ifile_size: -1,
            save_orig_name: false,
            unzip_crc: 0,
            bytes_in: 0,
            bytes_out: 0,
            insize: 0,
            inptr: 0,
            outcnt: 0,
            // Two more than the C's: unlzw reads 3 bytes from the last one.
            inbuf: vec![0; INBUFSIZ + INBUF_EXTRA + 2],
            outbuf: vec![0; OUTBUFSIZ + OUTBUF_EXTRA],
            window: vec![0; 2 * WSIZE],
            prev: vec![0; 1 << BITS],
            crc: 0,
            bb: 0,
            bk: 0,
            fresh: false,
            pkzip: false,
            ext_header: false,
            unpack: unpack::State::default(),
            unlzh: unlzh::State::default(),
            diag: Diag::Stderr,
        }
    }
}

/// The C library's words for an error (`strerror`), without Rust's
/// " (os error N)".
pub fn strerror(e: &io::Error) -> String {
    // The same words on every system, as glibc's.
    match e.kind() {
        io::ErrorKind::NotFound => return "No such file or directory".into(),
        io::ErrorKind::PermissionDenied => return "Permission denied".into(),
        io::ErrorKind::AlreadyExists => return "File exists".into(),
        io::ErrorKind::IsADirectory => return "Is a directory".into(),
        io::ErrorKind::BrokenPipe => return "Broken pipe".into(),
        _ => {}
    }
    let s = e.to_string();
    match s.rfind(" (os error ") {
        Some(i) if s.ends_with(')') => s[..i].to_owned(),
        _ => s,
    }
}

/// What the work on a member gives back: the next step of the loop.
pub type Work = fn(&mut Gzip, &mut Io<'_>) -> R<i32>;

impl Gzip {
    /// Says `text` on stderr.
    pub fn say(&mut self, text: &[u8]) {
        match &mut self.diag {
            Diag::Stderr => {
                let mut e = io::stderr().lock();
                let _ = e.write_all(text);
                let _ = e.flush();
            }
            Diag::Kept(v) => v.extend_from_slice(text),
        }
    }

    /// `WARN`: a warning, unless quiet; the exit code becomes 2 if it was 0.
    pub fn warn(&mut self, text: &[u8]) {
        if !self.quiet {
            self.say(text);
        }
        if self.exit_code == OK {
            self.exit_code = WARNING;
        }
    }

    /// "prog: ifname: " and the rest, for messages.
    fn about(&self, lead: &str, rest: &str) -> Vec<u8> {
        let mut m = Vec::new();
        m.extend_from_slice(lead.as_bytes());
        m.extend_from_slice(self.program_name.as_bytes());
        m.extend_from_slice(b": ");
        m.extend_from_slice(&self.ifname);
        m.extend_from_slice(rest.as_bytes());
        m
    }

    /// `gzip_error`: the message, and the command stops.
    pub fn gzip_error(&mut self, m: &str) -> Stop {
        let text = self.about("\n", &format!(": {m}\n"));
        self.say(&text);
        Stop(ERROR)
    }

    /// `read_error`: what the system said, or "unexpected end of file".
    pub fn read_error(&mut self, e: Option<&io::Error>) -> Stop {
        let why = e.map_or_else(|| "unexpected end of file".to_owned(), strerror);
        let text = self.about("\n", &format!(": {why}\n"));
        self.say(&text);
        Stop(ERROR)
    }

    /// `write_error`: a broken pipe ends quietly with 2, others with 1.
    pub fn write_error(&mut self, e: &io::Error) -> Stop {
        let code = if e.kind() == io::ErrorKind::BrokenPipe {
            WARNING
        } else {
            ERROR
        };
        if !(code == WARNING && self.quiet) {
            let mut m = Vec::new();
            m.extend_from_slice(b"\n");
            m.extend_from_slice(self.program_name.as_bytes());
            m.extend_from_slice(b": ");
            m.extend_from_slice(&self.ofname);
            m.extend_from_slice(format!(": {}\n", strerror(e)).as_bytes());
            self.say(&m);
        }
        Stop(code)
    }

    /// `clear_bufs`.
    pub fn clear_bufs(&mut self) {
        self.outcnt = 0;
        self.insize = 0;
        self.inptr = 0;
        self.bytes_in = 0;
        self.bytes_out = 0;
    }

    /// `updcrc(NULL, 0)`.
    fn crc_reset(&mut self) {
        self.crc = 0;
    }

    fn crc_update(&mut self, data: &[u8]) {
        self.crc = hivepack::deflate::crc32(self.crc, data);
    }

    /// `getcrc`.
    pub fn crc(&self) -> u32 {
        self.crc
    }

    /// `read_buffer`, retried when interrupted.
    fn read_buffer(input: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            match input.read(buf) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                r => return r,
            }
        }
    }

    /// `fill_inbuf`: the buffer read full again, or up to the end of the
    /// input; its first byte, or `None` at the end when that is all right.
    /// At an end that is not, what was decoded is written, then the
    /// command stops.
    fn fill_inbuf(&mut self, io: &mut Io<'_>, eof_ok: bool) -> R<Option<u8>> {
        self.insize = 0;
        loop {
            match Self::read_buffer(io.input, &mut self.inbuf[self.insize..INBUFSIZ]) {
                Ok(0) => break,
                Ok(n) => self.insize += n,
                Err(e) => return Err(self.read_error(Some(&e))),
            }
            if self.insize >= INBUFSIZ {
                break;
            }
        }
        if self.insize == 0 {
            if eof_ok {
                return Ok(None);
            }
            self.flush_window(io)?;
            return Err(self.read_error(None));
        }
        self.bytes_in += self.insize as i64;
        self.inptr = 1;
        Ok(Some(self.inbuf[0]))
    }

    /// `get_byte`: the next input byte; the end of the input stops the
    /// command.
    pub fn get_byte(&mut self, io: &mut Io<'_>) -> R<u8> {
        if self.inptr < self.insize {
            self.inptr += 1;
            return Ok(self.inbuf[self.inptr - 1]);
        }
        Ok(self.fill_inbuf(io, false)?.unwrap_or(0))
    }

    /// `try_byte`: the next input byte, or `None` at its end.
    pub fn try_byte(&mut self, io: &mut Io<'_>) -> R<Option<u8>> {
        if self.inptr < self.insize {
            self.inptr += 1;
            return Ok(Some(self.inbuf[self.inptr - 1]));
        }
        self.fill_inbuf(io, true)
    }

    /// `write_buf`: counted, and written unless testing.
    fn write_buf(
        output: &mut dyn Write,
        test: bool,
        bytes_out: &mut i64,
        buf: &[u8],
    ) -> io::Result<()> {
        *bytes_out += buf.len() as i64;
        if test {
            return Ok(());
        }
        output.write_all(buf)
    }

    /// Writes `buf` out as `write_buf` does, stopping the command when the
    /// output fails.
    pub fn write_bytes(&mut self, io: &mut Io<'_>, buf: &[u8]) -> R<()> {
        Self::write_buf(io.output, self.test, &mut self.bytes_out, buf)
            .map_err(|e| self.write_error(&e))
    }

    /// `flush_window`: the window's bytes into the CRC and out.
    pub fn flush_window(&mut self, io: &mut Io<'_>) -> R<()> {
        if self.outcnt == 0 {
            return Ok(());
        }
        let n = self.outcnt;
        self.crc = hivepack::deflate::crc32(self.crc, &self.window[..n]);
        let r = Self::write_buf(io.output, self.test, &mut self.bytes_out, &self.window[..n]);
        r.map_err(|e| self.write_error(&e))?;
        self.outcnt = 0;
        Ok(())
    }

    /// `outbuf[..n]` out.
    fn write_outbuf(&mut self, io: &mut Io<'_>, n: usize) -> R<()> {
        let r = Self::write_buf(io.output, self.test, &mut self.bytes_out, &self.outbuf[..n]);
        r.map_err(|e| self.write_error(&e))
    }

    /// `put_ubyte`: a byte into the window, written out when it is full.
    fn put_ubyte(&mut self, io: &mut Io<'_>, c: u8) -> R<()> {
        self.window[self.outcnt] = c;
        self.outcnt += 1;
        if self.outcnt == WSIZE {
            self.flush_window(io)?;
        }
        Ok(())
    }

    /// `discard_input_bytes`: `n` bytes, or up to a zero byte when `None`,
    /// into the header's CRC when it has one.
    fn discard_input_bytes(&mut self, io: &mut Io<'_>, n: Option<usize>, flags: u8) -> R<()> {
        let mut n = n;
        loop {
            if n == Some(0) {
                return Ok(());
            }
            let c = self.get_byte(io)?;
            if flags & HEADER_CRC != 0 {
                self.crc_update(&[c]);
            }
            match &mut n {
                Some(k) => *k -= 1,
                None if c == 0 => return Ok(()),
                None => {}
            }
        }
    }

    /// `input_eof`: nonzero when the input is at its end.
    pub fn input_eof(&mut self, io: &mut Io<'_>) -> R<bool> {
        if !self.decompress || self.last_member {
            return Ok(true);
        }
        if self.inptr == self.insize {
            if self.insize != INBUFSIZ || self.fill_inbuf(io, true)?.is_none() {
                return Ok(true);
            }
            // Unget the byte fill_inbuf got.
            self.inptr = 0;
        }
        Ok(false)
    }

    /// `get_method`: the magic number of the next member, read, and its
    /// header: the method, -1 for an error, -2 for a warning (trailing
    /// garbage), -3 for trailing zeros. Sets the work to do on the member,
    /// and the output name from the header when it has one.
    pub fn get_method(&mut self, io: &mut Io<'_>, work: &mut Option<Work>) -> R<i32> {
        let mut magic = [0u8; 10];
        let imagic0: Option<u8>;
        let imagic1: Option<u8>;
        if self.force > 0 && self.to_stdout {
            imagic0 = self.try_byte(io)?;
            magic[0] = imagic0.unwrap_or(0xff);
            imagic1 = self.try_byte(io)?;
            magic[1] = imagic1.unwrap_or(0xff);
        } else {
            magic[0] = self.get_byte(io)?;
            imagic0 = Some(magic[0]);
            if magic[0] != 0 {
                magic[1] = self.get_byte(io)?;
                imagic1 = Some(magic[1]);
            } else {
                imagic1 = self.try_byte(io)?;
                magic[1] = imagic1.unwrap_or(0xff);
            }
        }
        self.method = -1;
        self.part_nb += 1;
        self.header_bytes = 0;
        self.last_member = false;

        if magic[..2] == GZIP_MAGIC || magic[..2] == OLD_GZIP_MAGIC {
            self.method = i32::from(self.get_byte(io)?);
            if self.method != DEFLATED {
                let m = self.about(
                    "",
                    &format!(": unknown method {} -- not supported\n", self.method),
                );
                self.say(&m);
                self.exit_code = ERROR;
                return Ok(-1);
            }
            *work = Some(Gzip::unzip);
            let flags = self.get_byte(io)?;
            if flags & ENCRYPTED != 0 {
                let m = self.about("", " is encrypted -- not supported\n");
                self.say(&m);
                self.exit_code = ERROR;
                return Ok(-1);
            }
            if flags & RESERVED != 0 {
                let m = self.about("", &format!(" has flags 0x{flags:x} -- not supported\n"));
                self.say(&m);
                self.exit_code = ERROR;
                if self.force <= 1 {
                    return Ok(-1);
                }
            }
            let mut stamp = 0u32;
            for i in 0..4 {
                stamp |= u32::from(self.get_byte(io)?) << (8 * i);
            }
            if stamp != 0 && !self.no_time {
                // 64-bit time_t holds every 32-bit stamp.
                self.time_stamp = Stamp {
                    sec: i64::from(stamp),
                    nsec: 0,
                };
            }
            magic[8] = self.get_byte(io)?;
            magic[9] = self.get_byte(io)?;
            if flags & HEADER_CRC != 0 {
                magic[2] = DEFLATED as u8;
                magic[3] = flags;
                magic[4..8].copy_from_slice(&stamp.to_le_bytes());
                self.crc_reset();
                self.crc_update(&magic);
            }
            if flags & EXTRA_FIELD != 0 {
                let lenbuf = [self.get_byte(io)?, self.get_byte(io)?];
                let len = u16::from_le_bytes(lenbuf);
                if self.verbose > 0 {
                    let m = self.about("", &format!(": extra field of {len} bytes ignored\n"));
                    self.say(&m);
                }
                if flags & HEADER_CRC != 0 {
                    self.crc_update(&lenbuf);
                }
                self.discard_input_bytes(io, Some(usize::from(len)), flags)?;
            }
            if flags & ORIG_NAME != 0 {
                if self.no_name || (self.to_stdout && !self.list) || self.part_nb > 1 {
                    self.discard_input_bytes(io, None, flags)?;
                } else {
                    // The base name, kept after any directory of ofname's.
                    let dir = base_start(&self.ofname);
                    self.ofname.truncate(dir);
                    let mut name = Vec::new();
                    loop {
                        let c = self.get_byte(io)?;
                        name.push(c);
                        if c == 0 {
                            break;
                        }
                        if dir + name.len() >= MAX_PATH_LEN {
                            return Err(self.gzip_error("corrupted input -- file name too large"));
                        }
                    }
                    if flags & HEADER_CRC != 0 {
                        self.crc_update(&name);
                    }
                    name.pop();
                    let base = base_start(&name);
                    self.ofname.extend_from_slice(&name[base..]);
                }
            }
            if flags & COMMENT != 0 {
                self.discard_input_bytes(io, None, flags)?;
            }
            if flags & HEADER_CRC != 0 {
                let crc16 = self.crc & 0xffff;
                let header16 = u32::from(self.get_byte(io)?) | u32::from(self.get_byte(io)?) << 8;
                if header16 != crc16 {
                    let m = self.about(
                        "",
                        &format!(
                            ": header checksum 0x{header16:04x} != computed checksum 0x{crc16:04x}\n"
                        ),
                    );
                    self.say(&m);
                    self.exit_code = ERROR;
                    if self.force <= 1 {
                        return Ok(-1);
                    }
                }
            }
            if self.part_nb == 1 {
                self.header_bytes = self.inptr as i64 + 2 * 4;
            }
        } else if magic[..2] == PKZIP_MAGIC[..2]
            && self.inptr == 2
            && self.inbuf[..4] == PKZIP_MAGIC
        {
            self.inptr = 0;
            *work = Some(Gzip::unzip);
            if !self.check_zipfile()? {
                return Ok(-1);
            }
            self.last_member = true;
        } else if magic[..2] == PACK_MAGIC {
            *work = Some(Gzip::unpack);
            self.method = PACKED;
        } else if magic[..2] == LZW_MAGIC {
            *work = Some(Gzip::unlzw);
            self.method = COMPRESSED;
            self.last_member = true;
        } else if magic[..2] == LZH_MAGIC {
            *work = Some(Gzip::unlzh);
            self.method = LZHED;
            self.last_member = true;
        } else if self.force > 0 && self.to_stdout && !self.list {
            // Passed on unchanged.
            self.method = STORED;
            *work = Some(Gzip::copy);
            if imagic1.is_some() {
                self.inptr -= 1;
            }
            self.last_member = true;
            if imagic0.is_some() {
                self.write_bytes(io, &magic[..1])?;
            }
        }
        if self.method >= 0 {
            return Ok(self.method);
        }
        if self.part_nb == 1 {
            let m = self.about("\n", ": not in gzip format\n");
            self.say(&m);
            self.exit_code = ERROR;
            return Ok(-1);
        }
        if magic[0] == 0 {
            let mut inbyte = imagic1;
            while inbyte == Some(0) {
                inbyte = self.try_byte(io)?;
            }
            if inbyte.is_none() {
                if self.verbose > 0 {
                    let m = self.about("\n", ": decompression OK, trailing zero bytes ignored\n");
                    self.warn(&m);
                }
                return Ok(-3);
            }
        }
        let m = self.about("\n", ": decompression OK, trailing garbage ignored\n");
        self.warn(&m);
        Ok(-2)
    }

    /// `check_zipfile`: a zip archive's first local header, its name and
    /// extra field skipped; false (said) when it cannot be read.
    fn check_zipfile(&mut self) -> R<bool> {
        let h = self.inptr;
        let sh = |p: usize, b: &[u8]| usize::from(u16::from_le_bytes([b[p], b[p + 1]]));
        self.inptr += 30 + sh(h + 26, &self.inbuf) + sh(h + 28, &self.inbuf);
        let lg = u32::from_le_bytes([
            self.inbuf[h],
            self.inbuf[h + 1],
            self.inbuf[h + 2],
            self.inbuf[h + 3],
        ]);
        if self.inptr > self.insize || lg != 0x0403_4b50 {
            let m = self.about("\n", ": not a valid zip file\n");
            self.say(&m);
            self.exit_code = ERROR;
            return Ok(false);
        }
        self.method = i32::from(self.inbuf[h + 8]);
        if self.method != STORED && self.method != DEFLATED {
            let m = self.about("\n", ": first entry not deflated or stored -- use unzip\n");
            self.say(&m);
            self.exit_code = ERROR;
            return Ok(false);
        }
        if self.inbuf[h + 6] & 1 != 0 {
            let m = self.about("\n", ": encrypted file -- use unzip\n");
            self.say(&m);
            self.exit_code = ERROR;
            return Ok(false);
        }
        self.ext_header = self.inbuf[h + 6] & 8 != 0;
        self.pkzip = true;
        Ok(true)
    }

    fn le32(b: &[u8]) -> u32 {
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    /// `unzip`: a gzip member, or a zip archive's entry, decompressed and
    /// checked.
    pub fn unzip(&mut self, io: &mut Io<'_>) -> R<i32> {
        let mut orig_crc = 0u32;
        let mut orig_len = 0u32;
        let mut err = OK;
        self.crc_reset();
        if self.pkzip && !self.ext_header {
            orig_crc = Self::le32(&self.inbuf[14..]);
            orig_len = Self::le32(&self.inbuf[22..]);
        }
        if self.method == DEFLATED {
            let res = self.inflate(io)?;
            if res == 3 {
                let m = format!("\n{}: memory_exhausted\n", self.program_name);
                self.say(m.as_bytes());
                return Err(Stop(ERROR));
            } else if res != 0 {
                return Err(self.gzip_error("invalid compressed data--format violated"));
            }
        } else if self.pkzip && self.method == STORED {
            let mut n = Self::le32(&self.inbuf[22..]);
            let siz = Self::le32(&self.inbuf[18..]);
            if n != siz {
                self.say(format!("len {n}, siz {siz}\n").as_bytes());
                return Err(self.gzip_error("invalid compressed data--length mismatch"));
            }
            while n > 0 {
                n -= 1;
                let c = self.get_byte(io)?;
                self.put_ubyte(io, c)?;
            }
            self.flush_window(io)?;
        } else {
            return Err(self.gzip_error("internal error, invalid method"));
        }

        if !self.pkzip {
            let mut buf = [0u8; 8];
            for b in &mut buf {
                *b = self.get_byte(io)?;
            }
            orig_crc = Self::le32(&buf);
            orig_len = Self::le32(&buf[4..]);
        } else if self.ext_header {
            let mut buf = [0u8; 16];
            for b in &mut buf {
                *b = self.get_byte(io)?;
            }
            orig_crc = Self::le32(&buf[4..]);
            orig_len = Self::le32(&buf[12..]);
        }

        if orig_crc != self.crc {
            let m = self.about("\n", ": invalid compressed data--crc error\n");
            self.say(&m);
            err = ERROR;
        }
        if orig_len != self.bytes_out as u32 {
            let m = self.about("\n", ": invalid compressed data--length error\n");
            self.say(&m);
            err = ERROR;
        }
        if self.pkzip
            && self
                .inptr
                .checked_add(4)
                .is_some_and(|end| end < self.insize)
            && Self::le32(&self.inbuf[self.inptr..]) == 0x0403_4b50
        {
            if self.to_stdout {
                let m = self.about("", " has more than one entry--rest ignored\n");
                self.warn(&m);
            } else {
                let m = self.about("", " has more than one entry -- unchanged\n");
                self.say(&m);
                err = ERROR;
            }
        }
        self.ext_header = false;
        self.pkzip = false;
        self.unzip_crc = orig_crc;
        if err == OK {
            return Ok(OK);
        }
        self.exit_code = ERROR;
        if !self.test {
            return Err(Stop(ERROR));
        }
        Ok(err)
    }

    /// `copy`: the input passed on unchanged (`zcat -f` of what is not
    /// compressed).
    pub fn copy(&mut self, io: &mut Io<'_>) -> R<i32> {
        while self.insize > self.inptr {
            let (a, b) = (self.inptr, self.insize);
            let r = Self::write_buf(io.output, self.test, &mut self.bytes_out, &self.inbuf[a..b]);
            r.map_err(|e| self.write_error(&e))?;
            let got = match Self::read_buffer(io.input, &mut self.inbuf[..INBUFSIZ]) {
                Ok(n) => n,
                Err(e) => return Err(self.read_error(Some(&e))),
            };
            self.bytes_in += got as i64;
            self.insize = got;
            self.inptr = 0;
        }
        Ok(OK)
    }
}

/// `MAX_PATH_LEN`: the longest name the command keeps.
pub const MAX_PATH_LEN: usize = 1024;

/// Where a name's last component starts (`last_component`): after its
/// last slash, trailing slashes aside.
pub fn base_start(name: &[u8]) -> usize {
    let mut base = 0;
    let mut saw_slash = false;
    for (i, &c) in name.iter().enumerate() {
        if c == b'/' {
            saw_slash = true;
        } else if saw_slash {
            base = i;
            saw_slash = false;
        }
    }
    base
}

#[cfg(test)]
mod tests {
    use super::app::decompress;
    use super::*;

    #[test]
    fn reads_the_suites_samples() {
        // gzip's tests: unpack-valid (pack), helin-segv (LZW).
        let pack = b"\x1f\x1e\x00\x00\x00\x06\x03\x01\x01\x00\x61\x6e\x62\x16\xc8";
        assert_eq!(decompress(pack).as_deref(), Ok(&b"banana"[..]));
        let lzw = b"\x1f\x9d\x90\x00\x00\x00\xc4";
        assert_eq!(decompress(lzw).as_deref(), Ok(&b"\0\0"[..]));
    }

    #[test]
    fn round_trips() {
        let data: Vec<u8> = (0..300_000u32)
            .map(|i| (i % 251) as u8 ^ (i >> 11) as u8)
            .collect();
        for level in [1, 6, 9] {
            let gz = Compress {
                level,
                ..Compress::default()
            }
            .run(&data);
            assert_eq!(&gz[..3], &[0x1f, 0x8b, 8]);
            assert_eq!(decompress(&gz).as_deref(), Ok(&data[..]));
        }
        let empty = Compress::default().run(b"");
        assert_eq!(decompress(&empty).as_deref(), Ok(&b""[..]));
    }
}
