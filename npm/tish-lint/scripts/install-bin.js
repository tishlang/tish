#!/usr/bin/env node
'use strict';

/**
 * Copy platform/<os>-<arch>/tish-lint to bin/tish-lint so the npm bin is the native binary.
 * If not found, attempts to build it from source using cargo.
 */

const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

const platformKey = `${process.platform}-${process.arch}`;
const binaryName = process.platform === 'win32' ? 'tish-lint.exe' : 'tish-lint';
const crateName = 'tishlang_lint';

const root = path.join(__dirname, '..');
const src = path.join(root, 'platform', platformKey, binaryName);
const dest = path.join(root, 'bin', 'tish-lint');

fs.mkdirSync(path.dirname(dest), { recursive: true });

if (!fs.existsSync(src)) {
  if (process.env.TISH_NPM_PACK_REQUIRE === '1') {
    console.error(`[tish-lint] No prebuilt binary for this platform: ${platformKey}`);
    console.error(`[tish-lint] Expected: ${src}`);
    process.exit(1);
  }
  
  console.warn(`[tish-lint] No prebuilt binary for ${platformKey} found at: ${src}`);
  console.warn(`[tish-lint] Attempting to build from source via cargo...`);
  try {
    execSync(`cargo build --release -p ${crateName} --target-dir target`, { stdio: 'inherit', cwd: root });
    const builtSrc = path.join(root, 'target', 'release', binaryName);
    if (!fs.existsSync(builtSrc)) {
      throw new Error(`Expected built binary at ${builtSrc}`);
    }
    fs.copyFileSync(builtSrc, dest);
    console.log(`[tish-lint] Successfully built from source.`);
  } catch (err) {
    console.error(`[tish-lint] Failed to build from source.`);
    console.error(`[tish-lint] Make sure Rust and Cargo are installed (https://rustup.rs/).`);
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
