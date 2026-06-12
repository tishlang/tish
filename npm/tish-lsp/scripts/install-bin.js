#!/usr/bin/env node
'use strict';

/**
 * Copy platform/<os>-<arch>/tish-lsp to bin/tish-lsp so the npm bin is the native binary.
 * If not found, attempts to build it from source using cargo.
 */

const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

const platformKey = `${process.platform}-${process.arch}`;
const binaryName = process.platform === 'win32' ? 'tish-lsp.exe' : 'tish-lsp';
const crateName = 'tishlang_lsp';

const root = path.join(__dirname, '..');
const src = path.join(root, 'platform', platformKey, binaryName);
const dest = path.join(root, 'bin', 'tish-lsp');

fs.mkdirSync(path.dirname(dest), { recursive: true });

if (!fs.existsSync(src)) {
  if (process.env.TISH_NPM_PACK_REQUIRE === '1') {
    console.error(`[tish-lsp] No prebuilt binary for this platform: ${platformKey}`);
    console.error(`[tish-lsp] Expected: ${src}`);
    process.exit(1);
  }

  console.warn(`[tish-lsp] No prebuilt binary for ${platformKey} found at: ${src}`);
  console.warn(`[tish-lsp] Attempting to build from source via cargo...`);
  try {
    execSync(`cargo build --release -p ${crateName} --target-dir target`, { stdio: 'inherit', cwd: root });
    const builtSrc = path.join(root, 'target', 'release', binaryName);
    if (!fs.existsSync(builtSrc)) {
      throw new Error(`Expected built binary at ${builtSrc}`);
    }
    fs.copyFileSync(builtSrc, dest);
    console.log(`[tish-lsp] Successfully built from source.`);
  } catch (err) {
    console.error(`[tish-lsp] Failed to build from source.`);
    console.error(`[tish-lsp] Make sure Rust and Cargo are installed (https://rustup.rs/).`);
    console.error(err);
    process.exit(1);
  }
} else {
  fs.copyFileSync(src, dest);
}

try {
  fs.chmodSync(dest, 0o755);
} catch (_) {
  /* Windows may ignore chmod */
}
