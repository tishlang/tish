# A bitwise value consumed by an integer costs 27x on GBA

**Status:** CLOSED. Five consumer sites, three commits: `49ac5739a` (element store, array index,
integer-typed local binding), `b32827ca9` (the masked accumulate), `4f0798f98` (the element READ
index). Every one was found by timing production code in `tish-gba`, never by predicting it.
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

## The FIFTH site, found last and most cheaply missed: the READ index

```
fn copy, BITWISE index   22541 -> 2371 /1000   9.5x
fn copy, flat index       2273 -> 2273         control, unmoved
```

`arr[a | b]` lowered its element STORE index integrally — that was fixed first — and its element
READ index as `((a|b) as f64) as usize`, two soft-float calls per read. So the packed-grid access
`P[dc|r] = P[sc|r] & keep` paid on exactly half of itself.

The cause is mundane and worth recording as a shape to look for rather than a bug to remember: the
read site held a **hand-inlined copy of `emit_index_usize`**, byte-identical to it down to the panic
message, written before the int-domain shortcut existed. Teaching the helper therefore fixed the
store and left the read behind, and nothing pointed at the divergence because the two spellings had
been identical for as long as anyone had looked. The fix deletes the copy and calls the helper.

The shortcut is gated where it already was: `emit_int32_for_int_consumer` requires the ROOT operator
to be bitwise, so `arr[i + j]` still lowers through f64 and the `Add`/`Sub` arm above cannot wrap a
huge index into a small valid one. Inside `arr[(i + j) | 0]` it does apply, and correctly — that
spelling IS ToInt32, which is wrapping by definition.

Measured in `tish-gba/examples/grid-demo`, whose AI copies a 63-cell board per candidate:

```
board copy      2103 -> 480 ticks   4.4x   (33 -> 7.6 per cell)
AI frame avg    4463 -> 2336
AI frame peak   5672 -> 3176        was OVER a 4389-tick frame, now inside one
```

and in `examples/drop-tish`, the full ported rules, with no change to that repo at all:

```
rules frame avg  1947 -> 1392
rules frame peak 5189 -> 4037       likewise now inside ONE frame
```

### How it hid, which is the part worth generalising

`bench-grid` reported nothing, because every pass it had indexed from **top level with a plain loop
counter** — which the compiler substitutes for a `usize` binding, so no pass ever exercised a
bitwise index inside a function. The bench measured the shape someone wrote down, not the shape the
code ships.

It surfaced only from a phase timer inside the game, where a 63-cell copy came back at 57% of the
whole evaluation. It had been dismissed by reading the code twice; its own comment said it was
bounded to the board's real height, which was true and beside the point.

`bench-grid` now has pass **M** for it, asserted rather than merely printed, and the assertion was
checked by reverting the fix: **ratio 1.04 with it, 9.9 without, checksum identical either way.**

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
