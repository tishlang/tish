# Typed Structs

Model data with `interface` types and process a typed array of structs. Demonstrates that type
annotations are a **performance lever, not a different language**: the same source runs on the
interpreter, the VM, transpiled JS, and the native backend.

## Features Used

- `interface` type declarations (including a nested struct type)
- A typed array of structs (`Body[]`)
- Typed function parameters and return types (`function momentum(b: Body): number`)
- In-place struct-field mutation (`bodies[1].vel = …`)
- Sorting a typed struct array by a field
- Nested struct-field access (`bodies[i].pos.x`)

## Why it matters

On the native (tish→Rust) backend these `interface` types lower to real Rust `struct`s and the array
to a `Vec<struct>`, so field reads/writes and arithmetic compile to plain machine operations with no
boxing. On the interpreter / VM the identical code runs as ordinary objects — one program, every backend.

## Run

```
tish run src/main.tish
```
