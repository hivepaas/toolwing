<div align="center">

# toolwing

**Fast, native developer tools for macOS, Windows and Linux.**

The everyday conversions, encoders, decoders and inspectors in one GPU-rendered desktop app, and the same tools on the command line: `tw`, a jq-compatible query tool, base64, hashes, JWTs, timestamps and more, built in Rust.

[![Rust](https://img.shields.io/badge/Rust-1.99+-DEA584?style=flat&logo=rust)](https://www.rust-lang.org)
[![GPUI](https://img.shields.io/badge/UI-GPUI-7B61FF?style=flat)](https://github.com/zed-industries/zed/tree/main/crates/gpui)
[![License](https://img.shields.io/badge/License-GPL--3.0-blue.svg)](LICENSE)

</div>

---

> **Status:** early. The workspace, the CLI and the app run with the first
> tools (`base64`, `url`, `uuid`); the plan is in [docs/design](docs/design/README.md).

## Building

Rust 1.99 (rustup reads `rust-toolchain.toml`) and, on Linux, the packages GPUI
links (Wayland, X11, Vulkan, fontconfig: the list is in `.github/workflows/ci.yml`).

```sh
cargo run -p toolwing-cli -- base64 --help   # tw
cargo run -p toolwing                        # the app
cargo test --workspace
```

`scripts/compat_record.sh` records what the reference tools give for the
compatibility cases (Docker); `cargo test` checks `tw` against them.

## License

toolwing is free software under the [GNU General Public License v3.0 or later](LICENSE).
The name and logo are covered by [TRADEMARKS.md](TRADEMARKS.md).
