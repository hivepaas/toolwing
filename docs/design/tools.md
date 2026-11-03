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
| `jq` | Query and transform JSON (and YAML, TOML, XML, CBOR, CSV with `--from`/`--to`) | jq 1.8.2 | drop-in | `toolwing-jq` on `jaq-core`, `jaq-std`, `jaq-fmts` (jaq 3.1), `onig` ([jq.md](jq.md)) |
| `json` | Format, minify, validate, sort keys | `jq .`, `jq -c .`, `jq -S .` | output | `toolwing-jq`'s writer, so it formats as `tw jq` does |
| `convert` | JSON ⇄ YAML ⇄ TOML ⇄ CSV ⇄ XML | YAML 1.2, TOML 1.1, RFC 4180 | standard | `jaq-fmts` |
| `base64`, `base32` | Encode, decode (`-d`, `-w`, `-i`), URL-safe alphabet | GNU coreutils 9.7 | drop-in | `data-encoding` |
| `hex` | Encode, decode | `xxd -p`, `xxd -r -p` | output | `data-encoding` |
| `url` | Encode, decode, parse into fields | RFC 3986, WHATWG URL | standard | `percent-encoding`, `url` |
| `md5sum`, `sha1sum`, `sha256sum`, `sha512sum`, `b2sum` | Hash files and check lists (`-c`, `--tag`, `-z`, `-l`) | GNU coreutils 9.7 | drop-in | RustCrypto `md-5`, `sha1`, `sha2`; `blake2b_simd` (blake2 0.11 takes no length at run time) |
| `b3sum` | BLAKE3 | b3sum 1.8 | drop-in | `blake3`; the app shows BLAKE3 already, the command waits for a reference image to record its cases from (`debian:13` has no b3sum) |
| `hmac` | HMAC-SHA1/256/512 of input with a key | `openssl dgst -hmac` | output | RustCrypto `hmac` |
| `jwt` | Decode (times in your zone), verify (HMAC secret, PEM, JWK or JWKS by `kid`; a key of the wrong kind refused; exp/nbf with a leeway, iss, aud), encode | RFC 7519, 7515 (its appendix A.1 is a test) | standard | `jsonwebtoken` with its pure-Rust crypto; its `rsa` crate carries RUSTSEC-2023-0071 (Marvin, timing of private-key operations: RS/PS signing, not verifying) |
| `time` | Epoch (unit guessed from its size) ⇄ RFC 3339/ISO 8601/RFC 9557/RFC 2822, time zones, relative ("in 3 days", "2 hours ago"), `--format` strftime | GNU date 9.7 (`-R`, `%s`, `%:z`; ISO in UTC ends in `Z` where `date -Iseconds` writes `+00:00`) | output | `jiff`, its zone database bundled so containers with no tzdata know every zone |
| `uuid` | Generate v4/v7 (v1, v3, v5 too), inspect version and time | RFC 9562, `uuidgen` | standard | `uuid` |
| `ulid` | Generate, inspect, convert to UUID | ULID spec | standard | `ulid` |
| `cron` | Explain an expression, list the next runs | crontab(5), Quartz and Kubernetes forms | standard | `croner` |
| `regex` | Test, list matches and groups, replace | Rust `regex`; PCRE-style with `--pcre` | standard | `regex`, `fancy-regex` |
| `diff` | Compare two texts or files (unified), JSON-aware with `--json-values` | `diff -u` | drop-in | `similar` |
| `case` | camel, pascal, snake, constant, kebab, train, dot, title, sentence; every one at once by default | — | standard | `heck`'s word boundaries |
| `random` | Passwords, tokens, bytes as hex or base64 | `openssl rand -hex/-base64` | output | `rand` with the OS generator |
| `base` | Number bases 2–36, `0x`/`0o`/`0b` prefixes, grouping, up to 128 bits, signed and unsigned at a width (`--bits`) | — | standard | std |

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

`tw jq` runs on jaq's engine with a compatibility layer of our own. How close
jaq is (measured), what differs, where each difference is fixed and the gates
for calling it a drop-in replacement: [jq.md](jq.md).
