//! A certificate or chain, PEM or DER, read as `tw cert` reads it.
#![no_main]

use toolwing_core::{Ctx, Value, Values};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Some(tool) = toolwing_tools::find("cert") {
        let mut values = Values::new();
        values.set("at", Value::Text("2026-11-04T00:00:00Z".into()));
        let _ = tool.run(&Ctx::new(), data, &values);
    }
});
