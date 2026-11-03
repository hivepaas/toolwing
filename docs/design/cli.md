# The command line: `tw`

## Command shape

```
tw <group> [<action>] [options] [input]
```

```sh
curl -s api.example.com/users | tw jq -r '.[].email'
tw base64 -d < blob.b64 > blob.bin
tw sha256sum -c SHA256SUMS
tw jwt decode "$TOKEN"
tw time 1762156800 --tz Asia/Ho_Chi_Minh
tw uuid --v7 -n 5
kubectl get cm app -o yaml | tw convert yaml json | tw jq '.data'
```

Two kinds of command, and every tool says which it is (tools.md):

- **Drop-in.** A reference tool exists and people know its flags: `jq`,
  `base64`, `base32`, `md5sum`, `sha1sum`, `sha256sum`, `sha512sum`, `b2sum`,
  `b3sum`, `diff -u`. The command is named after it, takes its flags, reads
  its inputs (files, `-` for stdin) and writes the same bytes and exit codes.
  `tw jq` can stand in for `jq` in a script with no other change.
- **Native.** No single reference tool, or one whose interface is not worth
  copying (`jwt`, `time`, `cron`, `url`, `uuid`). These follow the conventions
  below.

## Input (native commands)

- The positional argument is the value itself: `tw url encode 'a b&c'`.
- With no positional argument, the input is stdin. `-` says so explicitly.
- `-i FILE` / `--input FILE` reads a file. Input is bytes; tools that need text
  require UTF-8 and say where the first invalid byte is.
- A trailing newline on stdin is dropped for single-value tools (`printf` vs
  `echo` should not change a JWT); `--keep-newline` keeps it.
- Secrets (`--secret`, `--key`) also come from `--secret-file FILE` or an
  environment variable named with `--secret-env NAME`, so they need not appear
  in shell history or `ps`.

## Output

- Results go to stdout, messages to stderr. Text results end with one newline;
  `--no-newline` drops it (long only: `-n` is a count, as in `head -n`).
  Binary results are written as they are, and refused on a terminal.
- **`--json`** on every command that produces `Fields` or several values: one
  JSON object, keys in a documented, stable order. Without it, aligned
  `key: value` lines meant for people, which scripts should not parse.
- Color only on a terminal: `--color auto|always|never`, `NO_COLOR` and
  `CLICOLOR_FORCE` respected. Drop-in commands color only where the reference
  does (`jq -C`).
- Errors: `tw: <command>: <message>`, with `at line 3, column 17` when the
  error has a span, and the offending line with a caret on a terminal.

## Exit codes

| Code | Native commands |
| --- | --- |
| 0 | Success |
| 1 | A check failed: a signature, hash or password did not verify, a regex did not match (as `grep`) |
| 2 | Usage error: unknown flag, missing argument |
| 3 | Invalid input: not base64, not JSON, an expired JWT with `--require-valid` |
| 4 | A file could not be read or written |

Drop-in commands keep their reference's codes: `tw jq` exits 2 on a usage or
system error, 3 on a filter that does not compile, 5 on a runtime error, and
with `-e` 1 or 4 as jq does; `tw sha256sum -c` exits 1 when a line fails.

## Large input

Tools that can (`Meta::streams`) run in constant memory: hashes, base64,
base32, hex and conversions of newline-delimited JSON. `tw sha256sum` of a
50 GB file uses a few megabytes. Others read the whole input; `tw jq` reads
values one at a time as jq does, and `--slurp` reads them all.

## What else ships

- `tw --help`, `tw <group> --help`: generated from the registry, the same text
  as the app's tooltips. Help text is snapshot-tested (quality.md).
- `tw completions bash|zsh|fish|powershell|elvish` (clap_complete), and man
  pages `tw(1)`, `tw-<group>(1)` built at release (clap_mangen).
- `tw detect [input]`: which tools think the input is theirs, best first.
- `tw list [--json]`: every tool, its group, category and reference.
- No configuration file and no `TW_*` defaults in the first release: a command
  means the same on every machine.

## Stability

The commands, flags, exit codes and `--json` output are the public API: a
change that breaks a script waits for a major version. The human-readable
output of native commands may change in a minor version. Drop-in commands
follow their reference: a difference from it is a bug.

## Platforms

Linux x86_64 and aarch64 (static, musl), macOS universal, Windows x86_64 and
aarch64. Packages: `cargo install toolwing-cli`, Homebrew, `.deb` and `.rpm`
(as shellwing builds them), winget, Scoop, and a container image with `tw`
alone.

**The name `tw`.** The binary is `tw`. Before the first release, its name is
checked against Debian, Fedora, Arch, Homebrew, Scoop and winget; where it is
taken, packages install `toolwing-cli` and leave `tw` out.

**Open:** whether `tw jq` is also installed as `jq` by an opt-in package, for
machines that have none.
