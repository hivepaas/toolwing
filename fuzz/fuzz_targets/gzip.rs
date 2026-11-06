//! gzip's decoders as `tw gzip -d` runs them, on any bytes: inflate, LZW,
//! pack, LZH and a zip archive's entry (the first byte picks the magic
//! number put first, to reach each), and what is compressed decompresses
//! back to itself.
#![no_main]

use toolwing_tools::gzip::{Compress, decompress};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else {
        return;
    };
    let magic: &[u8] = match pick % 7 {
        0 => b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x00\x03",
        1 => b"\x1f\x9d",
        2 => b"\x1f\x1e",
        3 => b"\x1f\xa0",
        4 => b"PK\x03\x04",
        _ => b"",
    };
    let input = [magic, rest].concat();
    let _ = decompress(&input);

    let gz = Compress {
        level: i32::from(pick % 9) + 1,
        ..Compress::default()
    }
    .run(rest);
    assert_eq!(decompress(&gz).as_deref(), Ok(rest));
});
