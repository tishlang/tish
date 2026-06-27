# Confirmed bug repros (validated via TDD during triage)

All five were reproduced on current `main`. Each `triage/bug-repros/issue-<N>.tish` is a minimal
repro; run it across backends and compare to node. None are fixtures (they reproduce *wrong*
behavior) — they're evidence for prioritization/fixing.

## #330 — severity:high
_native: return-value forwarding fn mis-promotes a forward-only array param to f64 (panics "expected number")_

**Validation:** Wrote the issue's repro to ./target/triage/330/repro.tish and ran it on node (JS reference), interp, vm, native boxed (TISH_NATIVE_OPT=0), and native typed (default, via `tish build`). Also grepped crates/tish_compile/src/codegen.rs to confirm the named root-cause symbols exist.

**Observed:** node => r:8 (exit 0); interp => r:8 (exit 0); vm => r:8 (exit 0); native boxed TISH_NATIVE_OPT=0 => r:8 (exit 0); native typed (default `tish build`) => panics at src/main.rs:230 'expected number' (exit 134). So the default typed native build crashes on valid input while every other path is correct (typed != boxed). Root-cause symbols classify_vec_param, collect_native_fns, infer_vec_fn_sig_forwarding all present in crates/tish_compile/src/codegen.rs as the issue states.

Repro: `triage/bug-repros/issue-330.tish`

## #218 — severity:high
_[native] Trivial cross-module accessor hangs at runtime (codegen/optimizer Heisenbug); VM + JS targets are correct_

**Validation:** Built a minimal 4-module repro under ./target/triage/218 (color.tish: hexToRgb/rgbToHex/mix; theme.tish: module-level mutable `current` state + cross-module accessors like `helpKey() => mix(current.dim,current.fg,0.55)`; layout.tish: deep accessor chain viewLines->helpBottom->helpShortLine->helpKey; repro.tish: paint() loop). Ran on interp, vm, JS(node), native cranelift backend, and native rust backend; compared output and exit codes. Also checked CPU state during the hang with ps.

**Observed:** interp: correct, '#96a6ae #b0c0c8 #b6c6ce | #d0e0e8#506068#101418' x3, exit 0. vm: same correct output, exit 0. JS target via node: same correct output, exit 0. native --native-backend cranelift: same correct output, exit 0. native --native-backend rust (the default): HANGS with ZERO output, never exits (timeout 124), reproducible across re-runs. During the hang the process is in state S (sleeping) at 0.0%% CPU with ~0 accumulated CPU time => it is a deadlock/blocking-wait in the generated Rust, NOT a hot infinite loop. This isolates the defect to the rust transpile/codegen path exactly as the author's narrowing comment states (cranelift backend + VM are fine). NOTE: contrary to the issue's 'could not minimize / Heisenbug' claim, this small 4-module reduction reproduces reliably on current main.

Repro: `triage/bug-repros/issue-218.tish`

## #244 — severity:medium
_[lang] Remove loose equality (==/!=) — reject uniformly at parse; only native-rust rejects it today_

**Validation:** Wrote target/triage/244/repro.tish (let x=1; if (x==1) ...). Ran on interp, vm, node, and built on native rust + cranelift. Also tested != variant and a === control on native rust.

**Observed:** interp -> eq; vm -> eq; node -> eq; native cranelift -> builds + runs eq; native rust (default) -> 'Error: 2:5: Loose equality not supported' (exit 1, build fails). != on native rust also fails identically; === control compiles + runs 'strict-eq' on native rust, proving the rejection is specific to loose ==/!=. Source still has the rejecting arm at crates/tish_compile/src/codegen.rs:19750-19755 (BinOp::Eq | BinOp::Ne -> CompileError 'Loose equality not supported').

Repro: `triage/bug-repros/issue-244.tish`

## #247 — severity:low
_Cross-backend divergences surfaced by parity probes (toFixed, -0, includes(NaN), parseInt radix, Math.round/min/max/hypot)_

**Validation:** Ran minimal .tish repros on ./target/debug/tish run --backend interp, --backend vm, and node for every item the issue lists. /Users/a_/Projects/tish/tish/target/triage/247/repro.tish covers the 7 table rows; hypot.tish, at.tish, findlast.tish, instanceof.tish cover the interp/vm gap and feature-gap items.

**Observed:** MOST listed divergences are now FIXED on current main. The 7 table rows all agree and are correct: (2.5).toFixed(0)=3, (-0).toString()=0, [1,NaN].includes(NaN)=true, parseInt("0x1F",16)=31, Math.round(-2.5)=-2, Math.min()=Infinity, Math.max()=-Infinity — identical across interp/vm/node. The interp!=vm gap is closed: Math.hypot(3,4) returns 5 on BOTH interp and vm now. Feature gaps String.prototype.at (->"a") and Array.prototype.findLast (->2) now work on both backends. STILL BROKEN: the `instanceof` operator is a parse error on interp AND vm ("Parse error: Expected Comma, got Ident") while node returns true. So this umbrella issue still has one open sub-item (instanceof); the rest can be checked off / locked into parity probes.

Repro: `triage/bug-repros/issue-247.tish`

## #157 — severity:low
_[formatter] Multi-line JSX uses hardcoded 2-space indent (depth-blind); JsxProp::Spread emits stray trailing space but is unreachable/latent_

**Validation:** Built target/debug/tish-fmt (cargo build --bin tish-fmt) and ran it on the issue's exact repros under ./target/triage/157/. Compared formatter output vs input for the indent claim; for the Spread claim, exercised the reachable spread-prop form and checked idempotency + re-parse via `tish build --target js`. Inspected current source (tish_fmt/src/lib.rs jsx_children @1162-1177, JsxProp::Spread @1436-1440; tish_parser/src/parser.rs parse_jsx_element @2627-2693) and git history.

**Observed:** The issue's TWO claims have diverged from current main. (1) INDENT/CORRUPTION claim (the issue's headline 'real' claim) = ALREADY-FIXED. Formatting the issue's multi-line `<div>...</div>` repro now preserves it verbatim and idempotently (children stay at original 4 spaces, `</div>` at 2; no hardcoded depth-blind 2-space indent, no blank `  ` lines, no `{<child>}`-wrapped element children). The cited lib.rs:1390/1401/1425 reflow logic was replaced by `jsx_children` (lib.rs:1162-1177) in commit 82e6dda5 (#205, 2026-06-13 00:57, ~4 min AFTER the issue was filed at 00:53). (2) SPREAD claim = the 'latent/unreachable' premise is STALE but the underlying defect is LIVE. The issue cites parser.rs:2411-2415 as the only Spread constructor and calls it dead, but the JSX Spread arm at parser.rs:2653-2657 (added 2026-03-08, commit 609c225a, i.e. BEFORE the issue) does construct JsxProp::Spread. Reachable form `const e = <div ...props} />` parses and builds to JS fine; formatting it yields `const e = <div {...props}  />` — a SPURIOUS DOUBLE SPACE from lib.rs:1439 `push_str("} ")` + the self-close `' />'`. `<div ...props} id="x" />` similarly formats to `{...props}  id="x"` (double space before next attr, exactly the issue's predicted symptom). Worse, the formatter emits a leading `{` (`{...props}`) the parser has NO LBrace arm for, so the formatter's own output FAILS to re-parse (`Unexpected token in JSX props: Some(LBrace)`) — non-idempotent. The braced source `<div {...props} />` and bare `<div ...props />` still fail to parse. Net: the high-severity corruption is gone; a low-severity live formatter defect (double-space + non-idempotent Spread output) remains.

Repro: `triage/bug-repros/issue-157.tish`
