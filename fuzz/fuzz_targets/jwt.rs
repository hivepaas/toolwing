//! A JWT, decoded as `tw jwt decode` does.
#![no_main]

use toolwing_core::{Ctx, Values};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Some(tool) = toolwing_tools::find("jwt.decode") {
        let _ = tool.run(&Ctx::new(), data, &Values::new());
    }
});
