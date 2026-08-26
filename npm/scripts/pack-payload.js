#!/usr/bin/env node
// Shared pack-time Rust payload handling for the npm packages that ship crate sources:
// @tishlang/tish, tish-format, tish-lint, tish-lsp.
//
// Why these packages ship Rust at all: tishlang_runtime_gba is not on crates.io, so a generated GBA
// project has to path-depend on the shipped crates; and the wrapper packages' `install-bin.js`
// falls back to `cargo build -p <crate>` inside the shipped workspace when no prebuilt binary fits
// the consumer's platform. That makes the tarball's workspace part of the public contract.
//
// This module is NOT shipped — it lives outside every package directory and only ever runs from a
// checkout, at `npm pack` / `npm publish` time.
'use strict';
const fs = require('fs');
const path = require('path');

// The four entries every payload-shipping package lists in `files`.
const PAYLOAD = ['Cargo.toml', 'crates', 'LICENSE', 'justfile'];

// An excluded crate keeps its target/ INSIDE the crate directory rather than in the workspace root,
// so crates/tish_runtime_gba/target exists on any machine that has built it — 913 files and 200 MB,
// silently added to the tarball by a plain recursive copy.
const skipBuildDirs = (src) => path.basename(src) !== 'target';

/**
 * Copy the payload from the repo root into `pkgDir`, trim the shipped workspace `members` list to
 * what actually ships, and stamp `version` into the package's own crate manifest.
 *
 * @param {object} opts
 * @param {string} opts.pkgDir      absolute path to the npm package directory
 * @param {string} opts.crate       crate under crates/ whose [package] version this package publishes
 * @param {string} [opts.version]   version to stamp; defaults to the package's package.json version
 */
function copyPayload({ pkgDir, crate, version }) {
  const root = path.resolve(pkgDir, '../..');
  const label = path.basename(pkgDir);

  for (const name of PAYLOAD) {
    // rmSync on a symlink removes the link, not its target — local dev may have symlinks here.
    fs.rmSync(path.join(pkgDir, name), { recursive: true, force: true });
    fs.cpSync(path.join(root, name), path.join(pkgDir, name),
      { recursive: true, dereference: true, filter: skipBuildDirs });
  }

  // postpack does not run when prepack throws, so undo the copy here — otherwise a failed pack
  // leaves an unbuildable half-payload in the working tree for whoever packs next.
  try {
    trimMembers(pkgDir, label);
    stampVersion(pkgDir, crate, version || require(path.join(pkgDir, 'package.json')).version, label);
  } catch (err) {
    removePayload(pkgDir);
    throw err;
  }

  console.log(`prepack(${label}): copied Rust payload from ${root}`);
}

// The root manifest lists examples/ffi/mathext as a member; these packages do not ship examples. A
// workspace root naming a member that is not there cannot be loaded at all, which made every crate
// inheriting `{ workspace = true }` unreadable for consumers — the bug that shipped in 3.9.2.
//
// Dropping anything OTHER than an example is a packaging mistake, not a normal trim: it means
// `files` stopped shipping a crate the workspace still names. Fail rather than quietly shrink the
// manifest, because once it is trimmed no downstream check can tell the crate was ever expected.
function trimMembers(pkgDir, label) {
  const manifest = path.join(pkgDir, 'Cargo.toml');
  const src = fs.readFileSync(manifest, 'utf8');
  const members = src.match(/^members\s*=\s*\[([\s\S]*?)\]/m);
  if (!members) return;

  const all = (members[1].match(/"([^"]+)"/g) || []).map((q) => q.slice(1, -1));
  const kept = all.filter((rel) => fs.existsSync(path.join(pkgDir, rel, 'Cargo.toml')));
  const dropped = all.filter((rel) => !kept.includes(rel));

  const unexpected = dropped.filter((rel) => !rel.startsWith('examples/'));
  if (unexpected.length) {
    throw new Error(
      `prepack(${label}): workspace members ${unexpected.join(', ')} are not in the package — ` +
      `add them to "files" in ${label}/package.json, or drop them from the root workspace`);
  }
  if (dropped.length) console.log(`prepack(${label}): trimmed non-shipped members: ${dropped.join(', ')}`);

  fs.writeFileSync(manifest,
    src.replace(members[0], `members = [\n${kept.map((k) => `    "${k}",`).join('\n')}\n]`));
}

// The published crate version must match the npm version. The release workflow stamps
// package.json before packing, and the copy above has just overwritten the crate manifest with the
// repo-root one (which carries the PREVIOUS release's version), so this has to happen after it.
function stampVersion(pkgDir, crate, version, label) {
  const manifest = path.join(pkgDir, 'crates', crate, 'Cargo.toml');
  const src = fs.readFileSync(manifest, 'utf8');
  const stamped = src.replace(/^version\s*=\s*".*"$/m, `version = "${version}"`);
  if (stamped === src) {
    throw new Error(`prepack(${label}): no [package] version line to stamp in crates/${crate}/Cargo.toml`);
  }
  fs.writeFileSync(manifest, stamped);
  console.log(`prepack(${label}): stamped crates/${crate} version ${version}`);
}

/**
 * Remove the copies copyPayload made, so packing leaves nothing behind.
 *
 * Local dev may have had symlinks here instead; `scripts/dev-setup.sh` in the chuggie repo
 * recreates them. Nothing about a release depends on them.
 */
function removePayload(pkgDir) {
  for (const name of PAYLOAD) {
    fs.rmSync(path.join(pkgDir, name), { recursive: true, force: true });
  }
  console.log(`postpack(${path.basename(pkgDir)}): removed the copied payload`);
}

module.exports = { PAYLOAD, copyPayload, removePayload };
