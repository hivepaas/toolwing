# `tw jq`

`tw jq` stands in for `jq` 1.8.2: the same flags, the same output bytes, the
same exit codes. It runs on jaq's engine with a compatibility layer of our own,
because jaq alone is not close enough.

## How close jaq is: measured

Measured on 3 November 2026: jaq 3.1.2 (built from crates.io) against the
official jq 1.8.2 binary, with the scripts in `scripts/jq-compat/`.

**jq's own test suite** (`tests/*.test` from jq 1.8.2, 879 cases, judged as
jq's runner judges them: outputs compared as values, in order). jq passes all
879 with `PAGER=less` set, as its `mantest` sets it.

| Suite | jaq 3.1.2 | jaq + prelude |
| --- | --- | --- |
| `jq.test` (550) | 373 (67.8%) | 396 (72.0%) |
| `man.test`, the manual's examples (231) | 209 (90.5%) | 218 (94.4%) |
| `onig.test` + `manonig.test`, regex (66) | 54 (81.8%) | 54 |
| `base64.test` + `uri.test` (30) | 20 (66.7%) | 20 |
| `optional.test` (2) | 2 | 2 |
| **All (879)** | **658 (74.9%)** | **690 (78.5%)** |

**Real-world programs** (`scripts/jq-compat/idioms.py`, 170 programs as people
run them on kubectl, AWS, GitHub, Docker and package.json output, logs, number
formatting, `@csv`/`@tsv`/`@sh`, regex and dates, compared **byte for byte**
with exit codes):

| | jaq 3.1.2 | jaq + prelude |
| --- | --- | --- |
| Identical output and exit code | 148 / 170 (87.1%) | 156 / 170 (91.8%) |

The suite compares values, so it misses what bytes show: jaq's `del(.a)`
reorders the remaining keys (`{"d":4,"b":2,"c":3}`), and it prints `10 / 5` as
`2.0`. Both pass the suite and both change what a script sees.

## What differs, and where it is fixed

| Difference | Seen in | Fixed in |
| --- | --- | --- |
| Builtins missing or defined differently: `IN`, `INDEX`, `JOIN`, `tostream`, `fromstream`, `truncate_stream`, `trimstr`, `pick`, `have_literal_numbers`; `join` with nulls, `from_entries` with `Key`/`Value` (AWS tags), `limit`/`skip`/`nth`/`flatten` with negative counts | suite 32, idioms 8 | **Prelude**: jq's own definitions from `src/builtin.jq` (MIT), loaded before every program. Done in the prototype. |
| Assigning into `null` (`null \| .a.b = 1`, `jq -n '.x = 1'`), extending arrays (`.[3] = 1`), deleting keeps key order, `delpaths` with several paths | suite ~23, idioms 4 | **Value type**: our own, started from `jaq-json` (MIT, ~2,000 lines): its `map_index` fails on `null`, refuses indices past the end and uses `swap_remove`. |
| Number output: literals in jq's canonical form (`1e3` → `1E+3`), computed doubles as jq prints them (`2`, not `2.0`; `1e-07`), `-0`, integers past 2^53 rounded as doubles in arithmetic | suite ~30, idioms 6 | **Value type and writer**: jq's number formatting, and `tostring`/`tojson` through it. |
| Regex: jaq uses `regex-bites` (no look-around, ASCII `\b`, optional groups dropped from `capture`) | suite 12, idioms 2 | **Oniguruma**, the engine jq uses, through the `onig` crate, as native `_match_impl`. Same engine, same results. |
| `@base64d` refuses unpadded input (every JWT payload: `split(".")[1] \| @base64d`), `@urid` accepts invalid input | suite 10, idioms 2 | **Native filters** with jq's rules. |
| Division by zero gives `Infinity`, `NaN` literals in input, `strftime` on short arrays | suite ~6 | Value type and natives. |
| Syntax jaq lacks: `?//` destructuring alternatives, `$__loc__`, `{$name: pattern}`, `reduce`/`foreach` with `/` in the source | suite ~28, of which 4 in the manual | **jaq-core**: pull requests upstream; a patch taken in only while one waits, as shellwing does with `gpui-pre-linux`. |
| Modules: `import "x" as $data`, `modulemeta`, `builtins` | suite ~12 | jaq-core and natives. |
| Depth: jaq overflows its stack at 10,000 levels (`reduce range(10000) as $_ ([];[.]) \| tojson \| fromjson` aborts the process); jq stops with "too deep" | suite 14 | Depth limits of 10,000 in the parser, writer, comparison and paths, as jq has. |
| Error message text (`try … catch .` returns jq's wording) | suite 34 | jq's messages for the common errors, in the value type and natives. |

## Expected results

Estimates from the measured categories, not yet measured:

| After | Suite | Real-world programs |
| --- | --- | --- |
| Prelude (prototype, measured) | 78.5% | 91.8% |
| + value type, number writer, Oniguruma, natives | ~88% | 100% |
| + jaq-core syntax, modules, depth limits | ~94% | 100% |
| + jq's error messages | ~97–99% | 100% |

## Release gates

`tw jq` is called a drop-in replacement only when all of these pass in CI:

1. **Real-world programs: 100% byte-identical** with jq 1.8.2, output and exit
   code. The corpus grows to at least 500 programs before 1.0, taken from the
   jq manual, the documentation of kubectl, the AWS CLI, `gh` and Docker, and
   the most-viewed jq questions on Stack Overflow.
2. **`man.test`: 100%.** What the manual documents works as documented.
3. **The whole suite: at least 90% for 1.0, 97% for 1.1.** Every case that
   still fails is listed in `tests/jq/differences.md` with its reason and the
   issue tracking it.

The prelude, the value type and the natives live in `toolwing-jq`
(architecture.md); jaq-core and jaq-std are used as published, apart from
patches waiting upstream.

## The command line

jq's flags, including those jaq's CLI does not have: `--seq`, `--stream`,
`--stream-errors`, `--jsonargs`, `-a`/`--ascii-output`, `--unbuffered`.
jaq's `--from`/`--to` (YAML, TOML, XML, CBOR, CSV) are kept under the same
names. `--stream` needs a streaming parser of its own; it is the largest of
these.
