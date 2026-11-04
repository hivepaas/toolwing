//! HTML character references read as a browser reads them, in text and in
//! attributes; and any text escaped reads back as itself.
#![no_main]

use toolwing_tools::html;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let _ = html::unescape(text, false);
    let _ = html::unescape(text, true);
    for ascii in [false, true] {
        assert_eq!(html::unescape(&html::escape(text, false, ascii), false), text);
        assert_eq!(html::unescape(&html::escape(text, false, ascii), true), text);
    }
});
