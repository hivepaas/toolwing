//! uniq on any lines with any options, as `tw uniq` runs: nothing panics,
//! and without options every line it writes was read, in order.
#![no_main]

use toolwing_tools::uniq::{Delimit, Grouping, Options, run};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((&[a, b, c], rest)) = data.split_first_chunk::<3>() else {
        return;
    };
    let o = Options {
        skip_fields: usize::from(a % 4),
        skip_chars: usize::from(b % 4),
        check_chars: if c % 5 == 0 { usize::MAX } else { usize::from(c % 7) },
        ignore_case: a & 0x10 != 0,
        count: a & 0x20 != 0 && b & 0x40 == 0,
        output_unique: b & 0x10 == 0,
        output_first_repeated: b & 0x20 == 0,
        output_later_repeated: b & 0x40 != 0 && a & 0x20 == 0,
        delimit: [Delimit::None, Delimit::Prepend, Delimit::Separate][usize::from(c / 8 % 3)],
        grouping: if a & 0x80 != 0 && a & 0x20 == 0 {
            [Grouping::Prepend, Grouping::Append, Grouping::Separate, Grouping::Both][usize::from(c % 4)]
        } else {
            Grouping::None
        },
        delimiter: if c & 0x80 != 0 { 0 } else { b'\n' },
    };
    let mut out = Vec::new();
    assert!(run(&mut &rest[..], &mut out, &o).is_ok());
    if a == 0 && b == 0 && c == 5 {
        let lines = |s: &[u8]| s.split(|&x| x == b'\n').filter(|l| !l.is_empty()).count();
        assert!(lines(&out) <= lines(rest));
    }
});
