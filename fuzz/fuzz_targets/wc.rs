//! wc's counts of any bytes, as `tw wc` counts them: the same whether the
//! input comes at once or in pieces cut anywhere (inside a character too).
#![no_main]

use toolwing_tools::wc::{Counter, count};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((&cut, rest)) = data.split_first() else {
        return;
    };
    let whole = count(&mut &rest[..], false).0;
    let mut c = Counter::new(false);
    for piece in rest.chunks(usize::from(cut % 7) + 1) {
        c.feed(piece);
    }
    assert_eq!(c.finish(), whole);
    assert!(whole.chars <= whole.bytes && whole.lines <= whole.bytes);
});
