//! A CSS color as `tw color` reads it: refused, or read; what is read is
//! written in every form, and its computed value reads back as itself.
#![no_main]

use toolwing_tools::color::{forms, parse};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(c) = parse(text) {
        let f = forms(&c);
        for form in [f.hex(), f.rgb(), f.hsl(), f.hwb(), f.lab(), f.lch(), f.oklab(), f.oklch()] {
            assert!(parse(&form).is_ok(), "{text:?} wrote {form:?}");
        }
        let css = c.computed();
        if !css.contains("calc(") {
            let again = parse(&css).unwrap_or_else(|e| panic!("{text:?} computed {css:?}: {e}"));
            assert_eq!(again.computed(), css, "{text:?}");
        }
    }
});
