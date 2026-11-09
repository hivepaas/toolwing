//! sort on any lines with any keys, as `tw sort` orders them: nothing
//! panics, the output is the input reordered, it checks as sorted, and
//! `strtold` (sort -g) reads any bytes.
#![no_main]

use toolwing_tools::sort::{Ignore, Key, NONE, Sorter, float::strtold, lines};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((&[a, b, c, d], rest)) = data.split_first_chunk::<4>() else {
        return;
    };
    let _ = strtold(rest);
    let key = Key {
        sword: if a % 3 == 0 { NONE } else { usize::from(a % 3) },
        schar: usize::from(b % 3),
        eword: if b & 0x80 != 0 { NONE } else { usize::from(b % 4) },
        echar: usize::from(c % 3),
        ignore: [Ignore::None, Ignore::Nondictionary, Ignore::Nonprinting][usize::from(c / 4 % 3)],
        translate: c & 0x40 != 0,
        skipsblanks: a & 0x10 != 0,
        skipeblanks: a & 0x20 != 0,
        numeric: d % 8 == 1,
        general_numeric: d % 8 == 2,
        human_numeric: d % 8 == 3,
        month: d % 8 == 4,
        version: d % 8 == 5,
        random: d % 8 == 6,
        reverse: d & 0x80 != 0,
        traditional_used: false,
    };
    let s = Sorter {
        keys: if d & 0x40 != 0 { Vec::new() } else { vec![key] },
        tab: if a & 0x40 != 0 { Some(b':') } else { None },
        eol: b'\n',
        reverse: d & 0x20 != 0,
        stable: d & 0x10 != 0,
        unique: d & 0x08 != 0,
        seed: [c; 16],
    };
    let input = lines(rest, b'\n');
    let mut sorted = input.clone();
    s.sort(&mut sorted);
    let mut a1 = input.clone();
    let mut a2 = sorted.clone();
    a1.sort();
    a2.sort();
    assert_eq!(a1, a2);
    let out = s.unique_lines(sorted);
    assert_eq!(s.check(&out), None);
    for l in &out {
        let _ = s.debug_line(l);
    }
});
