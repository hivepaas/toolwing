# Architecture

## Crates

```
crates/
  core/    toolwing-core    the tool model: Meta, Param, Value, Output, Error, Ctx
  tools/   toolwing-tools   every tool, one module per group; the registry
  cli/     toolwing-cli     binary `tw`: commands built from the registry
  app/     toolwing         binary `toolwing`: GPUI + gpui-component
```

Dependencies point one way: `core` ← `tools` ← `cli`, `app`.

- **`core` is small and has no heavy dependencies**, so its types change
  rarely and everything builds on them quickly.
- **`tools` never depends on GPUI, clap or anything that does I/O on its own.**
  It reads the input it is handed and writes to the output it is handed; files,
  stdin and the clipboard belong to `cli` and `app`.
- **`cli` never depends on GPUI.** `tw` has to run on a server or in a
  container with no window system: Linux builds are static (musl) and link no
  X11, Wayland or Vulkan libraries.
- Workspace layout, lints and profiles as in shellwing: `resolver = "3"`,
  `members = ["crates/*"]`, `unsafe_code = "forbid"`, `unwrap_used = "warn"`.

GPUI as the other apps take it, a newer snapshot:

```toml
gpui = { package = "gpui-pre", version = "=0.3.11" }
gpui_platform = { package = "gpui-pre-platform", version = "=0.3.11", default-features = false, features = ["font-kit", "runtime_shaders"] }
gpui-component = "=0.9.0"
```

gpui-component pins one `gpui-pre` per release, so the two move together, in a
pull request of their own. shellwing's Wayland patch to `gpui-pre-linux` is not
taken in: it matters when one window activates another, and toolwing has one.

## The tool model

A tool is one operation: `base64.encode`, `jwt.decode`, `jq`. Tools that
belong together share a group (`base64`), which is one page in the app and one
command in the CLI.

```rust
pub struct Meta {
    pub id: &'static str,             // "base64.encode"
    pub group: &'static str,          // "base64"
    pub title: &'static str,          // "Base64 encode"
    pub summary: &'static str,        // one line, for the palette and `tw --help`
    pub category: Category,           // Encoding, Data, Crypto, Time, Text, Web, Network, ...
    pub keywords: &'static [&'static str],
    pub input: InputKind,             // None, Text, Bytes
    pub streams: bool,                // can run in constant memory (run_stream)
    pub reference: Option<Reference>, // the tool we match, e.g. GNU coreutils base64 9.12
}

pub struct Param {
    pub key: &'static str,            // "wrap"
    pub label: &'static str,          // "Wrap lines at"
    pub help: &'static str,
    pub kind: ParamKind,              // Flag, Choice, Int { min, max }, Text, Secret, SecondInput
    pub default: Value,
    pub cli: Cli,                     // long and short names, as the reference spells them
}

pub trait Tool: Sync {
    fn meta(&self) -> &'static Meta;
    fn params(&self) -> &'static [Param] { &[] }
    fn run(&self, ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error>;
    fn run_stream(&self, ctx: &Ctx, input: &mut dyn Read, out: &mut dyn Write, values: &Values)
        -> Result<(), Error> { /* default: read all, run, write */ }
    fn detect(&self, sample: &[u8]) -> Option<Score> { None }
}
```

- **`Output`** is `Text`, `Bytes`, `Fields` (named values, such as a JWT's header
  and claims or a certificate's subject and expiry) or `Image` (a QR code). The
  CLI prints `Fields` as aligned text or, with `--json`, as an object; the app
  shows them as rows with a copy button each.
- **`Error`** has a kind (`Usage`, `Input`, `Check` for a verification that
  failed) and an optional span: the line and column in the input or the filter
  where parsing stopped. The CLI prints the position; the app underlines it.
- **`Ctx`** carries a cancellation flag and limits. Long loops (a jq filter,
  a regex over a large text, a bcrypt cost) check it; the CLI never cancels,
  the app cancels when the input changes.
- **`detect`** looks at up to 64 KB and says how sure it is that the sample is
  its kind of input: a JWT, a UUID, an epoch timestamp, a PEM block. The app
  uses it to suggest a tool for the clipboard; `tw detect` prints the ranking.
- `SecondInput` is for tools with two inputs: the other side of a diff, the key
  of an HMAC, the secret that verifies a JWT. `Secret` values are never printed
  in "copy as command" (app.md) and are zeroized when dropped.

## The registry

`toolwing-tools` exports one static list:

```rust
pub static TOOLS: &[&dyn Tool] = &[&base64::Encode, &base64::Decode, &jq::Jq, /* ... */];
```

A plain list rather than `inventory` or linker tricks: the order is the order
shown in `tw --help` and the app's sidebar, and a missing tool is a compile
error, not a silent absence. A unit test walks the list and checks that ids are
unique, every param has help text, every group has compatibility cases
(quality.md) and every reference names a version.

## How the CLI and the app use it

- **CLI:** each group becomes a clap subcommand, built with clap's builder API
  from `Meta` and `Param`, so flags cannot drift from the app's options. Tools
  whose reference has an unusual command line, `jq` and the `*sum` hashes, have
  a hand-written parser that fills the same `Values`.
- **App:** each group gets a generic page built from its params: input editor,
  options bar, output view. A group can register its own page instead (color,
  QR, diff, cron), still calling the same `Tool::run`.
- **Round trip:** for every tool, a test fills random `Values`, turns them into a
  `tw` command line the way "copy as command" does, parses it back with the CLI
  and checks the two are equal.

## Threads and the worker

- The app never runs a tool on the UI thread. Each run goes to the background
  executor with a generation number; a result whose generation is not the
  latest is dropped, and the previous run is cancelled through `Ctx`.
- **Untrusted filters run in a worker process.** jaq does not guard against
  resource exhaustion: `def f: 1+f; f` overflows the stack, which aborts the
  whole process. So the app runs `jq`, regex and the data conversions in a
  child process (the app's own binary started with `--worker`), with a time and
  memory limit; a crash ends the worker, not the app, and shows as an error. A
  new worker is started on demand. The CLI runs in-process, as jq does.
- **Open:** whether every tool goes through the worker, for one code path, or
  only those that run user-written code or parse arbitrarily nested input.
