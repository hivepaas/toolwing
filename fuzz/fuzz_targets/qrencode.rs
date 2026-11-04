//! Any text as `tw qrencode` encodes it, in every mode and level: refused
//! as too large, or a symbol of the width its version has.
#![no_main]

use qrcode::EcLevel;
use toolwing_tools::qr::{Options, encode};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((&first, text)) = data.split_first() else {
        return;
    };
    let level = [EcLevel::L, EcLevel::M, EcLevel::Q, EcLevel::H][usize::from(first & 3)];
    let o = Options {
        version: u32::from(first >> 2) % 41,
        level,
        eight_bit: first & 0x40 != 0,
        kanji: first & 0x80 != 0,
        case_sensitive: first & 0x20 == 0,
    };
    if let Ok(s) = encode(text, &o) {
        assert_eq!(s.width, 17 + 4 * s.version as usize);
        assert_eq!(s.modules.len(), s.width * s.width);
        assert!(s.version >= o.version);
    }
});
