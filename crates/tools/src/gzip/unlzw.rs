//! gzip's unlzw.c: compress's LZW (`.Z`), as the command decodes it, its
//! output written each time its buffer fills.
//!
//! Ported from GNU gzip 1.13's unlzw.c; Copyright (C) 1997-2023 Free
//! Software Foundation, Inc.; Copyright (C) 1992-1993 Jean-loup Gailly.
//! GPL-3.0-or-later.

use super::{BITS, ERROR, Gzip, INBUF_EXTRA, INBUFSIZ, Io, OK, OUTBUFSIZ, R};

const INIT_BITS: u32 = 9;
const BIT_MASK: u8 = 0x1f;
const BLOCK_MODE: u8 = 0x80;
const LZW_RESERVED: u8 = 0x60;
const CLEAR: i64 = 256;
const FIRST: i64 = CLEAR + 1;

fn maxcode(n: u32) -> i64 {
    1i64 << n
}

/// `posbits` moved on to the next multiple of a code group, as the C
/// rounds it when the code size changes.
fn round(posbits: i64, n_bits: u32) -> i64 {
    let g = i64::from(n_bits) << 3;
    (posbits - 1) + (g - (posbits - 1 + g) % g)
}

impl Gzip {
    /// `unlzw`.
    pub fn unlzw(&mut self, io: &mut Io<'_>) -> R<i32> {
        let flags = self.get_byte(io)?;
        let block_mode = flags & BLOCK_MODE != 0;
        if flags & LZW_RESERVED != 0 {
            let m = self.about(
                "\n",
                &format!(": warning, unknown flags 0x{:x}\n", flags & LZW_RESERVED),
            );
            self.warn(&m);
        }
        let maxbits = u32::from(flags & BIT_MASK);
        let maxmaxcode = maxcode(maxbits);
        if maxbits > BITS {
            let m = self.about(
                "\n",
                &format!(": compressed with {maxbits} bits, can only handle {BITS} bits\n"),
            );
            self.say(&m);
            self.exit_code = ERROR;
            return Ok(ERROR);
        }
        let mut rsize = self.insize;
        let mut n_bits = INIT_BITS;
        let mut maxc = maxcode(n_bits) - 1;
        let mut bitmask: i64 = (1 << n_bits) - 1;
        let mut oldcode: i64 = -1;
        let mut finchar: u8 = 0;
        let mut outpos = 0usize;
        let mut posbits = (self.inptr as i64) << 3;
        let mut free_ent = if block_mode { FIRST } else { 256 };

        // `clear_tab_prefixof`: the C zeroes 256 bytes, 128 entries.
        self.prev[..128].fill(0);
        for code in 0..256 {
            self.window[code] = code as u8;
        }
        // The decoded string, built backwards (`de_stack`).
        let mut stack: Vec<u8> = Vec::with_capacity(1 << 16);

        'resetbuf: loop {
            let o = (posbits >> 3) as usize;
            let e = self.insize.saturating_sub(o);
            if e > 0 {
                self.inbuf.copy_within(o..o + e, 0);
            }
            self.insize = e;
            posbits = 0;

            if self.insize < INBUF_EXTRA {
                let at = self.insize;
                rsize = match Self::read_buffer(io.input, &mut self.inbuf[at..at + INBUFSIZ]) {
                    Ok(n) => n,
                    Err(e) => return Err(self.read_error(Some(&e))),
                };
                self.insize += rsize;
                self.bytes_in += rsize as i64;
            }
            let insize = self.insize as i64;
            let inbits = if rsize != 0 {
                (insize - insize % i64::from(n_bits)) << 3
            } else {
                (insize << 3) - (i64::from(n_bits) - 1)
            };

            while inbits > posbits {
                if free_ent > maxc {
                    posbits = round(posbits, n_bits);
                    n_bits += 1;
                    maxc = if n_bits == maxbits {
                        maxmaxcode
                    } else {
                        maxcode(n_bits) - 1
                    };
                    bitmask = (1 << n_bits) - 1;
                    continue 'resetbuf;
                }
                let p = (posbits >> 3) as usize;
                let word = i64::from(self.inbuf[p])
                    | i64::from(self.inbuf[p + 1]) << 8
                    | i64::from(self.inbuf[p + 2]) << 16;
                let mut code = (word >> (posbits & 7)) & bitmask;
                posbits += i64::from(n_bits);

                if oldcode == -1 {
                    if code >= 256 {
                        return Err(self.gzip_error("corrupt input."));
                    }
                    finchar = code as u8;
                    oldcode = code;
                    self.outbuf[outpos] = finchar;
                    outpos += 1;
                    continue;
                }
                if code == CLEAR && block_mode {
                    self.prev[..128].fill(0);
                    free_ent = FIRST - 1;
                    posbits = round(posbits, n_bits);
                    n_bits = INIT_BITS;
                    maxc = maxcode(n_bits) - 1;
                    bitmask = (1 << n_bits) - 1;
                    continue 'resetbuf;
                }
                let incode = code;
                stack.clear();
                if code >= free_ent {
                    if code > free_ent {
                        if outpos > 0 {
                            self.write_outbuf(io, outpos)?;
                        }
                        return Err(self.gzip_error(if self.to_stdout {
                            "corrupt input."
                        } else {
                            "corrupt input. Use zcat to recover some data."
                        }));
                    }
                    stack.push(finchar);
                    code = oldcode;
                }
                while code >= 256 {
                    stack.push(self.window[code as usize]);
                    code = i64::from(self.prev[code as usize]);
                }
                finchar = self.window[code as usize];
                stack.push(finchar);

                // Out in forward order.
                let mut rest = stack.len();
                if outpos + rest >= OUTBUFSIZ {
                    while rest > 0 {
                        let i = rest.min(OUTBUFSIZ - outpos);
                        for k in 0..i {
                            self.outbuf[outpos + k] = stack[rest - 1 - k];
                        }
                        outpos += i;
                        rest -= i;
                        if outpos >= OUTBUFSIZ {
                            self.write_outbuf(io, outpos)?;
                            outpos = 0;
                        }
                    }
                } else {
                    for k in 0..rest {
                        self.outbuf[outpos + k] = stack[rest - 1 - k];
                    }
                    outpos += rest;
                }

                let code = free_ent;
                if code < maxmaxcode {
                    self.prev[code as usize] = oldcode as u16;
                    self.window[code as usize] = finchar;
                    free_ent = code + 1;
                }
                oldcode = incode;
            }
            if rsize == 0 {
                break;
            }
        }
        if outpos > 0 {
            self.write_outbuf(io, outpos)?;
        }
        Ok(OK)
    }
}
