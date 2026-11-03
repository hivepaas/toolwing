# Quality

What "production-ready" means here, checked by CI rather than promised.

## Compatibility with the reference tools

Every tool with a reference (tools.md) has compatibility cases:

```
tests/compat/base64/wrap-0.case
  args:   base64 -w 0
  stdin:  @files/random-1k.bin
  stdout: @expected/wrap-0.out
  exit:   0
```

- **Recorded from the real tool, not written by hand.**
  `scripts/compat_record.sh` runs each case against the reference at its pinned
  version (jq 1.8.2, GNU coreutils 9.12, b3sum 1.8, OpenSSL 3) in a container,
  and the expected output is committed.
- CI runs every case against `tw` on Linux on each pull request, and on macOS
  and Windows nightly and before a release (their minutes cost more, as in
  shellwing's CI). A weekly job records them again from the real tools and fails on
  any drift, so a new reference version is noticed.
- Cases cover the commonly used flags, empty input, input without a trailing
  newline, invalid input, large input, and non-ASCII text.
- `tw jq` also runs jq's own suite (tools.md, "jq compatibility"), with the known
  differences listed and each one explained.

## Other tests

- **Round trips** with `proptest`: decode(encode(x)) == x for every encoding,
  conversions that keep the value (JSON → YAML → JSON), and the "copy as
  command" round trip (architecture.md).
- **Snapshots** with `insta` for `--help`, `--json` output and error messages,
  so a change to the public surface shows in review.
- **Fuzzing** with `cargo-fuzz`: one target for each parser (JSON, YAML, TOML,
  XML, CSV, JWT, X.509, cron, base64/32/hex decoding, jq filter compilation).
  A nightly job runs each for 10 minutes; crashes become regression tests.
  Nothing a tool is given may panic.
- **UI tests** headless, with the same `headless` feature and screenshots as
  tracewing.

## Performance

- Benchmarks with `criterion` for the tools, and `hyperfine` for the CLI
  against the reference on the same corpus, run nightly; the numbers go into
  `docs/benchmarks.md`.
- Budgets that fail the nightly job: no tool slower than 1.5 times its
  reference; `tw --version` under 5 ms; streaming tools in constant memory;
  the app showing its first frame under 150 ms and keeping its frame rate while
  results come in.

## Security

- **No network in the tools or the CLI.** `cargo-deny` bans HTTP and socket
  crates from `toolwing-tools` and `toolwing-cli`'s dependency trees; the app
  alone may reach the network, for update checks as shellwing does.
- `unsafe_code = "forbid"` in our crates.
- Randomness for passwords, tokens and keys from the operating system's
  generator, never a seeded one. Comparisons of MACs and hashes in
  verification are constant-time.
- Secrets in the app are zeroized when dropped and never logged.
- `cargo-deny` for advisories and licenses (all compatible with
  GPL-3.0-or-later), Dependabot, CodeQL for Rust and GitHub Actions: the
  `main` ruleset requires code scanning results.
- Resource exhaustion in jq and the parsers is handled by the worker process
  in the app (architecture.md); the CLI behaves as jq does.

## Releases

- Semantic versions. The CLI's commands, flags, exit codes and `--json` are the
  public API (cli.md); breaking them waits for a major version.
- Releases as in shellwing: signed tags (the tag rulesets require signatures),
  a signed list of releases for the app's update check, notes per release.
- Each release lists the reference versions it was tested against.
- CI on every pull request: `cargo fmt --check`, `cargo clippy -D warnings`,
  tests and compatibility cases on Linux, `cargo-deny`; the three systems by
  hand (Actions > CI > Run workflow) and nightly.
