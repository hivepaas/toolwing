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

| Suite | jaq 3.1.2 | jaq + prelude (prototype) | `tw jq`: prelude | `tw jq`: + values, natives, Oniguruma |
| --- | --- | --- | --- | --- |
| `jq.test` (550) | 373 (67.8%) | 396 (72.0%) | 394 (71.6%) | 428 (77.8%) |
| `man.test`, the manual's examples (231) | 209 (90.5%) | 216 (93.5%) | 216 (93.5%) | 221 (95.7%) |
| `onig.test` + `manonig.test`, regex (66) | 54 (81.8%) | 54 | 54 | **66 (100%)** |
| `base64.test` + `uri.test` (30) | 20 (66.7%) | 20 | 20 | **30 (100%)** |
| `optional.test` (2) | 2 | 2 | 2 | 2 |
| **All (879)** | **658 (74.9%)** | **688 (78.3%)** | **686 (78.0%)** | **747 (85.0%)** |

The prototype pasted the prelude in front of each program, which made two
`module` cases in `jq.test` fail to compile as they must, by accident; `tw jq`
loads the prelude as definitions, as jq does, and those two fail for their
real reason (jaq accepts any `module` metadata).

**Real-world programs** (`scripts/jq-compat/idioms.py`, 170 programs as people
run them on kubectl, AWS, GitHub, Docker and package.json output, logs, number
formatting, `@csv`/`@tsv`/`@sh`, regex and dates, compared **byte for byte**
with exit codes):

| | jaq 3.1.2 | jaq + prelude (prototype) | `tw jq`: prelude | `tw jq`: + values, natives, Oniguruma |
| --- | --- | --- | --- | --- |
| Identical output and exit code | 148 / 170 (87.1%) | 156 / 170 (91.8%) | 156 / 170 (91.8%) | **170 / 170 (100%)** |

The suite compares values, so it misses what bytes show: jaq's `del(.a)`
reorders the remaining keys (`{"d":4,"b":2,"c":3}`), and it prints `10 / 5` as
`2.0`. Both pass the suite and both change what a script sees.

## What differs, and where it is fixed

| Difference | Seen in | Fixed in |
| --- | --- | --- |
| Builtins missing or defined differently: `IN`, `INDEX`, `JOIN`, `tostream`, `fromstream`, `truncate_stream`, `trimstr`, `pick`, `have_literal_numbers`; `join` with nulls, `from_entries` with `Key`/`Value` (AWS tags), `limit`/`skip`/`nth`/`flatten` with negative counts | suite 32, idioms 8 | **Prelude**: jq's own definitions from `src/builtin.jq` (MIT), loaded before every program. **Done**: `crates/jq/src/prelude.jq`. |
| Assigning into `null` (`null \| .a.b = 1`, `jq -n '.x = 1'`), extending arrays (`.[3] = 1`), deleting keeps key order, `delpaths` with several paths, fractional indices | suite ~23, idioms 4 | **Value type**: jaq-json vendored (`vendor/jaq-json`, MIT) and patched for the whole build: `map_index` sets into `null` and past an array's end, deletes with `shift_remove`. **Done** (vendor/README.md). |
| Number output: literals in jq's canonical form (`1e3` → `1E+3`), computed doubles as jq prints them (`2`, not `2.0`; `1e-07`), `-0`, integers past 2^53 rounded as doubles in arithmetic | suite ~30, idioms 6 | **Value type and writer**: jq's `jvp_dtoa_fmt` for doubles, decNumber's canonical form for literals, arithmetic in doubles. **Done**. |
| Regex: jaq uses `regex-bites` (no look-around, ASCII `\b`, optional groups dropped from `capture`) | suite 12, idioms 2 | **Oniguruma**, the engine jq uses (6.9.10, through the `onig` crate), as native `_match_impl` ported from jq's `f_match`; `match`, `test`, `capture`, `scan`, `splits`, `sub`, `gsub` from jq's builtin.jq. **Done**: `crates/jq/src/regex.rs`. |
| `@base64d` refuses unpadded input (every JWT payload: `split(".")[1] \| @base64d`), `@urid` accepts invalid input | suite 10, idioms 2 | **Native filters** with jq's rules and messages. **Done**: `crates/jq/src/natives.rs`. |
| Division by zero gives `Infinity`, `%` on fractions, `NaN` literals in input, `strftime` on short arrays | suite ~6 | Value type and natives. **Done** for division and `%`; the others remain. |
| Syntax jaq lacks: `?//` destructuring alternatives, `$__loc__`, `{$name: pattern}`, `reduce`/`foreach` with `/` in the source | suite ~28, of which 4 in the manual | **jaq-core**: pull requests upstream; a patch taken in only while one waits, as shellwing does with `gpui-pre-linux`. |
| Modules: `import "x" as $data`, `modulemeta`, `builtins` | suite ~12 | jaq-core and natives. |
| Depth: jaq overflows its stack at 10,000 levels (`reduce range(10000) as $_ ([];[.]) \| tojson \| fromjson` aborts the process); jq stops with "too deep" | suite 14 | Depth limits of 10,000 in the parser, writer, comparison and paths, as jq has. |
| Error message text (`try … catch .` returns jq's wording) | suite 34 | jq's messages for the common errors, in the value type and natives. |

## Expected results

Estimates from the measured categories, the first two rows measured:

| After | Suite | Real-world programs |
| --- | --- | --- |
| Prelude (measured) | 78.0% | 91.8% |
| + value type, number writer, Oniguruma, natives (measured) | 85.0% | 100% |
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
   still fails is listed with its reason and the issue tracking it.

The prelude, the value type and the natives live in `toolwing-jq`
(architecture.md); jaq-core and jaq-std are used as published, apart from
patches waiting upstream.

## How it is checked

Both measurements run in CI, through the `tw` binary:

- **Compatibility cases**, `crates/cli/tests/compat/jq/`: the 170 real-world
  programs, 46 cases of the command line itself (options after the
  filter, `-c` against `--tab`, a name given twice to `--arg`, `--`, `-1` as
  a filter, several files, `-f`, `-L`, unreadable files, `halt_error`, the
  exit codes), and 32 of values and regex (setting into `null`, padding,
  fractional indices, the arithmetic, number printing, match objects,
  look-around, modifiers, `@base64d`, `@urid`). `scripts/compat_record.sh
  jq` records each from jq 1.8.2's release binary (its sha256 checked) and
  `tw` must give the same stdout and exit code. A case that still differs
  has a `known` file saying why (3 now: the flags not done yet); it must
  keep differing, so a fix fails the test until its `known` is removed.
- **jq's suite**, `crates/cli/tests/jq-suite/` (jq 1.8.2's `tests/*.test` and
  modules, MIT): `jq_suite.rs` judges each case as jq's runner does, and
  holds each file's count of passing cases as a ratchet. Fewer fails the
  test; more fails it too, until the count in `PASSING` is raised.
  `TW_JQ_SUITE_VERBOSE=1` lists the cases that fail, and why.

## The command line

`crates/cli/src/jq.rs` reads the arguments as jq's `main.c` does, by hand:
`--args` and `--jsonargs` change what the arguments after them are, and
options may come between the filter and the files, which clap cannot
describe. What it follows from jq:

- An option is `-` and a letter, or `--`: `-` is stdin, `-1` a filter.
  Short options combine (`-nr`); `-L` takes the rest of its argument or the
  next one. `--` ends the options.
- `-c`, `--tab` and `--indent N` undo each other, the last one counting;
  `--indent 0` keeps the newlines and drops the indent.
- `--arg` and the like keep the first value of a name given twice.
- Stdin, or the files, are one stream of inputs: `input` and `inputs` read
  across files, `-s` slurps them all into one array, `-Rs` into one string,
  and `input_filename` names the file of the input at hand.
- A runtime error on one input is reported and the next input runs. The
  last input decides the exit code: 5 when it failed. With `-e`, the code
  comes from the last output: 1 when it is false or null, 4 when there was
  none. An input that does not parse ends the run with 5, an unreadable
  file with 2, and a filter that does not compile with 3; `halt_error`
  exits with its code.
- Without a filter it runs `.`, unless stdin and stdout are both terminals.

Not done yet, and refused with exit 2 and a message rather than ignored:
`--seq`, `--stream`, `--stream-errors` and `-a`/`--ascii-output`.
`--stream` needs a streaming parser of its own; it is the largest of these.
jaq's `--from`/`--to` (YAML, TOML, XML, CBOR, CSV) are kept under the same
names.
