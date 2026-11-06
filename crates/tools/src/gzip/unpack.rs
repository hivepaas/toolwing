//! gzip's unpack.c: pack's Huffman coding (`.z`), as the command decodes it.
//!
//! Ported from GNU gzip 1.13's unpack.c; Copyright (C) 1997-2023 Free
//! Software Foundation, Inc.; Copyright (C) 1992-1993 Jean-loup Gailly.
//! GPL-3.0-or-later.

use super::{Gzip, Io, OK, R};

const MAX_BITLEN: usize = 25;
const LITERALS: usize = 256;
const MAX_PEEK: usize = 12;

/// unpack.c's statics, kept from one file to the next as the C's are.
pub(super) struct State {
    orig_len: u32,
    max_len: usize,
    literal: [u8; LITERALS],
    lit_base: [i32; MAX_BITLEN + 1],
    leaves: [i32; MAX_BITLEN + 1],
    parents: [i32; MAX_BITLEN + 1],
    peek_bits: usize,
    bitbuf: u64,
    valid: usize,
}

impl Default for State {
    fn default() -> Self {
        State {
            orig_len: 0,
            max_len: 0,
            literal: [0; LITERALS],
            lit_base: [0; MAX_BITLEN + 1],
            leaves: [0; MAX_BITLEN + 1],
            parents: [0; MAX_BITLEN + 1],
            peek_bits: 0,
            bitbuf: 0,
            valid: 0,
        }
    }
}

impl Gzip {
    /// `look_bits`: the next `bits` bits, not skipped.
    fn look_bits(&mut self, io: &mut Io<'_>, bits: usize, mask: u64) -> R<u64> {
        while self.unpack.valid < bits {
            let b = self.get_byte(io)?;
            self.unpack.bitbuf = (self.unpack.bitbuf << 8) | u64::from(b);
            self.unpack.valid += 8;
        }
        Ok((self.unpack.bitbuf >> (self.unpack.valid - bits)) & mask)
    }

    /// `read_tree`.
    fn read_tree(&mut self, io: &mut Io<'_>) -> R<()> {
        let mut orig_len = 0u32;
        for _ in 0..4 {
            orig_len = (orig_len << 8) | u32::from(self.get_byte(io)?);
        }
        self.unpack.orig_len = orig_len;
        let max_len = usize::from(self.get_byte(io)?);
        if !(0 < max_len && max_len <= MAX_BITLEN) {
            return Err(
                self.gzip_error("invalid compressed data -- Huffman code bit length out of range")
            );
        }
        self.unpack.max_len = max_len;
        let mut n = 0i32;
        let mut max_leaves = 1i32;
        for len in 1..=max_len {
            let leaves = i32::from(self.get_byte(io)?);
            self.unpack.leaves[len] = leaves;
            if max_leaves - i32::from(len == max_len) < leaves {
                return Err(self.gzip_error("too many leaves in Huffman tree"));
            }
            max_leaves = (max_leaves - leaves + 1) * 2 - 1;
            n += leaves;
        }
        if LITERALS as i32 <= n {
            return Err(self.gzip_error("too many leaves in Huffman tree"));
        }
        self.unpack.leaves[max_len] += 1;
        let mut base = 0usize;
        for len in 1..=max_len {
            self.unpack.lit_base[len] = base as i32;
            for _ in 0..self.unpack.leaves[len] {
                let b = self.get_byte(io)?;
                if let Some(slot) = self.unpack.literal.get_mut(base) {
                    *slot = b;
                }
                base += 1;
            }
        }
        self.unpack.leaves[max_len] += 1;
        Ok(())
    }

    /// `build_tree`: the parents at each length, and the prefix table (in
    /// `outbuf`, as the C's `prefix_len`).
    fn build_tree(&mut self) -> R<()> {
        let u = &mut self.unpack;
        let mut nodes = 0i32;
        for len in (1..=u.max_len).rev() {
            nodes >>= 1;
            u.parents[len] = nodes;
            u.lit_base[len] -= nodes;
            nodes += u.leaves[len];
        }
        if (nodes >> 1) != 1 {
            return Err(self.gzip_error("too few leaves in Huffman tree"));
        }
        let u = &mut self.unpack;
        u.peek_bits = u.max_len.min(MAX_PEEK);
        let mut prefixp = 1usize << u.peek_bits;
        for len in 1..=u.peek_bits {
            let prefixes = (u.leaves[len] as usize) << (u.peek_bits - len);
            for _ in 0..prefixes {
                if prefixp == 0 {
                    break;
                }
                prefixp -= 1;
                self.outbuf[prefixp] = len as u8;
            }
        }
        self.outbuf[..prefixp].fill(0);
        Ok(())
    }

    /// `unpack`.
    pub fn unpack(&mut self, io: &mut Io<'_>) -> R<i32> {
        self.read_tree(io)?;
        self.build_tree()?;
        self.unpack.valid = 0;
        self.unpack.bitbuf = 0;
        let peek_bits = self.unpack.peek_bits;
        let peek_mask = (1u64 << peek_bits) - 1;
        let max_len = self.unpack.max_len;
        let eob = (self.unpack.leaves[max_len] - 1) as u64;
        loop {
            let mut peek = self.look_bits(io, peek_bits, peek_mask)?;
            let mut len = usize::from(self.outbuf[peek as usize]);
            if len > 0 {
                peek >>= peek_bits - len;
            } else {
                let mut mask = peek_mask;
                len = peek_bits;
                while len <= MAX_BITLEN && (peek as i64) < i64::from(self.unpack.parents[len]) {
                    len += 1;
                    mask = (mask << 1) + 1;
                    peek = self.look_bits(io, len, mask)?;
                }
            }
            if peek == eob && len == max_len {
                break;
            }
            let at = peek as i64 + i64::from(*self.unpack.lit_base.get(len).unwrap_or(&0));
            let c = usize::try_from(at)
                .ok()
                .and_then(|i| self.unpack.literal.get(i))
                .copied()
                .unwrap_or(0);
            self.put_ubyte(io, c)?;
            self.unpack.valid = self.unpack.valid.saturating_sub(len);
        }
        self.flush_window(io)?;
        if self.unpack.orig_len != self.bytes_out as u32 {
            return Err(self.gzip_error("invalid compressed data--length error"));
        }
        Ok(OK)
    }
}
