//! Base64, Base32 and hex decoding, with and without ignoring garbage: the
//! first byte chooses which.
#![no_main]

use toolwing_core::{Ctx, Value, Values};

const DECODERS: [&str; 3] = ["base64.decode", "base32.decode", "hex.decode"];

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((&which, input)) = data.split_first() else {
        return;
    };
    let Some(tool) = toolwing_tools::find(DECODERS[usize::from(which) % 3]) else {
        return;
    };
    let mut values = Values::new();
    values.set("ignore_garbage", Value::Bool(which & 0x80 != 0));
    let _ = tool.run(&Ctx::new(), input, &values);
});
