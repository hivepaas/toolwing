//! gzip's inflate.c: deflate decoded as the command decodes it, its window
//! written out each time it fills, so that what is written before an error
//! is the command's. Its Huffman tables (`huft_build`), multi-level, are
//! kept in one arena by index rather than in malloc'd lists.
//!
//! Ported from GNU gzip 1.13's inflate.c, derived from code put in the
//! public domain by Mark Adler; Copyright (C) 1997-2023 Free Software
//! Foundation, Inc. GPL-3.0-or-later.

use super::{Gzip, Io, R, WSIZE};

/// A table entry (`struct huft`): `e` extra bits (15 end of block, 16 a
/// literal, 17 to 31 a link to a table of `e - 16` bits, 99 an invalid
/// code), `b` bits of this code, and the value or the linked table.
#[derive(Clone, Copy, Default)]
struct Huft {
    e: u8,
    b: u8,
    v: u32,
}

const BORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];
const CPLENS: [u16; 31] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258, 0, 0,
];
const CPLEXT: [u16; 31] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0, 99, 99,
];
const CPDIST: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const CPDEXT: [u16; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
const MASK_BITS: [u32; 17] = [
    0x0000, 0x0001, 0x0003, 0x0007, 0x000f, 0x001f, 0x003f, 0x007f, 0x00ff, 0x01ff, 0x03ff, 0x07ff,
    0x0fff, 0x1fff, 0x3fff, 0x7fff, 0xffff,
];
const LBITS: i32 = 9;
const DBITS: i32 = 6;
const BMAX: usize = 16;
const N_MAX: usize = 288;

/// `huft_build`: the tables for code lengths `b[..n]`, values below `s`
/// simple (literals, and 256 the end of a block), the others from `d`
/// and `e`. Gives 0, 1 for an incomplete set (the tables made all the
/// same), 2 for an invalid one; the first table's start in the arena, and
/// its bits in `m`.
fn huft_build(
    arena: &mut Vec<Huft>,
    b: &[u32],
    n: usize,
    s: u32,
    d: &[u16],
    e: &[u16],
    m: &mut i32,
) -> (i32, usize) {
    let mut c = [0u32; BMAX + 1];
    for &len in &b[..n] {
        c[len as usize] += 1;
    }
    if c[0] as usize == n {
        // All zero lengths: a table of two invalid codes.
        let q = arena.len();
        arena.push(Huft::default());
        arena.push(Huft { e: 99, b: 1, v: 0 });
        arena.push(Huft { e: 99, b: 1, v: 0 });
        *m = 1;
        return (0, q + 1);
    }

    let mut l = *m;
    let mut j = 1usize;
    while j <= BMAX && c[j] == 0 {
        j += 1;
    }
    let mut k = j as i32;
    if (l as usize) < j {
        l = j as i32;
    }
    let mut i = BMAX;
    while i > 0 && c[i] == 0 {
        i -= 1;
    }
    let g = i as i32;
    if l as usize > i {
        l = i as i32;
    }
    *m = l;

    let mut y: i64 = 1 << j;
    while j < i {
        y -= i64::from(c[j]);
        if y < 0 {
            return (2, 0);
        }
        j += 1;
        y <<= 1;
    }
    y -= i64::from(c[i]);
    if y < 0 {
        return (2, 0);
    }
    c[i] += y as u32;

    let mut x = [0u32; BMAX + 1];
    let mut jj = 0u32;
    for (xi, ci) in (2..=g as usize).zip(1..) {
        jj += c[ci];
        x[xi] = jj;
    }

    let mut v = [0u32; N_MAX];
    for (idx, &len) in b[..n].iter().enumerate() {
        if len != 0 {
            v[x[len as usize] as usize] = idx as u32;
            x[len as usize] += 1;
        }
    }
    let nv = x[g as usize] as usize;

    x[0] = 0;
    let mut code: u32 = 0;
    let mut p = 0usize;
    let mut h: i32 = -1;
    let mut w: i32 = -l;
    let mut u = [0usize; BMAX];
    let mut q = 0usize;
    let mut z: u32 = 0;
    let mut first: Option<usize> = None;

    while k <= g {
        let mut a = c[k as usize];
        while a > 0 {
            a -= 1;
            while k > w + l {
                h += 1;
                w += l;
                z = (g - w) as u32;
                if z > l as u32 {
                    z = l as u32;
                }
                let mut jb = (k - w) as u32;
                let mut f: u32 = 1 << jb;
                if f > a + 1 {
                    f -= a + 1;
                    let mut xp = k as usize;
                    if jb < z {
                        loop {
                            jb += 1;
                            if jb >= z {
                                break;
                            }
                            f <<= 1;
                            xp += 1;
                            if f <= c[xp] {
                                break;
                            }
                            f -= c[xp];
                        }
                    }
                }
                z = 1 << jb;
                let start = arena.len();
                arena.resize(start + z as usize + 1, Huft::default());
                q = start + 1;
                first.get_or_insert(q);
                u[h as usize] = q;
                if h > 0 {
                    x[h as usize] = code;
                    let r = Huft {
                        b: l as u8,
                        e: (16 + jb) as u8,
                        v: q as u32,
                    };
                    let jl = (code >> (w - l)) as usize;
                    arena[u[h as usize - 1] + jl] = r;
                }
            }

            let mut r = Huft {
                b: (k - w) as u8,
                ..Huft::default()
            };
            if p >= nv {
                r.e = 99;
            } else if v[p] < s {
                r.e = if v[p] < 256 { 16 } else { 15 };
                r.v = v[p];
                p += 1;
            } else {
                let at = (v[p] - s) as usize;
                r.e = e[at] as u8;
                r.v = u32::from(d[at]);
                p += 1;
            }

            let f = 1u32 << (k - w);
            let mut jf = code >> w;
            while jf < z {
                arena[q + jf as usize] = r;
                jf += f;
            }

            let mut jb = 1u32 << (k - 1);
            while code & jb != 0 {
                code ^= jb;
                jb >>= 1;
            }
            code ^= jb;

            while (code & ((1u32 << w) - 1)) != x[h as usize] {
                h -= 1;
                w -= l;
            }
        }
        k += 1;
    }

    (i32::from(y != 0 && g != 1), first.unwrap_or(0))
}

impl Gzip {
    /// `NEEDBITS`: at least `n` bits in the bit buffer; the end of the
    /// input writes out what was decoded, and stops the command.
    fn needbits(&mut self, io: &mut Io<'_>, n: u32) -> R<()> {
        while self.bk < n {
            let c = self.get_byte(io)?;
            self.bb |= u64::from(c) << self.bk;
            self.bk += 8;
        }
        Ok(())
    }

    fn dumpbits(&mut self, n: u32) {
        self.bb >>= n;
        self.bk -= n;
    }

    /// `flush_output`.
    fn flush_output(&mut self, io: &mut Io<'_>) -> R<()> {
        self.fresh = false;
        self.flush_window(io)
    }

    /// `inflate_codes`: a block's codes, until its end; 1 for an error.
    fn inflate_codes(
        &mut self,
        io: &mut Io<'_>,
        arena: &[Huft],
        tl: usize,
        td: usize,
        bl: i32,
        bd: i32,
    ) -> R<i32> {
        let ml = MASK_BITS[bl as usize];
        let md = MASK_BITS[bd as usize];
        loop {
            self.needbits(io, bl as u32)?;
            let mut t = tl + (self.bb as u32 & ml) as usize;
            let mut e = u32::from(arena[t].e);
            while e > 16 {
                if e == 99 {
                    return Ok(1);
                }
                self.dumpbits(u32::from(arena[t].b));
                e -= 16;
                self.needbits(io, e)?;
                t = arena[t].v as usize + (self.bb as u32 & MASK_BITS[e as usize]) as usize;
                e = u32::from(arena[t].e);
            }
            self.dumpbits(u32::from(arena[t].b));
            if e == 16 {
                let w = self.outcnt;
                self.window[w] = arena[t].v as u8;
                self.outcnt += 1;
                if self.outcnt == WSIZE {
                    self.flush_output(io)?;
                }
                continue;
            }
            if e == 15 {
                return Ok(0);
            }

            self.needbits(io, e)?;
            let mut n = arena[t].v + (self.bb as u32 & MASK_BITS[e as usize]);
            self.dumpbits(e);

            self.needbits(io, bd as u32)?;
            t = td + (self.bb as u32 & md) as usize;
            e = u32::from(arena[t].e);
            while e > 16 {
                if e == 99 {
                    return Ok(1);
                }
                self.dumpbits(u32::from(arena[t].b));
                e -= 16;
                self.needbits(io, e)?;
                t = arena[t].v as usize + (self.bb as u32 & MASK_BITS[e as usize]) as usize;
                e = u32::from(arena[t].e);
            }
            self.dumpbits(u32::from(arena[t].b));
            self.needbits(io, e)?;
            let w = self.outcnt as u32;
            let mut d = w
                .wrapping_sub(arena[t].v)
                .wrapping_sub(self.bb as u32 & MASK_BITS[e as usize]);
            self.dumpbits(e);
            if self.fresh && w <= d {
                return Ok(1);
            }
            loop {
                let mut w = self.outcnt as u32;
                d &= WSIZE as u32 - 1;
                let mut ee = WSIZE as u32 - if d > w { d } else { w };
                if ee > n {
                    ee = n;
                }
                n -= ee;
                while ee > 0 {
                    self.window[w as usize] = self.window[d as usize];
                    w += 1;
                    d += 1;
                    ee -= 1;
                }
                self.outcnt = w as usize;
                if self.outcnt == WSIZE {
                    self.flush_output(io)?;
                }
                if n == 0 {
                    break;
                }
            }
        }
    }

    /// `inflate_stored`.
    fn inflate_stored(&mut self, io: &mut Io<'_>) -> R<i32> {
        let n = self.bk & 7;
        self.dumpbits(n);
        self.needbits(io, 16)?;
        let mut n = self.bb as u32 & 0xffff;
        self.dumpbits(16);
        self.needbits(io, 16)?;
        if n != (!(self.bb as u32)) & 0xffff {
            return Ok(1);
        }
        self.dumpbits(16);
        while n > 0 {
            n -= 1;
            self.needbits(io, 8)?;
            self.window[self.outcnt] = self.bb as u8;
            self.outcnt += 1;
            if self.outcnt == WSIZE {
                self.flush_output(io)?;
            }
            self.dumpbits(8);
        }
        Ok(0)
    }

    /// `inflate_fixed`.
    fn inflate_fixed(&mut self, io: &mut Io<'_>) -> R<i32> {
        let mut l = [0u32; 288];
        for (i, x) in l.iter_mut().enumerate() {
            *x = match i {
                0..=143 => 8,
                144..=255 => 9,
                256..=279 => 7,
                _ => 8,
            };
        }
        let mut arena = Vec::new();
        let mut bl = 7;
        let (r, tl) = huft_build(&mut arena, &l, 288, 257, &CPLENS, &CPLEXT, &mut bl);
        if r != 0 {
            return Ok(r);
        }
        let l = [5u32; 30];
        let mut bd = 5;
        let (r, td) = huft_build(&mut arena, &l, 30, 0, &CPDIST, &CPDEXT, &mut bd);
        if r > 1 {
            return Ok(r);
        }
        if self.inflate_codes(io, &arena, tl, td, bl, bd)? != 0 {
            return Ok(1);
        }
        Ok(0)
    }

    /// `inflate_dynamic`.
    fn inflate_dynamic(&mut self, io: &mut Io<'_>) -> R<i32> {
        let mut ll = [0u32; 286 + 30];
        self.needbits(io, 5)?;
        let nl = 257 + (self.bb as u32 & 0x1f) as usize;
        self.dumpbits(5);
        self.needbits(io, 5)?;
        let nd = 1 + (self.bb as u32 & 0x1f) as usize;
        self.dumpbits(5);
        self.needbits(io, 4)?;
        let nb = 4 + (self.bb as u32 & 0xf) as usize;
        self.dumpbits(4);
        if nl > 286 || nd > 30 {
            return Ok(1);
        }
        for &at in &BORDER[..nb] {
            self.needbits(io, 3)?;
            ll[at] = self.bb as u32 & 7;
            self.dumpbits(3);
        }
        for &at in &BORDER[nb..] {
            ll[at] = 0;
        }

        let mut arena = Vec::new();
        let mut bl = 7;
        let (r, tl) = huft_build(&mut arena, &ll, 19, 19, &[], &[], &mut bl);
        if r != 0 {
            return Ok(r);
        }

        let n = nl + nd;
        let m = MASK_BITS[bl as usize];
        let mut i = 0usize;
        let mut l = 0u32;
        while i < n {
            self.needbits(io, bl as u32)?;
            let td = arena[tl + (self.bb as u32 & m) as usize];
            self.dumpbits(u32::from(td.b));
            if td.e == 99 {
                return Ok(2);
            }
            let j = td.v;
            if j < 16 {
                l = j;
                ll[i] = l;
                i += 1;
            } else {
                let (bits, base, value) = match j {
                    16 => (2, 3, l),
                    17 => (3, 3, 0),
                    _ => (7, 11, 0),
                };
                self.needbits(io, bits)?;
                let count = base + (self.bb as u32 & MASK_BITS[bits as usize]) as usize;
                self.dumpbits(bits);
                if i + count > n {
                    return Ok(1);
                }
                for x in &mut ll[i..i + count] {
                    *x = value;
                }
                i += count;
                l = value;
            }
        }

        let mut arena = Vec::new();
        let mut bl = LBITS;
        let (r, tl) = huft_build(&mut arena, &ll, nl, 257, &CPLENS, &CPLEXT, &mut bl);
        if r != 0 {
            return Ok(r);
        }
        let mut bd = DBITS;
        let (r, td) = huft_build(&mut arena, &ll[nl..], nd, 0, &CPDIST, &CPDEXT, &mut bd);
        if r != 0 {
            return Ok(r);
        }
        Ok(i32::from(
            self.inflate_codes(io, &arena, tl, td, bl, bd)? != 0,
        ))
    }

    /// `inflate_block`: one block; `last` set for the last one.
    fn inflate_block(&mut self, io: &mut Io<'_>, last: &mut bool) -> R<i32> {
        self.needbits(io, 1)?;
        *last = self.bb & 1 != 0;
        self.dumpbits(1);
        self.needbits(io, 2)?;
        let t = self.bb as u32 & 3;
        self.dumpbits(2);
        match t {
            2 => self.inflate_dynamic(io),
            0 => self.inflate_stored(io),
            1 => self.inflate_fixed(io),
            _ => Ok(2),
        }
    }

    /// `inflate`: a deflate stream, its last block included; 0, or the
    /// error of a block. The bytes read past it are given back.
    pub(super) fn inflate(&mut self, io: &mut Io<'_>) -> R<i32> {
        self.outcnt = 0;
        self.bk = 0;
        self.bb = 0;
        self.fresh = true;
        let mut last = false;
        while !last {
            let r = self.inflate_block(io, &mut last)?;
            if r != 0 {
                return Ok(r);
            }
        }
        // Bytes read ahead go back; across a refill of the buffer the C's
        // unsigned index wraps, and so does this one.
        while self.bk >= 8 {
            self.bk -= 8;
            self.inptr = self.inptr.wrapping_sub(1);
        }
        self.flush_output(io)?;
        Ok(0)
    }
}
