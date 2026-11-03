//! Reading Yaml as `tw jq --from` and `tw convert` read it.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = toolwing_jq::doc::parse(toolwing_jq::Format::Yaml, data);
});
