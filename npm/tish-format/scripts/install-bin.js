#!/usr/bin/env node
'use strict';

/**
 * Copy platform/<os>-<arch>/tish-format to bin/tish-format so the npm bin is the native binary.
 * If not found, attempts to build it from source using cargo.
 */

const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

const platformKey = `${process.platform}-${process.arch}`;
const binaryName = process.platform === 'win32' ? 'tish-fmt.exe' : 'tish-fmt';
const crateName = 'tishlang_fmt';

const root = path.join(__dirname, '..');
const src = path.join(root, 'platform', platformKey, binaryName);
const dest = path.join(root, 'bin', 'tish-format');

fs.mkdirSync(path.dirname(dest), { recursive: true });

if (!fs.existsSync(src)) {
  if (process.env.TISH_NPM_PACK_REQUIRE === '1') {
    console.error(`[tish-format] No prebuilt binary for this platform: ${platformKey}`);
    console.error(`[tish-format] Expected: ${src}`);
    process.exit(1);
  }
  
  console.warn(`[tish-format] No prebuilt binary for ${platformKey} found at: ${src}`);
  console.warn(`[tish-format] Attempting to build from source via cargo...`);
  try {
    execSync(`cargo build --release -p ${crateName} --target-dir target`, { stdio: 'inherit', cwd: root });
    const builtSrc = path.join(root, 'target', 'release', binaryName);
    if (!fs.existsSync(builtSrc)) {
      throw new Error(`Expected built binary at ${builtSrc}`);
    }
    fs.copyFileSync(builtSrc, dest);
    console.log(`[tish-format] Successfully built from source.`);
  } catch (err) {
    console.error(`[tish-format] Failed to build from source.`);
    console.error(`[tish-format] Make sure Rust and Cargo are installed (https://rustup.rs/).`);
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
