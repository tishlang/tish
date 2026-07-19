# Tish Numeric & Formatting Semantics

Tish's chosen rules for numeric formatting and parsing built-ins (#438, split from #247). These are
**tish's own documented semantics** — node/V8 is our benchmark and reference, not the correctness
oracle. In every row below the chosen rule happens to coincide with V8/node (these are user-facing
format/parse surfaces where matching the ecosystem minimizes surprise), so the same tests double as a
node-parity lock.

Every rule is asserted across **interp / vm / native / cranelift / wasi / js** by
[`tests/core/parity_builtins.tish`](../tests/core/parity_builtins.tish) — all backends must agree.

## `Number.prototype.toFixed(digits)`

Round the number's **true decimal value** to `digits` fraction digits, **ties away from zero**.

The value is a binary double, so an input that *looks* like a tie usually isn't: `2.675` is stored as
`2.67499999999999982…`, so `(2.675).toFixed(2)` is **`2.67`** (rounds down), while a genuine tie like
`(2.5).toFixed(0)` is **`3`** and `(0.125).toFixed(2)` is **`0.13`** (round away from zero).

Implementation note: computing `(num * 10^digits).round() / 10^digits` is **wrong** — float scaling
nudges just-below-tie values up over the tie (it produced `2.68`). The shared implementation
(`tish_builtins::to_fixed_str`) instead renders the double's exact decimal expansion well past
`digits` and rounds that digit string, so non-ties round the way the real value dictates.

| input | `toFixed` | why |
|---|---|---|
| `(2.675).toFixed(2)` | `2.67` | stored as 2.6749…, rounds down |
| `(1.45).toFixed(1)` | `1.4` | stored as 1.4499… |
| `(5.55).toFixed(1)` | `5.5` | stored as 5.5499… |
| `(2.5).toFixed(0)` | `3` | exact tie → away from zero |
| `(0.125).toFixed(2)` | `0.13` | exact tie → away from zero |
| `(9.999).toFixed(2)` | `10.00` | carry propagates into integer part |

`NaN` → `"NaN"`, `±Infinity` → `"Infinity"`/`"-Infinity"`, and `|num| ≥ 1e21` falls back to the
default (exponential) formatting, all matching JS.

## `parseInt(string, radix?)`

Radix inference: a leading `0x`/`0X` (after an optional sign and trimmed leading whitespace) implies
base 16; otherwise base 10. A leading `0` is **decimal**, not octal (`parseInt("077") === 77`).
Parsing consumes the longest valid prefix and ignores trailing junk (`parseInt("  42abc") === 42`);
no valid digits yields `NaN`.

## `Math.round(x)`

Round half **up toward +∞**: `round(2.5) === 3`, `round(-2.5) === -2`, `round(0.5) === 1`,
`round(-0.5) === -0`. (This is JS's rule, not round-half-to-even.)

## `Math.min()` / `Math.max()`

Empty-args identity: `Math.min() === +Infinity`, `Math.max() === -Infinity`. Any `NaN` argument makes
the result `NaN`.

## `includes` / `indexOf` / `Set` / `Map` keys

Collection membership uses **SameValueZero**: `NaN` matches `NaN`, and `+0`/`-0` are equal.
So `[NaN].includes(NaN)` is `true`, `[0].includes(-0)` is `true`, `new Set([NaN]).has(NaN)` is
`true`. `Array.prototype.indexOf` uses **strict equality** instead, so `[NaN].indexOf(NaN) === -1`.

## `-0` stringification (already decided; recorded here)

`ToString` drops the sign: `String(-0)`, `"" + (-0)`, `` `${-0}` ``, and `(-0).toString()` all yield
`"0"`. Only the `console.log` **inspect** form preserves `-0`.
