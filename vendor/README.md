# Vendored crates

Crates taken in as source because toolwing needs them to behave differently,
patched in for the whole dependency graph with `[patch.crates-io]` in the
workspace's `Cargo.toml`. Each is first committed as published, so `git log`
on its folder shows every change made to it. They are not workspace members:
they keep their own lints, and their own tests run with
`cargo test --manifest-path vendor/<crate>/Cargo.toml`.

## jaq-json 2.0.3

jaq's value type, its JSON reader and writer (MIT, `jaq-json/LICENSE-MIT`).
Changed so that `tw jq` behaves as jq 1.8.2 (docs/design/jq.md):

- (none yet)
