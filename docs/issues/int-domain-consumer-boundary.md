# A bitwise value consumed by an integer costs 27x on GBA

**Status:** open. Three consumer sites fixed in `b2c9f9ac2`; the accumulate site remains.
**Found by:** `tish-gba/examples/bench-grid`, porting the Magical Drop rules from Rust to tish.

## The measurement

On device, GBA, one frame = 4,389 Timer2 ticks. 512 iterations, ticks per iteration:

| expression | ticks/iter |
|---|---|
| `acc = acc + arr[i]` | **1.64** |
| `acc = acc + (arr[i] & 255)` | **44.4** |

**27x, for a mask.** `arr` is a module-level `i32[]` in both cases; only the `& 255` differs.

The same bench isolates the two other candidate explanations and rules both out:

* **not the RefCell borrow.** `arr` is closure-captured in both rows, so both pay
  `(*arr.borrow()).get(i).copied()`. The unmasked row is 1.64 ticks — the borrow is cheap.
* **not element boxing.** The generated Rust keeps `Vec<i32>` and emits `.copied().unwrap_or(0)`.

## The mechanism

`types.rs:301-323` `result_type_of_binop` types every bitwise/shift result `RustType::F64`:

```rust
// Bitwise / shift ops: JS coerces both sides to int32, computes, and
// returns a Number — so the native result is still F64.
BinOp::BitAnd | ... | BinOp::UShr => Some(RustType::F64),
```

That is correct about JS and it is a real win against boxing. The unmeasured consequence is at the
**consumer**: an f64 meeting something that wants an integer converts back, and on ARM7TDMI both
directions are soft-float calls. Emitted for the masked row:

```rust
ka = to_int32(((ka as f64) + ((( /* i32 block */ ) & 255i32) as f64))) as i32
```

The `& 255i32` is already computed in the integer domain by the fusion at `codegen.rs:23744-23762`.
It is then widened to f64, added in f64, and narrowed by `to_int32`. The integer result was in a
register and got a round trip anyway.

By contrast the unmasked row, whose operands are both `I32`, lowers through the existing integer
binop path at `codegen.rs:23640-23712` to `nb.wrapping_add(...)` — no excursion.

## What was fixed

`b2c9f9ac2` hands the int-domain form straight to three consumers rather than routing it through
f64: the `Vec<i32>` element store, the array index, and an integer-typed local binding.

```
module i32[] write pass          22765 -> 1009   22.6x
arr[j] = j & 255                 23162 -> 1652   14.0x
let v: i32 = j & 255; arr[j] = v 23435 -> 1558   15.0x
arr[((j>>5)<<5) | (j&31)]        20318 -> 1494   13.6x
63-cell flood fill                3826 -> 1468    2.6x
```

Soundness: only the SIGNED ops qualify (`& | ^ << >>`) — their value is by construction inside i32,
so `(x as f64) as i32` is the identity. `>>>` is excluded: its uint32 can exceed `i32::MAX` and
Rust's float→int `as` saturates, so the two routes genuinely disagree. The index form carries
`.max(0)` to reproduce the saturating f64→usize cast exactly.

## The accumulate — FIXED, and my first diagnosis of it was wrong

```
acc = acc + (arr[i] & 255)     22742 -> 813   28x
acc = acc + arr[i]  (control)    838 -> 838
```

A masked read now costs the same as an unmasked one.

**The fix did NOT belong at the consumer**, which is what this document originally said and what
three attempts assumed. It belongs inside `emit_int32_operand`, which has an `Add`/`Sub` arm now,
placed ABOVE its generic leaf fold.

That fold was the whole problem, and it is worth naming because it defeats every consumer-side
patch silently: for `acc + (x & 255)` it falls through to `emit_typed_expr`, sees an `F64`, and
returns `Some("to_int32((acc as f64) + (... as f64))")`. It reports SUCCESS while emitting exactly
the round trip this function exists to remove. So the assign site's first branch took it, every
later branch was unreachable, and adding code to those branches changed nothing — which is precisely
what happened three times before an `eprintln` at the arm's entry showed `int32op=true` on the case
that was demonstrably slow.

Soundness is licensed by this function's own contract: its result is ToInt32'd by the caller, and
for int32 operands `to_int32(a + b) == a.wrapping_add(b)` exactly, since `|a + b| < 2^32` is far
inside f64's exact-integer range. `+` is still NOT a ToInt32 context in general and is untouched
outside this one.

The operand helper `int32_or_native_i32` accepts what `emit_int32_operand` lowers, plus what the
type system already calls `i32` — an `: i32` local, a `Vec<i32>` element read. Those need no ToInt32
lowering because they ARE integers, and the leaf fold only recognises `F64` leaves, so it declines
them. Every accumulator has one on its left-hand side. The helper also rejects a returned string
containing `to_int32`, so the fold's fake success cannot be mistaken for the real thing.

### Still not to be attempted: typing bitwise results `I32`

The obvious one-line root fix is a measured **5x pessimisation**. The int-domain fusion at
`codegen.rs:23759` is *gated on* `result_ty == RustType::F64`, so retyping the result disables the
fusion that makes bitwise chains fast in the first place. The F64 typing and the fusion are a matched
pair. Measured with that change in: flood fill 1,470 -> 7,531, K1 22,742 -> 25,582.

## Scope of the win

It is large where code ACCUMULATES a masked value and nil where it stores or compares one — those
were already covered above. `tish-gba`'s `packages/drop_game.tish` was unmoved (5,216 -> 5,189 peak)
because its hot path stores and compares. `bench-grid`'s K1 is where the shape lives.

## Why it matters

This is the single measured constraint on writing performance-sensitive tish. `tish-gba`'s
`packages/grid.tish` is a packed-cell-word grid — cell byte, engine planes and a cached match mask
in one `i32` — and *every* read of it is `w & FIELD` or `(w >> SHIFT) & FIELD`. The whole design
pays this wherever a masked value is accumulated rather than stored.

## Reproducing

```bash
cd tish-gba/examples/bench-grid && npm run build
GBA_SHOT_LOG=1 ../../scripts/screenshot.sh bench-grid.gba /tmp/bg.png 450
```

Passes K1 / K1b / K2 are the three rows above plus a user-function-call cost. Note the bench's own
trap, recorded in its README: adding a function that touches a module array deoptimises **every**
access to that array, including from top-level code that never calls the function — pass C went 487
to 1,586 ticks/1000 that way. The callee reads a private array so the rest stays comparable.
