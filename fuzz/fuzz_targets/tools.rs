//! Every tool that takes input, given any: the first byte chooses the tool,
//! which runs with its defaults.
#![no_main]

use toolwing_core::{Ctx, InputKind, Values};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let tools: Vec<_> = toolwing_tools::TOOLS
        .iter()
        .filter(|t| t.meta().input != InputKind::None)
        .collect();
    let Some((&which, input)) = data.split_first() else {
        return;
    };
    let tool = tools[usize::from(which) % tools.len()];
    let _ = tool.run(&Ctx::new(), input, &Values::new());
});
