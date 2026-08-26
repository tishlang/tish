#!/usr/bin/env node
// Copy the Rust payload into the package before packing.
//
// The package ships crate sources because tishlang_runtime_gba is not on crates.io yet, so a
// generated GBA project has to path-depend on them. They used to arrive as symlinks that are not
// tracked in git, so CI packed a tarball with ZERO crates while a developer's machine packed 33.
// Copy from the repo root, which is tracked, so the tarball is the same everywhere.
'use strict';
const fs = require('fs');
const path = require('path');

const here = path.resolve(__dirname, '..');
const root = path.resolve(here, '../..');

// Skip build artifacts. An excluded crate keeps its target/ INSIDE the crate directory rather than
// in the workspace root, so crates/tish_runtime_gba/target exists on any machine that has built it
// — 913 files and 200 MB, silently added to the tarball by a plain recursive copy.
const skipBuildDirs = (src) => path.basename(src) !== 'target';

for (const name of ['Cargo.toml', 'crates', 'LICENSE', 'justfile']) {
  fs.rmSync(path.join(here, name), { recursive: true, force: true });
  fs.cpSync(path.join(root, name), path.join(here, name),
    { recursive: true, dereference: true, filter: skipBuildDirs });
}

// The root manifest lists examples/ffi/mathext as a member; the package does not ship examples. A
// workspace root naming a member that is not there cannot be loaded at all, which made every crate
// inheriting `{ workspace = true }` unreadable for consumers.
const manifest = path.join(here, 'Cargo.toml');
const src = fs.readFileSync(manifest, 'utf8');
const members = src.match(/^members\s*=\s*\[([\s\S]*?)\]/m);
if (members) {
  const kept = (members[1].match(/"([^"]+)"/g) || [])
    .map((q) => q.slice(1, -1))
    .filter((rel) => fs.existsSync(path.join(here, rel, 'Cargo.toml')));
  fs.writeFileSync(manifest,
    src.replace(members[0], `members = [\n${kept.map((k) => `    "${k}",`).join('\n')}\n]`));
}
console.log(`prepack: copied Rust payload from ${root}`);
