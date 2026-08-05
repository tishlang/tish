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

## What is still open: the ACCUMULATE

`acc = acc + (w & MASK)` — the shape above, and the commonest expression in any packed-word data
structure, since `w & FIELD` is how you read a field.

**Do not fix it by typing bitwise results `I32`.** That is the obvious one-line change and it is a
measured **5x pessimisation**: the int-domain fusion at `codegen.rs:23759` is *gated on*
`result_ty == RustType::F64`, so retyping the result disables the fusion that makes bitwise chains
fast in the first place. The F64 typing and the fusion are a matched pair. Measured with that
change in: flood fill 1,470 → 7,531, K1 22,742 → 25,582.

The fix belongs at the consumer, like the three above. Sketch that was attempted:

* at the `RustType::I32` arm of `Expr::Assign` (`codegen.rs:4034`), when the RHS is `Add`/`Sub` and
  both operands can be produced in the integer domain, emit `wrapping_add`/`wrapping_sub`.
* sound because the target is `i32`, so the result is ToInt32'd regardless, and for int32 operands
  `to_int32(a + b) == a.wrapping_add(b)` exactly — `|a + b| < 2^32` is far inside f64's
  exact-integer range. The truncation the assignment already performs is what licenses it.

**Why it did not land:** producing the operands needs a helper broader than `emit_int32_operand`,
which declines two shapes it is right to decline — an expression the type system already calls
`i32` (no ToInt32 needed), and a bitwise node with one unmodelled leaf (an array element read,
which discards the whole expression). A version accepting both was written, and the arm was reached
for `mb = mb + readCell(mj)` (`li=true ri=false`, correctly declining a call) but never for
`ka = ka + (arr[i] & 255)`, which returns earlier in the arm for a reason not yet pinned down.
Reproducing that needs an `eprintln` at each early return in the arm; it is a half-day, not a
mystery.

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
