# Design notes

What toolwing is and how it is built, as agreed in November 2026. These are
decisions to build against, not descriptions of code that exists: none of it is
implemented yet. Questions still to settle are marked **Open**.

| Note | Covers |
| --- | --- |
| [architecture.md](architecture.md) | Crates, the tool model, the registry, threads and workers |
| [cli.md](cli.md) | `tw`: command shape, input and output, exit codes, the stable surface |
| [tools.md](tools.md) | Which tools, in which order, matching which reference tool, on which crate |
| [app.md](app.md) | The desktop app: layout, live results, the palette, clipboard suggestions |
| [quality.md](quality.md) | Compatibility tests, fuzzing, benchmarks, security, releases |

Principles shared by all of them:

- **Popular first.** A tool earns its place by being used every day across the
  industry: it has a widely used command-line counterpart (`jq`, `base64`,
  `sha256sum`, `date`) or is a task developers do by hand all the time
  (decoding a JWT, reading a certificate). Breadth comes after depth.
- **Match the tools people know.** Where a reference tool exists, `tw` takes
  its flags and writes the same bytes, and the tests check it against the real
  tool. Where it does not, `tw` follows the common CLI conventions in
  [cli.md](cli.md).
- **One implementation.** The app and the CLI run the same code for every tool;
  the app can show the `tw` command that gives the result on screen.
- **Scripts are first-class.** Stable output, documented exit codes, `--json`
  for structured results, streaming for large input. The CLI's flags and JSON
  are a public API under semver.
- **Local only.** No tool touches the network, and nothing is collected. Input
  may be a production secret; it never leaves the machine and the app keeps
  none of it unless asked.
- **The same stack as the other -wing apps.** Rust 1.99, edition 2024,
  GPL-3.0-or-later, the workspace lints of shellwing; GPUI as `gpui-pre`, here
  with gpui-component on top.
