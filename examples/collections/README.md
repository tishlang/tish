# Collections & Immutability

A tour of the standard collection types and the modern (ES2023) array surface. One program that runs
identically on the interpreter, the VM, the native backend, and transpiled JS.

## Features Used

- **`Set`** — de-duplication, `.add` / `.has` / `.size`, `for…of` iteration
- **`Map`** — keyed frequency counting, `.set` / `.get` / `.size`, entry iteration
- **`Object.freeze` / `Object.isFrozen`** — an immutable config that rejects writes (caught)
- **Modern arrays** — non-mutating `toSorted` and `with`, `Array.from(iterable, mapFn)`, and
  `structuredClone`

## Run

```
tish run src/main.tish
```
