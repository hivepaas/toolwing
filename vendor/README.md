# Vendored crates

Crates taken in as source because toolwing needs them to behave differently,
patched in for the whole dependency graph with `[patch.crates-io]` in the
workspace's `Cargo.toml`. Each is first committed as published, so `git log`
on its folder shows every change made to it. They are not workspace members:
they keep their own lints, and their own tests run with
`cargo test --manifest-path vendor/<crate>/Cargo.toml`.

## jaq-json 2.0.3

jaq's value type, its JSON reader and writer (MIT, `jaq-json/LICENSE-MIT`).
Changed so that `tw jq` behaves as jq 1.8.2 (docs/design/jq.md):

- **Setting and deleting** (`map_index`, `map_range`): `null` is an empty
  object or array to set in, and deleting from it gives `null`; setting past
  an array's end pads it with nulls (`Out of bounds negative array index`
  and `Array index too large` as jq has them); deleting a key keeps the
  other keys in order (`shift_remove`, not `swap_remove`).
- **Indices**: a fraction is truncated toward zero; in a slice the start
  floors and the end ceils; a slice of `null` is `null`.
- **Arithmetic** in doubles, as jq's: integers stay integers only while
  they are exact as doubles (to 2^53), `/` always gives a double, `%`
  truncates both operands; dividing by zero is an error with jq's message.
  String repetition takes any count (truncated; 0 gives `""`).
- **Writing numbers**: a double as jq's `jvp_dtoa_fmt` writes it (`2`,
  `1e-05`, `1e+17`; NaN `null`, the infinities the largest doubles), a
  literal in decNumber's canonical form (`1e3` is `1E+3`, `1.0` stays); `-0`
  read from JSON stays a literal.
- Tests: jaq's that encoded the old behaviour now hold jq 1.8.2's results,
  and jaq-std is a dev-dependency with its default features, as jaq's
  workspace gives it.

Each change is marked with a `toolwing:` comment.
