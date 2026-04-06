# Tish

<p>
  <a href="https://npmjs.com/package/@tishlang/tish?activeTab=readme"><img src="https://img.shields.io/npm/v/@tishlang/tish?style=flat-square&colorA=1C1C1C&colorB=B688FF" alt="npm version" /></a>
  <a href="https://npmcharts.com/compare/@tishlang/tish"><img src="https://img.shields.io/npm/dm/@tishlang/tish.svg?style=flat-square&colorA=1C1C1C&colorB=B688FF" alt="downloads" /></a>
  <a href="https://nodejs.org/en/about/previous-releases"><img src="https://img.shields.io/node/v/@tishlang/tish.svg?style=flat-square&colorA=1C1C1C&colorB=B688FF" alt="node version"></a>
  <a href="https://crates.io/crates/tishlang"><img src="https://img.shields.io/crates/v/tishlang?style=flat-square&colorA=1C1C1C&colorB=B688FF" alt="crate version" /></a>
  <a href="https://github.com/tishlang/tish/blob/main/LICENSE"><img src="https://img.shields.io/badge/License-PIF-blue.svg?style=flat-square&colorA=1C1C1C&colorB=B688FF" alt="license" /></a>

</p>

Tish is a TypeScript- and JavaScript-compatible language implemented in Rust. It is aimed at teams who want a familiar surface syntax, predictable semantics, and the option to ship either interpreted scripts or **native binaries**.

The same source can run on a tree-walking interpreter, a bytecode VM, or compiled targets (native, WASM/WASI, and others). Network, filesystem, and process APIs are **feature-gated** so defaults stay safe. For the full syntax and semantics, see the canonical spec in-repo; for tutorials and reference, see [tishlang.com/docs](https://tishlang.com/docs).

## 🔥 Features

- **JS-like surface** — `let` / `const`, `fn`, arrows, template literals, async/await, modules, and a large slice of familiar builtins (`console`, `Math`, `JSON`, arrays, strings, objects).
- **Two ways to run** — interpret for fast iteration (`tish run`, REPL); compile to native or WASM for distribution and performance (`tish build`).
- **Memory-safe implementation** — Rust-hosted runtime and compiler pipeline; no GC in the host language for the toolchain itself.
- **Secure by default** — I/O and platform APIs (`http`, `fs`, `process`, etc.) are opt-in via features.
- **No `undefined`** — `null` only where JS would use undefined; strict equality (`===` / `!==`) without loose coercion.
- **Optional types** — TypeScript-style annotations are parsed for tooling and future checking; see the language reference for status.

Full specification: [docs/LANGUAGE.md](docs/LANGUAGE.md). Implementation status, gaps, and JS compatibility: [docs/plan-gap-analysis.md](docs/plan-gap-analysis.md).

## 📚 Documentation

User-facing guides and reference:

| Section | Description |
|---------|-------------|
| [Getting started](https://tishlang.com/docs/getting-started/installation/) | Install Tish, build your first app |
| [First app](https://tishlang.com/docs/getting-started/first-app/) | Run and build workflows, targets |
| [Editor & IDE](https://tishlang.com/docs/getting-started/editor/) | VS Code, LSP, tasks, Neovim |
| [Language server](https://tishlang.com/docs/reference/language-server/) | `tish-lsp` capabilities |
| [Formatting](https://tishlang.com/docs/reference/formatting/) | `tish-fmt` |
| [Linting](https://tishlang.com/docs/reference/linting/) | `tish-lint` |
| [Interactive REPL](https://tishlang.com/docs/getting-started/repl/) | Multi-line input, completion, history |
| [Language overview](https://tishlang.com/docs/language/overview/) | Syntax, keywords, semantics |
| [Tish vs JavaScript](https://tishlang.com/docs/language/vs-javascript/) | Differences and additions from JS |
| [Builtins](https://tishlang.com/docs/builtins/overview/) | Console, Math, JSON, Array, String, Object |
| [Features (APIs)](https://tishlang.com/docs/features/http/) | `http`, `fs`, `process`, `regex` — feature-gated |
| [Native backend](https://tishlang.com/docs/reference/native-backend/) | Rust, Cranelift, LLVM compilation |
| [WASM targets](https://tishlang.com/docs/reference/wasm-targets/) | Web and WASI |
| [Deploy](https://tishlang.com/docs/deploy/overview/) | Platform and hosting |

### In-repo docs

Contributor- and spec-oriented material in [docs/](docs/):

| File | Purpose |
|------|---------|
| [LANGUAGE.md](docs/LANGUAGE.md) | Canonical language reference (syntax, semantics, builtins) |
| [ecma-alignment.md](docs/ecma-alignment.md) | ECMA-262 / test262 mapping |
| [plan-gap-analysis.md](docs/plan-gap-analysis.md) | Implementation audit, MVP checklist |
| [architecture-next-steps.md](docs/architecture-next-steps.md) | Crate layout, design decisions |
| [builtins-gap-analysis.md](docs/builtins-gap-analysis.md) | Builtins across Rust vs bytecode VM (Cranelift/WASI) |

## 🛠️ Toolchain

| Tool | Purpose |
|------|---------|
| **`tish`** | CLI — `run`, `repl`, `build`, `dump-ast` |
| **`tish-fmt`** | Formatter |
| **`tish-lint`** | Linter |
| **`tish-lsp`** | Language server; uses `tish_fmt` / `tish_lint` as libraries |
| **VS Code extension** | [tish-vscode](https://github.com/tishlang/tish-vscode) — grammar, snippets, LSP client, tasks |

Related docs on [tishlang.com](https://tishlang.com/docs): [Editor & IDE](https://tishlang.com/docs/getting-started/editor/), [Language server](https://tishlang.com/docs/reference/language-server/), [Formatting](https://tishlang.com/docs/reference/formatting/), [Linting](https://tishlang.com/docs/reference/linting/).

## 📦 Installation

Install globally:

```sh
brew tap tishlang/tish https://github.com/tishlang/tish
brew install tish
```

Or locally with npm:

```sh
npm install @tishlang/tish
```

More options: [Installation](https://tishlang.com/docs/getting-started/installation/).

## ⚡ Quick start

```sh
npx @tishlang/create-tish-app my-app
cd my-app
npx @tishlang/tish run src/main.tish
```

## ▶️ Run and build

```tish
// hello.tish
fn greeting(name) = `Hello, ${name}!`
console.log(greeting("World"))
```

```bash
tish run hello.tish
# Hello, World!

tish build hello.tish -o hello
./hello
# Hello, World!
```

Native binaries are standalone (no Tish or Rust runtime required on the machine that runs them). Backends, flags, and WASM are covered in [First app](https://tishlang.com/docs/getting-started/first-app/), [Native backend](https://tishlang.com/docs/reference/native-backend/), and [WASM targets](https://tishlang.com/docs/reference/wasm-targets/).

## 🤝 Contribution

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for building, testing, and code style. Tish is licensed under the [Pay It Forward License (PIF)](https://payitforwardlicense.com/).

## 💪 Performance

JavaScript equivalents live in `tests/core/*.js`. Compare Tish with Node.js or Bun:

```bash
./scripts/run_performance_manual.sh
```

Details: [docs/perf.md](docs/perf.md).

## 📝 License

Tish is licensed under the [Pay It Forward License (PIF)](https://payitforwardlicense.com/). See [LICENSE](LICENSE).
