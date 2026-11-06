//! gzip's zip.c: a gzip member written as the command writes it, its header
//! (name, time, extra flags, OS) and trailer (CRC-32, size) the command's,
//! the deflate stream between them zlib's (hivepack's port of zlib 1.3.2,
//! at the same level), not gzip's own encoder: valid, read by any gunzip,
//! but not the same bytes.
//!
//! Ported from GNU gzip 1.13's zip.c; Copyright (C) 1997-2023 Free Software
//! Foundation, Inc.; Copyright (C) 1992-1993 Jean-loup Gailly.
//! GPL-3.0-or-later.

use hivepack::deflate::{Deflate, Flush, InBuffer, OutBuffer, Status};

use super::{DEFLATED, Gzip, Io, OK, ORIG_NAME, R, base_start};

/// The OS byte, Unix (`OS_CODE`), whatever the system, as gzip built for
/// Unix writes it.
const OS_CODE: u8 = 0x03;
/// The extra flags: `FAST` for level 1, `SLOW` for 9.
const FAST: u8 = 4;
const SLOW: u8 = 2;

/// A compression for the app: the level, the name and time to keep.
#[derive(Clone, Debug)]
pub struct Compress {
    pub level: i32,
    pub name: Option<Vec<u8>>,
    pub mtime: u32,
}

impl Default for Compress {
    fn default() -> Self {
        Compress {
            level: 6,
            name: None,
            mtime: 0,
        }
    }
}

impl Compress {
    /// `input` as one gzip member.
    pub fn run(&self, input: &[u8]) -> Vec<u8> {
        let mut gz = Gzip {
            level: self.level,
            diag: super::Diag::Kept(Vec::new()),
            ..Gzip::default()
        };
        gz.time_stamp = if self.mtime == 0 {
            super::Stamp::UNKNOWN
        } else {
            super::Stamp {
                sec: i64::from(self.mtime),
                nsec: 0,
            }
        };
        if let Some(n) = &self.name {
            gz.save_orig_name = true;
            gz.ifname = n.clone();
        }
        let mut out = Vec::new();
        let mut input = input;
        let mut io = Io {
            input: &mut input,
            output: &mut out,
        };
        // Reading a slice and writing a vector cannot fail.
        let _ = gz.zip(&mut io);
        out
    }
}

impl Gzip {
    /// `zip`: the input compressed, as one member.
    pub fn zip(&mut self, io: &mut Io<'_>) -> R<i32> {
        self.method = DEFLATED;
        let mut head = vec![0x1f, 0x8b, DEFLATED as u8];
        head.push(if self.save_orig_name { ORIG_NAME } else { 0 });
        let stamp: u32 = if !self.time_stamp.known() {
            0
        } else if 0 < self.time_stamp.sec && self.time_stamp.sec <= 0xffff_ffff {
            self.time_stamp.sec as u32
        } else {
            // Zero too: gzip keeps it for "no time".
            let m = self.about(
                "",
                ": warning: file timestamp out of range for gzip format\n",
            );
            self.warn(&m);
            0
        };
        head.extend_from_slice(&stamp.to_le_bytes());
        head.push(match self.level {
            1 => FAST,
            9 => SLOW,
            _ => 0,
        });
        head.push(OS_CODE);
        if self.save_orig_name {
            let base = base_start(&self.ifname);
            head.extend_from_slice(&self.ifname[base..]);
            head.push(0);
        }
        self.header_bytes = head.len() as i64;
        self.write_bytes(io, &head)?;

        self.crc_reset();
        let mut z = match Deflate::new(self.level, -15, 8, 0) {
            Ok(z) => z,
            Err(_) => return Err(self.gzip_error("internal error, invalid level")),
        };
        let mut buf = vec![0u8; 0x8000];
        let mut out = vec![0u8; 0x10000];
        loop {
            let n = match Self::read_buffer(io.input, &mut buf) {
                Ok(n) => n,
                Err(e) => return Err(self.read_error(Some(&e))),
            };
            self.crc_update(&buf[..n]);
            self.bytes_in += n as i64;
            let flush = if n == 0 {
                Flush::Finish
            } else {
                Flush::NoFlush
            };
            let mut inb = InBuffer::new(&buf[..n]);
            loop {
                let mut outb = OutBuffer::new(&mut out);
                let st = z.deflate(&mut outb, &mut inb, flush);
                let w = outb.pos;
                if w > 0 {
                    let r = Self::write_buf(io.output, self.test, &mut self.bytes_out, &out[..w]);
                    r.map_err(|e| self.write_error(&e))?;
                }
                match st {
                    Ok(Status::StreamEnd) => break,
                    // More to write out, or (finishing) more to finish.
                    Ok(_) if w == out.len() || flush == Flush::Finish => {}
                    Ok(_) if inb.pos < n => {}
                    Ok(_) => break,
                    // No progress possible: all taken, all written.
                    Err(_) if flush != Flush::Finish => break,
                    Err(_) => return Err(self.gzip_error("internal error in deflate")),
                }
            }
            if n == 0 {
                break;
            }
        }

        if self.ifile_size != -1 && self.bytes_in != self.ifile_size {
            let m = self.about("", ": file size changed while zipping\n");
            self.say(&m);
        }
        let mut tail = self.crc().to_le_bytes().to_vec();
        tail.extend_from_slice(&(self.bytes_in as u32).to_le_bytes());
        self.write_bytes(io, &tail)?;
        self.header_bytes += 8;
        Ok(OK)
    }
}
