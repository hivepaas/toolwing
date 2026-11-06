//! gzip's unlzh.c: SCO `compress -H` (LZH, -lh5-), as the command decodes
//! it. Its tables share the command's buffers as the C's do: `c_len` is
//! `outbuf`, `left` and `right` are `prev` and its upper half.
//!
//! Where the C would read or write past an array, or loop for ever, on
//! crafted input (undefined behaviour), this stops with the format's own
//! error, "Bad table".
//!
//! Ported from GNU gzip 1.13's unlzh.c, derived from the public domain
//! 'ar002' by Haruhiko Okumura; Copyright (C) 1997-2023 Free Software
//! Foundation, Inc. GPL-3.0-or-later.

use super::{Gzip, Io, OK, R, Stop, WSIZE};

const DICBIT: u32 = 13;
const DICSIZ: usize = 1 << DICBIT;
const BITBUFSIZ: u32 = 16;
const MAXMATCH: usize = 256;
const THRESHOLD: usize = 3;
const NC: usize = 255 + MAXMATCH + 2 - THRESHOLD;
const CBIT: u32 = 9;
const CODE_BIT: usize = 16;
const NP: usize = DICBIT as usize + 1;
const NT: usize = CODE_BIT + 3;
const PBIT: u32 = 4;
const TBIT: u32 = 5;
const NPT: usize = 1 << TBIT;

/// unlzh.c's statics.
pub(super) struct State {
    pt_len: [u8; NPT],
    pt_table: [u16; 256],
    c_table: Vec<u16>,
    blocksize: u32,
    bitbuf: u16,
    subbitbuf: u32,
    bitcount: u32,
    /// decode's: bytes left to copy, the end reached, where to copy from.
    j: i32,
    done: bool,
    i: usize,
}

impl Default for State {
    fn default() -> Self {
        State {
            pt_len: [0; NPT],
            pt_table: [0; 256],
            c_table: vec![0; 4096],
            blocksize: 0,
            bitbuf: 0,
            subbitbuf: 0,
            bitcount: 0,
            j: 0,
            done: false,
            i: 0,
        }
    }
}

/// Which table `make_table` fills.
#[derive(Clone, Copy)]
enum Table {
    Pt,
    C,
}

impl Gzip {
    fn bad_table(&mut self) -> Stop {
        self.gzip_error("Bad table\n")
    }

    /// `fillbuf`: the bit buffer shifted `n` bits on.
    fn fillbuf(&mut self, io: &mut Io<'_>, n: u32) -> R<()> {
        let mut n = n;
        let s = &mut self.unlzh;
        s.bitbuf = (u32::from(s.bitbuf) << n) as u16;
        while n > self.unlzh.bitcount {
            n -= self.unlzh.bitcount;
            let s = &mut self.unlzh;
            s.bitbuf |= (s.subbitbuf << n) as u16;
            self.unlzh.subbitbuf = u32::from(self.try_byte(io)?.unwrap_or(0));
            self.unlzh.bitcount = 8;
        }
        let s = &mut self.unlzh;
        s.bitcount -= n;
        s.bitbuf |= (s.subbitbuf >> s.bitcount) as u16;
        Ok(())
    }

    fn getbits(&mut self, io: &mut Io<'_>, n: u32) -> R<u32> {
        let x = u32::from(self.unlzh.bitbuf) >> (BITBUFSIZ - n);
        self.fillbuf(io, n)?;
        Ok(x)
    }

    /// `left[i]` and `right[i]`: `prev` and its upper half.
    fn child(&mut self, right: bool, i: usize) -> R<usize> {
        let at = if right { WSIZE + i } else { i };
        match self.prev.get(at) {
            Some(&v) => Ok(usize::from(v)),
            None => Err(self.bad_table()),
        }
    }

    fn set_child(&mut self, right: bool, i: usize, v: u16) -> R<()> {
        let at = if right { WSIZE + i } else { i };
        match self.prev.get_mut(at) {
            Some(slot) => {
                *slot = v;
                Ok(())
            }
            None => Err(self.bad_table()),
        }
    }

    /// `make_table`: the lookup table of `tablebits` bits for the code
    /// lengths `bitlen[..nchar]` (`pt_len` or `c_len`), longer codes in the
    /// tree of `left` and `right`.
    fn make_table(&mut self, nchar: usize, which: Table, tablebits: u32) -> R<()> {
        let bitlen: Vec<u8> = match which {
            Table::Pt => self.unlzh.pt_len[..nchar].to_vec(),
            Table::C => self.outbuf[..nchar].to_vec(),
        };
        let mut count = [0u16; 17];
        let mut weight = [0u16; 17];
        let mut start = [0u16; 18];
        for &l in &bitlen {
            match count.get_mut(usize::from(l)) {
                Some(c) => *c = c.wrapping_add(1),
                None => return Err(self.bad_table()),
            }
        }
        for i in 1..=16 {
            start[i + 1] = start[i].wrapping_add(((u32::from(count[i])) << (16 - i)) as u16);
        }
        if start[17] != 0 {
            return Err(self.bad_table());
        }
        let jutbits = 16 - tablebits;
        let tb = tablebits as usize;
        for i in 1..=tb {
            start[i] >>= jutbits;
            weight[i] = 1 << (tablebits - i as u32);
        }
        for (i, w) in weight.iter_mut().enumerate().skip(tb + 1) {
            *w = 1 << (16 - i);
        }
        let table_len = 1usize << tablebits;
        let mut i = usize::from(start[tb + 1] >> jutbits);
        if i != 0 {
            while i != table_len {
                self.set_entry(which, i, 0)?;
                i += 1;
            }
        }
        let mut avail = nchar;
        let mask: u32 = 1 << (15 - tablebits);
        for (ch, &len) in bitlen.iter().enumerate() {
            let len = usize::from(len);
            if len == 0 {
                continue;
            }
            let nextcode = u32::from(start[len]) + u32::from(weight[len]);
            if len <= tb {
                if (1u32 << tablebits) < nextcode {
                    return Err(self.bad_table());
                }
                for k in u32::from(start[len])..nextcode {
                    self.set_entry(which, k as usize, ch as u16)?;
                }
            } else {
                let mut k = u32::from(start[len]);
                // Where the code goes: a table entry, or a tree node's side.
                let mut at: (Option<bool>, usize) = (None, (k >> jutbits) as usize);
                for _ in 0..len - tb {
                    let cur = self.get_at(which, at)?;
                    let node = if cur == 0 {
                        self.set_child(true, avail, 0)?;
                        self.set_child(false, avail, 0)?;
                        self.set_at(which, at, avail as u16)?;
                        avail += 1;
                        avail - 1
                    } else {
                        cur
                    };
                    at = (Some(k & mask != 0), node);
                    k <<= 1;
                }
                self.set_at(which, at, ch as u16)?;
            }
            start[len] = nextcode as u16;
        }
        Ok(())
    }

    fn set_entry(&mut self, which: Table, i: usize, v: u16) -> R<()> {
        let slot = match which {
            Table::Pt => self.unlzh.pt_table.get_mut(i),
            Table::C => self.unlzh.c_table.get_mut(i),
        };
        match slot {
            Some(s) => {
                *s = v;
                Ok(())
            }
            None => Err(self.bad_table()),
        }
    }

    fn get_at(&mut self, which: Table, at: (Option<bool>, usize)) -> R<usize> {
        match at {
            (Some(right), i) => self.child(right, i),
            (None, i) => {
                let v = match which {
                    Table::Pt => self.unlzh.pt_table.get(i),
                    Table::C => self.unlzh.c_table.get(i),
                };
                match v {
                    Some(&v) => Ok(usize::from(v)),
                    None => Err(self.bad_table()),
                }
            }
        }
    }

    fn set_at(&mut self, which: Table, at: (Option<bool>, usize), v: u16) -> R<()> {
        match at {
            (Some(right), i) => self.set_child(right, i, v),
            (None, i) => self.set_entry(which, i, v),
        }
    }

    /// A code longer than its table: down the tree from `j`, a bit of the
    /// buffer at a time from `mask`, while it is a node (`limit` or more).
    fn walk(&mut self, mut j: usize, limit: usize, mut mask: u32) -> R<usize> {
        let mut steps = 0usize;
        while j >= limit {
            // Once the mask is spent the C goes left; a cycle there would
            // be for ever.
            steps += 1;
            if steps > WSIZE {
                return Err(self.bad_table());
            }
            j = self.child(u32::from(self.unlzh.bitbuf) & mask != 0, j)?;
            mask >>= 1;
        }
        Ok(j)
    }

    /// `read_pt_len`.
    fn read_pt_len(&mut self, io: &mut Io<'_>, nn: usize, nbit: u32, i_special: i32) -> R<()> {
        let n = self.getbits(io, nbit)? as usize;
        if n == 0 {
            let c = self.getbits(io, nbit)? as u16;
            self.unlzh.pt_len[..nn].fill(0);
            self.unlzh.pt_table.fill(c);
            return Ok(());
        }
        let mut i = 0usize;
        while i < n {
            let mut c = u32::from(self.unlzh.bitbuf) >> (BITBUFSIZ - 3);
            if c == 7 {
                let mut mask: u32 = 1 << (BITBUFSIZ - 1 - 3);
                while mask & u32::from(self.unlzh.bitbuf) != 0 {
                    mask >>= 1;
                    c += 1;
                }
                if 16 < c {
                    return Err(self.bad_table());
                }
            }
            self.fillbuf(io, if c < 7 { 3 } else { c - 3 })?;
            self.set_pt_len(i, c as u8)?;
            i += 1;
            if i as i32 == i_special {
                let c = self.getbits(io, 2)?;
                for _ in 0..c {
                    self.set_pt_len(i, 0)?;
                    i += 1;
                }
            }
        }
        while i < nn {
            self.set_pt_len(i, 0)?;
            i += 1;
        }
        self.make_table(nn, Table::Pt, 8)
    }

    fn set_pt_len(&mut self, i: usize, v: u8) -> R<()> {
        match self.unlzh.pt_len.get_mut(i) {
            Some(s) => {
                *s = v;
                Ok(())
            }
            None => Err(self.bad_table()),
        }
    }

    /// `read_c_len`: the literal and length codes' lengths, into `outbuf`.
    fn read_c_len(&mut self, io: &mut Io<'_>) -> R<()> {
        let n = self.getbits(io, CBIT)? as usize;
        if n == 0 {
            let c = self.getbits(io, CBIT)? as u16;
            self.outbuf[..NC].fill(0);
            self.unlzh.c_table.fill(c);
            return Ok(());
        }
        let mut i = 0usize;
        while i < n {
            let mut c = usize::from(self.unlzh.pt_table[usize::from(self.unlzh.bitbuf >> 8)]);
            if c >= NT {
                c = self.walk(c, NT, 1 << (BITBUFSIZ - 1 - 8))?;
            }
            let len = u32::from(self.unlzh.pt_len[c]);
            self.fillbuf(io, len)?;
            if c <= 2 {
                let count = match c {
                    0 => 1,
                    1 => self.getbits(io, 4)? + 3,
                    _ => self.getbits(io, CBIT)? + 20,
                };
                for _ in 0..count {
                    self.outbuf[i] = 0;
                    i += 1;
                }
            } else {
                self.outbuf[i] = (c - 2) as u8;
                i += 1;
            }
        }
        while i < NC {
            self.outbuf[i] = 0;
            i += 1;
        }
        self.make_table(NC, Table::C, 12)
    }

    /// `decode_c`: the next literal or length, `NC` at the end.
    fn decode_c(&mut self, io: &mut Io<'_>) -> R<usize> {
        if self.unlzh.blocksize == 0 {
            self.unlzh.blocksize = self.getbits(io, 16)?;
            if self.unlzh.blocksize == 0 {
                return Ok(NC);
            }
            self.read_pt_len(io, NT, TBIT, 3)?;
            self.read_c_len(io)?;
            self.read_pt_len(io, NP, PBIT, -1)?;
        }
        self.unlzh.blocksize -= 1;
        let mut j = usize::from(self.unlzh.c_table[usize::from(self.unlzh.bitbuf >> 4)]);
        if j >= NC {
            j = self.walk(j, NC, 1 << (BITBUFSIZ - 1 - 12))?;
        }
        let len = u32::from(self.outbuf[j]);
        self.fillbuf(io, len)?;
        Ok(j)
    }

    /// `decode_p`: the next distance.
    fn decode_p(&mut self, io: &mut Io<'_>) -> R<u32> {
        let mut j = usize::from(self.unlzh.pt_table[usize::from(self.unlzh.bitbuf >> 8)]);
        if j >= NP {
            j = self.walk(j, NP, 1 << (BITBUFSIZ - 1 - 8))?;
        }
        let len = u32::from(self.unlzh.pt_len[j]);
        self.fillbuf(io, len)?;
        let mut j = j as u32;
        if j != 0 {
            j = (1 << (j - 1)) + self.getbits(io, j - 1)?;
        }
        Ok(j)
    }

    /// `decode`: up to `count` bytes into the window; how many.
    fn decode(&mut self, io: &mut Io<'_>, count: usize) -> R<usize> {
        let mut r = 0usize;
        loop {
            self.unlzh.j -= 1;
            if self.unlzh.j < 0 {
                break;
            }
            let i = self.unlzh.i;
            self.window[r] = self.window[i];
            self.unlzh.i = (i + 1) & (DICSIZ - 1);
            r += 1;
            if r == count {
                return Ok(r);
            }
        }
        loop {
            let c = self.decode_c(io)?;
            if c == NC {
                self.unlzh.done = true;
                return Ok(r);
            }
            if c <= 255 {
                self.window[r] = c as u8;
                r += 1;
                if r == count {
                    return Ok(r);
                }
            } else {
                self.unlzh.j = (c - (255 + 1 - THRESHOLD)) as i32;
                let p = self.decode_p(io)? as usize;
                self.unlzh.i = r.wrapping_sub(p).wrapping_sub(1) & (DICSIZ - 1);
                loop {
                    self.unlzh.j -= 1;
                    if self.unlzh.j < 0 {
                        break;
                    }
                    let i = self.unlzh.i;
                    self.window[r] = self.window[i];
                    self.unlzh.i = (i + 1) & (DICSIZ - 1);
                    r += 1;
                    if r == count {
                        return Ok(r);
                    }
                }
            }
        }
    }

    /// `unlzh`.
    pub fn unlzh(&mut self, io: &mut Io<'_>) -> R<i32> {
        self.unlzh.bitbuf = 0;
        self.unlzh.subbitbuf = 0;
        self.unlzh.bitcount = 0;
        self.fillbuf(io, BITBUFSIZ)?;
        self.unlzh.blocksize = 0;
        self.unlzh.j = 0;
        self.unlzh.done = false;
        while !self.unlzh.done {
            let n = self.decode(io, DICSIZ)?;
            if n > 0 {
                let r =
                    Self::write_buf(io.output, self.test, &mut self.bytes_out, &self.window[..n]);
                r.map_err(|e| self.write_error(&e))?;
            }
        }
        Ok(OK)
    }
}
