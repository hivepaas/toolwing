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
| `convert` | JSON ⇄ YAML ⇄ TOML ⇄ CSV ⇄ XML; the input's format found when not given; CSV's first row its header; XML as xmltodict maps it (`@` attributes, `#text`) | YAML 1.2, TOML 1.0, RFC 4180, XML 1.0 | standard | `jaq-fmts` |
| `base64`, `base32` | Encode, decode (`-d`, `-w`, `-i`), URL-safe alphabet | GNU coreutils 9.7 | drop-in | `data-encoding` |
| `hex` | Encode, decode | `xxd -p`, `xxd -r -p` | output | `data-encoding` |
| `url` | Encode, decode, parse into fields | RFC 3986, WHATWG URL | standard | `percent-encoding`, `url` |
| `md5sum`, `sha1sum`, `sha256sum`, `sha512sum`, `b2sum` | Hash files and check lists (`-c`, `--tag`, `-z`, `-l`) | GNU coreutils 9.7 | drop-in | RustCrypto `md-5`, `sha1`, `sha2`; `blake2b_simd` (blake2 0.11 takes no length at run time) |
| `b3sum` | BLAKE3: `--keyed`, `--derive-key`, `-l`, `--seek`, `--tag`, `--raw`, `-c` | b3sum 1.8.2 | drop-in | `blake3`; cases recorded from Alpine 3.22's b3sum (`debian:13` has none) |
| `hmac` | HMAC-SHA1/256/512 of input with a key | `openssl dgst -hmac` | output | RustCrypto `hmac` |
| `jwt` | Decode (times in your zone), verify (HMAC secret, PEM, JWK or JWKS by `kid`; a key of the wrong kind refused; exp/nbf with a leeway, iss, aud), encode | RFC 7519, 7515 (its appendix A.1 is a test) | standard | `jsonwebtoken` with its pure-Rust crypto; its `rsa` crate carries RUSTSEC-2023-0071 (Marvin, timing of private-key operations: RS/PS signing, not verifying) |
| `time` | Epoch (unit guessed from its size) ⇄ RFC 3339/ISO 8601/RFC 9557/RFC 2822, time zones, relative ("in 3 days", "2 hours ago"), `--format` strftime | GNU date 9.7 (`-R`, `%s`, `%:z`; ISO in UTC ends in `Z` where `date -Iseconds` writes `+00:00`) | output | `jiff`, its zone database bundled so containers with no tzdata know every zone |
| `uuid` | Generate v4/v7 (v1, v3, v5 too), inspect version and time | RFC 9562, `uuidgen` | standard | `uuid` |
| `ulid` | Generate, inspect, convert to UUID | ULID spec | standard | `ulid` |
| `cron` | Explain an expression, its fields, the next runs in a zone (a time daylight saving skips does not run, a repeated one runs once) | crontab(5) of Vixie cron, as Kubernetes takes it: 5 fields or 6 with seconds, names, `@daily`…, the day-of-month-or-weekday rule; Quartz's `L`, `W`, `#` not yet | standard | our own parser and scheduler on `jiff`, so zones are as in `tw time` |
| `regex` | List matches with their place and groups (exit 1 on none, as `grep`), replace with `$1`/`${name}` | Rust `regex` (linear time); PCRE-style with `--pcre` (look-around, backreferences; stopped after a million backtracking steps) | standard | `regex`, `fancy-regex` |
| `diff` | Compare two files: GNU's normal format, `-u`/`-U N`, `--label`, `-q`, `-s`, `-i`, `-w`, `-b`, binary files; `--json-values` not yet | GNU diffutils 3.10 | drop-in (33 cases recorded) | `similar` (Myers) |
| `case` | camel, pascal, snake, constant, kebab, train, dot, title, sentence; every one at once by default | — | standard | `heck`'s word boundaries |
| `random` | Passwords, tokens, bytes as hex or base64 | `openssl rand -hex/-base64` | output | `rand` with the OS generator |
| `base` | Number bases 2–36, `0x`/`0o`/`0b` prefixes, grouping, up to 128 bits, signed and unsigned at a width (`--bits`) | — | standard | std |

## 1.x: next, in this order

| Command | Does | Reference | Crate |
| --- | --- | --- | --- |
| `cert` | Read X.509 (PEM/DER): subject, SANs, issuer, validity, fingerprints, chain order | `openssl x509 -noout -text` | `x509-parser` |
| `bcrypt`, `argon2` | Hash and verify passwords. `bcrypt hash` (`-C` cost, `--prefix` 2b/2y/2a, `--salt`) and `bcrypt verify --hash`; a password past bcrypt's 72 bytes, or with a NUL, is refused rather than cut. `argon2` is drop-in for the reference C's command (65 cases recorded from Debian 13's `argon2`, its timing line set aside), plus `tw argon2 verify ENCODED`. The app runs them on Ctrl+Enter, at most bcrypt cost 16 and Argon2 1 GiB, 64 passes | `htpasswd -B` (output; crypt_blowfish's test vectors), `argon2` 20190702 (drop-in) | Keats' `bcrypt` 0.19, RustCrypto `argon2` 0.6 |
| `ipcalc` | An IPv4 or IPv6 network worked out (network, mask, broadcast, hosts, IANA address space, reverse zone), split (`-S`), a range as networks (`-d`), `VAR=value` and JSON output. Drop-in for ipcalc 1.0.3 (Red Hat's; Debian's `ipcalc-ng`): its `getopt_long` arguments and output, 265 cases recorded, 163 of them its own tests' commands. Names are not looked up (`-h`, `-o`): tools stay off the network. Where the reference crashes (`--reverse-dns` with no zone) or never ends (a range up to the last address), tw does not. "Contains" is not in it | ipcalc 1.0.3 (drop-in) | our own, on glibc's `inet_pton`/`inet_ntop` rules |
| `html` | Escape (`& < > " '`, `--no-quotes`, `--ascii` for numeric references past ASCII) and unescape (`--attribute` for an attribute value's rules), as the WHATWG tokenizer reads references: tested with html5lib-tests' 4,626 cases of them | WHATWG HTML (standard) | `htmlize` |
| `xml` | Format (`--indent`, `--tab`) and minify, the document checked well-formed (XML 1.0, its internal DTD read: entities, element and attribute declarations), as `xmllint --format` and `--noblanks` write it: blanks dropped where libxml2's `areBlanks` drops them, the DTD one declaration a line, elements indented only where they hold no text. 79 documents compared with xmllint's output both ways. Not: other encodings than UTF-8 (unless the document is all ASCII), XHTML's added `<meta>`, parameter entities in the DTD | libxml2 2.9.14's xmllint (output) | our own parser and writer, after libxml2's |
| `qrencode` | Text as a QR code, module for module as qrencode 4.1.1 makes it (libqrencode's split into modes, its choice of version and of mask, ported; the symbol itself by `qrcode`), written as its writers write it: ASCII, UTF-8 half blocks, ANSI, SVG (`--rle`, `--svg-path`, `--inline`, colours), EPS, XPM, PNG (its pixels and chunks; another zlib stream). Drop-in: 234 cases recorded, 120 of them random text in every mode. Not yet: Micro QR (`-M`), structured symbols (`-S`); reading a code from a picture | qrencode 4.1.1 (drop-in) | `qrcode`, `png` |
| `color` | A CSS color (hex, named, `rgb()`, `hsl()`, `hwb()`, `lab()`, `lch()`, `oklab()`, `oklch()`, `color()` in its predefined spaces, `none`, `calc()`) read and computed as browsers do it (web-platform-tests' 4,726 computed and 395 invalid cases), in every other form, mapped into sRGB as CSS Color 4 §13.2 maps it, with WCAG 2.1 contrast (`--against`); `--to` writes one form. Conversions are the spec's matrices, checked against Color.js 0.5.2. Not: relative colors, `color-mix()`, `currentcolor`, system colors | CSS Color 4 (standard) | our own |
| `zstd` | Compress, decompress, `--test`, `--list`: drop-in for zstd 1.5.7, its frames byte for byte (libzstd 1.5.7 as hivepack ports it, in safe Rust, with the parameters the command sets: checksum, content size when known, its multi-threaded mode), its argument parser, file names, overwriting and `--rm`, and its messages up to `-v`. 122 cases recorded, stderr included. The app gives Base64 to paste. Not: `-b`, `--train`, `--patch-from`, `--zstd=`, other formats, `-r`; from `-vv` on, the command's parameter and timing lines; the progress counter | zstd 1.5.7 (drop-in) | `zstd` (hivepack 0.8: libzstd 1.5.7 ported) |
| `gzip` | Compress, decompress, `--test`, `--list`: drop-in for GNU gzip 1.13. Decompression is gzip's own decoders ported (inflate, compress's LZW `.Z`, pack, SCO LZH, a zip's first entry), so output, partial output before an error, and messages are the command's; its options as glibc's `getopt_long` reads them and the GZIP variable, file names and suffixes (`-S`), the overwrite question, mode, times and owner kept, `-k`, `-r`, `-n`/`-N`, `-l`/`-lv`, exit codes. Compression is zlib's deflate (hivepack's port) in gzip's header and trailer: name, time, flags and sizes as gzip writes them, the deflate bytes not (gzip has its own encoder; compat cases compare header, trailer and content). 221 cases recorded, stderr and the folder after included. The app gives Base64 to paste. Not: `-Z` (gzip 1.13 refuses it too), `--rsyncable`'s block boundaries (accepted, ignored); `-V`/`-L` name tw first | gzip 1.13 (drop-in; compression output-level) | `gzip` |
| `wc` | Lines, words, characters, bytes and the widest line: drop-in for GNU coreutils 9.7's wc as in the C.UTF-8 locale (glibc 2.41's `iswspace`, `iswprint` and `wcwidth` for every code point, generated from glibc itself, and its UTF-8 decoder, which takes code points past U+10FFFF), `--files0-from`, `--total`, the column width from the files' sizes, names quoted as coreutils quote them (gnulib's quotearg ported). 131 cases recorded, stderr included, and 400 random inputs compared. Not: `--debug`'s avx2 line on another processor than the reference's | wc 9.7 (drop-in, C.UTF-8) | std |
| `uniq` | Adjacent repeated lines merged, counted (`-c`), only the repeated (`-d`, `-D`, `--all-repeated`) or the unique (`-u`), grouped (`--group`), compared after fields and characters (`-f`, `-s`, `-w`), ASCII case ignored (`-i`), NUL-ended lines (`-z`): drop-in for GNU coreutils 9.7's uniq as in the C.UTF-8 locale (glibc's blanks, UTF-8 characters), its arguments in order with the obsolete `-N` and `+N`, INPUT and OUTPUT. 9.7's `-w` with a multibyte character first (N times its length, compared past the line into what earlier lines left in the buffer) is kept, its line buffers modelled. 311 cases recorded, and 600 random inputs with random options compared | uniq 9.7 (drop-in, C.UTF-8) | std |
| `sort` | Lines sorted: keys (`-k`, `-t`, the obsolescent `+POS1 -POS2`), every ordering (`-b -d -f -g -h -i -M -n -R -r -V`, `--sort`), the last-resort comparison, `-s`, `-u`, `-c`/`-C`, `-m` (merged as sort merges, `--batch-size` at a time), `-o` (an input too), `-z`, `--files0-from`, `--random-source`, `--debug` and its warnings: drop-in for GNU coreutils 9.7's sort as in the C.UTF-8 locale, where its collation is the bytes' order (glibc's `strcoll` and `strxfrm` checked to leave UTF-8 as it is). `-g` compares x87 long doubles: strtold ported, rounded from the exact decimal or hexadecimal value. `-n`, `-h` gnulib's `strnumcmp`, `-V` its `filevercmp`. 325 cases recorded, and 1,600 random inputs with random options compared. Not: temporary files (all in memory), so `-T`, `-S` and `--compress-program` are checked but change nothing | sort 9.7 (drop-in, C.UTF-8) | std, `num-bigint`, `md-5` |

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
