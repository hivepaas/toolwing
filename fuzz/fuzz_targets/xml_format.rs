//! XML as `tw xml format` and `minify` read it: anything is refused or
//! written, and what is written reads back. (Not always as itself: xmllint
//! drops blanks a second reading judges ignorable, and so does tw.)
#![no_main]

use toolwing_tools::xml::{Style, rewrite};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    for format in [true, false] {
        let style = Style { format, indent: "  ".into() };
        if let Ok(out) = rewrite(data, &style) {
            let again = rewrite(out.as_bytes(), &style);
            assert!(again.is_ok(), "{:?} wrote {out:?}: {again:?}", String::from_utf8_lossy(data));
        }
    }
});
