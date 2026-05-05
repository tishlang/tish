# JavaScript emit: philosophy and scope

Tish is **not** JavaScript. It is a separate language with its own grammar, semantics, and runtimes (interpreter, bytecode VM, Rust codegen, WASM). One supported **output** is JavaScript, so Tish can run in browsers and Node without shipping the VM.

This document is the **architecture** stance for that path: what we optimize for, what we refuse to chase, and how we decide when a parser or emitter change is in scope.

---

## 1. JS is a compile target, not a second source language

- **Goal:** Emit **readable, correct** JS for programs that are valid Tish and use patterns we support.
- **Non-goal:** ECMAScript completeness, byte-for-byte compatibility with hand-written JS, or supporting every legal JS spelling inside Tish source.
- **Implication:** Gaps between “what JS allows” and “what Tish accepts” are **normal** unless they cause **obvious** breakage for real Tish code (wrong scope, wrong control flow, or parse failures on idiomatic Tish that targets the DOM or JSX).

For ECMA-262 alignment at a high level, see [ecma-alignment.md](ecma-alignment.md). That table is descriptive; this file is **normative** for how aggressive we are about growing the JS-shaped corners of the parser.

---

## 2. “Obvious failures” (in scope)

We fix issues when they are clearly **bugs** in our pipeline, not missing features relative to JS:

| Class | Example | Direction |
|--------|---------|-----------|
| **Wrong scope / structure in emit** | Lexical declarations effectively isolated from later uses because the AST grouped statements incorrectly (e.g. brace blocks plus indent tokens). | Parser / lowering so emitted JS matches Tish **lexical scope** intent. |
| **Invalid JS from valid Tish** | `if (cond) const x = …` is invalid ECMAScript; emit must wrap the body. | JS backend (`tishlang_compile_js`) emits braces where ECMAScript requires a block. |
| **Keyword vs host API collision** | Tish reserves `type` for type aliases; the lexer emits `TokenKind::Type`, but DOM and JSX use the identifier `type` (`label.type = …`, `type="button"`). Rejecting that is an **obvious** failure for real programs, not “supporting all JS.” | Parser accepts `type` only in **narrow** positions documented below. |

We do **not** treat “JS allows X as a property name” as sufficient reason to extend Tish. Each exception needs a **concrete** Tish + target failure (runtime error, wrong behavior, or cannot parse) and a **narrow** fix.

---

## 3. Explicit exceptions today: the word `type`

`type` is a Tish keyword. The lexer maps the spelling `type` to `TokenKind::Type`, not a generic identifier.

**Architectural decision:** Allow `type` as a **name** only where host APIs and JSX already use that spelling and rejecting it is clearly wrong:

1. **JSX opening tags:** `type="button"` (attribute name).
2. **Member access after `.` or `?.`:** `element.type = "…"` (property name).

Everywhere else, `type` continues to start a type alias (or other keyword grammar as defined in [LANGUAGE.md](LANGUAGE.md)).

**Not a precedent** for “all JS reserved words as properties.” New collisions are handled **case by case** when they block obvious, idiomatic Tish that talks to JS APIs. Prefer renaming in Tish source **only** when the fix would sprawl (e.g. many keywords as members).

---

## 4. JS backend behavior vs Tish semantics

The JS emitter may add parentheses, `?? null`, or block braces to preserve Tish null semantics and valid ECMAScript. That is **lowering detail**, not a promise that Tish is becoming JS.

---

## 5. Summary

| Statement | Meaning |
|-----------|---------|
| Tish ≠ JS | Different language; JS is one output. |
| No infinite JS surface | We do not aim to accept every JS identifier or expression form inside Tish. |
| Obvious failures only | Wrong scope, wrong control flow, invalid emit from valid Tish, or a **documented** keyword collision with DOM/JSX. |
| Narrow exceptions | e.g. `type` in JSX attrs and after `.` / `?.`; not a blanket “keywords as members” rule. |

Related: [LANGUAGE.md](LANGUAGE.md) (semantics and keywords), [ecma-alignment.md](ecma-alignment.md) (spec mapping).
