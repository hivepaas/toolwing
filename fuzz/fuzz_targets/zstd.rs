//! zstd frames as `tw zstd` reads them: `--list`'s walk over any bytes,
//! decompressing them (refused, or written), and what is compressed
//! decompresses back to itself.
#![no_main]

use toolwing_tools::zstd::{Compress, Decompress, compress, decompress, info};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = info(data);
    let _ = decompress(&mut &data[..], &mut std::io::sink(), &Decompress::default());
    let mut z = Vec::new();
    let o = Compress { level: 1, pledged: Some(data.len() as u64), ..Compress::default() };
    if compress(&mut &data[..], &mut z, &o).is_ok() {
        let mut back = Vec::new();
        assert!(decompress(&mut &z[..], &mut back, &Decompress::default()).is_ok() || data.is_empty());
        assert_eq!(back, data);
        let (i, e, _) = info(&z);
        assert!(e.is_none(), "{e:?}");
        assert_eq!(i.decompressed, data.len() as u64);
    }
});
