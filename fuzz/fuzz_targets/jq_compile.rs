//! Compiling a jq filter, as `tw jq` and the app's jq page do on each key.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(code) = std::str::from_utf8(data) {
        // On the stack tw jq and the app's worker compile on.
        let compiled = || toolwing_jq::compile("<fuzz>", code, &[], &[]).is_ok();
        let _ = toolwing_jq::doc::on_large_stack(compiled);
    }
});
