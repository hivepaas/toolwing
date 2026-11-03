//! Base64 and Base32 as GNU coreutils 9.7 writes and reads them: the
//! engine behind `base64` and `base32` (drop-in: tools.md).
//!
//! Encoding is RFC 4648's, wrapped into lines. Decoding follows GNU's rules,
//! checked against GNU coreutils 9.7: newlines are skipped (no other
//! whitespace), padding may end any group and a new one may follow, a last
//! group needs no padding, and unused bits must be zero. A group of n
//! characters is valid when it gives a byte at least and leaves fewer bits
//! than one character holds: 2, 3 or 4 of Base64's, and 2, 4, 5, 7 or 8 of
//! Base32's. On invalid input, the bytes decoded before it are written,
//! then the error; but a last group cut short with padding in it gives
//! nothing in Base32 (`MY=`), where Base64 gives its bytes (`YQ=`).

use std::io::{Read, Write};

use toolwing_core::{Ctx, Error};

/// An alphabet and how its characters group.
pub struct Codec {
    alphabet: &'static [u8],
    /// Bits a character holds.
    bits: u32,
    /// Characters in a whole group: 4 for Base64 (3 bytes), 8 for Base32 (5).
    group: usize,
    /// Whether a last group cut short but padded (Base64's `YQ=`) gives its
    /// bytes before it fails: GNU's Base64 does, its Base32 (`MY=`) does not.
    short_padded_decodes: bool,
    encoding: &'static data_encoding::Encoding,
}

pub static BASE64: Codec = Codec {
    alphabet: b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
    bits: 6,
    group: 4,
    short_padded_decodes: true,
    encoding: &data_encoding::BASE64,
};

pub static BASE32: Codec = Codec {
    alphabet: b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567",
    bits: 5,
    group: 8,
    short_padded_decodes: false,
    encoding: &data_encoding::BASE32,
};

/// How many bytes are read at a time when streaming: a multiple of 3 and 5
/// (whole groups for both codecs to encode) and of 4 and 8.
pub const CHUNK: usize = 3 * 5 * 4 * 8 * 1024;

impl Codec {
    /// The bytes a whole group encodes.
    fn group_bytes(&self) -> usize {
        self.group * self.bits as usize / 8
    }

    fn value(&self, c: u8) -> Option<u8> {
        self.alphabet.iter().position(|&a| a == c).map(|i| i as u8)
    }

    /// Encodes all of `input`, wrapping lines at `wrap` characters (0: one
    /// line, with no newline at its end, as GNU does).
    pub fn encode(
        &self,
        ctx: &Ctx,
        input: &mut dyn Read,
        out: &mut dyn Write,
        wrap: usize,
    ) -> Result<(), Error> {
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
            let take = if last {
                end
            } else {
                end - end % self.group_bytes()
            };
            encoded.clear();
            encoded.resize(self.encoding.encode_len(take), 0);
            self.encoding.encode_mut(&buf[..take], &mut encoded);
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

    /// Decodes all of `input`; with `ignore_garbage`, characters outside the
    /// alphabet (and `=`) are skipped.
    pub fn decode(
        &self,
        ctx: &Ctx,
        input: &mut dyn Read,
        out: &mut dyn Write,
        ignore_garbage: bool,
    ) -> Result<(), Error> {
        let mut buf = vec![0; CHUNK];
        let mut group = Group::new(self);
        let mut decoded = Vec::with_capacity(CHUNK);
        loop {
            ctx.check()?;
            let n = read_some(input, &mut buf)?;
            if n == 0 {
                return group.finish(out);
            }
            decoded.clear();
            let result = buf[..n]
                .iter()
                .filter(|&&c| {
                    c != b'\n' && (!ignore_garbage || c == b'=' || self.value(c).is_some())
                })
                .try_for_each(|&c| group.push(c, &mut decoded));
            out.write_all(&decoded)?;
            result?;
        }
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

fn invalid() -> Error {
    Error::input("invalid input")
}

/// The characters of the group being read.
struct Group<'c> {
    codec: &'c Codec,
    chars: [u8; 8],
    len: usize,
}

impl<'c> Group<'c> {
    fn new(codec: &'c Codec) -> Self {
        Group {
            codec,
            chars: [0; 8],
            len: 0,
        }
    }

    fn push(&mut self, c: u8, out: &mut Vec<u8>) -> Result<(), Error> {
        self.chars[self.len] = c;
        self.len += 1;
        if self.len == self.codec.group {
            let complete = self.decode(out);
            self.len = 0;
            complete?;
        }
        Ok(())
    }

    /// The end of the input: a group cut short is decoded as if padded.
    fn finish(&mut self, out: &mut dyn Write) -> Result<(), Error> {
        let mut decoded = Vec::new();
        let result = match self.len {
            0 => Ok(()),
            _ => self.decode(&mut decoded),
        };
        out.write_all(&decoded)?;
        result
    }

    /// Writes what the group's leading alphabet characters give, then says
    /// whether the group was valid: whole, or padded (or at the end) with
    /// its unused bits zero.
    fn decode(&self, out: &mut Vec<u8>) -> Result<(), Error> {
        let q = &self.chars[..self.len];
        let values: Vec<u8> = q.iter().map_while(|&c| self.codec.value(c)).collect();
        let short_padded = q.len() < self.codec.group && q.len() > values.len();
        if short_padded && !self.codec.short_padded_decodes {
            return Err(invalid());
        }
        let bits = self.codec.bits as usize;
        let total = values.len() * bits;
        let acc = values
            .iter()
            .fold(0u64, |acc, &v| acc << bits | u64::from(v));
        let bytes = total / 8;
        let unused = total - bytes * 8;
        for i in 0..bytes {
            out.push((acc >> (total - 8 * (i + 1))) as u8);
        }
        let padding = &q[values.len()..];
        let padded =
            padding.iter().all(|&c| c == b'=') && q.len() == self.codec.group || padding.is_empty();
        let unused_zero = acc & ((1 << unused) - 1) == 0;
        if bytes > 0 && unused < bits && padded && unused_zero {
            Ok(())
        } else {
            Err(invalid())
        }
    }
}

pub fn read_some(input: &mut dyn Read, buf: &mut [u8]) -> std::io::Result<usize> {
    loop {
        match input.read(buf) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            other => return other,
        }
    }
}
