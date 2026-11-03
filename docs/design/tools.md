# Tools

Which tools, in which order. A tool is listed only if it is used widely: it has
a well-known command-line counterpart, or it is something developers do by
hand every day. Each one has a **reference**, the tool or standard whose
behaviour it matches and is tested against (quality.md), and a crate chosen for
being maintained and widely used, not for being new.

Compatibility levels: **drop-in** (flags, output bytes and exit codes of the
reference; cli.md), **output** (the same results as the reference, our own
flags), **standard** (a specification, no single tool).

## 1.0: the first release

Every tool here is complete when it ships: the reference's commonly used flags,
`--json` where it has fields, compatibility cases, a fuzz target for each
parser.

| Command | Does | Reference | Level | Crate |
| --- | --- | --- | --- | --- |
| `jq` | Query and transform JSON (and YAML, TOML, XML, CBOR, CSV with `--from`/`--to`) | jq 1.8.2 | drop-in | `jaq-core`, `jaq-std`, `jaq-json`, `jaq-fmts` (jaq 3.1) |
| `json` | Format, minify, validate, sort keys | `jq .`, `jq -c .`, `jq -S .` | output | `jaq-json`, so it formats as `tw jq` does |
| `convert` | JSON ⇄ YAML ⇄ TOML ⇄ CSV ⇄ XML | YAML 1.2, TOML 1.1, RFC 4180 | standard | `jaq-fmts` |
| `base64`, `base32` | Encode, decode (`-d`, `-w`, `-i`), URL-safe alphabet | GNU coreutils 9.12 | drop-in | `data-encoding` |
| `hex` | Encode, decode | `xxd -p`, `xxd -r -p` | output | `data-encoding` |
| `url` | Encode, decode, parse into fields | RFC 3986, WHATWG URL | standard | `percent-encoding`, `url` |
| `md5sum`, `sha1sum`, `sha256sum`, `sha512sum`, `b2sum` | Hash files and check lists (`-c`, `--tag`, `-z`) | GNU coreutils 9.12 | drop-in | RustCrypto `md-5`, `sha1`, `sha2`, `blake2` |
| `b3sum` | BLAKE3 | b3sum 1.8 | drop-in | `blake3` |
| `hmac` | HMAC-SHA1/256/512 of input with a key | `openssl dgst -hmac` | output | RustCrypto `hmac` |
| `jwt` | Decode, verify (HS/RS/ES/PS/EdDSA, JWKS file), encode | RFC 7519, 7515 | standard | `jsonwebtoken` |
| `time` | Epoch ⇄ RFC 3339/ISO 8601/RFC 2822, time zones, relative ("in 3 days"), durations | `date -d @N`, IANA tz | output | `jiff` |
| `uuid` | Generate v4/v7 (v1, v3, v5 too), inspect version and time | RFC 9562, `uuidgen` | standard | `uuid` |
| `ulid` | Generate, inspect, convert to UUID | ULID spec | standard | `ulid` |
| `cron` | Explain an expression, list the next runs | crontab(5), Quartz and Kubernetes forms | standard | `croner` |
| `regex` | Test, list matches and groups, replace | Rust `regex`; PCRE-style with `--pcre` | standard | `regex`, `fancy-regex` |
| `diff` | Compare two texts or files (unified), JSON-aware with `--json-values` | `diff -u` | drop-in | `similar` |
| `case` | camel, snake, kebab, pascal, constant, title, sentence | — | standard | `heck` |
| `random` | Passwords, tokens, bytes as hex or base64 | `openssl rand -hex/-base64` | output | `rand` with the OS generator |
| `base` | Number bases 2–36, with grouping | — | standard | std |

## 1.x: next, in this order

| Command | Does | Reference | Crate |
| --- | --- | --- | --- |
| `cert` | Read X.509 (PEM/DER): subject, SANs, issuer, validity, fingerprints, chain order | `openssl x509 -noout -text` | `x509-parser` |
| `bcrypt`, `argon2` | Hash and verify passwords | `htpasswd -B`, `argon2` CLI | `bcrypt`, `argon2` |
| `cidr` | Subnet calculator, contains, split | `ipcalc` | `ipnet` |
| `html` | Escape, unescape entities | WHATWG | to choose |
| `xml` | Format, minify | `xmllint --format` | `quick-xml` |
| `qr` | Generate (PNG, SVG, terminal), read from an image | — | `qrcode` |
| `color` | Convert hex/RGB/HSL/OKLCH, contrast ratio | CSS Color 4 | to choose |
| `gzip`, `zstd` | Compress, decompress (with base64 for pasting) | `gzip`, `zstd` | `flate2`, `zstd` |
| `text` | Sort, unique, count lines/words/bytes, trim | `sort`, `uniq`, `wc` | std |

## Later, if asked for

`sql` formatting, `chmod` calculator, HTTP status and MIME lookup, lorem ipsum,
slugs, user-agent parsing, basic auth header, `.env` ⇄ JSON. Each needs the
same bar as the others: a reference and tests before it ships.

## Not in toolwing

- Anything that talks to the network: HTTP clients, DNS lookups, port scans.
  Tools stay local (README.md).
- What a dedicated tool does better and people already have: `curl`, `openssl`
  as a whole, `git diff`, image editing.

## jq compatibility

`tw jq` runs on jaq, which matches jq in most programs but not all. The
differences are part of the design, not discovered later:

- jq's own test suite (`tests/jq.test`, `tests/man.test` from jq 1.8.2) runs
  against `tw jq` in CI (quality.md). Each failing case is listed in
  `tests/jq/differences.md` with the reason.
- A difference that changes the result of a common program is fixed: upstream
  in jaq first, through a patch taken in (as shellwing does with
  `gpui-pre-linux`) only while waiting.
- Error messages are not compared word for word; exit codes and the position
  of a compile error are.
- The command line is jq's: `-r`, `-j`, `-a`, `-c`, `-n`, `-s`, `-e`, `-S`,
  `-C`/`-M`, `--tab`, `--indent n`, `--arg`, `--argjson`, `--slurpfile`,
  `--rawfile`, `--args`, `--jsonargs`, `-f`, `-L`, `--seq`, `--stream`,
  `--raw-input`/`-R`, `--raw-output0`. jaq's extras (`--from`, `--to`) are
  added under the same names.

**Open:** how far below 100% of jq's suite the 1.0 release may be, and which
of jaq's intended differences (it calls some of them corrections) we keep.
