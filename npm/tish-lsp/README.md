# @tishlang/tish-lsp

The [Tish](https://github.com/tishlang/tish) language server (`tish-lsp`) as an npm package — prebuilt native binaries for macOS/Linux/Windows, with a cargo source-build fallback.

It speaks LSP over stdio and provides diagnostics (`tishlang_lint`), formatting (`tishlang_fmt`), completion, hover, go-to-definition, references, and rename.

```sh
npm i @tishlang/tish-lsp        # installs the prebuilt tish-lsp for your platform
```

Pin a specific version the same way you pin `@tishlang/tish`:

```jsonc
// package.json
"dependencies": { "@tishlang/tish-lsp": "2.0.3" }
```

The `tish-lsp` binary is exposed on `bin`, and the per-platform binaries live under `platform/<os>-<arch>/`.
